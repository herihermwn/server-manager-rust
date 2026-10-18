//! Phase 5 · Databases: PostgreSQL list/create/link/drop, credentials shown once, pg_dump backups.
//! Routes are relative to /api and already behind auth.
//!
//! - Admin connection = role `server_manager` (CREATEDB CREATEROLE, not superuser). Its URL comes
//!   from `PG_ADMIN_URL` (env) or `<data_dir>/pg.env`, re-read on every request, so installing
//!   PostgreSQL later needs no restart. One short-lived connection per request (localhost, cheap).
//! - Only databases listed in SQLite `databases` are ever touched: `postgres` and friends are safe.
//! - The owner password exists in memory and in the one create response. It is never written to
//!   SQLite, activity or logs; pg_dump gets the admin password through the environment.
//! - Backups: `pg_dump -Fc` into `~/backups/postgres/<db>-<YYYYMMDD-HHMMSS>.dump` (WIB), newest 7
//!   per database kept. A daily task runs at 02:00 WIB.

use crate::{
    activity,
    auth::Admin,
    exec::Cmd,
    http::{ApiResult, AppError, now},
    state::AppState,
    validate,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{
    Connection,
    postgres::{PgConnectOptions, PgConnection},
};
use std::{collections::HashMap, io::Read, os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt}, path::PathBuf, sync::Mutex, time::Duration};

const EXTENSIONS: [&str; 3] = ["pg_trgm", "uuid-ossp", "unaccent"];
const KEEP_BACKUPS: usize = 7;
const INSTALL_COMMAND: &str = "sudo apt install -y postgresql-16 postgresql-contrib";
/// Names that must never be created, owned or dropped from here.
const RESERVED_DB: [&str; 3] = ["postgres", "template0", "template1"];
const RESERVED_ROLE: [&str; 2] = ["postgres", "server_manager"];

/// Create and drop run one at a time (role + database + grants are several statements).
static DDL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Databases with a pg_dump running right now.
static BACKING_UP: Mutex<Vec<String>> = Mutex::new(Vec::new());

// ---------- identifiers and literals ----------

/// `^[a-z_][a-z0-9_]{0,62}$`
pub fn valid_ident(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=63).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0] == b'_')
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// SQL identifier in double quotes, embedded quotes doubled (same as PostgreSQL `quote_ident`).
fn qi(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// SQL string literal that reads the same whatever `standard_conforming_strings` is: an `E'…'`
/// string with `'` and `\` doubled. The caller rejects NUL bytes first.
fn ql(s: &str) -> String {
    format!("E'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
}

fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        let hex = |x: u8| (x as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 32 characters from [A-Za-z0-9] out of /dev/urandom (bytes >= 248 are skipped: no modulo bias).
fn random_password() -> std::io::Result<String> {
    const A: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut f = std::fs::File::open("/dev/urandom")?;
    let mut out = String::new();
    let mut buf = [0u8; 64];
    while out.len() < 32 {
        f.read_exact(&mut buf)?;
        for b in buf {
            if b < 248 && out.len() < 32 {
                out.push(A[(b % 62) as usize] as char);
            }
        }
    }
    Ok(out)
}

// ---------- the SQL we run (pure, so tests can read it) ----------

fn sql_create_role(owner: &str) -> String {
    format!("CREATE ROLE {} LOGIN", qi(owner))
}
/// Separate from CREATE ROLE so a failing CREATE ROLE never carries the password into the server log.
fn sql_set_password(owner: &str, password: &str) -> String {
    format!("ALTER ROLE {} PASSWORD {}", qi(owner), ql(password))
}
/// The admin must be able to act as the owner (CREATE/DROP DATABASE, pg_dump, trusted extensions).
fn sql_grant_owner(owner: &str, admin: &str, pg16: bool) -> String {
    let tail = if pg16 { " WITH INHERIT TRUE, SET TRUE" } else { "" };
    format!("GRANT {} TO {}{tail}", qi(owner), qi(admin))
}
fn sql_create_db(name: &str, owner: &str) -> String {
    format!("CREATE DATABASE {} OWNER {}", qi(name), qi(owner))
}
fn sql_revoke_public(name: &str) -> String {
    format!("REVOKE CONNECT ON DATABASE {} FROM PUBLIC", qi(name))
}
fn sql_create_ext(ext: &str) -> String {
    format!("CREATE EXTENSION IF NOT EXISTS {}", qi(ext))
}
fn sql_drop_db(name: &str) -> String {
    format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", qi(name))
}
fn sql_drop_role(owner: &str) -> String {
    format!("DROP ROLE IF EXISTS {}", qi(owner))
}

// ---------- connection ----------

#[derive(Clone, Debug, PartialEq)]
struct Pg {
    host: String,
    port: u16,
    user: String,
    password: String,
    db: String,
}

impl Pg {
    fn opts(&self, db: &str) -> PgConnectOptions {
        PgConnectOptions::new()
            .host(&self.host)
            .port(self.port)
            .username(&self.user)
            .password(&self.password)
            .database(db)
    }
    fn url(&self, user: &str, password: Option<&str>, db: &str) -> String {
        let pw = password.map(|p| format!(":{}", pct_encode(p))).unwrap_or_default();
        format!("postgres://{}{pw}@{}:{}/{}", pct_encode(user), self.host, self.port, db)
    }
}

/// `postgres://user:pass@host:port/db?…` (user and password percent-encoded).
fn parse_url(s: &str) -> Option<Pg> {
    let rest = s
        .trim()
        .strip_prefix("postgres://")
        .or_else(|| s.trim().strip_prefix("postgresql://"))?;
    let rest = rest.split(['?', '#']).next()?;
    let (info, tail) = rest.rsplit_once('@')?;
    let (user, password) = info.split_once(':').unwrap_or((info, ""));
    let (hostport, db) = tail.split_once('/').unwrap_or((tail, ""));
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (h, p.parse().ok()?),
        _ => (hostport, 5432),
    };
    if user.is_empty() || host.is_empty() {
        return None;
    }
    Some(Pg {
        host: host.trim_matches(['[', ']']).to_owned(),
        port,
        user: pct_decode(user),
        password: pct_decode(password),
        db: if db.is_empty() {
            "postgres".into()
        } else {
            pct_decode(db)
        },
    })
}

/// `PG_ADMIN_URL=…` from a pg.env file (optional `export`, optional quotes).
fn url_from_env_file(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.trim().trim_start_matches("export ").trim_start();
        let v = l.strip_prefix("PG_ADMIN_URL=")?.trim();
        Some(v.trim_matches(['"', '\'']).to_owned()).filter(|v| !v.is_empty())
    })
}

fn load_pg(st: &AppState) -> Option<Pg> {
    let raw = std::env::var("PG_ADMIN_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| url_from_env_file(&std::fs::read_to_string(st.cfg.data_dir.join("pg.env")).ok()?))?;
    parse_url(&raw)
}

async fn connect(opts: &PgConnectOptions) -> Result<PgConnection, String> {
    match tokio::time::timeout(Duration::from_secs(5), PgConnection::connect_with(opts)).await {
        Ok(Ok(c)) => Ok(c),
        Ok(Err(e)) => Err(pg_msg(&e)),
        Err(_) => Err("timed out after 5 s".into()),
    }
}

/// The server's own message (never the statement), so a failed ALTER ROLE can't echo a password.
fn pg_msg(e: &sqlx::Error) -> String {
    match e.as_database_error() {
        Some(d) => d.message().to_owned(),
        None => e.to_string(),
    }
}

fn unavailable(msg: impl Into<String>) -> AppError {
    AppError::Status(StatusCode::SERVICE_UNAVAILABLE, msg.into())
}

fn conflict(msg: impl Into<String>) -> AppError {
    AppError::Status(StatusCode::CONFLICT, msg.into())
}

async fn admin(st: &AppState) -> ApiResult<(Pg, PgConnection)> {
    let pg = load_pg(st).ok_or_else(|| unavailable("PostgreSQL is not set up: PG_ADMIN_URL was not found."))?;
    let conn = connect(&pg.opts(&pg.db))
        .await
        .map_err(|e| unavailable(format!("Cannot reach PostgreSQL: {e}")))?;
    Ok((pg, conn))
}

async fn exists(conn: &mut PgConnection, sql: &str, v: &str) -> ApiResult<bool> {
    Ok(sqlx::query_scalar(sql).bind(v).fetch_one(&mut *conn).await?)
}

async fn run(conn: &mut PgConnection, sql: &str) -> Result<(), String> {
    sqlx::query(sql)
        .persistent(false)
        .execute(&mut *conn)
        .await
        .map(|_| ())
        .map_err(|e| pg_msg(&e))
}

// ---------- backup files ----------

fn backup_dir() -> PathBuf {
    match std::env::var("SM_BACKUP_DIR") {
        Ok(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("backups/postgres"),
    }
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's civil_from_days.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

const WIB: i64 = 7 * 3600;

/// `20261018-020000` in WIB (UTC+7, whatever the server's own zone is).
fn wib_stamp(unix: i64) -> String {
    let t = unix + WIB;
    let (y, m, d) = civil(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn backup_file_name(db: &str, unix: i64) -> String {
    format!("{db}-{}.dump", wib_stamp(unix))
}

/// `radio_api-20261018-020000.dump` → `radio_api`. Names hold no `-`, so `a` never matches `a_b`.
fn backup_db_of(file: &str) -> Option<&str> {
    let mut p = file.strip_suffix(".dump")?.rsplitn(3, '-');
    let (time, date, db) = (p.next()?, p.next()?, p.next()?);
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    (digits(time, 6) && digits(date, 8) && valid_ident(db)).then_some(db)
}

/// Dumps of `db` beyond the newest `keep` (the timestamp sorts as text).
fn to_prune(files: &[String], db: &str, keep: usize) -> Vec<String> {
    let mut mine: Vec<&String> = files.iter().filter(|f| backup_db_of(f) == Some(db)).collect();
    mine.sort();
    mine.reverse();
    mine.into_iter().skip(keep).cloned().collect()
}

/// Next 02:00 WIB strictly after `now` (unix seconds, UTC).
fn next_run(now: i64) -> i64 {
    let local = now + WIB;
    let mut at = local.div_euclid(86_400) * 86_400 + 2 * 3600;
    if at <= local {
        at += 86_400;
    }
    at - WIB
}

struct BackupGuard(String);
impl BackupGuard {
    fn take(db: &str) -> Option<Self> {
        let mut busy = BACKING_UP.lock().unwrap();
        if busy.iter().any(|b| b == db) {
            return None;
        }
        busy.push(db.to_owned());
        Some(BackupGuard(db.to_owned()))
    }
}
impl Drop for BackupGuard {
    fn drop(&mut self) {
        BACKING_UP.lock().unwrap().retain(|b| *b != self.0);
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct BackupInfo {
    file: String,
    size_bytes: i64,
    at: i64,
}

fn short(s: &str) -> String {
    let s = s.trim();
    match s.char_indices().nth(300) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// pg_dump one database, prune old dumps, remember the result in SQLite. The dump is written to
/// `*.part` and renamed, so a failed run never leaves a file that looks like a backup.
async fn backup_db(st: &AppState, pg: &Pg, name: &str) -> Result<BackupInfo, String> {
    let _guard = BackupGuard::take(name).ok_or("A backup of this database is already running.")?;
    let at = now();
    let res = dump(pg, name, at).await;
    let (status, size, err) = match &res {
        Ok(b) => ("ok", Some(b.size_bytes), None),
        Err(e) => ("failed", None, Some(short(e))),
    };
    let _ = sqlx::query(
        "UPDATE databases SET last_backup_at = ?, last_backup_status = ?, last_backup_size = ?, last_backup_error = ? WHERE name = ?",
    )
    .bind(at)
    .bind(status)
    .bind(size)
    .bind(err)
    .bind(name)
    .execute(&st.db)
    .await;
    res
}

async fn dump(pg: &Pg, name: &str, at: i64) -> Result<BackupInfo, String> {
    let dir = backup_dir();
    private_dir(&dir).map_err(|e| format!("Cannot prepare {}: {e}", dir.display()))?;
    let file = backup_file_name(name, at);
    let (fin, part) = (dir.join(&file), dir.join(format!("{file}.part")));
    // Private from the first byte: pg_dump reopens this file for writing and keeps its mode.
    private_file(&part).map_err(|e| format!("Cannot create {}: {e}", part.display()))?;
    let out = Cmd::new("pg_dump")
        .args(["-Fc", "--no-password", "-f"])
        .arg(part.to_string_lossy())
        .arg(name)
        .env("PGHOST", &pg.host)
        .env("PGPORT", pg.port.to_string())
        .env("PGUSER", &pg.user)
        .env("PGPASSWORD", &pg.password)
        .redact([pg.password.clone()])
        .timeout(Duration::from_secs(3600))
        .output()
        .await
        .map_err(|e| format!("Cannot run pg_dump: {e}"))?;
    if !out.exit.success() {
        let _ = std::fs::remove_file(&part);
        return Err(format!("pg_dump failed ({:?}): {}", out.exit, short(&out.stderr)));
    }
    std::fs::rename(&part, &fin).map_err(|e| format!("Cannot save the dump: {e}"))?;
    let size_bytes = std::fs::metadata(&fin).map(|m| m.len() as i64).unwrap_or(0);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    for old in to_prune(&names, name, KEEP_BACKUPS) {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(BackupInfo { file, size_bytes, at })
}

/// The backup folder: created with mode 700, and tightened if it already existed (dumps hold all the data).
fn private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// A new empty file with mode 600 (a leftover with the same name is replaced).
fn private_file(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    Ok(())
}

fn human(bytes: i64) -> String {
    let b = bytes as f64;
    match bytes {
        n if n >= 1 << 30 => format!("{:.1} GB", b / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.0} MB", b / (1u64 << 20) as f64),
        n => format!("{:.0} KB", (n as f64 / 1024.0).max(1.0)),
    }
}

// ---------- GET /databases/status ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PgStats {
    version: String,
    listen: String,
    databases: i64,
    size_bytes: i64,
    connections: i64,
    max_connections: i64,
    cache_hit: Option<f64>,
    shared_buffers: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupSummary {
    last_at: Option<i64>,
    ok: i64,
    total: i64,
    size_bytes: i64,
    dir: String,
    dir_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    /// `ok`, `missing` (no PG_ADMIN_URL found) or `unreachable`.
    state: &'static str,
    error: Option<String>,
    install_command: &'static str,
    pg: Option<PgStats>,
    backup: BackupSummary,
}

#[derive(sqlx::FromRow)]
struct Row {
    name: String,
    owner: String,
    app_name: Option<String>,
    extensions: String,
    created_at: i64,
    last_backup_at: Option<i64>,
    last_backup_status: Option<String>,
    last_backup_size: Option<i64>,
    last_backup_error: Option<String>,
}

const ROWS: &str = "SELECT d.name, d.owner, a.name AS app_name, d.extensions, d.created_at, d.last_backup_at, \
    d.last_backup_status, d.last_backup_size, d.last_backup_error \
    FROM databases d LEFT JOIN apps a ON a.id = d.app_id ORDER BY d.name";

async fn rows(st: &AppState) -> Result<Vec<Row>, sqlx::Error> {
    sqlx::query_as(ROWS).fetch_all(&st.db).await
}

/// (size in bytes, open connections) per database, for the given names only.
async fn db_stats(conn: &mut PgConnection, names: &[String]) -> Result<HashMap<String, (i64, i32)>, sqlx::Error> {
    let r: Vec<(String, i64, i32)> = sqlx::query_as(
        "SELECT d.datname::text, pg_database_size(d.datname)::int8, COALESCE(s.numbackends, 0)::int4 \
         FROM pg_database d LEFT JOIN pg_stat_database s ON s.datname = d.datname WHERE d.datname::text = ANY($1)",
    )
    .bind(names)
    .fetch_all(&mut *conn)
    .await?;
    Ok(r.into_iter().map(|(n, s, c)| (n, (s, c))).collect())
}

fn dir_bytes() -> u64 {
    std::fs::read_dir(backup_dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| backup_db_of(&e.file_name().to_string_lossy()).is_some())
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

async fn status(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    let list = rows(&st).await?;
    let backup = BackupSummary {
        last_at: list.iter().filter_map(|r| r.last_backup_at).max(),
        ok: list
            .iter()
            .filter(|r| r.last_backup_status.as_deref() == Some("ok"))
            .count() as i64,
        total: list.len() as i64,
        size_bytes: list
            .iter()
            .filter(|r| r.last_backup_status.as_deref() == Some("ok"))
            .filter_map(|r| r.last_backup_size)
            .sum(),
        dir: backup_dir().to_string_lossy().into_owned(),
        dir_bytes: dir_bytes(),
    };
    let mk = |state, error, pg| Status {
        state,
        error,
        install_command: INSTALL_COMMAND,
        pg,
        backup: BackupSummary { ..backup },
    };
    let Some(pg) = load_pg(&st) else {
        return Ok(Json(mk("missing", None, None)));
    };
    let mut conn = match connect(&pg.opts(&pg.db)).await {
        Ok(c) => c,
        Err(e) => return Ok(Json(mk("unreachable", Some(e), None))),
    };
    let names: Vec<String> = list.iter().map(|r| r.name.clone()).collect();
    let res: Result<PgStats, sqlx::Error> = async {
        let version: String = sqlx::query_scalar("SHOW server_version").fetch_one(&mut conn).await?;
        let max: String = sqlx::query_scalar("SHOW max_connections").fetch_one(&mut conn).await?;
        let shared_buffers: String = sqlx::query_scalar("SHOW shared_buffers").fetch_one(&mut conn).await?;
        let (connections, cache_hit): (i64, Option<f64>) = sqlx::query_as(
            "SELECT COALESCE(sum(numbackends), 0)::int8, \
             sum(blks_hit)::float8 / NULLIF(sum(blks_hit) + sum(blks_read), 0)::float8 FROM pg_stat_database",
        )
        .fetch_one(&mut conn)
        .await?;
        let sizes = db_stats(&mut conn, &names).await.unwrap_or_default();
        Ok(PgStats {
            version: version.split_whitespace().next().unwrap_or("").to_owned(),
            listen: format!("{}:{}", pg.host, pg.port),
            databases: names.len() as i64,
            size_bytes: sizes.values().map(|v| v.0).sum(),
            connections,
            max_connections: max.parse().unwrap_or(0),
            cache_hit,
            shared_buffers,
        })
    }
    .await;
    Ok(Json(match res {
        Ok(s) => mk("ok", None, Some(s)),
        Err(e) => mk("unreachable", Some(pg_msg(&e)), None),
    }))
}

// ---------- GET /databases ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LastBackup {
    at: i64,
    ok: bool,
    size_bytes: Option<i64>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DbItem {
    name: String,
    owner: String,
    size_bytes: Option<i64>,
    connections: Option<i32>,
    extensions: Vec<String>,
    app: Option<String>,
    /// Connection URL without the password (the password is shown once, at creation).
    url: String,
    created_at: i64,
    last_backup: Option<LastBackup>,
}

async fn list(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    let (pg, mut conn) = admin(&st).await?;
    let list = rows(&st).await?;
    let names: Vec<String> = list.iter().map(|r| r.name.clone()).collect();
    let stats = db_stats(&mut conn, &names).await.unwrap_or_else(|e| {
        eprintln!("[databases] stats: {}", pg_msg(&e));
        HashMap::new()
    });
    let items: Vec<DbItem> = list
        .into_iter()
        .map(|r| {
            let s = stats.get(&r.name);
            DbItem {
                size_bytes: s.map(|v| v.0),
                connections: s.map(|v| v.1),
                extensions: serde_json::from_str(&r.extensions).unwrap_or_default(),
                url: pg.url(&r.owner, None, &r.name),
                last_backup: r.last_backup_at.map(|at| LastBackup {
                    at,
                    ok: r.last_backup_status.as_deref() == Some("ok"),
                    size_bytes: r.last_backup_size,
                    error: r.last_backup_error.clone(),
                }),
                name: r.name,
                owner: r.owner,
                app: r.app_name,
                created_at: r.created_at,
            }
        })
        .collect();
    Ok(Json(items))
}

// ---------- POST /databases ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateReq {
    name: String,
    owner: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    extensions: Vec<String>,
    link_app: Option<String>,
}

/// Everything checkable without touching PostgreSQL.
fn check_create(r: &CreateReq) -> Result<(), String> {
    const RULE: &str = "use lowercase letters, digits and _, starting with a letter or _ (max 63).";
    if !valid_ident(&r.name) {
        return Err(format!("Database name: {RULE}"));
    }
    if RESERVED_DB.contains(&r.name.as_str()) {
        return Err(format!("\"{}\" is reserved by PostgreSQL.", r.name));
    }
    if !valid_ident(&r.owner) {
        return Err(format!("Owner user: {RULE}"));
    }
    if r.owner.starts_with("pg_") || RESERVED_ROLE.contains(&r.owner.as_str()) {
        return Err(format!("\"{}\" is reserved. Pick another owner name.", r.owner));
    }
    if !r.password.is_empty() && (r.password.len() < 8 || r.password.len() > 128 || r.password.contains('\0')) {
        return Err("Password: 8 to 128 characters.".into());
    }
    if let Some(bad) = r.extensions.iter().find(|e| !EXTENSIONS.contains(&e.as_str())) {
        return Err(format!(
            "Extension \"{bad}\" is not available here. Choose from {}.",
            EXTENSIONS.join(", ")
        ));
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    name: String,
    owner: String,
    host: String,
    port: u16,
    password: String,
    url: String,
    extensions: Vec<String>,
    linked_app: Option<String>,
    /// Non-fatal problems (an extension that would not install, the .env write).
    warnings: Vec<String>,
}

async fn create(
    State(st): State<AppState>,
    Extension(who): Extension<Admin>,
    Json(mut req): Json<CreateReq>,
) -> ApiResult<impl IntoResponse> {
    req.name = req.name.trim().to_owned();
    req.owner = req.owner.trim().to_owned();
    check_create(&req).map_err(AppError::bad_request)?;
    let app = match req.link_app.as_deref().filter(|a| !a.is_empty()) {
        Some(a) => {
            validate::app_name(a)?;
            let found: Option<i64> = sqlx::query_scalar("SELECT id FROM apps WHERE name = ?")
                .bind(a)
                .fetch_optional(&st.db)
                .await?;
            Some((
                a.to_owned(),
                found.ok_or_else(|| AppError::not_found("App to link was not found."))?,
            ))
        }
        None => None,
    };
    let _ddl = DDL_LOCK.lock().await;
    let (pg, mut conn) = admin(&st).await?;

    let known: Option<i64> = sqlx::query_scalar("SELECT id FROM databases WHERE name = ?")
        .bind(&req.name)
        .fetch_optional(&st.db)
        .await?;
    if known.is_some() {
        return Err(conflict(format!("Database \"{}\" already exists.", req.name)));
    }
    if exists(
        &mut conn,
        "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)",
        &req.name,
    )
    .await?
    {
        return Err(conflict(format!(
            "A PostgreSQL database named \"{}\" already exists.",
            req.name
        )));
    }
    if exists(
        &mut conn,
        "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)",
        &req.owner,
    )
    .await?
    {
        return Err(conflict(format!(
            "A user named \"{}\" already exists. Pick another owner name.",
            req.owner
        )));
    }

    let password = if req.password.is_empty() {
        random_password().map_err(|e| AppError::Internal(format!("random: {e}")))?
    } else {
        req.password.clone()
    };
    let fail = |step: &str, e: String| AppError::Status(StatusCode::BAD_GATEWAY, format!("{step} failed: {e}"));

    let admin_user: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut conn)
        .await
        .map_err(|e| AppError::Internal(format!("pg: {}", pg_msg(&e))))?;
    let pg16: bool = sqlx::query_scalar::<_, String>("SHOW server_version_num")
        .fetch_one(&mut conn)
        .await
        .map(|v| v.parse::<i64>().unwrap_or(0) >= 160_000)
        .unwrap_or(false);

    run(&mut conn, &sql_create_role(&req.owner))
        .await
        .map_err(|e| fail("Creating the user", e))?;
    let mut made_db = false;
    let steps: Result<(), (&str, String)> = async {
        run(&mut conn, &sql_set_password(&req.owner, &password))
            .await
            .map_err(|e| ("Setting the password", e))?;
        run(&mut conn, &sql_grant_owner(&req.owner, &admin_user, pg16))
            .await
            .map_err(|e| ("Granting the user", e))?;
        run(&mut conn, &sql_create_db(&req.name, &req.owner))
            .await
            .map_err(|e| ("Creating the database", e))?;
        made_db = true;
        run(&mut conn, &sql_revoke_public(&req.name))
            .await
            .map_err(|e| ("Locking the database", e))
    }
    .await;
    if let Err((step, e)) = steps {
        // Undo what we made so a retry starts clean.
        if made_db {
            let _ = run(&mut conn, &sql_drop_db(&req.name)).await;
        }
        let _ = run(&mut conn, &sql_drop_role(&req.owner)).await;
        return Err(fail(step, e));
    }

    let mut warnings = Vec::new();
    let mut installed = Vec::new();
    if !req.extensions.is_empty() {
        match connect(&pg.opts(&req.name)).await {
            Ok(mut dbc) => {
                for ext in EXTENSIONS.iter().filter(|e| req.extensions.iter().any(|x| x == *e)) {
                    match run(&mut dbc, &sql_create_ext(ext)).await {
                        Ok(()) => installed.push((*ext).to_owned()),
                        Err(e) => warnings.push(format!("Extension {ext} was not installed: {e}")),
                    }
                }
            }
            Err(e) => warnings.push(format!("Extensions were not installed: {e}")),
        }
    }

    sqlx::query("INSERT INTO databases (name, owner, app_id, extensions) VALUES (?, ?, ?, ?)")
        .bind(&req.name)
        .bind(&req.owner)
        .bind(app.as_ref().map(|a| a.1))
        .bind(serde_json::to_string(&installed).unwrap_or_else(|_| "[]".into()))
        .execute(&st.db)
        .await?;

    let url = pg.url(&req.owner, Some(&password), &req.name);
    let mut linked_app = None;
    if let Some((app_name, _)) = &app {
        let vars = [("DATABASE_URL".to_owned(), url.clone())];
        match super::env::upsert_vars(&st, app_name, &vars, Some(&format!("database:{}", req.name))).await {
            Ok(()) => linked_app = Some(app_name.clone()),
            Err(e) => warnings.push(format!("DATABASE_URL was not written to {app_name}'s .env: {e}")),
        }
    }
    let detail = format!(
        "owner {}, extensions [{}]{}",
        req.owner,
        installed.join(", "),
        linked_app
            .as_ref()
            .map(|a| format!(", linked to {a}"))
            .unwrap_or_default()
    );
    activity::log(&st.db, &who.username, "database.create", &req.name, &detail).await;

    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(Created {
            name: req.name,
            owner: req.owner,
            host: pg.host.clone(),
            port: pg.port,
            password,
            url,
            extensions: installed,
            linked_app,
            warnings,
        }),
    ))
}

// ---------- POST /databases/{name}/backup ----------

/// The name must be one we manage (never `postgres`, never something typed by hand into the URL).
async fn managed(st: &AppState, name: &str) -> ApiResult<String> {
    if !valid_ident(name) {
        return Err(AppError::bad_request("Invalid database name."));
    }
    let owner: Option<String> = sqlx::query_scalar("SELECT owner FROM databases WHERE name = ?")
        .bind(name)
        .fetch_optional(&st.db)
        .await?;
    owner.ok_or_else(|| AppError::not_found("Database not found."))
}

async fn backup(
    State(st): State<AppState>,
    Extension(who): Extension<Admin>,
    Path(name): Path<String>,
) -> ApiResult<impl IntoResponse> {
    managed(&st, &name).await?;
    let pg = load_pg(&st).ok_or_else(|| unavailable("PostgreSQL is not set up: PG_ADMIN_URL was not found."))?;
    let res = backup_db(&st, &pg, &name).await;
    let detail = match &res {
        Ok(b) => format!("{} ({})", b.file, human(b.size_bytes)),
        Err(e) => format!("failed: {}", short(e)),
    };
    activity::log(&st.db, &who.username, "database.backup", &name, &detail).await;
    match res {
        Ok(b) => Ok(Json(b)),
        Err(e) => Err(AppError::Status(StatusCode::BAD_GATEWAY, e)),
    }
}

// ---------- DELETE /databases/{name} ----------

#[derive(Deserialize)]
struct DropReq {
    confirm: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Dropped {
    backup: BackupInfo,
    role_dropped: bool,
    warnings: Vec<String>,
}

async fn drop_db(
    State(st): State<AppState>,
    Extension(who): Extension<Admin>,
    Path(name): Path<String>,
    Json(req): Json<DropReq>,
) -> ApiResult<impl IntoResponse> {
    let owner = managed(&st, &name).await?;
    if req.confirm != name {
        return Err(AppError::bad_request("Type the database name to confirm."));
    }
    let _ddl = DDL_LOCK.lock().await;
    let (pg, mut conn) = admin(&st).await?;
    let b = backup_db(&st, &pg, &name).await.map_err(|e| {
        AppError::Status(
            StatusCode::BAD_GATEWAY,
            format!("The database was not dropped because its backup failed: {e}"),
        )
    })?;
    run(&mut conn, &sql_drop_db(&name)).await.map_err(|e| {
        AppError::Status(
            StatusCode::BAD_GATEWAY,
            format!("Dropping the database failed (backup {} is saved): {e}", b.file),
        )
    })?;
    sqlx::query("DELETE FROM databases WHERE name = ?")
        .bind(&name)
        .execute(&st.db)
        .await?;

    // The user goes too, unless another managed database still uses it or it owns something else.
    let others: i64 = sqlx::query_scalar("SELECT count(*) FROM databases WHERE owner = ?")
        .bind(&owner)
        .fetch_one(&st.db)
        .await?;
    let (mut role_dropped, mut warnings) = (false, Vec::new());
    if others == 0 && !RESERVED_ROLE.contains(&owner.as_str()) {
        match run(&mut conn, &sql_drop_role(&owner)).await {
            Ok(()) => role_dropped = true,
            Err(e) => warnings.push(format!("User {owner} was kept: {e}")),
        }
    }
    activity::log(
        &st.db,
        &who.username,
        "database.drop",
        &name,
        &format!(
            "backup {}{}",
            b.file,
            if role_dropped {
                format!(", user {owner} dropped")
            } else {
                String::new()
            }
        ),
    )
    .await;
    Ok(Json(Dropped {
        backup: b,
        role_dropped,
        warnings,
    }))
}

// ---------- daily backup ----------

async fn backup_all(st: &AppState) {
    let Some(pg) = load_pg(st) else { return };
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM databases ORDER BY name")
        .fetch_all(&st.db)
        .await
        .unwrap_or_default();
    for name in names {
        let detail = match backup_db(st, &pg, &name).await {
            Ok(b) => format!("{} ({})", b.file, human(b.size_bytes)),
            Err(e) => format!("failed: {}", short(&e)),
        };
        activity::log(&st.db, "system", "database.backup", &name, &detail).await;
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/databases", get(list).post(create))
        .route("/databases/status", get(status))
        .route("/databases/{name}/backup", post(backup))
        .route("/databases/{name}", axum::routing::delete(drop_db))
}

/// Daily pg_dump of every managed database at 02:00 WIB.
pub fn spawn(st: &AppState) {
    let st = st.clone();
    tokio::spawn(async move {
        loop {
            let at = next_run(now());
            tokio::time::sleep(Duration::from_secs((at - now()).max(1) as u64)).await;
            if now() < at {
                continue; // woke early: wait again for the same slot
            }
            backup_all(&st).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        for ok in ["a", "_x", "radio_api", "db1", &"a".repeat(63)] {
            assert!(valid_ident(ok), "{ok}");
        }
        for bad in ["", "1a", "A", "a-b", "a b", "a;b", "a\"b", "ä", "a.b", &"a".repeat(64)] {
            assert!(!valid_ident(bad), "{bad}");
        }
    }

    #[test]
    fn quoting() {
        assert_eq!(qi("radio_api"), "\"radio_api\"");
        assert_eq!(qi("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(ql("plain"), "E'plain'");
        assert_eq!(ql("it's"), "E'it''s'");
        assert_eq!(ql(r"back\slash"), r"E'back\\slash'");
        assert_eq!(ql(r"a\'; DROP DATABASE x; --"), r"E'a\\''; DROP DATABASE x; --'");
        assert_eq!(
            sql_set_password("u", r#"p'w\"1"#),
            r#"ALTER ROLE "u" PASSWORD E'p''w\\"1'"#
        );
    }

    #[test]
    fn statements() {
        assert_eq!(sql_create_role("landing"), r#"CREATE ROLE "landing" LOGIN"#);
        assert_eq!(
            sql_create_db("landing", "landing"),
            r#"CREATE DATABASE "landing" OWNER "landing""#
        );
        assert_eq!(
            sql_revoke_public("landing"),
            r#"REVOKE CONNECT ON DATABASE "landing" FROM PUBLIC"#
        );
        assert_eq!(
            sql_create_ext("uuid-ossp"),
            r#"CREATE EXTENSION IF NOT EXISTS "uuid-ossp""#
        );
        assert_eq!(
            sql_grant_owner("o", "server_manager", true),
            r#"GRANT "o" TO "server_manager" WITH INHERIT TRUE, SET TRUE"#
        );
        assert_eq!(
            sql_grant_owner("o", "server_manager", false),
            r#"GRANT "o" TO "server_manager""#
        );
        assert_eq!(sql_drop_db("x"), r#"DROP DATABASE IF EXISTS "x" WITH (FORCE)"#);
    }

    fn req(name: &str, owner: &str, password: &str, ext: &[&str]) -> CreateReq {
        CreateReq {
            name: name.into(),
            owner: owner.into(),
            password: password.into(),
            extensions: ext.iter().map(|s| s.to_string()).collect(),
            link_app: None,
        }
    }

    #[test]
    fn create_checks() {
        assert!(check_create(&req("landing", "landing", "", &["pg_trgm", "uuid-ossp"])).is_ok());
        assert!(check_create(&req("landing", "landing", "longenough!", &[])).is_ok());
        for (n, o) in [
            ("Landing", "x"),
            ("x", "X y"),
            ("postgres", "x"),
            ("template1", "x"),
            ("x", "pg_admin"),
            ("x", "server_manager"),
            ("x", "postgres"),
        ] {
            assert!(check_create(&req(n, o, "", &[])).is_err(), "{n}/{o}");
        }
        assert!(check_create(&req("x", "y", "short", &[])).is_err());
        assert!(check_create(&req("x", "y", "with\0nul-byte", &[])).is_err());
        assert!(check_create(&req("x", "y", "", &["plpython3u"])).is_err());
        assert!(check_create(&req("x", "y", "", &["pg_trgm\"; DROP"])).is_err());
    }

    #[test]
    fn password_is_32_alnum() {
        let a = random_password().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(a, random_password().unwrap());
    }

    #[test]
    fn urls() {
        let p = parse_url("postgres://server_manager:p%40ss%2Fw%3Ard@127.0.0.1:5432/postgres?sslmode=disable").unwrap();
        assert_eq!(
            p,
            Pg {
                host: "127.0.0.1".into(),
                port: 5432,
                user: "server_manager".into(),
                password: "p@ss/w:rd".into(),
                db: "postgres".into()
            }
        );
        let d = parse_url("postgresql://u:pw@localhost/x").unwrap();
        assert_eq!((d.host.as_str(), d.port, d.db.as_str()), ("localhost", 5432, "x"));
        assert!(parse_url("mysql://u:p@h/x").is_none());
        assert!(parse_url("postgres://h/x").is_none());
        assert_eq!(
            p.url("landing", Some("a b/c'd"), "landing"),
            "postgres://landing:a%20b%2Fc%27d@127.0.0.1:5432/landing"
        );
        assert_eq!(
            p.url("landing", None, "landing"),
            "postgres://landing@127.0.0.1:5432/landing"
        );
        assert_eq!(pct_decode(&pct_encode("p@ss/w:rd é%")), "p@ss/w:rd é%");
        assert_eq!(pct_decode("100%"), "100%");
        assert_eq!(
            url_from_env_file("# c\nexport PG_ADMIN_URL='postgres://a:b@h/x'\n").as_deref(),
            Some("postgres://a:b@h/x")
        );
        assert_eq!(url_from_env_file("OTHER=1"), None);
    }

    #[test]
    fn backup_names() {
        // 2026-10-18 19:00:00 UTC = 2026-10-19 02:00:00 WIB
        let t = 1_792_350_000;
        assert_eq!(wib_stamp(t), "20261019-020000");
        assert_eq!(wib_stamp(0), "19700101-070000");
        assert_eq!(wib_stamp(1_709_164_800 - 7 * 3600), "20240229-000000"); // leap day
        assert_eq!(backup_file_name("radio_api", t), "radio_api-20261019-020000.dump");
        assert_eq!(backup_db_of("radio_api-20261019-020000.dump"), Some("radio_api"));
        assert_eq!(backup_db_of("radio_api-20261019-020000.dump.part"), None);
        assert_eq!(backup_db_of("notes.txt"), None);
        assert_eq!(backup_db_of("x-2026-1.dump"), None);
    }

    #[test]
    fn retention_keeps_newest_seven_per_database() {
        let mut files: Vec<String> = (10..22).map(|d| format!("app-202610{d}-020000.dump")).collect();
        files.push("app_staging-20261001-020000.dump".into()); // different database, never pruned for `app`
        files.push("app-20261015-020000.dump.part".into());
        files.push("README".into());
        let gone = to_prune(&files, "app", 7);
        assert_eq!(gone.len(), 5);
        assert!(gone.iter().all(|f| {
            f.starts_with("app-2026101")
                && ["10", "11", "12", "13", "14"]
                    .iter()
                    .any(|d| f.contains(&format!("202610{d}")))
        }));
        assert!(to_prune(&files, "app", 20).is_empty());
        assert!(to_prune(&files, "app_staging", 7).is_empty());
    }

    #[test]
    fn schedule_is_0200_wib() {
        let at = 1_792_350_000; // 2026-10-19 02:00:00 WIB
        assert_eq!(next_run(at - 1), at);
        assert_eq!(next_run(at), at + 86_400, "strictly after");
        assert_eq!(next_run(at + 1), at + 86_400);
        // 00:00 WIB = 17:00 UTC: next slot two hours later
        assert_eq!(next_run(at - 2 * 3600), at);
        // 19:00 WIB = 12:00 UTC: next slot is 7 h later (tomorrow 02:00)
        assert_eq!(next_run(at + 17 * 3600), at + 86_400);
        // whatever the time, the answer is within 24 h and at 19:00:00 UTC
        for n in [0, 1, 70_000, 1_700_000_123, 1_792_349_999] {
            let r = next_run(n);
            assert!(r > n && r - n <= 86_400 && r % 86_400 == 19 * 3600, "{n}");
        }
    }

    #[test]
    fn backup_dir_and_files_are_private() {
        let base = std::env::temp_dir().join(format!("sm-db-perm-{}", std::process::id()));
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = base.join("backups/postgres");
        private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        // An existing 755 folder (as on the server) is tightened.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        // A leftover world-readable .part is replaced by a 600 one.
        let part = dir.join("x-20261019-020000.dump.part");
        std::fs::write(&part, "old").unwrap();
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o644)).unwrap();
        private_file(&part).unwrap();
        assert_eq!(mode(&part), 0o600);
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 0);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "1 KB");
        assert_eq!(human(412 << 20), "412 MB");
        assert_eq!(human(3 << 29), "1.5 GB");
    }
}
