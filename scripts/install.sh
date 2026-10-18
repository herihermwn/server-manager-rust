#!/usr/bin/env bash
# Install or upgrade Server Manager on this server. Run it from a clone of the repo, as your normal
# sudo user (not root):   scripts/install.sh
# Re-running it is an upgrade: binary, frontend and unit are replaced, data and apps are kept.
#
# What it does:
#   1. cargo build --release + web build (skip with SKIP_BUILD=1 to use target/ and web/dist as they are)
#   2. binary   -> $PREFIX/bin/server-manager            (default /usr/local/bin/server-manager)
#      frontend -> $PREFIX/share/server-manager/web      (default /usr/local/share/server-manager/web)
#   3. creates, only if missing: data dir $DATA (root, 700), apps dir $APPS (owned by $RUN_AS),
#      nginx site dir $NGINX_DIR (when its parent exists)
#   4. writes /etc/systemd/system/$SERVICE.service from deploy/server-manager.service, enables it,
#      restarts it and checks that it answers on 127.0.0.1:$PORT
# It never edits nginx.conf or PostgreSQL: it checks them and prints the commands to run.
#
# Overrides (environment variables), with their defaults:
#   PREFIX=/usr/local  DATA=/var/lib/server-manager  APPS=$HOME/apps  RUN_AS=$(id -un)
#   SERVICE=server-manager  PORT=3900  NGINX_DIR=/etc/nginx/sites-manager  PUBLIC_IP=(detected by the app)
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(id -u)" -eq 0 ]; then
  echo "Run this as your normal sudo user, not root (it builds with your ~/.cargo and ~/.bun)." >&2
  exit 1
fi

PREFIX="${PREFIX:-/usr/local}"
DATA="${DATA:-/var/lib/server-manager}"
APPS="${APPS:-$HOME/apps}"
RUN_AS="${RUN_AS:-$(id -un)}"
SERVICE="${SERVICE:-server-manager}"
PORT="${PORT:-3900}"
NGINX_DIR="${NGINX_DIR:-/etc/nginx/sites-manager}"
PUBLIC_IP="${PUBLIC_IP:-}"
BIN="$PREFIX/bin/server-manager"
WEB="$PREFIX/share/server-manager/web"
UNIT="/etc/systemd/system/$SERVICE.service"

id "$RUN_AS" >/dev/null 2>&1 || { echo "RUN_AS: no user '$RUN_AS'" >&2; exit 1; }

if [ -z "${SKIP_BUILD:-}" ]; then
  echo "==> build"
  export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:$PATH"
  cargo build --release
  (cd web && bun run build)
fi
[ -x target/release/server-manager ] || { echo "target/release/server-manager is missing: build first" >&2; exit 1; }
[ -f web/dist/index.html ] || { echo "web/dist/index.html is missing: run (cd web && bun run build)" >&2; exit 1; }

echo "==> install $BIN and $WEB"
sudo install -d -m 755 "$PREFIX/bin" "$(dirname "$WEB")"
# copy then rename: never overwrite the running binary in place
sudo install -m 755 target/release/server-manager "$BIN.new"
sudo mv -f "$BIN.new" "$BIN"
sudo rm -rf "$WEB.new"
sudo cp -R web/dist "$WEB.new"
sudo chmod -R a+rX "$WEB.new"
sudo rm -rf "$WEB"
sudo mv "$WEB.new" "$WEB"

echo "==> folders"
sudo install -d -m 700 -o root -g root "$DATA"
if [ ! -d "$APPS" ]; then
  sudo install -d -m 755 -o "$RUN_AS" -g "$(id -gn "$RUN_AS")" "$APPS"
  echo "created $APPS"
fi
if [ ! -d "$NGINX_DIR" ] && [ -d "$(dirname "$NGINX_DIR")" ]; then
  sudo install -d -m 755 "$NGINX_DIR"
  echo "created $NGINX_DIR"
fi

echo "==> $UNIT"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
sed -e "s|@BIN@|$BIN|g" -e "s|@WEB@|$WEB|g" -e "s|@DATA@|$DATA|g" -e "s|@APPS@|$APPS|g" \
    -e "s|@PORT@|$PORT|g" -e "s|@RUN_AS@|$RUN_AS|g" -e "s|@NGINX_DIR@|$NGINX_DIR|g" \
    -e "s|@PUBLIC_IP@|$PUBLIC_IP|g" deploy/server-manager.service >"$tmp"
sudo install -m 644 "$tmp" "$UNIT"
sudo systemctl daemon-reload
sudo systemctl enable --quiet "$SERVICE"
sudo systemctl restart "$SERVICE"

code=000
for _ in $(seq 1 20); do
  code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/" || true)"
  [ "$code" = 200 ] && break
  sleep 0.5
done
if [ "$code" != 200 ]; then
  echo "$SERVICE did not answer on 127.0.0.1:$PORT. Logs: sudo journalctl -u $SERVICE -n 50" >&2
  exit 1
fi
echo "$SERVICE is running on http://127.0.0.1:$PORT"

echo "==> checks"
for c in git nginx certbot pg_dump setfacl; do
  command -v "$c" >/dev/null || echo "missing: $c (see README, Requirements)"
done
if [ -f /etc/nginx/nginx.conf ] && ! grep -qF "include $NGINX_DIR/*.conf;" /etc/nginx/nginx.conf; then
  echo "nginx: sites made in the dashboard are not loaded yet. Add this line inside http { } of"
  echo "       /etc/nginx/nginx.conf, after 'include /etc/nginx/sites-enabled/*;':"
  echo "         include $NGINX_DIR/*.conf;"
  echo "       then: sudo nginx -t && sudo systemctl reload nginx"
fi
if ! sudo test -f "$DATA/pg.env"; then
  echo "postgres: no $DATA/pg.env yet, so the Databases page is off (README, PostgreSQL)."
fi

cat <<EOF

Next:
  1. Create the owner account (asks for a password, min 10 characters):
       sudo env SM_DATA_DIR=$DATA $BIN create-admin admin
  2. On your computer, open a tunnel and browse to http://localhost:$PORT
       ssh -L $PORT:127.0.0.1:$PORT $(id -un)@<this-server>
EOF
