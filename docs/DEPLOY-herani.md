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

Public URL: https://herani.id/server-manager (Cloudflare, SSL mode Flexible). The nginx server block
and the Cloudflare real-IP list live in the `herani-landing` repo (`deploy/herani.id.conf`,
`deploy/cloudflare-realip.conf`). Tunnel: `ssh -L 3900:127.0.0.1:3900 server-43`.
