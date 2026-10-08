#!/usr/bin/env bash
# Usage: install-supervisor.sh
#   Symlinks systemd/quaqua-supervisor.{service,timer} into
#   ~/.config/systemd/user/ and reloads the user manager. Does NOT enable or
#   start the timer — do that yourself:
#     systemctl --user enable --now quaqua-supervisor.timer
set -euo pipefail

src="$(cd "$(dirname "$0")/../systemd" && pwd)"
dst="$HOME/.config/systemd/user"
mkdir -p "$dst"
for unit in quaqua-supervisor.service quaqua-supervisor.timer; do
  ln -sfn "$src/$unit" "$dst/$unit"
done
systemctl --user daemon-reload
echo "installed into $dst — enable with: systemctl --user enable --now quaqua-supervisor.timer"
