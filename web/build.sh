#!/usr/bin/env bash
# Build the dashboard into web/dist (served at / by the Rust binary, see src/web.rs).
# Asset URLs are relative (./index-*.js), so the same build works at / and under a prefix.
set -euo pipefail
cd "$(dirname "$0")"

if [ -f bun.lock ]; then bun install --frozen-lockfile; else bun install; fi

rm -rf dist
bunx tailwindcss -i dashboard/index.css -o dashboard/styles.css --minify
bun build ./dashboard/index.html --outdir dist --production --public-path ./
# Theme boot runs before first paint, so it stays a classic <head> script (Bun would fold it into the deferred module).
cp dashboard/theme.js dist/theme.js
sed -i.bak 's|<head>|<head><script src="./theme.js"></script>|' dist/index.html && rm dist/index.html.bak
