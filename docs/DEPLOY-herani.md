# Maintainer deploy (herani.id)

The maintainer's own instance does not use `scripts/install.sh`. It keeps the older layout:

| What | Where on the server |
|---|---|
| Binary | `~/server-manager/bin/server-manager` |
| Frontend | `~/server-manager/web/dist` |
| Data | `/home/ubuntu/.server-manager` |
| Apps | `/home/ubuntu/apps` |
| Unit | `deploy/herani/server-manager.service` → `/etc/systemd/system/server-manager.service` |

Deploy from the Mac (host alias `server-43` in `~/.ssh/config`, override with `DEPLOY_HOST`):

```bash
scripts/deploy-to-server.sh
```

It rsyncs the repo to `~/server-manager-src`, builds there (`cargo build --release`, `bun run build`),
installs the binary and frontend, installs the unit and restarts it, then prints `SPA 200` and
`API (expect 401) 401`.

Public URL: https://monitor.herani.id (Cloudflare proxied, SSL mode Flexible). The nginx site is
`deploy/herani/monitor.herani.id.conf` (installed by the deploy script). The old
https://herani.id/server-manager path 301-redirects there; that block and the Cloudflare real-IP
list live in the `herani-landing` repo (`deploy/herani.id.conf`, `deploy/cloudflare-realip.conf`). Tunnel: `ssh -L 3900:127.0.0.1:3900 server-43`.

## Other apps on server-43 (migrated 2026-10-25)

`design-board` (design.herani.id, :3950), `replay` (hindsight.herani.id, :3930) and `shortlink`
(link.herani.id, :3920) are Server Manager apps: deployed from the dashboard (repos over the HTTPS
token in Settings), units `app-<name>.service`, running as `ubuntu`, `.env` in
`~/apps/<name>/shared/.env`, data still in `/var/lib/<name>` (owned by ubuntu).

- Hardening from their old units lives in drop-ins `deploy/herani/apps/app-<name>.hardening.conf`,
  installed by hand to `/etc/systemd/system/app-<name>.service.d/hardening.conf` (server-manager
  never rewrites drop-ins). `ReadWritePaths=/var/lib/<name>` is what lets them write their data.
- Builds share a warm cargo target per app: `CARGO_TARGET_DIR=~/.cache/sm-target/<name>`.
- nginx: Webserver sites with the whole old config in custom directives (their own `location /`,
  so the generated one is left out). Copies in `deploy/herani/sites/*.custom`; rate-limit zones and
  log formats stay in `/etc/nginx/conf.d/{design-board,replay}-zones.conf`. The pre-migration
  files are in `/root/sm-migrate/nginx-old/` and `/etc/nginx/sites-available/`.
- Databases `design_board`, `replay`, `shortlink` are registered in Databases (owner roles granted
  to `server_manager`), so the 02:00 WIB pg_dump covers them. The old crons
  (`/etc/cron.d/design-board-backup`, now as root, and `/etc/cron.d/replay-backup`) still run.
- Each repo's own `scripts/deploy-to-server.sh` exits with "Retired". The old units
  (`design-board`, `replay`, `shortlink`.service) are disabled, not deleted; rollback = stop the
  `app-` unit, chown `/var/lib/<name>` back to its user, re-enable the old unit and its nginx site.
- herani.id (landing) stays outside Server Manager (`/home/ubuntu` is 750, nginx can't read a
  static site under `~/apps`); its config is in the `herani-landing` repo.
