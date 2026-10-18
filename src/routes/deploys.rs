//! Phase 3 · Deploy pipeline (rencana §4), history, rollback, cancel, live log SSE, GitHub webhook.
//! Routes are relative to /api and already behind auth; `/hooks/{app}` checks its own signature.
//!
//! Pipeline: Pull (fetch + `git worktree add releases/<n>`) → Link shared → Build → Before restart
//! → Switch (`current` swapped with rename(2)) → Restart → Health check (30 s, else switch back +
//! restart = "rolled_back") → Clean up (keep N). Nothing touches the live release before Switch,
//! and cancel is only accepted until then. One deploy per app; one more may wait (a newer one
//! replaces it). Logs: `<data>/logs/<app>/<n>.log`, secrets from shared/.env masked.
//!
//! For other phases: `start` (queue a deploy), `active_for`, `env_secrets`.

use super::apps::{self, AppRow};
use crate::{
    activity,
    auth::Admin,
    exec::{self, Cmd, Exit, Line, Stream},
    http::{ApiResult, AppError, now},
    sse,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hmac::{KeyInit, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    path::{Path as FsPath, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Notify, broadcast, mpsc},
};

const HEALTH_SECS: u64 = 30;
const DISK_STOP_DEFAULT: f64 = 95.0;
const LOG_DAYS: u64 = 30;

// ---------- runtime state ----------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Deploy,
    /// Point `current` at an existing release again.
    Rollback {
        release: i64,
    },
}

struct Job {
    deploy_id: i64,
    number: i64,
    kind: Kind,
}

struct Current {
    deploy_id: i64,
    number: i64,
    started_at: i64,
    cancel: Arc<Notify>,
    cancel_requested: Arc<AtomicBool>,
    /// false once Switch starts: from there on the pipeline finishes (or rolls back) by itself.
    cancellable: Arc<AtomicBool>,
    sink: Arc<LogSink>,
}

#[derive(Default)]
struct Slot {
    cur: Option<Current>,
    queued: Option<Job>,
}

/// app name → its running deploy + the one waiting. A slot exists while its run loop is alive.
static SLOTS: LazyLock<Mutex<HashMap<String, Slot>>> = LazyLock::new(Default::default);

/// (deploy id, number) running, or else waiting, for this app.
pub fn active_for(app: &str) -> Option<(i64, i64)> {
    let m = SLOTS.lock().unwrap();
    let s = m.get(app)?;
    s.cur
        .as_ref()
        .map(|c| (c.deploy_id, c.number))
        .or(s.queued.as_ref().map(|j| (j.deploy_id, j.number)))
}

/// Append-only deploy log + live fan-out. Reading the file and subscribing happen under the same
/// lock as writing, so a follower sees every line exactly once.
struct LogSink {
    path: PathBuf,
    inner: Mutex<(std::fs::File, broadcast::Sender<String>)>,
}

impl LogSink {
    fn open(path: &FsPath) -> std::io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        Ok(LogSink {
            path: path.to_owned(),
            inner: Mutex::new((f, broadcast::channel(2048).0)),
        })
    }
    fn line(&self, s: &str) {
        let mut g = self.inner.lock().unwrap();
        let _ = writeln!(g.0, "{s}");
        let _ = g.1.send(s.to_owned());
    }
    fn follow(&self) -> (String, broadcast::Receiver<String>) {
        let g = self.inner.lock().unwrap();
        (std::fs::read_to_string(&self.path).unwrap_or_default(), g.1.subscribe())
    }
}

fn conflict(msg: impl Into<String>) -> AppError {
    AppError::Status(StatusCode::CONFLICT, msg.into())
}

// ---------- .env helpers ----------

/// KEY=value pairs of shared/.env (comments, `export`, matching quotes handled). Used as the
/// build environment. ponytail: simple parser, no multi-line values; phase 4 owns the real one.
fn dotenv(app: &AppRow) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(app.env_path()).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let l = l.strip_prefix("export ").unwrap_or(l);
            if l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            Some((k.trim().to_owned(), v.to_owned()))
        })
        .filter(|(k, _)| crate::validate::env_key(k).is_ok())
        .collect()
}

/// Values to mask in deploy/process logs: every .env value of 6+ characters that is not a plain
/// number (ports and counts stay readable). Over-masking beats leaking.
pub fn env_secrets(app: &AppRow) -> Vec<String> {
    dotenv(app)
        .into_iter()
        .map(|(_, v)| v)
        .filter(|v| v.chars().count() >= 6 && !v.chars().all(|c| c.is_ascii_digit()) && v != "production")
        .collect()
}

// ---------- disk ----------

fn disk_used_pct(p: &FsPath) -> Option<f64> {
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
    // SAFETY: statvfs fills the zeroed struct; c is a valid NUL-terminated path.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let used = (s.f_blocks - s.f_bfree) as f64;
    let total = used + s.f_bavail as f64;
    (total > 0.0).then(|| used / total * 100.0)
}

/// Some(message) when the apps disk is above the stop threshold (settings `disk_stop_pct`, default 95).
async fn disk_full(st: &AppState) -> Option<String> {
    let limit: f64 = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'disk_stop_pct'")
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DISK_STOP_DEFAULT);
    let used = disk_used_pct(&st.cfg.apps_dir)?;
    (used > limit).then(|| format!("Disk is {used:.0}% full (deploys stop above {limit:.0}%). Free some space first."))
}

// ---------- queue ----------

/// Create the deploy row (status queued) and its log file with `preface` lines (e.g. the clone).
async fn create_row(
    st: &AppState,
    app: &AppRow,
    kind: Kind,
    trigger: &str,
    actor: &str,
    preface: &[String],
) -> ApiResult<Job> {
    let number: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(number), 0) + 1 FROM deploys WHERE app_id = ?")
        .bind(app.id)
        .fetch_one(&st.db)
        .await?;
    let (release, commit, message) = match kind {
        Kind::Deploy => (number, None, None),
        Kind::Rollback { release } => {
            let r: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT commit_sha, commit_message FROM deploys WHERE app_id = ? AND release = ? AND status = 'success' ORDER BY id DESC LIMIT 1",
            )
            .bind(app.id)
            .bind(release)
            .fetch_optional(&st.db)
            .await?;
            let (c, m) = r.unwrap_or_default();
            (release, c, m)
        }
    };
    let log_path = st.cfg.logs_dir().join(&app.name).join(format!("{number}.log"));
    let sink = LogSink::open(&log_path)?;
    for l in preface {
        sink.line(l);
    }
    let deploy_id = sqlx::query(
        "INSERT INTO deploys (app_id, number, release, commit_sha, commit_message, branch, trigger, status, log_path, actor)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?)",
    )
    .bind(app.id)
    .bind(number)
    .bind(release)
    .bind(commit)
    .bind(message)
    .bind(&app.branch)
    .bind(trigger)
    .bind(log_path.to_string_lossy())
    .bind(actor)
    .execute(&st.db)
    .await?
    .last_insert_rowid();
    Ok(Job {
        deploy_id,
        number,
        kind,
    })
}

/// Queue a deploy (or rollback) for `app`. Runs now when the app is idle; otherwise it waits, and
/// replaces a deploy that was already waiting. Returns (deploy id, queued).
pub async fn start(
    st: &AppState,
    app: &AppRow,
    kind: Kind,
    trigger: &str,
    actor: &str,
    preface: &[String],
) -> ApiResult<(i64, bool)> {
    if let Some(msg) = disk_full(st).await {
        return Err(conflict(msg));
    }
    let job = create_row(st, app, kind, trigger, actor, preface).await?;
    let id = job.deploy_id;
    let (queued, replaced, run) = {
        let mut m = SLOTS.lock().unwrap();
        match m.get_mut(&app.name) {
            Some(slot) => (true, slot.queued.replace(job).map(|j| j.deploy_id), None),
            None => {
                m.insert(app.name.clone(), Slot::default());
                (false, None, Some(job))
            }
        }
    };
    if let Some(old) = replaced {
        finish_row(st, old, "cancelled", Some(&format!("Replaced by deploy id {id}."))).await;
    }
    if let Some(job) = run {
        tokio::spawn(run_loop(st.clone(), app.name.clone(), job));
    }
    let what = match kind {
        Kind::Deploy => "deploy.start",
        Kind::Rollback { .. } => "deploy.rollback",
    };
    activity::log(
        &st.db,
        actor,
        what,
        &app.name,
        &format!("{trigger}{}", if queued { " · queued" } else { "" }),
    )
    .await;
    Ok((id, queued))
}

async fn run_loop(st: AppState, app: String, first: Job) {
    let mut next = Some(first);
    while let Some(job) = next {
        let id = job.deploy_id;
        let task = tokio::spawn(run_one(st.clone(), app.clone(), job, Svc::Systemd));
        if task.await.is_err() {
            finish_row(&st, id, "failed", Some("Internal error in the deploy pipeline.")).await;
        }
        next = {
            let mut m = SLOTS.lock().unwrap();
            let slot = m.entry(app.clone()).or_default();
            slot.cur = None;
            let n = slot.queued.take();
            if n.is_none() {
                m.remove(&app);
            }
            n
        };
    }
}

async fn finish_row(st: &AppState, id: i64, status: &str, error: Option<&str>) {
    let _ = sqlx::query("UPDATE deploys SET status = ?, error = ?, finished_at = ? WHERE id = ?")
        .bind(status)
        .bind(error)
        .bind(now())
        .bind(id)
        .execute(&st.db)
        .await;
}

// ---------- pipeline ----------

/// How the pipeline talks to the process manager. Tests swap systemd for a fake whose health
/// check passes only when the live release contains a file named `healthy`.
#[derive(Clone, Copy)]
enum Svc {
    Systemd,
    #[cfg(test)]
    Fake,
}

impl Svc {
    async fn restart(self, app: &AppRow) -> Result<(), String> {
        match self {
            Svc::Systemd => {
                apps::install_unit(app).await?;
                apps::restart_app(&app.name).await
            }
            #[cfg(test)]
            Svc::Fake => Ok(()),
        }
    }

    /// 2xx from GET 127.0.0.1:<port><health_path> within 30 s (or, without a path, the unit still
    /// active after 5 s).
    async fn health(self, app: &AppRow) -> Result<(), String> {
        match self {
            Svc::Systemd => {
                let deadline = Instant::now() + Duration::from_secs(HEALTH_SECS);
                if app.health_path.is_empty() {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let out = apps::systemctl(&["is-active", &apps::unit_name(&app.name)]).await;
                    return out
                        .map(|_| ())
                        .map_err(|e| format!("Process is not running ({}).", e.trim()));
                }
                let port = app.port.unwrap_or(0) as u16;
                let mut last = "no answer".to_owned();
                loop {
                    match http_status(port, &app.health_path).await {
                        Some(c) if (200..300).contains(&c) => return Ok(()),
                        Some(c) => last = format!("HTTP {c}"),
                        None => {}
                    }
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "GET :{port}{} gave {last} for {HEALTH_SECS} s.",
                            app.health_path
                        ));
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            #[cfg(test)]
            Svc::Fake => {
                if app.dir().join("current/healthy").exists() {
                    Ok(())
                } else {
                    Err("fake: unhealthy".into())
                }
            }
        }
    }
}

/// Status code of a plain HTTP/1.0 GET, or None when nothing answers within 3 s.
async fn http_status(port: u16, path: &str) -> Option<u16> {
    let t = Duration::from_secs(3);
    let mut s = tokio::time::timeout(t, tokio::net::TcpStream::connect(("127.0.0.1", port)))
        .await
        .ok()?
        .ok()?;
    let req = format!(
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: server-manager\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    while !buf.contains(&b'\n') && buf.len() < 4096 {
        let n = tokio::time::timeout(t, s.read(&mut chunk)).await.ok()?.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf).split_whitespace().nth(1)?.parse().ok()
}

/// `current` → `releases/<n>`, swapped atomically (symlink to a temp name, then rename over).
fn switch(dir: &FsPath, release: i64) -> std::io::Result<()> {
    let tmp = dir.join("current.tmp");
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(format!("releases/{release}"), &tmp)?;
    std::fs::rename(&tmp, dir.join("current"))
}

fn live_release(dir: &FsPath) -> Option<i64> {
    std::fs::read_link(dir.join("current"))
        .ok()?
        .to_str()?
        .strip_prefix("releases/")?
        .parse()
        .ok()
}

struct Fail {
    status: &'static str,
    error: String,
}

fn fail(error: impl Into<String>) -> Fail {
    Fail {
        status: "failed",
        error: error.into(),
    }
}

struct Ctx {
    st: AppState,
    app: AppRow,
    deploy_id: i64,
    number: i64,
    sink: Arc<LogSink>,
    cancel: Arc<Notify>,
    cancel_requested: Arc<AtomicBool>,
    cancellable: Arc<AtomicBool>,
    secrets: Vec<String>,
    /// apps.run_as, resolved: builds run as this user.
    user: exec::RunAs,
    steps: Vec<&'static str>,
    svc: Svc,
}

fn step_plan(app: &AppRow, kind: Kind, number: i64) -> Vec<(&'static str, String)> {
    let first_line = |s: &str| {
        let mut l = s.lines();
        let f = l.next().unwrap_or("").to_owned();
        let more = l.count();
        if more > 0 { format!("{f} (+{more})") } else { f }
    };
    let health = if app.health_path.is_empty() {
        format!("systemctl is-active {}", apps::unit_name(&app.name))
    } else {
        format!("GET :{}{}", app.port.unwrap_or(0), app.health_path)
    };
    let restart = format!("systemctl restart {}", apps::unit_name(&app.name));
    match kind {
        Kind::Deploy => vec![
            ("Pull", format!("git fetch origin {}", app.branch)),
            ("Link shared", app.shared().join(", ")),
            ("Build", first_line(&app.build_cmd)),
            ("Before restart", first_line(&app.before_restart_cmd)),
            ("Switch", format!("releases/{number} → current")),
            ("Restart", restart),
            ("Health check", health),
            ("Clean up", format!("keep {} releases", app.keep_releases)),
        ],
        Kind::Rollback { release } => vec![
            ("Switch", format!("releases/{release} → current")),
            ("Restart", restart),
            ("Health check", health),
        ],
    }
}

impl Ctx {
    fn log(&self, s: &str) {
        self.sink.line(s);
    }

    fn idx(&self, step: &str) -> i64 {
        self.steps.iter().position(|s| *s == step).unwrap_or(0) as i64
    }

    async fn mark(&self, step: &str, status: &str, code: Option<i32>) {
        let col = if status == "running" {
            "started_at"
        } else {
            "finished_at"
        };
        let _ = sqlx::query(&format!(
            "UPDATE deploy_steps SET status = ?, exit_code = ?, {col} = ? WHERE deploy_id = ? AND idx = ?"
        ))
        .bind(status)
        .bind(code)
        .bind(now())
        .bind(self.deploy_id)
        .bind(self.idx(step))
        .execute(&self.st.db)
        .await;
    }

    fn check_cancel(&self) -> Result<(), Fail> {
        if self.cancel_requested.load(Ordering::SeqCst) {
            Err(Fail {
                status: "cancelled",
                error: "Cancelled.".into(),
            })
        } else {
            Ok(())
        }
    }

    /// Run one command of `step`, streaming its lines into the log.
    async fn exec(&self, step: &str, cmd: Cmd) -> Result<(), Fail> {
        self.log(&format!("$ {}", cmd.display()));
        let (tx, mut rx) = mpsc::channel::<Line>(256);
        let pump = async {
            while let Some(l) = rx.recv().await {
                self.sink.line(&l.text);
            }
        };
        let (exit, ()) = tokio::join!(
            cmd.redact(self.secrets.clone())
                .cancel_on(self.cancel.clone())
                .stream(tx),
            pump
        );
        match exit {
            Ok(Exit::Code(0)) => Ok(()),
            Ok(Exit::Cancelled) => {
                self.mark(step, "cancelled", None).await;
                Err(Fail {
                    status: "cancelled",
                    error: format!("Cancelled during {step}."),
                })
            }
            Ok(Exit::TimedOut) => {
                self.mark(step, "failed", None).await;
                Err(fail(format!("{step} timed out.")))
            }
            Ok(Exit::Code(c)) => {
                self.mark(step, "failed", Some(c)).await;
                Err(fail(format!("{step} failed: exit code {c}.")))
            }
            Ok(Exit::Signal(s)) => {
                self.mark(step, "failed", None).await;
                Err(fail(format!("{step} killed by signal {s}.")))
            }
            Err(e) => {
                self.mark(step, "failed", None).await;
                Err(fail(format!("{step}: could not start: {e}")))
            }
        }
    }

    async fn user_script(&self, step: &str, script: &str, cwd: &FsPath) -> Result<(), Fail> {
        if script.trim().is_empty() {
            self.mark(step, "skipped", None).await;
            return Ok(());
        }
        self.mark(step, "running", None).await;
        let cmd = exec::run_shell_user_command(&format!("set -e\n{script}"), Some(&self.user))
            .cwd(cwd)
            .timeout(Duration::from_secs(self.app.build_timeout_secs.max(60) as u64));
        let cmd = dotenv(&self.app).into_iter().fold(cmd, |c, (k, v)| c.env(k, v));
        self.exec(step, cmd).await?;
        self.mark(step, "success", Some(0)).await;
        Ok(())
    }

    /// Steps 1-4: new release in releases/<n>, built. The live release is untouched.
    async fn prepare(&self) -> Result<(), Fail> {
        let dir = self.app.dir();
        let repo = dir.join("repo").to_string_lossy().to_string();
        let rel = dir.join("releases").join(self.number.to_string());
        let rel_s = rel.to_string_lossy().to_string();
        let b = &self.app.branch;

        self.mark("Pull", "running", None).await;
        let refspec = format!("+refs/heads/{b}:refs/remotes/origin/{b}");
        self.exec(
            "Pull",
            apps::git(&self.st)
                .args(["-C", &repo, "fetch", "--no-tags", "origin", &refspec])
                .timeout(Duration::from_secs(300)),
        )
        .await?;
        let o = apps::git(&self.st)
            .args([
                "-C",
                &repo,
                "log",
                "-1",
                "--format=%H%x1f%s",
                &format!("refs/remotes/origin/{b}"),
            ])
            .output()
            .await
            .map_err(|e| fail(format!("git log: {e}")))?;
        let (sha, msg) = o
            .stdout
            .trim()
            .split_once('\x1f')
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .ok_or_else(|| fail("Branch not found after fetch."))?;
        let _ = sqlx::query("UPDATE deploys SET commit_sha = ?, commit_message = ? WHERE id = ?")
            .bind(&sha)
            .bind(&msg)
            .bind(self.deploy_id)
            .execute(&self.st.db)
            .await;
        if rel.exists() {
            std::fs::remove_dir_all(&rel).map_err(|e| fail(format!("remove old {rel_s}: {e}")))?;
        }
        let _ = apps::git(&self.st)
            .args(["-C", &repo, "worktree", "prune"])
            .output()
            .await;
        self.exec(
            "Pull",
            apps::git(&self.st).args(["-C", &repo, "worktree", "add", "--detach", "--force", &rel_s, &sha]),
        )
        .await?;
        for d in [dir.join("repo"), rel.clone()] {
            apps::chown_r(&self.user, &d)
                .await
                .map_err(|e| fail(format!("Pull: {e}")))?;
        }
        self.mark("Pull", "success", Some(0)).await;
        self.check_cancel()?;

        self.mark("Link shared", "running", None).await;
        for p in self.app.shared() {
            let name = p.trim_end_matches('/');
            let src = dir.join("shared").join(name);
            let dst = rel.join(name);
            let r = (|| -> std::io::Result<()> {
                if !src.exists() {
                    if let Some(parent) = src.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    if dst.exists() {
                        std::fs::rename(&dst, &src)?; // first deploy: seed shared/ from the repo
                    } else if p.ends_with('/') {
                        std::fs::create_dir_all(&src)?;
                    } else {
                        std::fs::write(&src, "")?;
                    }
                }
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                match std::fs::symlink_metadata(&dst) {
                    Ok(m) if m.is_dir() => std::fs::remove_dir_all(&dst)?,
                    Ok(_) => std::fs::remove_file(&dst)?,
                    Err(_) => {}
                }
                std::os::unix::fs::symlink(&src, &dst)
            })();
            match r {
                Ok(()) => self.log(&format!("linked {name} → shared/{name}")),
                Err(e) => {
                    self.mark("Link shared", "failed", None).await;
                    return Err(fail(format!("Link shared {name}: {e}")));
                }
            }
        }
        if let Err(e) = apps::chown_r(&self.user, &dir.join("shared")).await {
            self.mark("Link shared", "failed", None).await;
            return Err(fail(format!("Link shared: {e}")));
        }
        self.mark("Link shared", "success", None).await;
        self.check_cancel()?;

        self.user_script("Build", &self.app.build_cmd, &rel).await?;
        self.check_cancel()?;
        self.user_script("Before restart", &self.app.before_restart_cmd, &rel)
            .await?;
        self.check_cancel()
    }

    /// Steps 5-7 for `target`. On a failed restart / health check, `current` goes back to `prev`.
    async fn go_live(&self, target: i64, prev: Option<i64>) -> Result<(), Fail> {
        let dir = self.app.dir();
        self.mark("Switch", "running", None).await;
        if let Err(e) = switch(&dir, target) {
            self.mark("Switch", "failed", None).await;
            return Err(fail(format!("Switch: {e}")));
        }
        self.log(&format!("current → releases/{target}"));
        self.mark("Switch", "success", None).await;

        self.mark("Restart", "running", None).await;
        self.log(&format!("$ systemctl restart {}", apps::unit_name(&self.app.name)));
        let r = match self.svc.restart(&self.app).await {
            Ok(()) => {
                self.mark("Restart", "success", None).await;
                self.mark("Health check", "running", None).await;
                self.log(&format!(
                    "health check: {}",
                    step_plan(&self.app, Kind::Rollback { release: 0 }, 0)[2].1
                ));
                let h = self.svc.health(&self.app).await;
                self.mark("Health check", if h.is_ok() { "success" } else { "failed" }, None)
                    .await;
                h.map_err(|e| format!("Health check failed: {e}"))
            }
            Err(e) => {
                self.mark("Restart", "failed", None).await;
                Err(format!("Restart failed: {e}"))
            }
        };
        let Err(err) = r else {
            self.log("healthy");
            let _ = sqlx::query("UPDATE apps SET current_release = ? WHERE id = ?")
                .bind(target)
                .bind(self.app.id)
                .execute(&self.st.db)
                .await;
            return Ok(());
        };
        self.log(&format!("error: {err}"));
        match prev.filter(|p| *p != target && dir.join("releases").join(p.to_string()).is_dir()) {
            Some(p) => {
                self.log(&format!("rolling back: current → releases/{p}"));
                let back = switch(&dir, p).map_err(|e| e.to_string());
                let back = match back {
                    Ok(()) => self.svc.restart(&self.app).await,
                    Err(e) => Err(e),
                };
                let _ = sqlx::query("UPDATE apps SET current_release = ? WHERE id = ?")
                    .bind(p)
                    .bind(self.app.id)
                    .execute(&self.st.db)
                    .await;
                match back {
                    Ok(()) => self.log(&format!("release #{p} is live again")),
                    Err(e) => self.log(&format!("error: rollback restart failed: {e}")),
                }
                Err(Fail {
                    status: "rolled_back",
                    error: format!("{err} Rolled back to release #{p}."),
                })
            }
            None => Err(fail(err)),
        }
    }

    /// Keep the newest `keep_releases` (plus live), remove the rest, prune worktrees.
    async fn clean_up(&self) {
        self.mark("Clean up", "running", None).await;
        let dir = self.app.dir();
        let live = live_release(&dir);
        let mut nums: Vec<i64> = std::fs::read_dir(dir.join("releases"))
            .map(|d| d.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).collect())
            .unwrap_or_default();
        nums.sort_unstable_by(|a, b| b.cmp(a));
        for n in nums.into_iter().skip(self.app.keep_releases.max(1) as usize) {
            if Some(n) != live && std::fs::remove_dir_all(dir.join("releases").join(n.to_string())).is_ok() {
                self.log(&format!("removed releases/{n}"));
            }
        }
        let _ = apps::git(&self.st)
            .args(["-C", &dir.join("repo").to_string_lossy(), "worktree", "prune"])
            .output()
            .await;
        self.mark("Clean up", "success", None).await;
    }

    async fn run(&self, kind: Kind) -> Result<(), Fail> {
        let dir = self.app.dir();
        let prev = live_release(&dir);
        match kind {
            Kind::Deploy => {
                if let Err(f) = self.prepare().await {
                    // The half-built release is never live: drop it.
                    let rel = dir.join("releases").join(self.number.to_string());
                    std::fs::remove_dir_all(&rel).ok();
                    let _ = apps::git(&self.st)
                        .args(["-C", &dir.join("repo").to_string_lossy(), "worktree", "prune"])
                        .output()
                        .await;
                    if prev.is_some() {
                        self.log(&format!(
                            "release #{} left running, nothing was restarted",
                            prev.unwrap_or(0)
                        ));
                    }
                    return Err(f);
                }
                self.cancellable.store(false, Ordering::SeqCst);
                self.check_cancel()?;
                self.go_live(self.number, prev).await?;
                self.clean_up().await;
                Ok(())
            }
            Kind::Rollback { release } => {
                self.cancellable.store(false, Ordering::SeqCst);
                if !dir.join("releases").join(release.to_string()).is_dir() {
                    self.mark("Switch", "failed", None).await;
                    return Err(fail(format!("releases/{release} no longer exists.")));
                }
                self.go_live(release, prev).await
            }
        }
    }
}

async fn run_one(st: AppState, app_name: String, job: Job, svc: Svc) {
    let Ok(app) = apps::load(&st, &app_name).await else {
        return finish_row(&st, job.deploy_id, "failed", Some("App was deleted.")).await;
    };
    let path = st.cfg.logs_dir().join(&app.name).join(format!("{}.log", job.number));
    let sink = match LogSink::open(&path) {
        Ok(s) => Arc::new(s),
        Err(e) => return finish_row(&st, job.deploy_id, "failed", Some(&format!("log file: {e}"))).await,
    };
    let user = match apps::lookup_user(&app.run_as).await {
        Ok(u) => u,
        Err(_) => {
            let msg = format!("Run as user \"{}\" does not exist on this server.", app.run_as);
            sink.line(&format!("error: {msg}"));
            return finish_row(&st, job.deploy_id, "failed", Some(&msg)).await;
        }
    };
    let plan = step_plan(&app, job.kind, job.number);
    let ctx = Ctx {
        user,
        secrets: env_secrets(&app),
        st: st.clone(),
        deploy_id: job.deploy_id,
        number: job.number,
        sink: sink.clone(),
        cancel: Arc::new(Notify::new()),
        cancel_requested: Arc::new(AtomicBool::new(false)),
        cancellable: Arc::new(AtomicBool::new(true)),
        steps: plan.iter().map(|(n, _)| *n).collect(),
        svc,
        app,
    };
    let started = now();
    SLOTS.lock().unwrap().entry(app_name.clone()).or_default().cur = Some(Current {
        deploy_id: job.deploy_id,
        number: job.number,
        started_at: started,
        cancel: ctx.cancel.clone(),
        cancel_requested: ctx.cancel_requested.clone(),
        cancellable: ctx.cancellable.clone(),
        sink,
    });
    let _ = sqlx::query("UPDATE deploys SET status = 'running', started_at = ? WHERE id = ?")
        .bind(started)
        .bind(job.deploy_id)
        .execute(&st.db)
        .await;
    for (i, (name, cmd)) in plan.iter().enumerate() {
        let _ = sqlx::query("INSERT OR IGNORE INTO deploy_steps (deploy_id, idx, name, command) VALUES (?, ?, ?, ?)")
            .bind(job.deploy_id)
            .bind(i as i64)
            .bind(name)
            .bind(cmd)
            .execute(&st.db)
            .await;
    }
    ctx.log(&format!(
        "deploy #{} of {} ({})",
        job.number, ctx.app.name, ctx.app.branch
    ));
    let (status, error) = match ctx.run(job.kind).await {
        Ok(()) => ("success", None),
        Err(f) => (f.status, Some(f.error)),
    };
    let took = now() - started;
    ctx.log(&match &error {
        None => format!("deploy #{} succeeded in {took} s", job.number),
        Some(e) => format!("deploy #{} {}: {e}", job.number, status.replace('_', " ")),
    });
    finish_row(&st, job.deploy_id, status, error.as_deref()).await;
    activity::log(
        &st.db,
        "system",
        "deploy.finish",
        &ctx.app.name,
        &format!("#{} {status}", job.number),
    )
    .await;
}

// ---------- API ----------

const DEPLOY_COLS: &str = "id, number, release, commit_sha, commit_message, branch, trigger, status, error, actor, created_at, started_at, finished_at";

type DeployTuple = (
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<i64>,
    Option<i64>,
);

/// Deploy rows as API JSON. `filter` is a fixed SQL tail from this crate, `binds` its integer args.
pub async fn deploy_json(st: &AppState, filter: &str, binds: &[i64]) -> ApiResult<Vec<Value>> {
    let sql = format!("SELECT {DEPLOY_COLS} FROM deploys {filter}");
    let mut q = sqlx::query_as::<_, DeployTuple>(&sql);
    for b in binds {
        q = q.bind(b);
    }
    Ok(q.fetch_all(&st.db)
        .await?
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0, "number": r.1, "release": r.2, "commit": r.3.as_deref().map(|s| &s[..s.len().min(7)]),
                "commitFull": r.3, "message": r.4, "branch": r.5, "trigger": r.6, "status": r.7, "error": r.8,
                "actor": r.9, "createdAt": r.10, "startedAt": r.11, "finishedAt": r.12,
            })
        })
        .collect())
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

/// GET /apps/{name}/deploys?limit=20 (newest first).
async fn list(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let a = apps::load(&st, &name).await?;
    let rows = deploy_json(
        &st,
        "WHERE app_id = ? ORDER BY id DESC LIMIT ?",
        &[a.id, q.limit.unwrap_or(20).clamp(1, 200)],
    )
    .await?;
    let on_disk: Vec<i64> = std::fs::read_dir(a.dir().join("releases"))
        .map(|d| d.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).collect())
        .unwrap_or_default();
    Ok(Json(
        json!({ "data": rows, "releasesOnDisk": on_disk, "currentRelease": a.current_release }),
    ))
}

async fn find(st: &AppState, name: &str, id: i64) -> ApiResult<(AppRow, Value)> {
    let a = apps::load(st, name).await?;
    let d = deploy_json(st, "WHERE app_id = ? AND id = ?", &[a.id, id])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::not_found("Deploy not found."))?;
    Ok((a, d))
}

/// GET /apps/{name}/deploys/{id} → deploy + steps.
async fn detail(State(st): State<AppState>, Path((name, id)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let (_, mut d) = find(&st, &name, id).await?;
    type StepTuple = (i64, String, String, String, Option<i64>, Option<i64>, Option<i64>);
    let steps: Vec<StepTuple> = sqlx::query_as(
        "SELECT idx, name, command, status, exit_code, started_at, finished_at FROM deploy_steps WHERE deploy_id = ? ORDER BY idx",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await?;
    d["steps"] = steps
        .into_iter()
        .map(|s| json!({ "idx": s.0, "name": s.1, "command": s.2, "status": s.3, "exitCode": s.4, "startedAt": s.5, "finishedAt": s.6 }))
        .collect();
    let cancellable = {
        let m = SLOTS.lock().unwrap();
        m.get(&name).is_some_and(|s| {
            s.cur
                .as_ref()
                .is_some_and(|c| c.deploy_id == id && c.cancellable.load(Ordering::SeqCst))
                || s.queued.as_ref().is_some_and(|j| j.deploy_id == id)
        })
    };
    d["cancellable"] = json!(cancellable);
    Ok(Json(json!({ "data": d })))
}

/// POST /apps/{name}/deploys → deploy the branch head now (or queue behind the running one).
async fn create(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    let a = apps::load(&st, &name).await?;
    let (id, queued) = start(&st, &a, Kind::Deploy, "manual", &admin.username, &[]).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "data": { "id": id, "queued": queued } })),
    )
        .into_response())
}

/// Ok(true) cancelled, Ok(false) too late (already switching), Err = not running.
fn cancel_deploy(app: &str, id: i64) -> Result<bool, ()> {
    let mut m = SLOTS.lock().unwrap();
    let slot = m.get_mut(app).ok_or(())?;
    if slot.queued.as_ref().is_some_and(|j| j.deploy_id == id) {
        slot.queued = None;
        return Ok(true);
    }
    let c = slot.cur.as_ref().filter(|c| c.deploy_id == id).ok_or(())?;
    if !c.cancellable.load(Ordering::SeqCst) {
        return Ok(false);
    }
    c.cancel_requested.store(true, Ordering::SeqCst);
    c.cancel.notify_one();
    Ok(true)
}

/// POST /apps/{name}/deploys/{id}/cancel: kills the running step's process group.
async fn cancel(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Path((name, id)): Path<(String, i64)>,
) -> ApiResult<Json<Value>> {
    let (a, d) = find(&st, &name, id).await?;
    match cancel_deploy(&a.name, id) {
        Ok(true) => {
            if d["status"] == "queued" {
                finish_row(&st, id, "cancelled", Some("Cancelled before it started.")).await;
            }
            activity::log(
                &st.db,
                &admin.username,
                "deploy.cancel",
                &a.name,
                &format!("#{}", d["number"]),
            )
            .await;
            Ok(Json(json!({ "data": { "ok": true } })))
        }
        Ok(false) => Err(conflict(
            "Too late to cancel: the new release is already being switched in.",
        )),
        Err(()) => Err(conflict("This deploy is not running.")),
    }
}

/// POST /apps/{name}/deploys/{id}/rollback: make that deploy's release live again.
async fn rollback(
    State(st): State<AppState>,
    Extension(admin): Extension<Admin>,
    Path((name, id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    let (a, d) = find(&st, &name, id).await?;
    let release = d["release"].as_i64().unwrap_or(0);
    if d["status"] != "success" {
        return Err(AppError::bad_request("Only a successful deploy can be rolled back to."));
    }
    if Some(release) == a.current_release {
        return Err(AppError::bad_request(format!("Release #{release} is already live.")));
    }
    if !a.dir().join("releases").join(release.to_string()).is_dir() {
        return Err(AppError::bad_request(format!(
            "Release #{release} was cleaned up and can't be restored."
        )));
    }
    let (id, queued) = start(&st, &a, Kind::Rollback { release }, "rollback", &admin.username, &[]).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "data": { "id": id, "queued": queued } })),
    )
        .into_response())
}

/// GET /apps/{name}/deploys/{id}/log → SSE: replay the log file, then follow while it runs.
/// Ends with `done` = {"code":0} on success, else {"code":null,"reason":<status>}.
async fn log(State(st): State<AppState>, Path((name, id)): Path<(String, i64)>) -> ApiResult<impl IntoResponse> {
    let (a, d) = find(&st, &name, id).await?;
    let path = d["number"]
        .as_i64()
        .map(|n| st.cfg.logs_dir().join(&a.name).join(format!("{n}.log")))
        .unwrap_or_default();
    let (tx, rx) = mpsc::channel(512);
    tokio::spawn(async move {
        let send = |t: String| {
            let tx = tx.clone();
            async move {
                tx.send(sse::line_event(&Line {
                    stream: Stream::Out,
                    text: t,
                }))
                .await
                .is_ok()
            }
        };
        'outer: loop {
            let sink = SLOTS
                .lock()
                .unwrap()
                .get(&a.name)
                .and_then(|s| s.cur.as_ref().filter(|c| c.deploy_id == id).map(|c| c.sink.clone()));
            if let Some(sink) = sink {
                let (text, mut follow) = sink.follow();
                drop(sink);
                for l in text.lines() {
                    if !send(l.to_owned()).await {
                        return;
                    }
                }
                loop {
                    match follow.recv().await {
                        Ok(l) => {
                            if !send(l).await {
                                return;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break 'outer,
                    }
                }
            }
            let status: Option<String> = sqlx::query_scalar("SELECT status FROM deploys WHERE id = ?")
                .bind(id)
                .fetch_optional(&st.db)
                .await
                .ok()
                .flatten();
            if status.as_deref() == Some("queued") {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            for l in std::fs::read_to_string(&path).unwrap_or_default().lines() {
                if !send(l.to_owned()).await {
                    return;
                }
            }
            break;
        }
        let status: String = sqlx::query_scalar("SELECT status FROM deploys WHERE id = ?")
            .bind(id)
            .fetch_one(&st.db)
            .await
            .unwrap_or_default();
        let data = if status == "success" {
            json!({ "code": 0 })
        } else {
            json!({ "code": null, "reason": status })
        };
        tx.send(
            axum::response::sse::Event::default()
                .event("done")
                .data(data.to_string()),
        )
        .await
        .ok();
    });
    Ok(sse::from_channel(rx))
}

/// GET /deploys/active → running deploys (phase 2's topbar).
async fn active() -> Json<Value> {
    let m = SLOTS.lock().unwrap();
    let mut data: Vec<Value> = m
        .iter()
        .filter_map(|(app, s)| {
            s.cur
                .as_ref()
                .map(|c| json!({ "app": app, "deployId": c.deploy_id, "number": c.number, "startedAt": c.started_at }))
        })
        .collect();
    data.sort_by_key(|v| v["startedAt"].as_i64());
    Json(json!({ "data": data }))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/deploys/active", get(active))
        .route("/apps/{name}/deploys", get(list).post(create))
        .route("/apps/{name}/deploys/{id}", get(detail))
        .route("/apps/{name}/deploys/{id}/cancel", post(cancel))
        .route("/apps/{name}/deploys/{id}/rollback", post(rollback))
        .route("/apps/{name}/deploys/{id}/log", get(log))
}

// ---------- webhook ----------

/// `X-Hub-Signature-256: sha256=<hex>` = HMAC-SHA256(secret, body), compared in constant time.
pub fn verify_signature(secret: &[u8], body: &[u8], header: &str) -> bool {
    let Some(sig) = header.strip_prefix("sha256=").and_then(|h| hex::decode(h.trim()).ok()) else {
        return false;
    };
    let Ok(mut mac) = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

fn random_hex(n: usize) -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut b = vec![0u8; n];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

/// The global webhook secret (settings `webhook_secret`), created on first use.
pub async fn webhook_secret(st: &AppState) -> Result<String, sqlx::Error> {
    if let Some(s) = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'webhook_secret'")
        .fetch_optional(&st.db)
        .await?
    {
        return Ok(s);
    }
    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('webhook_secret', ?)")
        .bind(random_hex(32))
        .execute(&st.db)
        .await?;
    sqlx::query_scalar("SELECT value FROM settings WHERE key = 'webhook_secret'")
        .fetch_one(&st.db)
        .await
}

fn reply(code: StatusCode, v: Value) -> Response {
    (code, Json(v)).into_response()
}

/// POST /hooks/{app}: GitHub push → deploy when auto-deploy is on and the branch matches. 202 fast.
async fn hook(State(st): State<AppState>, Path(name): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(a) = apps::load(&st, &name).await else {
        return reply(StatusCode::NOT_FOUND, json!({ "error": "Unknown app." }));
    };
    let secret = match a.webhook_secret.clone().filter(|s| !s.is_empty()) {
        Some(s) => s,
        None => match webhook_secret(&st).await {
            Ok(s) => s,
            Err(e) => return AppError::from(e).into_response(),
        },
    };
    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_signature(secret.as_bytes(), &body, sig) {
        return reply(StatusCode::UNAUTHORIZED, json!({ "error": "Bad signature." }));
    }
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if event == "ping" {
        return reply(StatusCode::OK, json!({ "ok": true, "pong": true }));
    }
    let r#ref = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["ref"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let ignored = if event != "push" {
        Some(format!("event {event} ignored"))
    } else if r#ref != format!("refs/heads/{}", a.branch) {
        Some(format!("push to {ref} ignored, deploy branch is {}", a.branch, ref = r#ref))
    } else if !a.auto_deploy {
        Some("auto-deploy is off".to_owned())
    } else {
        None
    };
    if let Some(why) = ignored {
        return reply(StatusCode::ACCEPTED, json!({ "ok": true, "ignored": why }));
    }
    match start(&st, &a, Kind::Deploy, "webhook", "webhook", &[]).await {
        Ok((id, queued)) => reply(
            StatusCode::ACCEPTED,
            json!({ "ok": true, "deployId": id, "queued": queued }),
        ),
        Err(e) => e.into_response(),
    }
}

/// Outside /api and without auth: `/hooks/{app}` verifies X-Hub-Signature-256 itself.
pub fn public_routes() -> Router<AppState> {
    Router::new().route("/hooks/{app}", post(hook))
}

// ---------- boot ----------

/// Deploys cut off by a manager restart are marked failed; webhook secret exists; logs older
/// than 30 days are removed once a day.
pub fn spawn(st: &AppState) {
    let st = st.clone();
    tokio::spawn(async move {
        let msg = "Server Manager restarted during this deploy.";
        let _ = sqlx::query(
            "UPDATE deploys SET status = 'failed', error = ?, finished_at = ? WHERE status IN ('queued', 'running')",
        )
        .bind(msg)
        .bind(now())
        .execute(&st.db)
        .await;
        let _ = sqlx::query("UPDATE deploy_steps SET status = 'failed' WHERE status = 'running'")
            .execute(&st.db)
            .await;
        if let Err(e) = webhook_secret(&st).await {
            eprintln!("[deploys] webhook secret: {e}");
        }
        loop {
            let cutoff = std::time::SystemTime::now() - Duration::from_secs(LOG_DAYS * 86400);
            for app in std::fs::read_dir(st.cfg.logs_dir()).into_iter().flatten().flatten() {
                for f in std::fs::read_dir(app.path()).into_iter().flatten().flatten() {
                    let old = f.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff);
                    if old && f.path().extension().is_some_and(|e| e == "log") {
                        std::fs::remove_file(f.path()).ok();
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(86400)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn webhook_signature() {
        let body = br#"{"ref":"refs/heads/main"}"#;
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(b"s3cret").unwrap();
        mac.update(body);
        let good = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_signature(b"s3cret", body, &good));
        assert!(!verify_signature(b"other", body, &good));
        assert!(!verify_signature(b"s3cret", b"{}", &good));
        assert!(!verify_signature(b"s3cret", body, &good.replace("sha256=", "sha1=")));
        assert!(!verify_signature(b"s3cret", body, "sha256=zz"));
        assert!(!verify_signature(b"s3cret", body, ""));
    }

    async fn sh(dir: &FsPath, script: &str) {
        let o = Cmd::new("sh").args(["-c", script]).cwd(dir).output().await.unwrap();
        assert!(o.exit.success(), "{script}: {}", o.stderr);
    }

    async fn status_of(st: &AppState, id: i64) -> (String, Option<String>) {
        sqlx::query_as("SELECT status, error FROM deploys WHERE id = ?")
            .bind(id)
            .fetch_one(&st.db)
            .await
            .unwrap()
    }

    async fn set_build(st: &AppState, build: &str) -> AppRow {
        sqlx::query("UPDATE apps SET build_cmd = ?")
            .bind(build)
            .execute(&st.db)
            .await
            .unwrap();
        apps::load(st, "demo").await.unwrap()
    }

    /// Real git + shell + temp dirs, fake systemd: success, failed build, health-check rollback,
    /// cancel, manual rollback, keep-N clean up.
    #[tokio::test]
    async fn pipeline_states() {
        let tmp = std::env::temp_dir().join(format!("sm-deploy-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (src, apps_dir, data) = (tmp.join("src"), tmp.join("apps"), tmp.join("data"));
        for d in [&src, &apps_dir, &data] {
            std::fs::create_dir_all(d).unwrap();
        }
        sh(
            &src,
            "git init -q -b main && git -c user.email=t@t -c user.name=t commit -q --allow-empty -m first",
        )
        .await;
        let app_dir = apps_dir.join("demo");
        std::fs::create_dir_all(app_dir.join("shared")).unwrap();
        std::fs::write(app_dir.join("shared/.env"), "TOKEN=abcdef123456\nPORT=3999\n").unwrap();
        sh(&app_dir, &format!("git clone -q --bare -b main {} repo", src.display())).await;

        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        let cfg = Config {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir: data.clone(),
            apps_dir: apps_dir.clone(),
            web_dist: "x".into(),
            trust_proxy: false,
        };
        let st = AppState::new(db, cfg, vec![0; 32]);
        sqlx::query("INSERT INTO apps (name, repo_url, branch, path, start_cmd, port, keep_releases) VALUES ('demo', ?, 'main', ?, 'true', 3999, 2)")
            .bind(src.to_string_lossy())
            .bind(app_dir.to_string_lossy())
            .execute(&st.db)
            .await
            .unwrap();

        let run = |kind: Kind| {
            let st = st.clone();
            async move {
                let app = apps::load(&st, "demo").await.unwrap();
                let job = create_row(&st, &app, kind, "manual", "test", &[]).await.unwrap();
                let id = job.deploy_id;
                run_one(st.clone(), "demo".into(), job, Svc::Fake).await;
                SLOTS.lock().unwrap().remove("demo");
                (id, status_of(&st, id).await)
            }
        };

        // #1 healthy → live, secret masked in the log.
        set_build(&st, "touch healthy\necho token=$TOKEN").await;
        let (id1, (s, _)) = run(Kind::Deploy).await;
        assert_eq!(s, "success");
        assert_eq!(live_release(&app_dir), Some(1));
        assert!(
            std::fs::read_link(app_dir.join("releases/1/.env")).is_ok(),
            ".env linked"
        );
        let log1 = std::fs::read_to_string(data.join("logs/demo/1.log")).unwrap();
        assert!(
            log1.contains("token=••••••") && !log1.contains("abcdef123456"),
            "{log1}"
        );

        // #2 build fails → failed, #1 stays live, releases/2 removed.
        set_build(&st, "echo building\nexit 3\necho never").await;
        let (_, (s, e)) = run(Kind::Deploy).await;
        assert_eq!(s, "failed");
        assert!(e.unwrap().contains("exit code 3"));
        assert_eq!(live_release(&app_dir), Some(1));
        assert!(!app_dir.join("releases/2").exists());
        let steps: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, status FROM deploy_steps WHERE deploy_id = (SELECT max(id) FROM deploys) ORDER BY idx",
        )
        .fetch_all(&st.db)
        .await
        .unwrap();
        assert_eq!(steps[2], ("Build".into(), "failed".into()));
        assert_eq!(steps[4], ("Switch".into(), "waiting".into()));

        // #3 builds but is unhealthy → switched back to #1 = rolled_back.
        set_build(&st, "true").await;
        let (_, (s, e)) = run(Kind::Deploy).await;
        assert_eq!(s, "rolled_back", "{e:?}");
        assert_eq!(live_release(&app_dir), Some(1));
        let cur: Option<i64> = sqlx::query_scalar("SELECT current_release FROM apps")
            .fetch_one(&st.db)
            .await
            .unwrap();
        assert_eq!(cur, Some(1));

        // #4 cancelled during a long build → cancelled, quickly, #1 still live.
        set_build(&st, "sleep 30").await;
        let app = apps::load(&st, "demo").await.unwrap();
        let job = create_row(&st, &app, Kind::Deploy, "manual", "test", &[])
            .await
            .unwrap();
        let id4 = job.deploy_id;
        let t = Instant::now();
        let h = tokio::spawn(run_one(st.clone(), "demo".into(), job, Svc::Fake));
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let build: Option<String> =
                sqlx::query_scalar("SELECT status FROM deploy_steps WHERE deploy_id = ? AND name = 'Build'")
                    .bind(id4)
                    .fetch_optional(&st.db)
                    .await
                    .unwrap();
            if build.as_deref() == Some("running") {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(cancel_deploy("demo", id4), Ok(true));
        h.await.unwrap();
        SLOTS.lock().unwrap().remove("demo");
        assert_eq!(status_of(&st, id4).await.0, "cancelled");
        assert!(t.elapsed() < Duration::from_secs(15));
        assert_eq!(live_release(&app_dir), Some(1));

        // #5, #6 healthy → keep 2 releases; then roll back to #5.
        set_build(&st, "touch healthy").await;
        let (id5, (s, _)) = run(Kind::Deploy).await;
        assert_eq!(s, "success");
        let (_, (s, _)) = run(Kind::Deploy).await;
        assert_eq!(s, "success");
        assert_eq!(live_release(&app_dir), Some(6));
        assert!(!app_dir.join("releases/1").exists(), "cleaned up");
        let (_, (s, _)) = run(Kind::Rollback { release: 5 }).await;
        assert_eq!(s, "success");
        assert_eq!(live_release(&app_dir), Some(5));
        let _ = (id1, id5);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
