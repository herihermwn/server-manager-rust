# server-manager

A small web dashboard for running your apps on **one** Ubuntu server: deploy from git, edit `.env`,
create PostgreSQL databases, set up nginx sites with Let's Encrypt, and watch CPU, memory and disk.
One Rust binary (axum + SQLite) that also serves its React frontend.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B%20%C2%B7%202024%20edition-orange.svg)](Cargo.toml)

## What it does

- **Deploy from git.** Clone a repo (SSH with a server deploy key, or HTTPS), build it on the
  server, and switch to the new release atomically. Every deploy gets its own `releases/<n>` folder,
  a live log, a health check, and an automatic rollback if the health check fails. Manual rollback
  to any kept release. Optional auto-deploy from a GitHub push webhook.
- **Run apps as services.** Each app is a systemd unit (`app-<name>.service`) that runs as the
  Linux user you pick. Start, stop, restart, process stats, `journalctl` log stream, crash-loop
  detection.
- **Edit `.env`.** A key/value editor for each app's `shared/.env` (mode 600). Values of keys that
  look secret (`*SECRET*`, `*KEY*`, `*TOKEN*`, `*PASSWORD*`, `*DSN*`, `DATABASE_URL`) are masked
  in the browser and in deploy logs; revealing one is written to the activity log. Paste a whole
  `.env` and review the diff before saving.
- **PostgreSQL.** Create a database with its own owner role (password shown once), add the
  `pg_trgm`, `uuid-ossp` or `unaccent` extensions, link it to an app (writes `DATABASE_URL` into
  the app's `.env`), daily `pg_dump` backups (02:00 WIB, newest 7 kept) plus manual ones, and a
  dump before every drop. It only ever touches databases it created.
- **nginx + HTTPS.** Proxy or static sites from a form (aliases, upload limit, websockets, gzip,
  custom directives). Every change runs `nginx -t` first and puts the old files back if the test
  or the reload fails. Certificates from Let's Encrypt through certbot's webroot mode, after
  checking that the domain's A record points to the server.
- **Server usage.** CPU, memory, swap, disk, network and load, sampled every 10 s (kept 24 h) and
  averaged per 5 min (kept 30 days); top processes; disk use per app folder; status of nginx,
  PostgreSQL, the certbot timer and the manager itself.
- **Settings.** Rotate the deploy key without downtime, store an HTTPS token for private GitHub
  repos, rotate the webhook secret, set the port range for new apps and the alert thresholds
  (disk, CPU, memory). Alerts are banners in the dashboard; there are no email or chat
  notifications. Deploys refuse to start when the disk is fuller than the stop threshold (default 95 %).
- **Activity log** of every write action, with who did it and when.

Measured on a 2 vCPU / 3.6 GB Ubuntu 24.04 server: the manager process uses about 29 MB RSS
(sampler running, no deploy in progress).

### What it is not

- **Not multi-server.** It manages the machine it runs on, nothing else.
- **Not Docker or Kubernetes.** Apps are plain processes under systemd; builds run directly on the
  server (`nice -n 10`, with a per-app timeout, default 15 min).
- **No roles or teams.** `create-admin` can make more than one account, but every account can do
  everything, and that includes running commands as root (see [Security model](#security-model)).
- **Not a general nginx editor.** It writes only its own files in `/etc/nginx/sites-manager/`
  and leaves the rest of your nginx config alone.
- **Times are WIB.** The UI shows times in UTC+7 (Asia/Jakarta) and the daily backup runs at
  02:00 WIB, whatever the server's own time zone is. This is fixed in the code.

## Requirements

- Ubuntu 24.04 (other systemd-based Debian/Ubuntu versions will probably work but are untested).
- A user with sudo. The installer runs as that user, not as root.
- To build: `git`, `build-essential`, [Rust](https://rustup.rs) 1.88 or newer and
  [bun](https://bun.sh) 1.x in that user's home (`~/.cargo/bin`, `~/.bun/bin`).
- At runtime, for the pages that need them: `nginx`, `certbot` and `acl` (Web server page; `acl`
  lets nginx read static folders), `postgresql` (Databases page), `git` (deploys). The rest of the
  dashboard works without them.
- The toolchains your apps need (for example cargo or bun) installed for the user each app runs
  as. App units get `~/.bun/bin`, `~/.cargo/bin` and `~/.local/bin` of that user on their `PATH`.

## Install

On a fresh Ubuntu 24.04 server, logged in as your sudo user:

```bash
# 1. System packages
sudo apt update
sudo apt install -y git build-essential curl unzip nginx postgresql certbot acl

# 2. Build toolchains for your user (official installers from rustup.rs and bun.sh)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
curl -fsSL https://bun.sh/install | bash

# 3. Get the code and install
git clone https://github.com/herihermwn/server-manager-rust.git
cd server-manager-rust
scripts/install.sh
```

[`scripts/install.sh`](scripts/install.sh) is short; read it before running it. It:

1. builds the binary (`cargo build --release`) and the frontend (`web/build.sh`);
2. installs them to `/usr/local/bin/server-manager` and `/usr/local/share/server-manager/web`;
3. creates `/var/lib/server-manager` (data, root only, mode 700), `~/apps` (your apps, owned by
   you) and `/etc/nginx/sites-manager`, each only if it does not exist yet;
4. writes `/etc/systemd/system/server-manager.service` from
   [`deploy/server-manager.service`](deploy/server-manager.service), enables and starts it, and
   checks that it answers on `127.0.0.1:3900`;
5. prints what is still missing (packages, the nginx `include`, the PostgreSQL role).

It never edits `nginx.conf` or PostgreSQL. Those two steps are below, so you can see exactly what
changes.

Installer options are environment variables, for example `APPS=/srv/apps scripts/install.sh`:

| Variable | Default | |
|---|---|---|
| `PREFIX` | `/usr/local` | Binary in `$PREFIX/bin`, frontend in `$PREFIX/share/server-manager/web` |
| `DATA` | `/var/lib/server-manager` | Data dir |
| `APPS` | `$HOME/apps` | Apps dir |
| `RUN_AS` | you | Default Linux user for new apps |
| `SERVICE` | `server-manager` | systemd unit name |
| `PORT` | `3900` | Port on 127.0.0.1 |
| `NGINX_DIR` | `/etc/nginx/sites-manager` | Where site configs are written |
| `PUBLIC_IP` | (detected by the app) | Sets `SM_PUBLIC_IP`, see [Configuration](#configuration) |
| `SKIP_BUILD` | unset | `1` = install the existing `target/release` and `web/dist` |

### Let nginx load the sites

The dashboard writes site files to `/etc/nginx/sites-manager/`. Add one `include` line to the
`http { }` block of `/etc/nginx/nginx.conf`, right after the `sites-enabled` one:

```bash
sudo sed -i 's|^\(\s*\)include /etc/nginx/sites-enabled/\*;|&\n\1include /etc/nginx/sites-manager/*.conf;|' /etc/nginx/nginx.conf
sudo nginx -t && sudo systemctl reload nginx
```

Open ports 80 and 443 in your firewall (`sudo ufw allow 'Nginx Full'` with ufw, and in your cloud
provider's security group). Certificates use the webroot `/var/www/html`, which the nginx package
creates.

### Give it a PostgreSQL role

The Databases page connects as a role that can create databases and roles but is **not** a
superuser. Its URL lives in `pg.env` in the data dir and is read on every request, so no restart
is needed:

```bash
PGPASS=$(openssl rand -hex 24)
sudo -u postgres psql -c "CREATE ROLE server_manager LOGIN CREATEDB CREATEROLE PASSWORD '$PGPASS'"
echo "PG_ADMIN_URL=postgres://server_manager:$PGPASS@127.0.0.1:5432/postgres" | sudo tee /var/lib/server-manager/pg.env >/dev/null
sudo chmod 600 /var/lib/server-manager/pg.env
unset PGPASS
```

## First login

Create the owner account (asks for a password of at least 10 characters; run it again later to
reset the password, which also signs out every session):

```bash
sudo env SM_DATA_DIR=/var/lib/server-manager server-manager create-admin admin
```

The manager listens on `127.0.0.1:3900` only. From your own computer, open an SSH tunnel and browse
to http://localhost:3900:

```bash
ssh -L 3900:127.0.0.1:3900 you@your-server
```

The tunnel is the safest way in. If you want a normal URL, put it behind nginx with HTTPS.

## Behind nginx and HTTPS

Example: the dashboard at `https://example.com/server-manager/`. Add this to the `server { }`
block of `example.com` (the trailing slash in `proxy_pass` strips the `/server-manager` prefix):

```nginx
location = /server-manager {
    return 301 /server-manager/;
}

location /server-manager/ {
    proxy_pass http://127.0.0.1:3900/;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto https;
    proxy_set_header Connection "";
    proxy_buffering off;          # live deploy logs (server-sent events)
    proxy_read_timeout 3600s;
    client_max_body_size 10m;
}
```

Then tell the manager where it lives. Use a systemd drop-in, which survives re-running the
installer:

```bash
sudo mkdir -p /etc/systemd/system/server-manager.service.d
sudo tee /etc/systemd/system/server-manager.service.d/override.conf >/dev/null <<'EOF'
[Service]
Environment=SM_BASE_PATH=/server-manager
Environment=SM_PUBLIC_URL=https://example.com/server-manager
Environment=SM_COOKIE_SECURE=1
Environment=SM_TRUST_PROXY=1
EOF
sudo systemctl daemon-reload && sudo systemctl restart server-manager
sudo nginx -t && sudo systemctl reload nginx
```

- `SM_BASE_PATH` puts the prefix in the session cookie's path. Serving it at the root of its own
  host (`location / { proxy_pass http://127.0.0.1:3900; … }`) works too: leave `SM_BASE_PATH` out.
- `SM_COOKIE_SECURE=1` marks the cookie `Secure`. Set it whenever the browser uses https.
- `SM_TRUST_PROXY=1` takes the client IP from `X-Real-IP`, so the login lockout counts real
  visitors. Only set it when nginx is the only way to reach the port.
- A dashboard login is root on this server. Use a long password.

### Cloudflare in front

- **Real visitor IP.** Without it, `X-Real-IP` is a Cloudflare edge address and the login lockout
  would block Cloudflare instead of the attacker. Put the ranges from
  https://www.cloudflare.com/ips/ in a file inside `http { }`, for example
  `/etc/nginx/conf.d/cloudflare-realip.conf`:

  ```nginx
  set_real_ip_from 173.245.48.0/20;
  set_real_ip_from 103.21.244.0/22;
  # … one line per range from https://www.cloudflare.com/ips-v4 and /ips-v6
  real_ip_header CF-Connecting-IP;
  ```

- **SSL mode Flexible**: Cloudflare talks plain http to your server, so the block above can sit in
  a `listen 80` server and `X-Forwarded-Proto https` stays hard-coded (nginx itself only sees
  http). Keep `SM_COOKIE_SECURE=1`. Add `absolute_redirect off;` to the server block so nginx's own
  redirects stay relative to https.
- **SSL mode Full (strict)** is better: the traffic between Cloudflare and your server is
  encrypted too. Give the server a certificate (Let's Encrypt or a Cloudflare origin certificate),
  put the block in the `listen 443 ssl` server, and keep the same env vars.
- Domains whose DNS record is proxied (orange cloud) resolve to Cloudflare, so the dashboard's
  "A record points to this server" check fails for them, and it will not ask certbot for a
  certificate. Issue the certificate while the record is DNS only (grey cloud), or use Flexible /
  an origin certificate for those sites.

## Configuration

All settings are environment variables of the service. The installer sets the ones in the
"installer" column; add your own in the drop-in shown above. Empty values count as unset.

| Variable | Default in the binary | Installer sets | What it does |
|---|---|---|---|
| `HOST` | `127.0.0.1` | `127.0.0.1` | Bind address. Keep it on localhost. |
| `PORT` | `3900` | `3900` | Bind port |
| `SM_DATA_DIR` | `$HOME/.server-manager` | `/var/lib/server-manager` | SQLite `state.db`, JWT secret, deploy key, logs (created with mode 700) |
| `SM_APPS_DIR` | `$HOME/apps` | `/home/<you>/apps` | Every app folder must be inside it |
| `SM_WEB_DIST` | `web/dist` (relative to the working directory) | `/usr/local/share/server-manager/web` | Built frontend |
| `SM_DEFAULT_RUN_AS` | `ubuntu` | `<you>` | Linux user preselected for new apps |
| `SM_BACKUP_DIR` | `<data dir's parent>/backups/postgres` if the data dir is named `.server-manager`, else `$HOME/backups/postgres` | `/var/lib/server-manager/backups/postgres` | `pg_dump` files |
| `SM_NGINX_DIR` | `/etc/nginx/sites-manager` | `/etc/nginx/sites-manager` | Site configs (`<domain>.conf`, `<domain>.conf.disabled`) |
| `SM_PUBLIC_IP` | the IPv4 address the server sends from | empty (= detect) | The IP a domain's A record must point to before certbot runs. Behind NAT (AWS, GCP, Tencent Cloud and others) the detected address is private: set your public IP here. The Web server page shows the value in use. |
| `SM_BASE_PATH` | empty | not set | URL prefix nginx strips, e.g. `/server-manager` |
| `SM_PUBLIC_URL` | `http(s)://<Host header><SM_BASE_PATH>` | not set | Public URL of the dashboard, used for the webhook URL shown in Settings |
| `SM_COOKIE_SECURE` | off | not set | `1` or `true`: `Secure` session cookie (also makes the fallback public URL https) |
| `SM_TRUST_PROXY` | off | not set | `1` or `true`: client IP from `X-Real-IP` |
| `SM_JWT_SECRET` | random, saved to `<data>/jwt.secret` | not set | Session signing key; ignored if shorter than 32 characters |
| `SM_UNIT_DIR` | `/etc/systemd/system` | not set | Where app units are written |
| `PG_ADMIN_URL` | read from `<data>/pg.env` | not set | PostgreSQL admin URL. Prefer the file: unlike `SM_*`, this variable is not removed from child processes. |
| `HOME` | | `/root` | Needed for the `$HOME` defaults above |

## How deploys work

Each app lives in `<apps dir>/<name>/`:

```
repo/            git clone (fetch happens here)
releases/<n>/    one checkout per deploy (git worktree)
current ->       symlink to the live release
shared/.env      and any other shared paths, symlinked into every release
```

A deploy runs these steps, each visible live in the dashboard and saved to
`<data>/logs/<app>/<n>.log` (kept 30 days, `.env` secrets masked):

1. **Pull**: `git fetch` the branch and check the commit out into `releases/<n>`.
2. **Link shared**: symlink `.env` (and other shared paths) from `shared/`. On the first deploy a
   shared file that exists in the repo is moved to `shared/`.
3. **Build**: your build command, as the app's user, stops on a non-zero exit or the timeout.
4. **Before restart**: optional command, for example database migrations.
5. **Switch**: `current` is replaced atomically (`rename(2)`). Nothing touches the live release
   before this step, and a deploy can be cancelled until here.
6. **Restart**: `systemctl restart app-<name>`.
7. **Health check**: `GET http://127.0.0.1:<port><health path>` must answer 2xx within 30 s. With
   no health path, the unit must still be active after 5 s. If it fails, `current` goes back to the
   previous release, the app restarts, and the deploy is marked **rolled back**.
8. **Clean up**: keep the newest N releases (default 5) for rollback.

One deploy runs per app at a time; one more can wait, and a newer one replaces the waiting one.
Restarting the manager stops a deploy that is running (builds are its child processes); the apps
themselves keep running, because they are separate units.

**Private repos.** Settings shows the server's SSH deploy key (`<data>/deploy_key`, ed25519). Add
it to the repository as a read-only deploy key and use the `git@github.com:…` URL. GitHub accepts a
given deploy key on one repository only; for more private repos, add the key to a GitHub account
(for example a machine user) that can read them, or save an HTTPS token in Settings and use
`https://github.com/…` URLs.

**Webhook (auto-deploy).** Turn on auto-deploy in the app, then in GitHub › repository › Settings ›
Webhooks add:

- Payload URL: `<public URL>/hooks/<app name>`, e.g. `https://example.com/server-manager/hooks/shop-api`
  (Settings shows the exact URL)
- Content type: `application/json`
- Secret: the webhook secret from Settings (one secret for all apps, stored in the manager's
  SQLite database; rotate it there)
- Events: just the push event

The manager checks `X-Hub-Signature-256` (HMAC-SHA256, constant-time compare), answers GitHub's
ping, and deploys only pushes to the app's branch. GitHub must be able to reach the URL, so this
needs the nginx setup above. To keep the dashboard on the tunnel and expose only the webhook:

```nginx
location /server-manager/hooks/ {
    proxy_pass http://127.0.0.1:3900/hooks/;
}
```

## Security model

Read this before exposing the dashboard.

- **The manager runs as root**, because it writes systemd units, nginx config and app folders,
  runs certbot and reloads nginx. **Anyone who can log in can run any command as root** (for
  example through an app's build command or by choosing `root` as the app user). Treat the
  dashboard password like the root password, keep `HOST=127.0.0.1`, and prefer the SSH tunnel.
- **Apps and builds do not run as root.** Each app has a "run as" user. Its unit has
  `User=<that user>`; build, before-restart and start commands go through
  `setpriv --reuid --regid --init-groups` and `env -i`, so they get only the app's `.env`, `HOME`,
  `USER`, `LOGNAME` and a fixed `PATH`. Variables that change how programs load (`LD_*`, `BASH_ENV`,
  `IFS`, …) are dropped, and the manager's own `SM_*` variables never reach a child process.
- **No shell except for your own commands.** git, systemctl, nginx, certbot, psql and the rest are
  started with fixed arguments and no shell; names and paths are validated first (app folders must
  be inside the apps dir). Only the build / before-restart / start commands you type are run with
  `bash -lc`.
- **Login.** Passwords are hashed with argon2id (minimum 10 characters). Sessions are HS256 JWTs
  (12 h) in the `sm_session` cookie: `HttpOnly`, `SameSite=Strict`, `Path=<base path>/api`, and
  `Secure` with `SM_COOKIE_SECURE=1`. Logging out or changing the password ends every session of
  that account.
- **Lockout.** 5 failed logins from one IP lock that IP for 15 minutes. The attempt is counted
  before the password is checked, and lockouts are stored in SQLite, so restarting does not reset
  them. If more than 30 wrong passwords arrive within a minute from all IPs together, password
  checks are slowed to one every 2 s server-wide. Unknown usernames cost the same time as wrong
  passwords.
- **Headers.** Every response has a Content-Security-Policy (`default-src 'self'`, scripts only
  from the same origin, `frame-ancestors 'none'`), `X-Frame-Options: DENY`, `nosniff` and
  `Referrer-Policy: no-referrer`.
- **Secrets.** `.env` files are mode 600 and owned by the app user. Secret values are masked in the
  API and in deploy logs; revealing one is logged. Database passwords are shown once and are not kept in
  the manager's database or logs (linking a database writes `DATABASE_URL` into the app's `.env`). The HTTPS token, deploy key and JWT secret are files in the data dir
  (mode 600, dir 700).
- **nginx changes are tested.** The previous files are kept, `nginx -t` runs, and on failure (or a
  failed reload) the old files go back, so a bad config is never loaded.
- **Activity log.** Every write action (deploys, settings, env saves, reveals, logins, lockouts) is
  recorded with the account and time, and shown on the Overview page.

Found a security problem? Please open a GitHub issue without exploit details, or contact the
maintainer through GitHub, and allow time for a fix before disclosure.

## Upgrading

```bash
cd server-manager-rust
git pull
scripts/install.sh
```

The installer rebuilds, replaces the binary, frontend and unit, and restarts the service. Database
migrations run on start. Your data, apps and drop-in settings are kept. A deploy that is running
during the restart is stopped; start it again afterwards.

## Backups

| What | Where (installer defaults) |
|---|---|
| Manager state: accounts, apps, deploy history, settings, metrics | `/var/lib/server-manager/state.db` (SQLite, WAL) |
| Deploy key, HTTPS token, JWT secret, `pg.env`, deploy logs | `/var/lib/server-manager/` |
| PostgreSQL dumps (made by the manager) | `/var/lib/server-manager/backups/postgres/` |
| App `.env` files and other shared files | `~/apps/<name>/shared/` |
| nginx sites made in the dashboard | `/etc/nginx/sites-manager/` |

The dumps sit on the same disk as the database: copy them off the server. For a consistent copy of
`state.db` while the service runs, use `sudo sqlite3 /var/lib/server-manager/state.db ".backup /root/state-backup.db"`
(needs the `sqlite3` package), or stop the service while copying.

## Uninstall

```bash
sudo systemctl disable --now server-manager
sudo rm -rf /etc/systemd/system/server-manager.service /etc/systemd/system/server-manager.service.d
sudo systemctl daemon-reload
sudo rm -rf /usr/local/bin/server-manager /usr/local/share/server-manager
```

This leaves your apps running. To remove them too, stop and delete their units
(`/etc/systemd/system/app-*.service`), the apps dir, the site files in `/etc/nginx/sites-manager/`
(then reload nginx) and, when you no longer need the data, `/var/lib/server-manager`.

## Development

Runs on Linux or macOS. When the manager is not root, everything runs as your own user, and the
systemd, nginx and certbot features only work on a Linux box that has them.

```bash
cd web && bun install && bun run build && cd ..          # frontend -> web/dist
export SM_DATA_DIR=/tmp/sm-data SM_APPS_DIR=/tmp/sm-apps   # keep dev state out of your home
cargo run -- create-admin admin                           # asks for a password
cargo run                                                 # http://127.0.0.1:3900
```

`create-admin` reads the password from stdin when it is not a terminal:
`echo 'long-password' | server-manager create-admin admin`.

Checks (all must pass before a pull request):

```bash
cargo test
cargo clippy --all-targets
cd web && bun run build && bunx tsc --noEmit -p .
```

### Project layout

```
src/
  main.rs          CLI (serve | create-admin), SQLite + migrations, router
  config.rs        env -> Config
  auth.rs          login/logout/password, sessions, lockout, require_auth, create-admin
  exec.rs          the only way to run programs: Cmd (no shell), owner commands via setpriv
  validate.rs      app names, env keys, paths under the apps dir
  activity.rs      activity log
  sse.rs           live output to the browser (server-sent events)
  web.rs           static frontend + security headers
  routes/
    metrics.rs     sampler, usage API, services
    apps.rs        apps, systemd units, deploy key, process log
    deploys.rs     deploy pipeline, rollback, GitHub webhook
    env.rs         .env editor
    databases.rs   PostgreSQL, backups
    nginx.rs       site generator, nginx -t, certbot
    settings.rs    settings
migrations/        SQLite schema (add new files, never edit old ones)
web/dashboard/     React 19 + Tailwind 3 frontend (bun), hash routes in app.tsx,
                   shared components in components/ (see components/README.md)
deploy/            systemd unit template used by scripts/install.sh
scripts/install.sh installer / upgrader
```

## Contributing

Issues and pull requests are welcome. Keep changes small and focused, run the checks above, and
describe how you tested anything that touches a real server (systemd, nginx, PostgreSQL).

## License

[MIT](LICENSE)
