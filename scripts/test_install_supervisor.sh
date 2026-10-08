#!/usr/bin/env bash
# Sandbox installer test: no real user service manager, codex, or HOME touched.
# Requires python3 to parse the generated plist independently of the renderer.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/test-install-supervisor.XXXXXX")
T=$(cd "$T" && pwd -P)
trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

root="$T/repo & <mac> \"quoted\" | back\\slash"
mkdir -p "$root/scripts" "$T/bin" "$T/codex & tools" "$T/home"
cp "$here/install-supervisor.sh" "$root/scripts/"
cp -R "$here/../systemd" "$root/"
if [[ -d $here/../launchd ]]; then cp -R "$here/../launchd" "$root/"; fi
ln -s "$BASH" "$T/bin/bash"  # proves the installer discovers, rather than hardcodes, Bash
cat > "$T/bin/uname" <<'EOF'
#!/bin/sh
printf '%s\n' "$TEST_OS"
EOF
cat > "$T/bin/systemctl" <<'EOF'
#!/bin/sh
printf 'systemctl %s\n' "$*" >> "$CALLS"
EOF
cat > "$T/bin/launchctl" <<'EOF'
#!/bin/sh
printf 'FORBIDDEN launchctl %s\n' "$*" >> "$CALLS"
exit 99
EOF
cat > "$T/codex & tools/codex" <<'EOF'
#!/bin/sh
printf 'FORBIDDEN codex %s\n' "$*" >> "$CALLS"
exit 99
EOF
chmod +x "$T/bin/uname" "$T/bin/systemctl" "$T/bin/launchctl" "$T/codex & tools/codex"
run() {
  : > "$T/calls"
  rc=0
  TEST_OS=$1 CALLS="$T/calls" HOME="${TEST_HOME:-$T/home}" \
    PATH="$T/bin:$T/codex & tools:$PATH" \
    "$BASH" "$root/scripts/install-supervisor.sh" > "$T/out" 2> "$T/err" || rc=$?
}

run Linux
[[ $rc -eq 0 ]] || fail "Linux: exit $rc: $(cat "$T/err")"
for unit in quaqua-supervisor.service quaqua-supervisor.timer; do
  dst="$T/home/.config/systemd/user/$unit"
  [[ -L $dst && $(readlink "$dst") == "$root/systemd/$unit" ]] \
    || fail "Linux: wrong unit symlink: $unit"
done
[[ $(cat "$T/calls") == 'systemctl --user daemon-reload' ]] \
  || fail "Linux: unexpected manager calls: $(cat "$T/calls")"
grep -q 'systemctl --user enable --now quaqua-supervisor.timer' "$T/out" \
  || fail "Linux: no manual enable instruction"
echo 'ok: Linux -> existing systemd symlinks + daemon-reload, no enable/start'

run Darwin
[[ $rc -eq 0 ]] || fail "Darwin: exit $rc: $(cat "$T/err")"
plist="$T/home/Library/LaunchAgents/com.quaqua.supervisor.plist"
[[ -f $plist ]] || fail 'Darwin: no LaunchAgents plist'
[[ -d $T/home/Library/Logs ]] || fail 'Darwin: log directory not created'
[[ ! -s $T/calls ]] || fail "Darwin: invoked a manager or codex: $(cat "$T/calls")"
python3 - "$plist" "$root" "$T/home" "$T/codex & tools" "$T/bin/bash" <<'PY'
import pathlib, plistlib, sys
plist, root, home, codex_dir, bash_bin = sys.argv[1:]
with open(plist, 'rb') as f:
    p = plistlib.load(f)
assert p['Label'] == 'com.quaqua.supervisor', p
assert p['ProgramArguments'] == [bash_bin, root + '/scripts/supervisor.sh'], p
assert all(pathlib.Path(v).is_absolute() for v in p['ProgramArguments']), p
assert p['StartCalendarInterval'] == [{'Hour': 7, 'Minute': 0}, {'Hour': 19, 'Minute': 0}], p
paths = p['EnvironmentVariables']['PATH'].split(':')
assert str(pathlib.Path(bash_bin).parent) in paths and codex_dir in paths, paths
assert '/usr/bin' in paths and '/bin' in paths, paths
for k in ['StandardOutPath', 'StandardErrorPath']:
    assert p[k] == home + '/Library/Logs/quaqua-supervisor.log', p
    assert pathlib.Path(p[k]).is_absolute(), p
assert not p.get('RunAtLoad') and not p.get('KeepAlive'), p
print('ok: Darwin -> valid XML, absolute pinned argv, PATH, 07:00/19:00, external logs')
PY
grep -qF 'launchctl bootstrap' "$T/out" || fail 'Darwin: no manual bootstrap instruction'
grep -qF "$plist" "$T/out" || fail 'Darwin: manual bootstrap names the installed plist'
cp "$plist" "$T/first.plist"
run Darwin
cmp "$plist" "$T/first.plist" || fail 'Darwin: reinstall changed plist'
[[ ! -s $T/calls ]] || fail 'Darwin: reinstall called a manager'
echo 'ok: Darwin -> idempotent file-only install; bootstrap printed, never executed'

run FreeBSD
[[ $rc -eq 1 ]] || fail "unsupported OS: exit $rc, want 1"
grep -qi 'unsupported' "$T/err" || fail 'unsupported OS: no diagnostic'
[[ ! -s $T/calls ]] || fail 'unsupported OS: called a manager'
echo 'ok: unsupported OS -> clear refusal without manager calls'

# Refuse an old Bash before installing anything; the test harness still uses
# its own modern $BASH. This stand-in models a failing >=4.4 version probe.
rm "$T/bin/bash"
cat > "$T/bin/bash" <<'EOF'
#!/bin/sh
exit 1
EOF
chmod +x "$T/bin/bash"
TEST_HOME="$T/old-home" run Darwin
[[ $rc -eq 1 ]] || fail "old Bash: exit $rc, want 1"
grep -q 'Bash >= 4.4' "$T/err" || fail 'old Bash: no version diagnostic'
[[ ! -d $T/old-home && ! -s $T/calls ]] || fail 'old Bash: installed or activated anyway'
echo 'ok: Darwin -> refuses Bash below 4.4 before writing or calling a manager'

if [[ $(/bin/bash -c 'printf "%s" "$BASH_VERSION"') == 3.* ]]; then
  rm "$T/bin/bash"
  ln -s /bin/bash "$T/bin/bash"
  TEST_HOME="$T/system-bash-home" run Darwin
  [[ $rc -eq 1 ]] || fail "system Bash 3.2: exit $rc, want 1"
  grep -q 'Bash >= 4.4' "$T/err" || fail 'system Bash 3.2: no version diagnostic'
  [[ ! -d $T/system-bash-home && ! -s $T/calls ]] || fail 'system Bash 3.2: installed anyway'
  echo 'ok: Darwin -> real macOS system Bash 3.2 refused'
fi

# No codex on the install PATH: checked before any destination is created.
rm "$T/bin/bash" "$T/codex & tools/codex"
ln -s "$BASH" "$T/bin/bash"
# Only essential native tools are visible; no user's codex can leak in.
rc=0
TEST_OS=Darwin CALLS="$T/calls" HOME="$T/missing-codex-home" \
  PATH="$T/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
  "$BASH" "$root/scripts/install-supervisor.sh" > "$T/out" 2> "$T/err" || rc=$?
[[ $rc -eq 1 ]] || fail "missing codex: exit $rc, want 1"
grep -q 'codex must be on PATH' "$T/err" || fail 'missing codex: no diagnostic'
[[ ! -d $T/missing-codex-home ]] || fail 'missing codex: installed anyway'
echo 'ok: Darwin -> requires codex on install PATH before writing files'
echo PASS
