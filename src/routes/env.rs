//! Phase 4 · Environment: read/write `<apps_dir>/<name>/shared/.env` (mode 600), secret masking,
//! reveal, paste + diff, optional restart. Routes are relative to /api and already behind auth.
//!
//! - The file is the source of truth for values. SQLite (`env_meta`) only remembers which keys are
//!   secret and where a key came from. Comments and blank lines in the file survive every save.
//! - Secret values never leave the server except through `POST .../env/reveal` (logged).
//! - Nothing here logs a value: activity rows and stderr carry key names only.

use crate::{
    activity,
    auth::Admin,
    exec::Cmd,
    http::{ApiResult, AppError},
    state::AppState,
    validate,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path as FsPath, PathBuf},
    time::Duration,
};

/// Read-modify-write on any .env is serialized here, so the editor and the databases phase can't
/// overwrite each other. ponytail: one global lock; per-app locks if saves ever queue up.
static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const MAX_VARS: usize = 1000;
const MAX_VALUE: usize = 64 * 1024;
const SECRET_HINTS: [&str; 6] = ["SECRET", "KEY", "TOKEN", "DSN", "PASSWORD", "DATABASE_URL"];

// ---------- parse / serialize ----------

#[derive(Debug, Clone, PartialEq)]
enum Line {
    /// Blank line, comment, or a line we could not read: kept verbatim.
    Raw(String),
    /// `line` is 1-based in the parsed text (0 when built in code).
    Var {
        key: String,
        value: String,
        export: bool,
        line: usize,
    },
}

#[derive(Debug, Default, Clone, PartialEq)]
struct EnvFile {
    lines: Vec<Line>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
struct ParseError {
    line: usize,
    message: String,
}

fn dup_msg(key: &str) -> String {
    format!("{key} already exists. Keys are case sensitive; use one.")
}

/// An existing key that equals `key` ignoring case (but not exactly): `port` next to `PORT`.
fn case_clash<'a>(others: impl IntoIterator<Item = &'a str>, key: &str) -> Option<&'a str> {
    others.into_iter().find(|o| *o != key && o.eq_ignore_ascii_case(key))
}

/// Dotenv subset: `[export ]KEY=value`, `#` comments, `'literal'`, `"escaped \n \" \\"` (may span
/// lines), unquoted values end at ` #`. Returns the file plus one error per unreadable line and per
/// repeated key (later duplicates are still kept so nothing is silently lost).
fn parse(text: &str) -> (EnvFile, Vec<ParseError>) {
    let src: Vec<&str> = text.lines().collect();
    let (mut out, mut errs) = (Vec::new(), Vec::new());
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut i = 0;
    while i < src.len() {
        let raw = src[i];
        let t = raw.trim();
        let ln = i + 1;
        let err = move |m: &str| ParseError {
            line: ln,
            message: m.into(),
        };
        if t.is_empty() || t.starts_with('#') {
            out.push(Line::Raw(raw.to_owned()));
            i += 1;
            continue;
        }
        let (body, export) = match t.strip_prefix("export ") {
            Some(b) => (b.trim_start(), true),
            None => (t, false),
        };
        let Some((k, rest)) = body.split_once('=') else {
            errs.push(err("Expected KEY=value."));
            out.push(Line::Raw(raw.to_owned()));
            i += 1;
            continue;
        };
        let key = k.trim_end();
        if validate::env_key(key).is_err() {
            errs.push(err(&format!(
                "Invalid key \"{key}\": use letters, digits and _, not starting with a digit."
            )));
            out.push(Line::Raw(raw.to_owned()));
            i += 1;
            continue;
        }
        let rest = rest.trim_start();
        let start = i;
        let value = if let Some(r) = rest.strip_prefix('"') {
            let (mut buf, mut cur) = (String::new(), r);
            loop {
                if scan_double(cur, &mut buf) {
                    break Some(buf);
                }
                buf.push('\n');
                i += 1;
                match src.get(i) {
                    Some(next) => cur = next,
                    None => break None,
                }
            }
        } else if let Some(r) = rest.strip_prefix('\'') {
            r.find('\'').map(|e| r[..e].to_owned())
        } else {
            let cut = rest
                .match_indices('#')
                .find(|(p, _)| rest[..*p].ends_with([' ', '\t']))
                .map_or(rest.len(), |(p, _)| p);
            Some(rest[..cut].trim_end().to_owned())
        };
        let Some(value) = value else {
            i = start;
            errs.push(err("Unterminated quote."));
            out.push(Line::Raw(raw.to_owned()));
            i += 1;
            continue;
        };
        if seen.insert(key.to_owned(), start + 1).is_some() {
            errs.push(ParseError {
                line: start + 1,
                message: dup_msg(key),
            });
        }
        out.push(Line::Var {
            key: key.to_owned(),
            value,
            export,
            line: start + 1,
        });
        i += 1;
    }
    (EnvFile { lines: out }, errs)
}

/// Read a double-quoted value up to the closing quote; true when the quote closed on this line.
fn scan_double(s: &str, buf: &mut String) -> bool {
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return true,
            '\\' => match it.next() {
                Some('n') => buf.push('\n'),
                Some('r') => buf.push('\r'),
                Some('t') => buf.push('\t'),
                Some(o @ ('"' | '\\')) => buf.push(o),
                Some(o) => {
                    buf.push('\\');
                    buf.push(o);
                }
                None => buf.push('\\'),
            },
            c => buf.push(c),
        }
    }
    false
}

/// Bare when safe for dotenv, systemd EnvironmentFile and `source`; else 'single' (literal), else
/// "double" with escapes (value contains ' or a newline).
fn quote(v: &str) -> String {
    let bare = |c: char| c.is_ascii_alphanumeric() || "_-.,/:@%+=~?".contains(c);
    if v.chars().all(bare) {
        v.to_owned()
    } else if !v.contains('\'') && !v.contains(['\n', '\r']) {
        format!("'{v}'")
    } else {
        let e = v
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
        format!("\"{e}\"")
    }
}

impl EnvFile {
    fn serialize(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            match l {
                Line::Raw(r) => s.push_str(r),
                Line::Var { key, value, export, .. } => {
                    if *export {
                        s.push_str("export ");
                    }
                    s.push_str(key);
                    s.push('=');
                    s.push_str(&quote(value));
                }
            }
            s.push('\n');
        }
        s
    }

    /// Unique keys in file order.
    fn keys(&self) -> Vec<&str> {
        let mut seen = HashSet::new();
        self.lines
            .iter()
            .filter_map(|l| match l {
                Line::Var { key, .. } if seen.insert(key.as_str()) => Some(key.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Dotenv semantics: the last definition wins.
    fn get(&self, k: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|l| match l {
            Line::Var { key, value, .. } if key == k => Some(value.as_str()),
            _ => None,
        })
    }

    /// Overwrite in place (first definition, later duplicates dropped) or append.
    fn set(&mut self, k: &str, v: &str) {
        let mut done = false;
        self.lines.retain_mut(|l| match l {
            Line::Var { key, value, .. } if key == k => {
                if done {
                    return false;
                }
                done = true;
                *value = v.to_owned();
                true
            }
            _ => true,
        });
        if !done {
            self.lines.push(Line::Var {
                key: k.to_owned(),
                value: v.to_owned(),
                export: false,
                line: 0,
            });
        }
    }

    fn remove(&mut self, k: &str) {
        self.lines.retain(|l| !matches!(l, Line::Var { key, .. } if key == k));
    }
}

fn check_value(key: &str, v: &str) -> Result<(), String> {
    if v.len() > MAX_VALUE {
        return Err(format!("{key}: value is longer than {} KB.", MAX_VALUE / 1024));
    }
    if v.contains('\0') {
        return Err(format!("{key}: value contains a NUL character."));
    }
    Ok(())
}

fn msg(e: AppError) -> String {
    match e {
        AppError::Status(_, m) | AppError::Internal(m) => m,
    }
}

/// Name-based guess: SECRET, KEY, TOKEN, DSN, PASSWORD or DATABASE_URL in the key.
fn auto_secret(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    SECRET_HINTS.iter().any(|h| k.contains(h))
}

// ---------- editing (pure) ----------

#[derive(Deserialize)]
struct PutVar {
    key: String,
    /// `null` = keep the value already in the file (the browser never receives secret values).
    value: Option<String>,
    secret: bool,
}

#[derive(Serialize, Default, Debug, PartialEq)]
struct Changes {
    added: Vec<String>,
    changed: Vec<String>,
    removed: Vec<String>,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
    /// Key names only, never values.
    fn summary(&self) -> String {
        let mut parts = Vec::new();
        for (label, keys) in [
            ("added", &self.added),
            ("changed", &self.changed),
            ("removed", &self.removed),
        ] {
            if !keys.is_empty() {
                parts.push(format!("{label} {}", keys.join(", ")));
            }
        }
        let s = parts.join("; ");
        if s.chars().count() > 500 {
            s.chars().take(500).collect::<String>() + "…"
        } else {
            s
        }
    }
}

/// Full replacement: keys missing from `vars` are removed, new keys appended, comments kept.
fn apply_put(orig: &mut EnvFile, vars: &[PutVar]) -> Result<Changes, String> {
    // Edit a copy: a failed put must leave the caller's file untouched.
    let mut work = orig.clone();
    let file = &mut work;
    if vars.len() > MAX_VARS {
        return Err(format!("At most {MAX_VARS} variables."));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for v in vars {
        validate::env_key(&v.key).map_err(msg)?;
        if !seen.insert(&v.key) {
            return Err(dup_msg(&v.key));
        }
        if let Some(val) = &v.value {
            check_value(&v.key, val)?;
        }
    }
    let existing: HashSet<&str> = file.keys().into_iter().collect();
    for (i, v) in vars
        .iter()
        .enumerate()
        .filter(|(_, v)| !existing.contains(v.key.as_str()))
    {
        // `port` next to `PORT`: name the one that was there first, like the design.
        let earlier = vars
            .iter()
            .enumerate()
            .filter(|(j, o)| *j < i || existing.contains(o.key.as_str()))
            .map(|(_, o)| o.key.as_str());
        if let Some(o) = case_clash(earlier, &v.key) {
            return Err(dup_msg(o));
        }
    }
    let mut ch = Changes::default();
    let drop: Vec<String> = file
        .keys()
        .into_iter()
        .filter(|k| !seen.contains(k))
        .map(str::to_owned)
        .collect();
    for k in drop {
        file.remove(&k);
        ch.removed.push(k);
    }
    for v in vars {
        match (file.get(&v.key), &v.value) {
            (None, None) => return Err(format!("{} has no saved value; type one.", v.key)),
            (None, Some(val)) => {
                file.set(&v.key, val);
                ch.added.push(v.key.clone());
            }
            (Some(old), Some(val)) if old != val => {
                file.set(&v.key, val);
                ch.changed.push(v.key.clone());
            }
            _ => {}
        }
    }
    *orig = work;
    Ok(ch)
}

#[derive(Serialize, Debug, PartialEq, Clone)]
struct DiffItem {
    key: String,
    /// new | changed | same
    kind: &'static str,
    secret: bool,
    /// Plain values only; secrets are `null` (masked).
    old: Option<String>,
    new: Option<String>,
    #[serde(skip)]
    value: String,
}

/// What pasting `incoming` into `file` would do. `secret_of` decides the flag per key.
fn diff(file: &EnvFile, incoming: &EnvFile, secret_of: impl Fn(&str, bool) -> bool) -> Vec<DiffItem> {
    let mut out = Vec::new();
    for k in incoming.keys() {
        let new = incoming.get(k).unwrap_or_default();
        let old = file.get(k);
        let secret = secret_of(k, old.is_some());
        let kind = match old {
            None => "new",
            Some(o) if o != new => "changed",
            Some(_) => "same",
        };
        let show = |v: Option<&str>| if secret { None } else { v.map(str::to_owned) };
        out.push(DiffItem {
            key: k.to_owned(),
            kind,
            secret,
            old: show(old),
            new: show(Some(new)),
            value: new.to_owned(),
        });
    }
    out
}

/// Parse errors plus pasted keys that differ from an earlier key only by case.
fn paste_errors(file: &EnvFile, incoming: &EnvFile, mut errs: Vec<ParseError>) -> Vec<ParseError> {
    let existing = file.keys();
    let mut earlier: Vec<&str> = existing.clone();
    for l in &incoming.lines {
        if let Line::Var { key, line, .. } = l {
            if !existing.contains(&key.as_str())
                && let Some(o) = case_clash(earlier.iter().copied(), key)
            {
                errs.push(ParseError {
                    line: *line,
                    message: dup_msg(o),
                });
            }
            earlier.push(key);
        }
    }
    errs.sort_by_key(|e| e.line);
    errs
}

// ---------- files ----------

/// Temp file in the same folder (mode 600 from the first byte), fsync, rename over the target.
fn write_atomic(path: &FsPath, text: &str) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent folder"))?;
    std::fs::create_dir_all(dir)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = dir.join(format!(".env.{}.{nanos}.tmp", std::process::id()));
    let r = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        std::fs::remove_file(&tmp).ok();
    }
    r
}

struct Loaded {
    file: EnvFile,
    mtime: Option<i64>,
    rev: String,
}

fn load(path: &FsPath) -> ApiResult<Loaded> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let m = std::fs::metadata(path)?;
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            Ok(Loaded {
                file: parse(&text).0,
                mtime: t.map(|d| d.as_secs() as i64),
                rev: format!("{}-{}", t.map_or(0, |d| d.as_nanos()), m.len()),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Loaded {
            file: EnvFile::default(),
            mtime: None,
            rev: "none".into(),
        }),
        Err(e) => Err(e.into()),
    }
}

// ---------- app + meta ----------

struct Target {
    id: i64,
    name: String,
    path: PathBuf,
    /// apps.run_as: owner of the .env file (the unit reads it as root, the app user may too).
    run_as: String,
}

/// write_atomic (mode 600), then hand the file to the app's run_as user when the manager is root.
async fn write_env(t: &Target, text: &str) -> std::io::Result<()> {
    write_atomic(&t.path, text)?;
    super::apps::chown_to(&t.run_as, &t.path).await
}

async fn target(st: &AppState, name: &str) -> ApiResult<Target> {
    validate::app_name(name)?;
    let row: Option<(i64, String)> = sqlx::query_as("SELECT id, run_as FROM apps WHERE name = ?")
        .bind(name)
        .fetch_optional(&st.db)
        .await?;
    let (id, run_as) = row.ok_or_else(|| AppError::not_found("App not found."))?;
    let path = validate::path_under(
        &st.cfg.apps_dir,
        &st.cfg.apps_dir.join(name).join("shared").join(".env"),
    )?;
    Ok(Target {
        id,
        name: name.to_owned(),
        path,
        run_as,
    })
}

#[derive(Default, Clone)]
struct Meta {
    secret: bool,
    source: Option<String>,
}

async fn load_meta(st: &AppState, app_id: i64) -> ApiResult<HashMap<String, Meta>> {
    let rows: Vec<(String, bool, Option<String>)> =
        sqlx::query_as("SELECT key, secret, source FROM env_meta WHERE app_id = ?")
            .bind(app_id)
            .fetch_all(&st.db)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(k, secret, source)| (k, Meta { secret, source }))
        .collect())
}

fn is_secret(meta: &HashMap<String, Meta>, key: &str) -> bool {
    meta.get(key).map_or_else(|| auto_secret(key), |m| m.secret)
}

async fn save_meta(
    st: &AppState,
    app_id: i64,
    key: &str,
    secret: bool,
    source: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO env_meta (app_id, key, secret, source) VALUES (?, ?, ?, ?)
         ON CONFLICT (app_id, key) DO UPDATE SET secret = excluded.secret, source = excluded.source",
    )
    .bind(app_id)
    .bind(key)
    .bind(secret)
    .bind(source)
    .execute(&st.db)
    .await?;
    Ok(())
}

async fn prune_meta(st: &AppState, app_id: i64, file: &EnvFile) -> Result<(), sqlx::Error> {
    let keep: HashSet<&str> = file.keys().into_iter().collect();
    let rows: Vec<String> = sqlx::query_scalar("SELECT key FROM env_meta WHERE app_id = ?")
        .bind(app_id)
        .fetch_all(&st.db)
        .await?;
    for k in rows.iter().filter(|k| !keep.contains(k.as_str())) {
        sqlx::query("DELETE FROM env_meta WHERE app_id = ? AND key = ?")
            .bind(app_id)
            .bind(k)
            .execute(&st.db)
            .await?;
    }
    Ok(())
}

// ---------- restart ----------

#[derive(Serialize)]
struct RestartOut {
    ok: bool,
    skipped: bool,
    note: String,
}

/// `systemctl restart app-<name>.service` (system unit). Skipped on macOS (no systemd).
async fn restart(name: &str) -> RestartOut {
    if cfg!(target_os = "macos") {
        return RestartOut {
            ok: false,
            skipped: true,
            note: "Restart skipped: systemd is not available on macOS.".into(),
        };
    }
    let unit = format!("app-{name}.service");
    let r = Cmd::new("systemctl")
        .args(["restart", &unit])
        .timeout(Duration::from_secs(30))
        .output()
        .await;
    match r {
        Ok(o) if o.exit.success() => RestartOut {
            ok: true,
            skipped: false,
            note: format!("Restarted {unit}."),
        },
        Ok(o) => {
            let err: String = o.stderr.trim().chars().take(300).collect();
            RestartOut {
                ok: false,
                skipped: false,
                note: format!("Restart failed ({:?}): {err}", o.exit),
            }
        }
        Err(e) => RestartOut {
            ok: false,
            skipped: false,
            note: format!("Could not run systemctl: {e}"),
        },
    }
}

// ---------- API ----------

#[derive(Serialize)]
struct VarOut {
    key: String,
    /// `null` for secrets.
    value: Option<String>,
    secret: bool,
    /// Where the key came from (`database:<name>`), if not typed by the owner.
    source: Option<String>,
    empty: bool,
}

#[derive(Serialize)]
struct EnvOut {
    path: String,
    exists: bool,
    mtime: Option<i64>,
    /// Opaque; send it back as `rev` on save to detect changes made in between.
    rev: String,
    vars: Vec<VarOut>,
}

async fn snapshot(st: &AppState, t: &Target) -> ApiResult<EnvOut> {
    let l = load(&t.path)?;
    let meta = load_meta(st, t.id).await?;
    let vars = l
        .file
        .keys()
        .into_iter()
        .map(|k| {
            let v = l.file.get(k).unwrap_or_default();
            let secret = is_secret(&meta, k);
            VarOut {
                key: k.to_owned(),
                value: if secret { None } else { Some(v.to_owned()) },
                secret,
                source: meta.get(k).and_then(|m| m.source.clone()),
                empty: v.is_empty(),
            }
        })
        .collect();
    Ok(EnvOut {
        path: t.path.display().to_string(),
        exists: l.mtime.is_some(),
        mtime: l.mtime,
        rev: l.rev,
        vars,
    })
}

/// GET /apps/{name}/env
async fn show(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Json<EnvOut>> {
    let t = target(&st, &name).await?;
    Ok(Json(snapshot(&st, &t).await?))
}

#[derive(Serialize)]
struct Saved {
    env: EnvOut,
    changes: Changes,
    restart: Option<RestartOut>,
}

#[derive(Deserialize)]
struct PutBody {
    vars: Vec<PutVar>,
    /// `rev` from the last GET; a different value on disk answers 409.
    rev: Option<String>,
    #[serde(default)]
    restart: bool,
}

/// PUT /apps/{name}/env  (full replacement, see `apply_put`)
async fn save(
    State(st): State<AppState>,
    Extension(me): Extension<Admin>,
    Path(name): Path<String>,
    Json(body): Json<PutBody>,
) -> ApiResult<Json<Saved>> {
    let t = target(&st, &name).await?;
    let _g = WRITE_LOCK.lock().await;
    let mut l = load(&t.path)?;
    if body.rev.as_deref().is_some_and(|r| r != l.rev) {
        return Err(AppError::Status(
            axum::http::StatusCode::CONFLICT,
            "The .env changed on the server since you opened it. Reload and apply your changes again.".into(),
        ));
    }
    let meta = load_meta(&st, t.id).await?;
    let changes = apply_put(&mut l.file, &body.vars).map_err(AppError::bad_request)?;
    if !changes.is_empty() || l.mtime.is_none() {
        write_env(&t, &l.file.serialize()).await?;
    }
    for v in &body.vars {
        let source = if changes.changed.contains(&v.key) {
            None
        } else {
            meta.get(&v.key).and_then(|m| m.source.as_deref())
        };
        save_meta(&st, t.id, &v.key, v.secret, source).await?;
    }
    prune_meta(&st, t.id, &l.file).await?;
    drop(_g);
    let restart = if body.restart {
        Some(restart(&t.name).await)
    } else {
        None
    };
    let detail = format!(
        "{}{}",
        if changes.is_empty() {
            "no changes".to_owned()
        } else {
            changes.summary()
        },
        match &restart {
            Some(r) if r.ok => "; restarted",
            Some(r) if r.skipped => "; restart skipped",
            Some(_) => "; restart failed",
            None => "",
        }
    );
    activity::log(&st.db, &me.username, "env.update", &t.name, &detail).await;
    Ok(Json(Saved {
        env: snapshot(&st, &t).await?,
        changes,
        restart,
    }))
}

#[derive(Deserialize)]
struct RevealBody {
    key: String,
}

/// POST /apps/{name}/env/reveal  {key} → {value}. Logged (key name only).
async fn reveal(
    State(st): State<AppState>,
    Extension(me): Extension<Admin>,
    Path(name): Path<String>,
    Json(body): Json<RevealBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let t = target(&st, &name).await?;
    validate::env_key(&body.key)?;
    let l = load(&t.path)?;
    let value = l
        .file
        .get(&body.key)
        .ok_or_else(|| AppError::not_found(format!("{} is not in the .env.", body.key)))?
        .to_owned();
    activity::log(&st.db, &me.username, "env.reveal", &t.name, &body.key).await;
    Ok(Json(serde_json::json!({ "value": value })))
}

#[derive(Deserialize)]
struct PasteBody {
    text: String,
    /// Flag NEW keys that look secret (SECRET, KEY, TOKEN, DSN, PASSWORD, DATABASE_URL).
    #[serde(default)]
    mark_secrets: bool,
    #[serde(default)]
    restart: bool,
}

#[derive(Serialize)]
struct Preview {
    items: Vec<DiffItem>,
    errors: Vec<ParseError>,
    new: usize,
    changed: usize,
    same: usize,
}

async fn plan(st: &AppState, t: &Target, body: &PasteBody) -> ApiResult<(Loaded, Vec<DiffItem>, Vec<ParseError>)> {
    if body.text.len() > 1 << 20 {
        return Err(AppError::bad_request("Pasted text is larger than 1 MB."));
    }
    let l = load(&t.path)?;
    let meta = load_meta(st, t.id).await?;
    let (incoming, errs) = parse(&body.text);
    let errs = paste_errors(&l.file, &incoming, errs);
    let items = diff(&l.file, &incoming, |k, exists| {
        if exists || meta.contains_key(k) {
            is_secret(&meta, k)
        } else {
            body.mark_secrets && auto_secret(k)
        }
    });
    for i in &items {
        check_value(&i.key, &i.value).map_err(AppError::bad_request)?;
    }
    if l.file.keys().len() + items.iter().filter(|i| i.kind == "new").count() > MAX_VARS {
        return Err(AppError::bad_request(format!("At most {MAX_VARS} variables.")));
    }
    Ok((l, items, errs))
}

/// POST /apps/{name}/env/preview  {text, mark_secrets} → diff, nothing is written.
async fn preview(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<PasteBody>,
) -> ApiResult<Json<Preview>> {
    let t = target(&st, &name).await?;
    let (_, items, errors) = plan(&st, &t, &body).await?;
    let count = |k| items.iter().filter(|i| i.kind == k).count();
    Ok(Json(Preview {
        new: count("new"),
        changed: count("changed"),
        same: count("same"),
        items,
        errors,
    }))
}

/// POST /apps/{name}/env/import  {text, mark_secrets, restart}: merge a pasted .env (keys not in
/// the paste stay). Refuses when the preview would show errors.
async fn import(
    State(st): State<AppState>,
    Extension(me): Extension<Admin>,
    Path(name): Path<String>,
    Json(body): Json<PasteBody>,
) -> ApiResult<Json<Saved>> {
    let t = target(&st, &name).await?;
    let _g = WRITE_LOCK.lock().await;
    let (mut l, items, errors) = plan(&st, &t, &body).await?;
    if let Some(e) = errors.first() {
        return Err(AppError::bad_request(format!("Line {}: {}", e.line, e.message)));
    }
    let mut changes = Changes::default();
    for i in items.iter().filter(|i| i.kind != "same") {
        l.file.set(&i.key, &i.value);
        if i.kind == "new" {
            &mut changes.added
        } else {
            &mut changes.changed
        }
        .push(i.key.clone());
    }
    if !changes.is_empty() {
        write_env(&t, &l.file.serialize()).await?;
    }
    // A pasted value replaces whatever the key was before, so its source is dropped too.
    for i in items.iter().filter(|i| i.kind != "same") {
        save_meta(&st, t.id, &i.key, i.secret, None).await?;
    }
    drop(_g);
    let restart = if body.restart && !changes.is_empty() {
        Some(restart(&t.name).await)
    } else {
        None
    };
    let detail = format!(
        "import: {}",
        if changes.is_empty() {
            "no changes".into()
        } else {
            changes.summary()
        }
    );
    activity::log(&st.db, &me.username, "env.import", &t.name, &detail).await;
    Ok(Json(Saved {
        env: snapshot(&st, &t).await?,
        changes,
        restart,
    }))
}

/// Set or overwrite variables in an app's `shared/.env` from server code (the databases phase
/// writes `DATABASE_URL` here after creating a database).
///
/// - Writes atomically with mode 600; comments, order and other keys are untouched; new keys are
///   appended.
/// - Every key is flagged secret, and `source` is recorded (use `Some("database:<db name>")`, the
///   UI shows "From database <db name>"). Pass `None` for no source.
/// - Logs one `env.upsert` activity row (actor `system`) with key names only, never values.
/// - Does not restart the app. Errors are plain messages: unknown app, invalid key, I/O.
#[allow(dead_code)] // called by phase 5 (routes/databases.rs)
pub async fn upsert_vars(
    st: &AppState,
    app: &str,
    vars: &[(String, String)],
    source: Option<&str>,
) -> Result<(), String> {
    let t = target(st, app).await.map_err(msg)?;
    for (k, v) in vars {
        validate::env_key(k).map_err(msg)?;
        check_value(k, v)?;
    }
    let _g = WRITE_LOCK.lock().await;
    let mut l = load(&t.path).map_err(msg)?;
    for (k, v) in vars {
        l.file.set(k, v);
    }
    write_env(&t, &l.file.serialize()).await.map_err(|e| format!("io: {e}"))?;
    for (k, _) in vars {
        save_meta(st, t.id, k, true, source)
            .await
            .map_err(|e| format!("db: {e}"))?;
    }
    drop(_g);
    let keys: Vec<&str> = vars.iter().map(|(k, _)| k.as_str()).collect();
    let src = source.map(|s| format!(" ({s})")).unwrap_or_default();
    activity::log(
        &st.db,
        "system",
        "env.upsert",
        &t.name,
        &format!("set {}{src}", keys.join(", ")),
    )
    .await;
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps/{name}/env", get(show).put(save))
        .route("/apps/{name}/env/reveal", post(reveal))
        .route("/apps/{name}/env/preview", post(preview))
        .route("/apps/{name}/env/import", post(import))
}

pub fn spawn(_st: &AppState) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::os::unix::fs::PermissionsExt;

    fn kv(f: &EnvFile) -> Vec<(String, String)> {
        f.lines
            .iter()
            .filter_map(|l| match l {
                Line::Var { key, value, .. } => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    fn put(key: &str, value: Option<&str>) -> PutVar {
        PutVar {
            key: key.into(),
            value: value.map(Into::into),
            secret: false,
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sm-env-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_quotes_comments_export_and_equals() {
        let text = "# top\n\nexport PORT=3010\nURL=postgres://u:p@h/db?a=b&c=d\nEMPTY=\nQ=\"two words\"\nS='it is $HOME'\nC=value # trailing\nHASH=a#b\nML=\"l1\nl2\"\nESC=\"a\\nb \\\"q\\\"\"\n";
        let (f, errs) = parse(text);
        assert!(errs.is_empty(), "{errs:?}");
        let m: HashMap<_, _> = kv(&f).into_iter().collect();
        assert_eq!(m["PORT"], "3010");
        assert_eq!(m["URL"], "postgres://u:p@h/db?a=b&c=d");
        assert_eq!(m["EMPTY"], "");
        assert_eq!(m["Q"], "two words");
        assert_eq!(m["S"], "it is $HOME");
        assert_eq!(m["C"], "value");
        assert_eq!(m["HASH"], "a#b");
        assert_eq!(m["ML"], "l1\nl2");
        assert_eq!(m["ESC"], "a\nb \"q\"");
        assert!(matches!(f.lines[2], Line::Var { export: true, .. }));
    }

    #[test]
    fn round_trip_keeps_order_comments_and_values() {
        let text = "# top\n\nexport PORT=3010\nURL=postgres://u:p@h/db?a=b&c=d\nEMPTY=\nQ='two words'\nHASH='a #b'\nML=\"l1\\nl2\"\nQUOTE=\"it's\"\n# end\n";
        let (f, errs) = parse(text);
        assert!(errs.is_empty(), "{errs:?}");
        let out = f.serialize();
        let (g, _) = parse(&out);
        assert_eq!(kv(&f), kv(&g));
        let strip = |s: &str| s.lines().filter(|l| l.starts_with('#') || l.is_empty()).count();
        assert_eq!(strip(text), strip(&out));
        assert_eq!(out.lines().next(), Some("# top"));
        assert!(out.contains("export PORT=3010\n"));
        assert!(out.contains("EMPTY=\n"));
        // already canonical text is byte-identical
        let canon = "# c\nA=1\nB='x y'\nC=\n";
        assert_eq!(parse(canon).0.serialize(), canon);
    }

    #[test]
    fn quoting_survives_hostile_values() {
        for v in [
            "",
            "plain",
            "a b",
            "it's",
            "a\"b",
            "back\\slash",
            "line1\nline2",
            "$HOME `id` ; rm",
            "#x",
            " lead",
            "trail ",
            "a=b=c",
            "'",
            "\"",
            "\\n",
        ] {
            let mut f = EnvFile::default();
            f.set("K", v);
            f.set("AFTER", "ok");
            let (g, errs) = parse(&f.serialize());
            assert!(errs.is_empty(), "{v:?}: {errs:?}");
            // unquoted values are trimmed by design; every other value must come back exactly
            assert_eq!(g.get("K"), Some(v), "{v:?} -> {:?}", f.serialize());
            assert_eq!(g.get("AFTER"), Some("ok"));
        }
    }

    #[test]
    fn reports_bad_lines_and_duplicates() {
        let (f, errs) = parse("OK=1\nnot a pair\n1BAD=x\nA=1\nA=2\nQ=\"open\nNEXT=1\n");
        let lines: Vec<usize> = errs.iter().map(|e| e.line).collect();
        assert_eq!(lines, vec![2, 3, 5, 6], "{errs:?}");
        assert_eq!(errs[2].message, "A already exists. Keys are case sensitive; use one.");
        assert_eq!(f.get("A"), Some("2"), "last wins");
        assert_eq!(f.keys(), vec!["OK", "A", "NEXT"]);
        // unreadable lines are kept verbatim
        assert!(f.serialize().contains("not a pair\n"));
    }

    #[test]
    fn put_rejects_duplicates_and_case_clashes() {
        let mut f = parse("PORT=3010\n").0;
        let e = apply_put(&mut f, &[put("PORT", None), put("PORT", Some("1"))]).unwrap_err();
        assert_eq!(e, "PORT already exists. Keys are case sensitive; use one.");
        let e = apply_put(&mut f, &[put("PORT", None), put("port", Some("3011"))]).unwrap_err();
        assert_eq!(e, "PORT already exists. Keys are case sensitive; use one.");
        assert!(apply_put(&mut f, &[put("A-B", Some("1"))]).is_err());
        assert!(
            apply_put(&mut f, &[put("NEW", None)])
                .unwrap_err()
                .contains("no saved value")
        );
        assert!(apply_put(&mut f, &[put("X", Some("a\0b"))]).is_err());
        assert_eq!(f.serialize(), "PORT=3010\n", "failed puts change nothing");
        // an old file that already has both cases can still be saved
        let mut g = parse("Path=a\nPATH=b\n").0;
        assert!(apply_put(&mut g, &[put("Path", None), put("PATH", None)]).is_ok());
    }

    #[test]
    fn put_replaces_and_keeps_comments() {
        let mut f = parse("# keep me\nA=1\nB=2\n\nC=3\n").0;
        let ch = apply_put(&mut f, &[put("C", None), put("A", Some("9")), put("D", Some("4"))]).unwrap();
        assert_eq!(
            ch,
            Changes {
                added: vec!["D".into()],
                changed: vec!["A".into()],
                removed: vec!["B".into()]
            }
        );
        assert_eq!(f.serialize(), "# keep me\nA=9\n\nC=3\nD=4\n");
        assert_eq!(ch.summary(), "added D; changed A; removed B");
    }

    #[test]
    fn diff_new_changed_same_with_masked_secrets() {
        let file = parse("PORT=3010\nRUST_LOG=info\nAPI_TOKEN=old\n").0;
        let (inc, _) = parse("PORT=3010\nRUST_LOG=debug\nAPI_TOKEN=new\nSENTRY_DSN=https://x\nPLAIN=1\n");
        let d = diff(&file, &inc, |k, exists| {
            exists && k == "API_TOKEN" || !exists && auto_secret(k)
        });
        let kinds: Vec<_> = d.iter().map(|i| (i.key.as_str(), i.kind, i.secret)).collect();
        assert_eq!(
            kinds,
            vec![
                ("PORT", "same", false),
                ("RUST_LOG", "changed", false),
                ("API_TOKEN", "changed", true),
                ("SENTRY_DSN", "new", true),
                ("PLAIN", "new", false)
            ]
        );
        assert_eq!(d[1].old.as_deref(), Some("info"));
        assert_eq!(d[1].new.as_deref(), Some("debug"));
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("https://x"), "secret value leaked: {json}");
        assert!(d[2].old.is_none() && d[2].new.is_none());
    }

    #[test]
    fn paste_flags_case_clash_with_line_number() {
        let file = parse("PORT=3010\n").0;
        let (inc, errs) = parse("A=1\nport=3011\n");
        let errs = paste_errors(&file, &inc, errs);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].line, 2);
        assert_eq!(
            errs[0].message,
            "PORT already exists. Keys are case sensitive; use one."
        );
    }

    #[test]
    fn secret_autodetect() {
        for k in [
            "JWT_SECRET",
            "api_key",
            "GITHUB_TOKEN",
            "SENTRY_DSN",
            "ADMIN_PASSWORD_HASH",
            "DATABASE_URL",
            "Stripe_Key",
        ] {
            assert!(auto_secret(k), "{k}");
        }
        for k in ["PORT", "HOST", "RUST_LOG", "CORS_ORIGINS", "TZ"] {
            assert!(!auto_secret(k), "{k}");
        }
    }

    #[test]
    fn atomic_write_is_mode_600_and_leaves_no_temp() {
        let d = tmpdir("atomic");
        let p = d.join("shared").join(".env");
        write_atomic(&p, "A=1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "A=1\n");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        // an existing 644 file is replaced by a 600 one
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&p, "A=2\n").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        let left: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from(".env")]);
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn upsert_vars_writes_file_meta_and_activity_without_values() {
        let d = tmpdir("upsert");
        let apps = d.join("apps");
        std::fs::create_dir_all(apps.join("demo/shared")).unwrap();
        std::fs::write(apps.join("demo/shared/.env"), "# mine\nPORT=3010\n").unwrap();
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        sqlx::query("INSERT INTO apps (name, repo_url, path) VALUES ('demo', 'x', ?)")
            .bind(apps.join("demo").to_str().unwrap())
            .execute(&db)
            .await
            .unwrap();
        let cfg = Config {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir: d.clone(),
            apps_dir: apps.clone(),
            web_dist: d.clone(),
            trust_proxy: false,
        };
        let st = AppState::new(db, cfg, vec![0; 32]);

        let vars = vec![("DATABASE_URL".to_string(), "postgres://u:hunter2pw@h/db".to_string())];
        upsert_vars(&st, "demo", &vars, Some("database:demo_db")).await.unwrap();
        let text = std::fs::read_to_string(apps.join("demo/shared/.env")).unwrap();
        assert!(
            text.starts_with("# mine\nPORT=3010\nDATABASE_URL=postgres://u:hunter2pw@h/db"),
            "{text}"
        );
        let p = apps.join("demo/shared/.env");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);

        let t = target(&st, "demo").await.unwrap();
        let snap = snapshot(&st, &t).await.unwrap();
        let v = snap.vars.iter().find(|v| v.key == "DATABASE_URL").unwrap();
        assert!(v.secret && v.value.is_none());
        assert_eq!(v.source.as_deref(), Some("database:demo_db"));
        let port = snap.vars.iter().find(|v| v.key == "PORT").unwrap();
        assert_eq!(port.value.as_deref(), Some("3010"));
        assert!(!serde_json::to_string(&snap.vars).unwrap().contains("hunter2pw"));

        let rows: Vec<(String, String)> = sqlx::query_as("SELECT action, detail FROM activity")
            .fetch_all(&st.db)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "env.upsert");
        assert!(rows[0].1.contains("DATABASE_URL") && !rows[0].1.contains("hunter2pw"));

        // unknown app / bad key / path escape
        assert!(upsert_vars(&st, "nope", &vars, None).await.is_err());
        assert!(
            upsert_vars(&st, "demo", &[("A B".into(), "x".into())], None)
                .await
                .is_err()
        );
        assert!(target(&st, "../etc").await.is_err());
        std::fs::remove_dir_all(&d).ok();
    }
}
