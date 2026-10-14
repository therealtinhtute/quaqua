#!/usr/bin/env bash
# Usage: install-auditor.sh
#   Linux: symlink systemd units and reload the user manager (no enable/start).
#   macOS: render a launchd plist in ~/Library/LaunchAgents (no manager calls).
#   Prints the platform's activation command for the user to run manually.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
case $(uname -s) in
  Darwin)
    bash_bin=$(command -v bash) \
      || { echo 'error: bash must be on PATH before installing the auditor' >&2; exit 1; }
    if [[ ! -x $bash_bin ]] || ! "$bash_bin" -c \
      '(( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) ))'; then
      echo 'error: auditor requires Bash >= 4.4 on PATH (macOS /bin/bash 3.2 is too old)' >&2
      exit 1
    fi
    bash_bin="$(cd "$(dirname "$bash_bin")" && pwd -P)/$(basename "$bash_bin")"
    codex=$(command -v codex) \
      || { echo 'error: codex must be on PATH before installing the auditor' >&2; exit 1; }
    codex="$(cd "$(dirname "$codex")" && pwd -P)/$(basename "$codex")"
    # launchd does not source shell profiles. Keep the install-time toolchain
    # (e.g. node/mise) as well as Bash and the directory containing codex.
    job_path="$(dirname "$bash_bin"):$(dirname "$codex"):${PATH:-/usr/bin:/bin}:/usr/bin:/bin:/usr/sbin:/sbin"
    dst="$HOME/Library/LaunchAgents"
    plist="$dst/com.quaqua.auditor.plist"
    mkdir -p "$dst" "$HOME/Library/Logs"
    # Escape XML text first, then escape sed's replacement metacharacters.
    # This preserves paths with spaces, &, quotes, and the chosen delimiter.
    xml_replacement() {
      printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' \
        -e 's/>/\&gt;/g' -e 's/"/\&quot;/g' -e "s/'/\&apos;/g" \
        -e 's/[\\&|]/\\&/g'
    }
    tmp=$(mktemp "$dst/.com.quaqua.auditor.XXXXXX")
    trap 'rm -f "$tmp"' EXIT
    sed -e "s|@ROOT@|$(xml_replacement "$root")|g" \
        -e "s|@HOME@|$(xml_replacement "$HOME")|g" \
        -e "s|@BASH@|$(xml_replacement "$bash_bin")|g" \
        -e "s|@PATH@|$(xml_replacement "$job_path")|g" \
      "$root/scripts/auditor/launchd/com.quaqua.auditor.plist.in" > "$tmp"
    chmod 644 "$tmp"
    mv -f "$tmp" "$plist"
    # %q also keeps the printed command safe for paths containing spaces.
    printf 'installed %s — activate manually with:\n' "$plist"
    printf 'launchctl bootstrap gui/%s %q\n' "$(id -u)" "$plist"
    ;;
  Linux)
    src="$root/scripts/auditor/systemd"
    dst="$HOME/.config/systemd/user"
    mkdir -p "$dst"
    for unit in quaqua-auditor.service quaqua-auditor.timer; do
      ln -sfn "$src/$unit" "$dst/$unit"
    done
    systemctl --user daemon-reload
    echo "installed into $dst — enable with: systemctl --user enable --now quaqua-auditor.timer"
    ;;
  *) echo 'error: unsupported OS (expected Darwin or Linux)' >&2; exit 1 ;;
esac
