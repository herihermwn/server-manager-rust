# server-manager

One Rust binary that manages apps on one Ubuntu server (`ubuntu@43.156.104.214`): deploy from git,
`.env`, PostgreSQL, nginx sites + SSL, and server usage. It is the API, the agent (runs commands on
the same machine) and the static frontend at once. Plan and decisions:
`tasks/server-manager/{README.md,rencana-implementasi.md}`. Stack and conventions follow
`radio-api-rs` (axum 0.8, sqlx 0.8, argon2, jsonwebtoken, React 19 + Tailwind 3 + bun).

Apps run as **systemd user units** (`app-<name>.service`), not PM2. Access is through an SSH tunnel:
`ssh -L 3900:127.0.0.1:3900 server-43`, then http://localhost:3900.

## Run locally (macOS)

```bash
cd web && bun install && bun run build && cd ..          # → web/dist
export SM_DATA_DIR=/tmp/sm-data                           # keep dev state out of ~/.server-manager
cargo run -- create-admin heri                            # asks for the password (min 10 chars)
cargo run                                                 # http://127.0.0.1:3900
```

Piped password for scripts: `echo 'long-password' | server-manager create-admin heri`. Running
`create-admin` for an existing user resets its password and signs out every session.

Checks: `cargo test`, `cargo clippy --all-targets`, `cd web && bun run build && bunx tsc --noEmit -p .`.

## Env vars

| Var | Default | Use |
|---|---|---|
| `HOST` / `PORT` | `127.0.0.1` / `3900` | Bind address. Keep it on 127.0.0.1. |
| `SM_DATA_DIR` | `~/.server-manager` | `state.db` (SQLite), `jwt.secret`, `logs/` (mode 700) |
| `SM_APPS_DIR` | `~/apps` | Every app folder must be inside it (`validate::path_under`) |
| `SM_WEB_DIST` | `web/dist` | Built frontend |
| `SM_JWT_SECRET` | random, saved to `<data>/jwt.secret` | Override the session signing key (≥ 32 chars) |
| `SM_TRUST_PROXY` | off | `1` = take the client IP from `X-Real-IP` (only behind our own nginx) |

`SM_*` vars are removed from the environment of every child process (`exec.rs`).

## Deploy

`scripts/deploy-to-server.sh` rsyncs the repo to `server-43:~/server-manager-src`, builds there,
installs `~/server-manager/bin/server-manager` + `~/server-manager/web/dist`, and (re)starts the
systemd system unit `deploy/server-manager.service` (User=ubuntu, 127.0.0.1:3900).

## Security model

- Owner login: argon2id hash in `admin_users`; JWT (HS256, 12 h) in cookie `sm_session`
  (`HttpOnly; SameSite=Strict; Path=/api`, `Secure` when nginx sends `X-Forwarded-Proto: https`).
- Every `/api/*` route except `POST /api/auth/login` goes through `auth::require_auth`, which also
  checks `admin_users.token_version`; changing the password bumps it (other sessions die).
- 5 failed logins per IP → locked 15 min (counted before the password check). Unknown usernames
  cost the same argon2 time.
- Response headers: CSP (`script-src 'self'`), `X-Frame-Options: DENY`, `nosniff`, `no-referrer`.
- Commands: `exec::Cmd` (fixed args, no shell) for everything; `exec::run_shell_user_command`
  only for the build/start commands the owner typed. Secrets are masked in output lines.
- Every write action → `activity::log`.

## Module map (where later phases plug in)

```
src/
  main.rs          CLI (serve | create-admin), SQLite open + migrations, router assembly
  config.rs        env → Config
  state.rs         AppState { db, cfg, jwt_secret, lockout }
  http.rs          AppError ({ "error": … }), api_not_found, now()
  auth.rs          login/logout/me/password, require_auth, Admin extension, lockout, create-admin
  activity.rs      activity::log(db, actor, action, target, detail) + GET /api/activity
  exec.rs          Cmd builder (output / stream, timeout, cwd, env, redact, cancel_on), run_shell_user_command
  sse.rs           from_channel, line_event, done_event, run_command (stream a Cmd to the browser)
  validate.rs      app_name, env_key, path_under
  web.rs           web/dist + SPA fallback, security headers
  routes/mod.rs    merges every module; nobody else edits it
  routes/metrics.rs    phase 2   routes/apps.rs     phase 3   routes/deploys.rs  phase 3 (+ /hooks)
  routes/env.rs        phase 4   routes/databases.rs phase 5  routes/nginx.rs    phase 6
  routes/settings.rs   phase 7
migrations/0001_init.sql   all tables; later phases ADD 0002_…sql files, never edit 0001
web/dashboard/
  app.tsx          hash routes → pages
  lib/             api.ts (authFetch, api<T>), auth.tsx, router.ts, useEventStream.ts, server.ts
  components/      copied from radio-api-rs + LogView, Placeholder (see components/README.md)
  pages/overview/index.tsx  pages/usage/index.tsx                       phase 2
  pages/apps/{index,new}.tsx  pages/apps/detail/{index,overview,settings}.tsx   phase 3
  pages/apps/detail/env.tsx                                             phase 4
  pages/databases/index.tsx                                             phase 5
  pages/nginx/{index,site}.tsx                                          phase 6
  pages/settings/index.tsx                                              phase 7
```

Rules for a phase module (`src/routes/<x>.rs`):

- `routes()` paths are relative to `/api` and already authenticated; take `Extension<Admin>` for the actor.
- `spawn(&AppState)` starts background tasks (sampler, backups, cleanup). It is called once at boot.
- Runtime state (per-app deploy lock, caches) lives in a `static` in the module, not in `AppState`.
- Validate names/paths with `validate.rs` before they reach `exec::Cmd` or the filesystem.
- Log every write with `activity::log`. Return `Json(...)` or `AppError`.
- Live output: build a `Cmd`, return `sse::run_command(cmd)`, or feed your own channel to `sse::from_channel`.
