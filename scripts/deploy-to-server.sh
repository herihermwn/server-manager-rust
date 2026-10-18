#!/usr/bin/env bash
# Build Server Manager ON the server and (re)start it. Run from your Mac: scripts/deploy-to-server.sh
#   1. rsync this repo to server-43:~/server-manager-src (no target/, node_modules/, web/dist/)
#   2. cargo build --release + web build there
#   3. install bin to ~/server-manager/bin, web/dist to ~/server-manager/web/dist
#   4. install deploy/server-manager.service as a systemd SYSTEM unit and restart it
# Then open it through the tunnel: ssh -L 3900:127.0.0.1:3900 server-43 → http://localhost:3900
# First time only, on the server: ~/server-manager/bin/server-manager create-admin <username>
set -euo pipefail
cd "$(dirname "$0")/.."

HOST="${DEPLOY_HOST:-server-43}"

echo "==> sync source to $HOST:~/server-manager-src"
rsync -az --delete \
  --exclude .git --exclude target --exclude web/node_modules --exclude web/dist --exclude web/dashboard/styles.css \
  ./ "$HOST:server-manager-src/"

echo "==> build + install on $HOST"
# bash -l so ~/.cargo/bin and ~/.bun/bin are on PATH.
ssh "$HOST" 'bash -lc "
  set -euo pipefail
  cd ~/server-manager-src
  cargo build --release
  (cd web && bun install && bun run build)

  mkdir -p ~/server-manager/bin ~/server-manager/web
  # copy then rename: never overwrite the running binary in place
  cp target/release/server-manager ~/server-manager/bin/server-manager.new
  mv -f ~/server-manager/bin/server-manager.new ~/server-manager/bin/server-manager
  rsync -a --delete web/dist/ ~/server-manager/web/dist/

  sudo install -m 644 deploy/server-manager.service /etc/systemd/system/server-manager.service
  sudo systemctl daemon-reload
  sudo systemctl enable server-manager.service
  sudo systemctl restart server-manager.service
  sleep 2
  systemctl is-active server-manager.service
  curl -fsS -o /dev/null -w \"SPA %{http_code}\n\" http://127.0.0.1:3900/
  curl -sS -o /dev/null -w \"API (expect 401) %{http_code}\n\" http://127.0.0.1:3900/api/auth/me
"'
echo "==> done. Tunnel: ssh -L 3900:127.0.0.1:3900 $HOST"
