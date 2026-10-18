//! Phase 3 · Apps: CRUD, wizard checks (git ls-remote, folder, port, runtime), deploy key,
//! start/stop/restart and process info via system `systemctl` (units run as apps.run_as),
//! process log SSE (journalctl).
//! Routes are relative to /api and already behind auth.
//!
//! Every app lives in `<apps_dir>/<name>/{repo,releases/<n>,current,shared}` (rencana §4) and runs
//! as the system unit `/etc/systemd/system/app-<name>.service` with `User=<run_as>`. The folder is derived from the name (phase 4
//! reads `<apps_dir>/<name>/shared/.env` too), so it can't be moved after creation.
//!
//! For other phases: `load`, `restart_app`, `systemctl`, `unit_name`, `git` (deploy-key aware).

use super::deploys;
use crate::{
    activity,
    auth::{Admin, write_private},
    exec::{Cmd, Exit, Line, RunAs},
    http::{ApiResult, AppError, now},
    sse,
    state::AppState,
    validate,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path as FsPath, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc};

/// Names that would collide with the static routes under /api/apps.
const RESERVED: [&str; 6] = [
    "new",
    "check-repo",
    "detect",
    "check-folder",
    "check-port",
    "deploy-key",
];

// ---------- model ----------

#[derive(sqlx::FromRow, Clone, Debug)]
pub struct AppRow {
    pub id: i64,
    pub name: String,
    pub repo_url: String,
    pub branch: String,
    pub path: String,
    pub runtime: String,
    pub build_cmd: String,
    pub before_restart_cmd: String,
    pub start_cmd: String,
    pub port: Option<i64>,
    pub health_path: String,
    pub shared_paths: String,
    pub keep_releases: i64,
    pub build_timeout_secs: i64,
    pub auto_deploy: bool,
    pub webhook_secret: Option<String>,
    pub current_release: Option<i64>,
    pub run_as: String,
    pub created_at: i64,
}

impl AppRow {
    pub fn dir(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
    pub fn env_path(&self) -> PathBuf {
        self.dir().join("shared").join(".env")
    }
    /// Relative paths linked from `shared/` into every release. `.env` is always one of them.
    pub fn shared(&self) -> Vec<String> {
        let mut v: Vec<String> = serde_json::from_str(&self.shared_paths).unwrap_or_default();
        if !v.iter().any(|p| p == ".env") {
            v.insert(0, ".env".into());
        }
        v
    }
}

/// The app row, or 404. Validates the name first.
pub async fn load(st: &AppState, name: &str) -> ApiResult<AppRow> {
    validate::app_name(name)?;
    sqlx::query_as("SELECT * FROM apps WHERE name = ?")
        .bind(name)
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| AppError::not_found("App not found."))
}

fn conflict(msg: impl Into<String>) -> AppError {
    AppError::Status(StatusCode::CONFLICT, msg.into())
}

// ---------- input checks ----------

/// git@host:path, ssh://, https://, file:// or an absolute path (local bare repo). No options, no spaces.
fn repo_url(s: &str) -> Result<&str, AppError> {
    let s_ok = !s.is_empty()
        && s.len() <= 300
        && !s.starts_with('-')
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
        && (["https://", "ssh://", "git@", "file://", "/"]
            .iter()
            .any(|p| s.starts_with(p)));
    if s_ok {
        Ok(s)
    } else {
        Err(AppError::bad_request(
            "Repository: use an SSH link (git@github.com:you/app.git) or an https:// link.",
        ))
    }
}

fn branch(s: &str) -> Result<&str, AppError> {
    let ok = (1..=100).contains(&s.len())
        && !s.starts_with('-')
        && !s.contains("..")
        && !s.ends_with('/')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b));
    if ok {
        Ok(s)
    } else {
        Err(AppError::bad_request("Branch: letters, digits, . _ / - only."))
    }
}

fn start_cmd(s: &str) -> Result<&str, AppError> {
    let s = s.trim();
    if s.is_empty() || s.len() > 1000 || s.contains(['\n', '\r', '\0']) {
        return Err(AppError::bad_request("Start command: one line, 1-1000 characters."));
    }
    Ok(s)
}

fn script(label: &str, s: &str) -> Result<String, AppError> {
    if s.len() > 4000 || s.contains('\0') {
        return Err(AppError::bad_request(format!("{label}: at most 4000 characters.")));
    }
    Ok(s.trim().replace("\r\n", "\n"))
}

fn health_path(s: &str) -> Result<String, AppError> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    let ok = s.starts_with('/') && s.len() <= 200 && !s.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok {
        Ok(s.into())
    } else {
        Err(AppError::bad_request("Health check: a path like /health, or empty."))
    }
}

fn shared_paths(v: &[String]) -> Result<Vec<String>, AppError> {
    let mut out: Vec<String> = vec![".env".into()];
    for p in v.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        let ok = p.len() <= 200
            && !p.starts_with('/')
            && !p.split('/').any(|c| c == ".." || c == "." || c == ".git")
            && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b));
        if !ok {
            return Err(AppError::bad_request(format!(
                "Shared path \"{p}\": relative, no .., letters, digits, . _ / -."
            )));
        }
        if !out.iter().any(|x| x == p) {
            out.push(p.to_owned());
        }
    }
    if out.len() > 20 {
        return Err(AppError::bad_request("At most 20 shared paths."));
    }
    Ok(out)
}

/// systemd unit files can't hold quotes, `%`, `$` or spaces in paths without escaping; we refuse them.
fn unit_safe_path(p: &FsPath) -> Result<(), AppError> {
    let s = p.to_string_lossy();
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b)) {
        Ok(())
    } else {
        Err(AppError::bad_request(format!(
            "App folder {s} has characters a systemd unit can't hold."
        )))
    }
}

async fn port_problem(st: &AppState, port: i64, except: Option<i64>) -> Result<Option<String>, AppError> {
    if !(1024..=65535).contains(&port) {
        return Ok(Some("Port must be 1024-65535.".into()));
    }
    let used: Option<String> = sqlx::query_scalar("SELECT name FROM apps WHERE port = ? AND id IS NOT ?")
        .bind(port)
        .bind(except)
        .fetch_optional(&st.db)
        .await?;
    if let Some(n) = used {
        return Ok(Some(format!("Used by app {n}.")));
    }
    if port == st.cfg.port as i64 {
        return Ok(Some("Used by Server Manager.".into()));
    }
    if std::net::TcpListener::bind(("127.0.0.1", port as u16)).is_err() {
        return Ok(Some("Something is already listening on this port.".into()));
    }
    Ok(None)
}

async fn suggest_port(st: &AppState) -> Result<i64, AppError> {
    for p in 3001..3999 {
        if port_problem(st, p, None).await?.is_none() {
            return Ok(p);
        }
    }
    Ok(3001)
}

// ---------- commands ----------

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// `SM_DEFAULT_RUN_AS`, else "ubuntu": who new apps run as.
pub fn default_run_as() -> String {
    std::env::var("SM_DEFAULT_RUN_AS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "ubuntu".into())
}

/// An existing system user (passwd entry) for apps.run_as. root is allowed when the owner picks it.
pub async fn lookup_user(name: &str) -> ApiResult<RunAs> {
    let ok = (1..=32).contains(&name.len())
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b));
    if !ok {
        return Err(AppError::bad_request("Run as: a system user name like ubuntu."));
    }
    if !is_root() {
        // Dev mode (macOS, non-root): everything runs as the manager's own user anyway.
        // SAFETY: getuid/getgid have no preconditions.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        let user = std::env::var("USER").unwrap_or_else(|_| name.to_owned());
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        return Ok(RunAs { user, uid, gid, home });
    }
    let o = Cmd::new("getent").args(["passwd", name]).output().await;
    let line = o
        .ok()
        .filter(|o| o.exit.success())
        .map(|o| o.stdout)
        .unwrap_or_default();
    // name:x:uid:gid:gecos:home:shell
    let f: Vec<&str> = line.trim().split(':').collect();
    match (
        f.get(2).and_then(|v| v.parse().ok()),
        f.get(3).and_then(|v| v.parse().ok()),
        f.get(5),
    ) {
        (Some(uid), Some(gid), Some(home)) if f[0] == name => Ok(RunAs {
            user: name.to_owned(),
            uid,
            gid,
            home: home.to_string(),
        }),
        _ => Err(AppError::bad_request(format!(
            "Run as: there is no system user \"{name}\" on this server."
        ))),
    }
}

/// `chown -R <user>: path` (no-op when the manager is not root, e.g. macOS dev). Symlinks are not followed.
pub async fn chown_r(user: &RunAs, path: &FsPath) -> Result<(), String> {
    if !is_root() || !path.exists() {
        return Ok(());
    }
    let o = Cmd::new("chown")
        .args(["-R", &format!("{}:{}", user.uid, user.gid)])
        .arg(path.to_string_lossy())
        .timeout(Duration::from_secs(300))
        .output()
        .await
        .map_err(|e| format!("chown: {e}"))?;
    if o.exit.success() {
        Ok(())
    } else {
        Err(format!("chown {}: {}", path.display(), o.stderr.trim()))
    }
}

/// chown one file (e.g. shared/.env) to the app user, keeping its mode. No-op when not root.
pub async fn chown_to(run_as: &str, path: &FsPath) -> std::io::Result<()> {
    if !is_root() {
        return Ok(());
    }
    let u = lookup_user(run_as)
        .await
        .map_err(|_| std::io::Error::other(format!("no system user {run_as}")))?;
    std::os::unix::fs::chown(path, Some(u.uid), Some(u.gid))
}

/// `systemctl <args>` (system manager). Ok(stdout) on exit 0, Err(stderr) otherwise.
pub async fn systemctl(args: &[&str]) -> Result<String, String> {
    let o = Cmd::new("systemctl")
        .args(args.iter().copied())
        .timeout(Duration::from_secs(30))
        .output()
        .await
        .map_err(|e| format!("could not run systemctl: {e}"))?;
    if o.exit.success() {
        Ok(o.stdout)
    } else {
        let e = o.stderr.trim();
        Err(if e.is_empty() {
            format!("systemctl {} failed ({:?})", args.join(" "), o.exit)
        } else {
            e.chars().take(400).collect()
        })
    }
}

pub fn unit_name(name: &str) -> String {
    format!("app-{name}.service")
}

fn unit_dir() -> PathBuf {
    std::env::var("SM_UNIT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| "/etc/systemd/system".into())
}

/// Quote for a systemd ExecStart word inside '…': systemd unescapes C-style `\\` and `\'`, expands
/// `%` specifiers and `$VAR`, so those are doubled / escaped. bash then gets the text verbatim.
fn systemd_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// System unit `app-<name>.service`, run as `user`. A crash loop (15 starts in 5 min) ends in the
/// failed state = "crashed".
pub fn unit_file(name: &str, dir: &FsPath, port: Option<i64>, start: &str, user: &RunAs) -> String {
    let d = dir.display();
    let port_line = port.map(|p| format!("Environment=PORT={p}\n")).unwrap_or_default();
    format!(
        "# Written by server-manager. Edits are overwritten on the next deploy or settings save.\n\
         [Unit]\n\
         Description=server-manager app {name}\n\
         After=network.target\n\
         StartLimitIntervalSec=300\n\
         StartLimitBurst=15\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={u}\n\
         Group={g}\n\
         WorkingDirectory={d}/current\n\
         Environment=HOME={h}\n\
         Environment=PATH={path}\n\
         {port_line}\
         EnvironmentFile=-{d}/shared/.env\n\
         ExecStart=/bin/bash -lc {}\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         TimeoutStopSec=20\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        systemd_quote(start),
        u = user.user,
        g = user.gid,
        h = user.home,
        path = user.path(),
    )
}

/// Write the unit (only when it changed) and daemon-reload.
pub async fn install_unit(app: &AppRow) -> Result<(), String> {
    let user = lookup_user(&app.run_as).await.map_err(|e| match e {
        AppError::Status(_, m) | AppError::Internal(m) => m,
    })?;
    let dir = unit_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join(unit_name(&app.name));
    let text = unit_file(&app.name, &app.dir(), app.port, &app.start_cmd, &user);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        // The manager's UMask is 0077; unit files are conventionally world-readable.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        systemctl(&["daemon-reload"]).await?;
    }
    Ok(())
}

/// reset-failed (so a crashed unit may start again) + restart. Phase 4/5 can call this after an
/// .env change.
pub async fn restart_app(name: &str) -> Result<(), String> {
    let unit = unit_name(name);
    let _ = systemctl(&["reset-failed", &unit]).await;
    systemctl(&["restart", &unit]).await.map(|_| ())
}

/// git (as the manager, i.e. root) with the server deploy key, no prompts. Its global config is
/// `<data>/gitconfig` with `safe.directory=*`, because app folders (and local source repos) belong
/// to other users; `-c` would not reach the upload-pack of a local clone. Known hosts: data dir.
pub fn git(st: &AppState) -> Cmd {
    let key = deploy_key_path(st);
    let gitconfig = st.cfg.data_dir.join("gitconfig");
    if !gitconfig.exists() {
        let _ = std::fs::write(&gitconfig, "[safe]\n\tdirectory = *\n");
    }
    Cmd::new("git")
        .env("GIT_CONFIG_GLOBAL", gitconfig.to_string_lossy())
        .env(
            "GIT_SSH_COMMAND",
            format!(
                "ssh -i '{}' -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile='{}' -o BatchMode=yes",
                key.display(),
                st.cfg.data_dir.join("known_hosts").display()
            ),
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .timeout(Duration::from_secs(60))
}

/// Last useful stderr lines of a failed git call, with a hint for the deploy-key case.
pub fn git_error(stderr: &str) -> String {
    if stderr.contains("Permission denied (publickey)") {
        return "Permission denied (publickey). Add the server deploy key to this repository (Settings › Deploy keys)."
            .into();
    }
    if stderr.contains("Repository not found") || stderr.contains("does not appear to be a git repository") {
        return "Repository not found, or the deploy key has no access to it.".into();
    }
    if stderr.contains("could not read Username") {
        return "This https repository needs a login. Use the SSH link with the deploy key instead.".into();
    }
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" ");
    if tail.is_empty() { "git failed.".into() } else { tail }
}

// ---------- deploy key ----------

fn deploy_key_path(st: &AppState) -> PathBuf {
    st.cfg.data_dir.join("deploy_key")
}

static KEYGEN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Create `<data>/deploy_key` (ed25519, no passphrase) if missing.
async fn ensure_deploy_key(st: &AppState) -> ApiResult<PathBuf> {
    let _g = KEYGEN.lock().await;
    let path = deploy_key_path(st);
    if path.exists() {
        return Ok(path);
    }
    let host = Cmd::new("hostname")
        .output()
        .await
        .map(|o| o.stdout.trim().to_owned())
        .unwrap_or_default();
    let o = Cmd::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C"])
        .arg(format!(
            "server-manager@{}",
            if host.is_empty() { "server" } else { &host }
        ))
        .arg("-f")
        .arg(path.to_string_lossy())
        .output()
        .await?;
    if !o.exit.success() {
        return Err(AppError::Internal(format!("ssh-keygen: {}", o.stderr.trim())));
    }
    Ok(path)
}

/// GET /apps/deploy-key → public key + fingerprint (never the private key).
async fn deploy_key(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let path = ensure_deploy_key(&st).await?;
    let pub_path = path.with_extension("pub");
    let public_key = std::fs::read_to_string(&pub_path)?.trim().to_owned();
    let o = Cmd::new("ssh-keygen")
        .arg("-lf")
        .arg(pub_path.to_string_lossy())
        .output()
        .await?;
    // "256 SHA256:abc… server-manager@host (ED25519)"
    let mut words = o.stdout.split_whitespace();
    let fingerprint = words.nth(1).unwrap_or_default().to_owned();
    let kind = o
        .stdout
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .trim_matches(['(', ')'])
        .to_lowercase();
    Ok(Json(
        json!({ "data": { "publicKey": public_key, "fingerprint": fingerprint, "type": kind } }),
    ))
}

// ---------- wizard checks ----------

#[derive(Deserialize)]
struct RepoBody {
    repo: String,
    branch: Option<String>,
}

/// POST /apps/check-repo {repo} → branches + default branch + head commit, or the git error.
async fn check_repo(State(st): State<AppState>, Json(b): Json<RepoBody>) -> ApiResult<Json<Value>> {
    let url = repo_url(b.repo.trim())?;
    ensure_deploy_key(&st).await?;
    let o = git(&st)
        .args(["ls-remote", "--symref", url, "HEAD", "refs/heads/*"])
        .timeout(Duration::from_secs(25))
        .output()
        .await?;
    if !o.exit.success() {
        let msg = if o.exit == Exit::TimedOut {
            "No answer from the git host after 25 s.".into()
        } else {
            git_error(&o.stderr)
        };
        return Err(AppError::bad_request(msg));
    }
    let (mut default, mut head, mut branches) = (None, None, Vec::new());
    for l in o.stdout.lines() {
        if let Some(r) = l.strip_prefix("ref: refs/heads/") {
            default = r.split('\t').next().map(str::to_owned);
        } else if let Some((sha, r)) = l.split_once('\t') {
            if r == "HEAD" {
                head = Some(sha.to_owned());
            } else if let Some(n) = r.strip_prefix("refs/heads/") {
                branches.push(json!({ "name": n, "commit": sha }));
            }
        }
    }
    if branches.is_empty() {
        return Err(AppError::bad_request(
            "The repository has no branches yet. Push a commit first.",
        ));
    }
    Ok(Json(
        json!({ "data": { "branches": branches, "defaultBranch": default, "headCommit": head } }),
    ))
}

#[derive(Debug, PartialEq)]
struct Detected {
    runtime: &'static str,
    from: &'static str,
    build: String,
    start: String,
}

/// Runtime + suggested commands from the top-level file names and a few file contents.
fn detect_runtime(files: &[String], read: impl Fn(&str) -> Option<String>) -> Detected {
    let has = |f: &str| files.iter().any(|x| x == f);
    let scripts = |pkg: &Option<String>| -> (bool, bool) {
        let v: Value = pkg
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        (v["scripts"]["build"].is_string(), v["scripts"]["start"].is_string())
    };
    if has("Cargo.toml") {
        let bin = read("Cargo.toml")
            .and_then(|t| {
                t.split("[package]").nth(1).and_then(|s| {
                    s.lines()
                        .take_while(|l| !l.trim_start().starts_with('['))
                        .find_map(|l| {
                            l.trim()
                                .strip_prefix("name")?
                                .trim()
                                .strip_prefix('=')
                                .map(|v| v.trim().trim_matches('"').to_owned())
                        })
                })
            })
            .unwrap_or_else(|| "app".into());
        return Detected {
            runtime: "rust",
            from: "Cargo.toml",
            build: "cargo build --release".into(),
            start: format!("./target/release/{bin}"),
        };
    }
    if has("bun.lockb") || has("bun.lock") {
        let (b, s) = scripts(&read("package.json"));
        let build = if b {
            "bun install --frozen-lockfile\nbun run build"
        } else {
            "bun install --frozen-lockfile"
        };
        let start = if s { "bun run start" } else { "bun run index.ts" };
        return Detected {
            runtime: "bun",
            from: if has("bun.lock") { "bun.lock" } else { "bun.lockb" },
            build: build.into(),
            start: start.into(),
        };
    }
    if has("package.json") {
        let (b, s) = scripts(&read("package.json"));
        let install = if has("package-lock.json") {
            "npm ci"
        } else {
            "npm install"
        };
        let build = if b {
            format!("{install}\nnpm run build")
        } else {
            install.into()
        };
        return Detected {
            runtime: "node",
            from: "package.json",
            build,
            start: if s { "npm start".into() } else { "node index.js".into() },
        };
    }
    if has("index.html") {
        return Detected {
            runtime: "static",
            from: "index.html",
            build: String::new(),
            start: "python3 -m http.server $PORT --bind 127.0.0.1".into(),
        };
    }
    Detected {
        runtime: "other",
        from: "",
        build: String::new(),
        start: String::new(),
    }
}

/// Keys in a .env.example (for "12 keys found").
fn env_keys(text: &str) -> usize {
    text.lines()
        .map(|l| l.trim().trim_start_matches("export "))
        .filter(|l| {
            !l.starts_with('#')
                && l.split_once('=')
                    .is_some_and(|(k, _)| validate::env_key(k.trim()).is_ok())
        })
        .count()
}

static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// POST /apps/detect {repo, branch} → shallow clone into a temp dir, read the top-level files,
/// suggest runtime + commands, latest commit, .env.example keys, a free port.
async fn detect(State(st): State<AppState>, Json(b): Json<RepoBody>) -> ApiResult<Json<Value>> {
    let url = repo_url(b.repo.trim())?.to_owned();
    let br = branch(b.branch.as_deref().unwrap_or("main"))?.to_owned();
    ensure_deploy_key(&st).await?;
    let tmp = st.cfg.data_dir.join("tmp").join(format!(
        "detect-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(tmp.parent().unwrap_or(&st.cfg.data_dir))?;
    let res = async {
        let t = tmp.to_string_lossy().to_string();
        let o = git(&st)
            .args([
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--no-checkout",
                "--single-branch",
                "-b",
                &br,
                "--",
                &url,
                &t,
            ])
            .timeout(Duration::from_secs(120))
            .output()
            .await?;
        if !o.exit.success() {
            return Err(AppError::bad_request(git_error(&o.stderr)));
        }
        let ls = Cmd::new("git")
            .args(["-C", &t, "ls-tree", "--name-only", "HEAD"])
            .output()
            .await?;
        let files: Vec<String> = ls.stdout.lines().map(str::to_owned).collect();
        let mut contents = HashMap::new();
        for f in ["Cargo.toml", "package.json", ".env.example"] {
            if files.iter().any(|x| x == f) {
                let o = Cmd::new("git")
                    .args(["-C", &t, "show", &format!("HEAD:{f}")])
                    .output()
                    .await?;
                contents.insert(f, o.stdout);
            }
        }
        let d = detect_runtime(&files, |f| contents.get(f).cloned());
        let log = Cmd::new("git")
            .args(["-C", &t, "log", "-1", "--format=%h%x1f%s%x1f%an%x1f%ct"])
            .output()
            .await?;
        let p: Vec<&str> = log.stdout.trim().split('\x1f').collect();
        let commit = (p.len() == 4)
            .then(|| json!({ "sha": p[0], "message": p[1], "author": p[2], "time": p[3].parse::<i64>().unwrap_or(0) }));
        Ok(json!({ "data": {
            "runtime": d.runtime, "detectedFrom": d.from, "buildCmd": d.build, "startCmd": d.start,
            "commit": commit,
            "envExample": contents.get(".env.example").map(|t| env_keys(t)),
            "suggestedPort": suggest_port(&st).await?,
            "defaultRunAs": default_run_as(),
        }}))
    }
    .await;
    std::fs::remove_dir_all(&tmp).ok();
    res.map(Json)
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

fn app_dir(st: &AppState, name: &str) -> ApiResult<PathBuf> {
    validate::app_name(name)?;
    if RESERVED.contains(&name) {
        return Err(AppError::bad_request(format!(
            "\"{name}\" is reserved, pick another name."
        )));
    }
    let dir = validate::path_under(&st.cfg.apps_dir, &st.cfg.apps_dir.join(name))?;
    unit_safe_path(&dir)?;
    Ok(dir)
}

/// The folder may not exist yet, or be an empty dir. Returns the problem, if any.
async fn folder_problem(st: &AppState, name: &str, dir: &FsPath) -> ApiResult<Option<String>> {
    let taken: Option<i64> = sqlx::query_scalar("SELECT id FROM apps WHERE name = ?")
        .bind(name)
        .fetch_optional(&st.db)
        .await?;
    if taken.is_some() {
        return Ok(Some(format!("An app named {name} already exists.")));
    }
    if dir.exists() && std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Ok(Some(format!("{} already exists and is not empty.", dir.display())));
    }
    Ok(None)
}

/// POST /apps/check-folder {name} → folder `<apps_dir>/<name>` free?
async fn check_folder(State(st): State<AppState>, Json(b): Json<NameBody>) -> ApiResult<Json<Value>> {
    let dir = app_dir(&st, b.name.trim())?;
    let problem = folder_problem(&st, b.name.trim(), &dir).await?;
    Ok(Json(
        json!({ "data": { "path": dir, "free": problem.is_none(), "message": problem.unwrap_or_else(|| "Free · will be created".into()) } }),
    ))
}

#[derive(Deserialize)]
struct PortBody {
    port: i64,
    app: Option<String>,
}

/// POST /apps/check-port {port, app?} → not used by another app and nothing listening.
async fn check_port(State(st): State<AppState>, Json(b): Json<PortBody>) -> ApiResult<Json<Value>> {
    let app = match b.app.as_deref() {
        Some(n) => Some(load(&st, n).await?),
        None => None,
    };
    // The app's own port is in use by the app itself.
    let problem = match &app {
        Some(a) if a.port == Some(b.port) => None,
        _ => port_problem(&st, b.port, app.as_ref().map(|a| a.id)).await?,
    };
    Ok(Json(
        json!({ "data": { "free": problem.is_none(), "message": problem.unwrap_or_else(|| "Free · sets PORT in .env".into()) } }),
    ))
}

// ---------- process info ----------

#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Proc {
    pub active_state: String,
    pub sub_state: String,
    pub result: String,
    pub pid: Option<u32>,
    pub restarts: u32,
    pub started_at: Option<i64>,
    pub mem_bytes: Option<u64>,
    pub cpu_pct: Option<f64>,
}

/// name → (when, CPUUsageNSec) of the previous read, for a CPU % between two polls.
static CPU_PREV: LazyLock<Mutex<HashMap<String, (Instant, u64)>>> = LazyLock::new(Default::default);

fn parse_show(out: &str) -> HashMap<String, Proc> {
    let mut map = HashMap::new();
    for block in out.split("\n\n") {
        let kv: HashMap<&str, &str> = block.lines().filter_map(|l| l.split_once('=')).collect();
        let Some(id) = kv.get("Id") else { continue };
        let num = |k: &str| kv.get(k).and_then(|v| v.parse::<u64>().ok());
        let name = id.trim_start_matches("app-").trim_end_matches(".service").to_owned();
        let p = Proc {
            active_state: kv.get("ActiveState").unwrap_or(&"unknown").to_string(),
            sub_state: kv.get("SubState").unwrap_or(&"").to_string(),
            result: kv.get("Result").unwrap_or(&"").to_string(),
            pid: num("MainPID").filter(|&p| p > 0).map(|p| p as u32),
            restarts: num("NRestarts").unwrap_or(0) as u32,
            started_at: kv
                .get("ExecMainStartTimestamp")
                .and_then(|v| v.strip_prefix('@'))
                .and_then(|v| v.parse().ok()),
            mem_bytes: num("MemoryCurrent").filter(|&m| m < u64::MAX),
            cpu_pct: num("CPUUsageNSec").filter(|&c| c < u64::MAX).and_then(|c| {
                let mut prev = CPU_PREV.lock().unwrap();
                let at = Instant::now();
                let pct = prev.get(&name).and_then(|&(t, c0)| {
                    let dt = at.duration_since(t).as_nanos() as f64;
                    (dt > 5e8 && c >= c0).then(|| ((c - c0) as f64 / dt * 1000.0).round() / 10.0)
                });
                prev.insert(name.clone(), (at, c));
                pct
            }),
        };
        map.insert(name, p);
    }
    map
}

/// One `systemctl show` for all given apps. Missing systemd (macOS) → empty map.
pub async fn procs(names: &[String]) -> HashMap<String, Proc> {
    if names.is_empty() {
        return HashMap::new();
    }
    let units: Vec<String> = names.iter().map(|n| unit_name(n)).collect();
    let mut args = vec![
        "show",
        "--timestamp=unix",
        "-p",
        "Id,ActiveState,SubState,Result,MainPID,NRestarts,ExecMainStartTimestamp,MemoryCurrent,CPUUsageNSec",
    ];
    args.extend(units.iter().map(String::as_str));
    systemctl(&args).await.map(|o| parse_show(&o)).unwrap_or_default()
}

/// running | stopped | crashed | deploying
pub fn status(p: Option<&Proc>, deploying: bool) -> &'static str {
    if deploying {
        return "deploying";
    }
    match p.map(|p| (p.active_state.as_str(), p.sub_state.as_str())) {
        Some(("failed", _)) | Some((_, "auto-restart")) => "crashed",
        Some(("active" | "activating" | "reloading", _)) => "running",
        _ => "stopped",
    }
}

/// {id, number, status, finishedAt} of the newest deploy.
async fn last_deploy(st: &AppState, app_id: i64) -> ApiResult<Value> {
    let r: Option<(i64, i64, String, Option<i64>)> =
        sqlx::query_as("SELECT id, number, status, finished_at FROM deploys WHERE app_id = ? ORDER BY id DESC LIMIT 1")
            .bind(app_id)
            .fetch_optional(&st.db)
            .await?;
    Ok(
        r.map(|(id, number, status, f)| json!({ "id": id, "number": number, "status": status, "finishedAt": f }))
            .unwrap_or(Value::Null),
    )
}

/// The deploy that put `current_release` live (a deploy or a rollback to it).
pub async fn live_deploy(st: &AppState, app: &AppRow) -> ApiResult<Value> {
    let Some(rel) = app.current_release else {
        return Ok(Value::Null);
    };
    let r = deploys::deploy_json(
        st,
        "WHERE app_id = ? AND release = ? AND status = 'success' ORDER BY id DESC LIMIT 1",
        &[app.id, rel],
    )
    .await?;
    Ok(r.into_iter().next().unwrap_or(Value::Null))
}

async fn domain(st: &AppState, app_id: i64) -> ApiResult<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT domain FROM sites WHERE app_id = ? AND enabled = 1 ORDER BY id LIMIT 1")
            .bind(app_id)
            .fetch_optional(&st.db)
            .await?,
    )
}

async fn summary(st: &AppState, a: &AppRow, p: Option<&Proc>) -> ApiResult<Value> {
    let live = live_deploy(st, a).await?;
    let active = deploys::active_for(&a.name);
    Ok(json!({
        "name": a.name,
        "runtime": a.runtime,
        "status": status(p, active.is_some()),
        "port": a.port,
        "domain": domain(st, a.id).await?,
        "branch": a.branch,
        "repo": a.repo_url,
        "commit": live.get("commit").cloned().unwrap_or(Value::Null),
        "release": a.current_release,
        "lastDeploy": last_deploy(st, a.id).await?,
        "cpuPct": p.and_then(|p| p.cpu_pct),
        "memBytes": p.and_then(|p| p.mem_bytes),
    }))
}

/// GET /apps → {data: [...]} (phase 2's Overview reads name/runtime/status/port/domain/branch/commit/lastDeploy).
async fn list(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let apps: Vec<AppRow> = sqlx::query_as("SELECT * FROM apps ORDER BY name")
        .fetch_all(&st.db)
        .await?;
    let ps = procs(&apps.iter().map(|a| a.name.clone()).collect::<Vec<_>>()).await;
    let mut data = Vec::with_capacity(apps.len());
    for a in &apps {
        data.push(summary(&st, a, ps.get(&a.name)).await?);
    }
    Ok(Json(json!({ "data": data })))
}

/// GET /apps/{name} → summary + settings + process + live release + active deploy.
async fn detail(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    let a = load(&st, &name).await?;
    let ps = procs(std::slice::from_ref(&a.name)).await;
    let p = ps.get(&a.name);
    let mut v = summary(&st, &a, p).await?;
    let o = v.as_object_mut().expect("object");
    o.insert("path".into(), json!(a.path));
    o.insert("runAs".into(), json!(a.run_as));
    o.insert("buildCmd".into(), json!(a.build_cmd));
    o.insert("beforeRestartCmd".into(), json!(a.before_restart_cmd));
    o.insert("startCmd".into(), json!(a.start_cmd));
    o.insert("healthPath".into(), json!(a.health_path));
    o.insert("sharedPaths".into(), json!(a.shared()));
    o.insert("keepReleases".into(), json!(a.keep_releases));
    o.insert("buildTimeoutSecs".into(), json!(a.build_timeout_secs));
    o.insert("autoDeploy".into(), json!(a.auto_deploy));
    o.insert("webhookPath".into(), json!(format!("/hooks/{}", a.name)));
    o.insert("unit".into(), json!(unit_name(&a.name)));
    o.insert("process".into(), json!(p));
    o.insert("live".into(), live_deploy(&st, &a).await?);
    o.insert(
        "activeDeploy".into(),
        deploys::active_for(&a.name)
            .map(|(id, number)| json!({ "id": id, "number": number }))
            .unwrap_or(Value::Null),
    );
    let deploys: i64 = sqlx::query_scalar("SELECT count(*) FROM deploys WHERE app_id = ?")
        .bind(a.id)
        .fetch_one(&st.db)
        .await?;
    o.insert("deployCount".into(), json!(deploys));
    o.insert("createdAt".into(), json!(a.created_at));
    Ok(Json(json!({ "data": v })))
}

// ---------- create / update / delete ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateBody {
    name: String,
    repo: String,
    branch: String,
    #[serde(default)]
    runtime: String,
    #[serde(default)]
    build_cmd: String,
    #[serde(default)]
    before_restart_cmd: String,
    start_cmd: String,
    port: i64,
    #[serde(default)]
    health_path: String,
    #[serde(default)]
    env_from_example: bool,
    run_as: Option<String>,
}

fn runtime(s: &str) -> &'static str {
    match s {
        "rust" => "rust",
        "bun" => "bun",
        "node" => "node",
        "static" => "static",
        _ => "other",
    }
}

/// POST /apps: check folder → git clone --bare (synchronously, so a refusal comes back right here
/// with the log and the folder removed) → shared/.env → unit → first deploy in the background.
/// 201 {data: {name, deployId}} or 400 {error, log: [lines]}.
async fn create(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Json(b): Json<CreateBody>,
) -> ApiResult<Response> {
    let name = b.name.trim().to_owned();
    let dir = app_dir(&st, &name)?;
    let url = repo_url(b.repo.trim())?.to_owned();
    let br = branch(b.branch.trim())?.to_owned();
    let start = start_cmd(&b.start_cmd)?.to_owned();
    let build = script("Build command", &b.build_cmd)?;
    let before = script("Before restart", &b.before_restart_cmd)?;
    let health = health_path(&b.health_path)?;
    let user = lookup_user(b.run_as.as_deref().map(str::trim).unwrap_or(&default_run_as())).await?;
    if let Some(p) = folder_problem(&st, &name, &dir).await? {
        return Err(conflict(p));
    }
    if let Some(p) = port_problem(&st, b.port, None).await? {
        return Err(AppError::bad_request(format!("Port {}: {p}", b.port)));
    }
    ensure_deploy_key(&st).await?;

    let existed = dir.exists();
    std::fs::create_dir_all(&dir)?;
    let repo = dir.join("repo");
    let clone = git(&st)
        .args(["clone", "--bare", "--single-branch", "-b", &br, "--", &url])
        .arg(repo.to_string_lossy())
        .timeout(Duration::from_secs(600));
    let mut log = vec![format!("$ {}", clone.display())];
    let (tx, mut rx) = mpsc::channel::<Line>(256);
    let collect = async {
        let mut v = Vec::new();
        while let Some(l) = rx.recv().await {
            v.push(l.text);
        }
        v
    };
    let (exit, lines) = tokio::join!(clone.stream(tx), collect);
    log.extend(lines);
    let exit = exit?;
    if !exit.success() {
        // Nothing may be left behind (AppNewFailed): remove what we created.
        if existed {
            std::fs::remove_dir_all(&repo).ok();
        } else {
            std::fs::remove_dir_all(&dir).ok();
        }
        log.push(format!(
            "Removed {} {}",
            if existed { "folder" } else { "empty folder" },
            if existed { repo.display() } else { dir.display() }
        ));
        let err = if exit == Exit::TimedOut {
            "Clone timed out after 10 min.".to_owned()
        } else {
            git_error(&log.join("\n"))
        };
        activity::log(&st.db, &admin.username, "app.create_failed", &name, &err).await;
        return Ok((StatusCode::BAD_REQUEST, Json(json!({ "error": err, "log": log }))).into_response());
    }

    // shared/.env (mode 600): .env.example if asked and present, plus PORT when missing.
    let shared = dir.join("shared");
    std::fs::create_dir_all(dir.join("releases"))?;
    std::fs::create_dir_all(&shared)?;
    let mut env = String::new();
    if b.env_from_example {
        let o = git(&st)
            .args(["-C", &repo.to_string_lossy(), "show", "HEAD:.env.example"])
            .output()
            .await?;
        if o.exit.success() {
            env = o.stdout;
            log.push("$ cp .env.example shared/.env".into());
        }
    }
    if !env
        .lines()
        .any(|l| l.trim_start().trim_start_matches("export ").starts_with("PORT="))
    {
        if !env.is_empty() && !env.ends_with('\n') {
            env.push('\n');
        }
        env.push_str(&format!("PORT={}\n", b.port));
    }
    write_private(&shared.join(".env"), &env)?;
    // Everything the manager (root) created belongs to the app user from here on.
    if let Err(e) = chown_r(&user, &dir).await {
        log.push(format!("warning: {e}"));
    }

    let id = sqlx::query(
        "INSERT INTO apps (name, repo_url, branch, path, runtime, build_cmd, before_restart_cmd, start_cmd, port, health_path, run_as)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(&url)
    .bind(&br)
    .bind(dir.to_string_lossy())
    .bind(runtime(&b.runtime))
    .bind(&build)
    .bind(&before)
    .bind(&start)
    .bind(b.port)
    .bind(&health)
    .bind(&user.user)
    .execute(&st.db)
    .await?
    .last_insert_rowid();
    let app: AppRow = sqlx::query_as("SELECT * FROM apps WHERE id = ?")
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    activity::log(
        &st.db,
        &admin.username,
        "app.create",
        &name,
        &format!("{url} · {br} · port {} · runs as {}", b.port, user.user),
    )
    .await;

    if let Err(e) = install_unit(&app).await {
        log.push(format!("warning: unit not loaded: {e}"));
    } else if let Err(e) = systemctl(&["enable", &unit_name(&name)]).await {
        log.push(format!("warning: enable failed: {e}"));
    }
    let (deploy_id, _) = deploys::start(&st, &app, deploys::Kind::Deploy, "initial", &admin.username, &log).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "data": { "name": name, "deployId": deploy_id } })),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateBody {
    repo: Option<String>,
    branch: Option<String>,
    build_cmd: Option<String>,
    before_restart_cmd: Option<String>,
    start_cmd: Option<String>,
    port: Option<i64>,
    health_path: Option<String>,
    shared_paths: Option<Vec<String>>,
    keep_releases: Option<i64>,
    build_timeout_secs: Option<i64>,
    auto_deploy: Option<bool>,
    run_as: Option<String>,
}

/// PUT /apps/{name}: settings. Commands apply on the next deploy; the unit file is rewritten now
/// (start command / port), the running process is not restarted.
async fn update(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Path(name): Path<String>,
    Json(b): Json<UpdateBody>,
) -> ApiResult<Json<Value>> {
    let mut a = load(&st, &name).await?;
    let mut changed = Vec::new();
    if let Some(r) = b.repo.as_deref().map(str::trim).filter(|r| *r != a.repo_url) {
        a.repo_url = repo_url(r)?.to_owned();
        let repo = a.dir().join("repo");
        let o = git(&st)
            .args([
                "-C",
                &repo.to_string_lossy(),
                "remote",
                "set-url",
                "origin",
                &a.repo_url,
            ])
            .output()
            .await?;
        if !o.exit.success() {
            return Err(AppError::bad_request(format!(
                "git remote set-url: {}",
                o.stderr.trim()
            )));
        }
        changed.push("repo");
    }
    if let Some(v) = &b.branch {
        a.branch = branch(v.trim())?.to_owned();
        changed.push("branch");
    }
    if let Some(v) = &b.build_cmd {
        a.build_cmd = script("Build command", v)?;
        changed.push("build");
    }
    if let Some(v) = &b.before_restart_cmd {
        a.before_restart_cmd = script("Before restart", v)?;
        changed.push("before_restart");
    }
    if let Some(v) = &b.start_cmd {
        a.start_cmd = start_cmd(v)?.to_owned();
        changed.push("start");
    }
    if let Some(p) = b.port.filter(|p| Some(*p) != a.port) {
        if let Some(problem) = port_problem(&st, p, Some(a.id)).await? {
            return Err(AppError::bad_request(format!("Port {p}: {problem}")));
        }
        a.port = Some(p);
        changed.push("port");
    }
    if let Some(v) = &b.health_path {
        a.health_path = health_path(v)?;
        changed.push("health");
    }
    if let Some(v) = &b.shared_paths {
        a.shared_paths = serde_json::to_string(&shared_paths(v)?).unwrap_or_default();
        changed.push("shared");
    }
    if let Some(k) = b.keep_releases {
        if !(1..=20).contains(&k) {
            return Err(AppError::bad_request("Releases to keep: 1-20."));
        }
        a.keep_releases = k;
        changed.push("keep");
    }
    if let Some(t) = b.build_timeout_secs {
        if !(60..=7200).contains(&t) {
            return Err(AppError::bad_request("Deploy timeout: 1-120 minutes."));
        }
        a.build_timeout_secs = t;
        changed.push("timeout");
    }
    if let Some(v) = b.auto_deploy {
        a.auto_deploy = v;
        changed.push("auto_deploy");
    }
    if let Some(u) = b.run_as.as_deref().map(str::trim).filter(|u| *u != a.run_as) {
        let user = lookup_user(u).await?;
        chown_r(&user, &a.dir()).await.map_err(AppError::Internal)?;
        a.run_as = user.user;
        changed.push("run_as");
    }
    sqlx::query(
        "UPDATE apps SET repo_url = ?, branch = ?, build_cmd = ?, before_restart_cmd = ?, start_cmd = ?, port = ?,
         health_path = ?, shared_paths = ?, keep_releases = ?, build_timeout_secs = ?, auto_deploy = ?, run_as = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&a.repo_url)
    .bind(&a.branch)
    .bind(&a.build_cmd)
    .bind(&a.before_restart_cmd)
    .bind(&a.start_cmd)
    .bind(a.port)
    .bind(&a.health_path)
    .bind(&a.shared_paths)
    .bind(a.keep_releases)
    .bind(a.build_timeout_secs)
    .bind(a.auto_deploy)
    .bind(&a.run_as)
    .bind(now())
    .bind(a.id)
    .execute(&st.db)
    .await?;
    let unit_note = install_unit(&a).await.err();
    activity::log(&st.db, &admin.username, "app.update", &a.name, &changed.join(", ")).await;
    Ok(Json(
        json!({ "data": { "ok": true, "changed": changed, "unitWarning": unit_note } }),
    ))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DeleteBody {
    #[serde(default)]
    delete_folder: bool,
}

/// DELETE /apps/{name} {deleteFolder}: stop + disable + remove the unit, the row (deploys cascade),
/// the deploy logs and, if asked, the folder. Linked databases and nginx sites are kept.
async fn remove(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Path(name): Path<String>,
    body: Option<Json<DeleteBody>>,
) -> ApiResult<Json<Value>> {
    let a = load(&st, &name).await?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    if deploys::active_for(&a.name).is_some() {
        return Err(conflict("A deploy is running. Cancel it first."));
    }
    let unit = unit_name(&a.name);
    let _ = systemctl(&["disable", "--now", &unit]).await;
    std::fs::remove_file(unit_dir().join(&unit)).ok();
    let _ = systemctl(&["daemon-reload"]).await;
    let _ = systemctl(&["reset-failed", &unit]).await;
    sqlx::query("DELETE FROM apps WHERE id = ?")
        .bind(a.id)
        .execute(&st.db)
        .await?;
    std::fs::remove_dir_all(st.cfg.logs_dir().join(&a.name)).ok();
    let mut folder_deleted = false;
    if b.delete_folder {
        let dir = validate::path_under(&st.cfg.apps_dir, &a.dir())?;
        std::fs::remove_dir_all(&dir)?;
        folder_deleted = true;
    }
    activity::log(
        &st.db,
        &admin.username,
        "app.delete",
        &a.name,
        if folder_deleted {
            "folder deleted"
        } else {
            "folder kept"
        },
    )
    .await;
    // TODO(phase 5): offer "Drop database" here; for now linked databases are always kept.
    Ok(Json(json!({ "data": {
        "ok": true,
        "folderDeleted": folder_deleted,
        "note": "Linked databases and nginx sites were kept. Remove them on the Databases and Web server pages."
    }})))
}

// ---------- start / stop / restart ----------

async fn control(st: &AppState, admin: &Admin, name: &str, action: &str) -> ApiResult<Json<Value>> {
    let a = load(st, name).await?;
    if a.current_release.is_none() && action != "stop" {
        return Err(conflict("No release yet. Deploy first."));
    }
    let unit = unit_name(&a.name);
    let r = match action {
        "stop" => systemctl(&["stop", &unit]).await.map(|_| ()),
        "start" => {
            install_unit(&a).await.map_err(AppError::Internal)?;
            let _ = systemctl(&["reset-failed", &unit]).await;
            systemctl(&["start", &unit]).await.map(|_| ())
        }
        _ => {
            install_unit(&a).await.map_err(AppError::Internal)?;
            restart_app(&a.name).await
        }
    };
    activity::log(
        &st.db,
        &admin.username,
        &format!("app.{action}"),
        &a.name,
        r.as_ref().err().map(String::as_str).unwrap_or(""),
    )
    .await;
    r.map_err(|e| AppError::bad_request(format!("{action} failed: {e}")))?;
    Ok(Json(json!({ "data": { "ok": true } })))
}

async fn start(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    control(&st, &admin, &name, "start").await
}
async fn stop(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    control(&st, &admin, &name, "stop").await
}
async fn restart(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    control(&st, &admin, &name, "restart").await
}

/// GET /apps/{name}/logs → SSE of `journalctl -u app-<name> -n 200 -f -o short-iso`.
/// The journalctl process is killed when the browser goes away (sse::run_command would keep it).
async fn process_log(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<impl IntoResponse> {
    let a = load(&st, &name).await?;
    let cancel = std::sync::Arc::new(Notify::new());
    let cmd = Cmd::new("journalctl")
        .args([
            "-u",
            &unit_name(&a.name),
            "-n",
            "200",
            "-f",
            "-o",
            "short-iso",
            "--no-hostname",
        ])
        .timeout(Duration::from_secs(6 * 3600))
        .redact(deploys::env_secrets(&a))
        .cancel_on(cancel.clone());
    let (etx, erx) = mpsc::channel(256);
    tokio::spawn(async move {
        let (ltx, mut lrx) = mpsc::channel(256);
        // Stop journalctl as soon as the browser is gone, even when no new line arrives.
        let forward = async {
            loop {
                tokio::select! {
                    l = lrx.recv() => match l {
                        Some(l) if etx.send(sse::line_event(&l)).await.is_ok() => {}
                        Some(_) => { cancel.notify_one(); break; }
                        None => break,
                    },
                    _ = etx.closed() => { cancel.notify_one(); break; }
                }
            }
            while lrx.recv().await.is_some() {}
        };
        let (exit, ()) = tokio::join!(cmd.stream(ltx), forward);
        etx.send(sse::done_event(&exit)).await.ok();
    });
    Ok(sse::from_channel(erx))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps", get(list).post(create))
        .route("/apps/deploy-key", get(deploy_key))
        .route("/apps/check-repo", post(check_repo))
        .route("/apps/detect", post(detect))
        .route("/apps/check-folder", post(check_folder))
        .route("/apps/check-port", post(check_port))
        .route("/apps/{name}", get(detail).put(update).delete(remove))
        .route("/apps/{name}/start", post(start))
        .route("/apps/{name}/stop", post(stop))
        .route("/apps/{name}/restart", post(restart))
        .route("/apps/{name}/logs", get(process_log))
}

pub fn spawn(_st: &AppState) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_file_escapes_for_systemd() {
        let user = RunAs {
            user: "ubuntu".into(),
            uid: 1000,
            gid: 1000,
            home: "/home/ubuntu".into(),
        };
        let u = unit_file(
            "web",
            FsPath::new("/home/u/apps/web"),
            Some(3010),
            r#"echo '$HOME' "50%" \n && ./run"#,
            &user,
        );
        assert!(u.contains("User=ubuntu\nGroup=1000\n"), "{u}");
        assert!(u.contains("Environment=HOME=/home/ubuntu\n"));
        assert!(u.contains("Environment=PATH=/home/ubuntu/.bun/bin:/home/ubuntu/.cargo/bin:"));
        assert!(u.contains("WantedBy=multi-user.target\n"));
        assert!(u.contains("WorkingDirectory=/home/u/apps/web/current\n"));
        assert!(u.contains("EnvironmentFile=-/home/u/apps/web/shared/.env\n"));
        assert!(u.contains("Environment=PORT=3010\n"));
        assert!(u.contains("StartLimitBurst=15\n") && u.contains("StartLimitIntervalSec=300\n"));
        assert!(u.contains("Restart=on-failure\n"));
        assert!(
            u.contains(r#"ExecStart=/bin/bash -lc 'echo \'$$HOME\' "50%%" \\n && ./run'"#),
            "{u}"
        );
        assert_eq!(systemd_quote("a"), "'a'");
        assert!(!unit_file("x", FsPath::new("/a"), None, "x", &user).contains("PORT"));
    }

    #[test]
    fn input_checks() {
        for ok in [
            "git@github.com:a/b.git",
            "https://github.com/a/b",
            "/home/u/repos/x.git",
            "file:///tmp/x",
        ] {
            assert!(repo_url(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-uhttps://x", "ext::sh -c id", "github.com/a b", "http://x"] {
            assert!(repo_url(bad).is_err(), "{bad}");
        }
        assert!(branch("feat/web-redesign").is_ok());
        for bad in ["-x", "a..b", "a b", "a/", ""] {
            assert!(branch(bad).is_err(), "{bad}");
        }
        assert!(start_cmd("a\nb").is_err() && start_cmd(" ").is_err() && start_cmd("./x").is_ok());
        assert_eq!(
            shared_paths(&["uploads/".into(), ".env".into()]).unwrap(),
            vec![".env", "uploads/"]
        );
        assert!(shared_paths(&["../x".into()]).is_err() && shared_paths(&["/etc".into()]).is_err());
        assert!(
            unit_safe_path(FsPath::new("/home/u/apps/a-b")).is_ok()
                && unit_safe_path(FsPath::new("/home/u/my apps/a")).is_err()
        );
    }

    #[test]
    fn detects_runtime() {
        let files = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let d = detect_runtime(&files(&["Cargo.toml", "src"]), |_| {
            Some("[package]\nname = \"radio-api-rs\"\n[dependencies]\nname = \"x\"".into())
        });
        assert_eq!((d.runtime, d.start.as_str()), ("rust", "./target/release/radio-api-rs"));
        let d = detect_runtime(&files(&["bun.lock", "package.json"]), |_| {
            Some(r#"{"scripts":{"build":"x","start":"y"}}"#.into())
        });
        assert_eq!(
            (d.runtime, d.build.as_str(), d.start.as_str()),
            ("bun", "bun install --frozen-lockfile\nbun run build", "bun run start")
        );
        let d = detect_runtime(&files(&["package.json", "package-lock.json"]), |_| Some("{}".into()));
        assert_eq!((d.runtime, d.build.as_str()), ("node", "npm ci"));
        assert_eq!(detect_runtime(&files(&["index.html"]), |_| None).runtime, "static");
        assert_eq!(detect_runtime(&files(&["README.md"]), |_| None).runtime, "other");
        assert_eq!(env_keys("# c\nA=1\nexport B=2\n\nnot a line\n1X=3"), 2);
    }

    #[test]
    fn systemctl_show_parsing() {
        let out = "Id=app-a.service\nActiveState=active\nSubState=running\nResult=success\nMainPID=42\nNRestarts=0\nExecMainStartTimestamp=@1700000000\nMemoryCurrent=8388608\nCPUUsageNSec=100\n\n\
                   Id=app-b.service\nActiveState=failed\nSubState=failed\nResult=start-limit-hit\nMainPID=0\nNRestarts=15\nExecMainStartTimestamp=\nMemoryCurrent=[not set]\nCPUUsageNSec=[not set]\n";
        let m = parse_show(out);
        let a = &m["a"];
        assert_eq!(
            (a.pid, a.started_at, a.mem_bytes),
            (Some(42), Some(1700000000), Some(8388608))
        );
        assert_eq!(status(Some(a), false), "running");
        assert_eq!(status(Some(a), true), "deploying");
        assert_eq!((m["b"].pid, m["b"].restarts, m["b"].mem_bytes), (None, 15, None));
        assert_eq!(status(Some(&m["b"]), false), "crashed");
        assert_eq!(status(None, false), "stopped");
    }

    #[test]
    fn git_errors_are_readable() {
        assert!(
            git_error("git@github.com: Permission denied (publickey).\nfatal: Could not read")
                .starts_with("Permission denied (publickey)")
        );
        assert_eq!(git_error("a\nb\nc\nfatal: d\n"), "b c fatal: d");
    }
}
