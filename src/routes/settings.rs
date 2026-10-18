//! Phase 7 · Settings: server facts, port range for new apps, deploy key rotation (+ per-repo
//! access check), HTTPS token, webhook secret, alert thresholds. Routes are relative to /api and
//! already behind auth. Every write goes to `activity::log` (never with a secret in it).
//!
//! Keys in `settings`: `port_from`, `port_to`, `alert_*` (read by metrics.rs), `disk_stop_pct`
//! (read by deploys.rs, kept equal to `alert_disk_stop`), `webhook_secret` (deploys.rs).
//! The HTTPS token is a file, `<data>/https_token` (0600), not a row.

use super::{apps, deploys, metrics};
use crate::{
    activity,
    auth::{Admin, write_private},
    exec::{Cmd, Exit},
    http::{ApiResult, AppError},
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::State,
    http::{HeaderMap, header},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_PORTS: (i64, i64) = (3001, 3999);

async fn get_setting(st: &AppState, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten()
}

async fn set_setting(st: &AppState, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
    )
    .bind(key)
    .bind(value)
    .execute(&st.db)
    .await
    .map(|_| ())
}

/// Ports offered to new apps (apps.rs `suggest_port`).
pub async fn port_range(st: &AppState) -> (i64, i64) {
    let n = |v: Option<String>| v.and_then(|v| v.parse::<i64>().ok());
    match (
        n(get_setting(st, "port_from").await),
        n(get_setting(st, "port_to").await),
    ) {
        (Some(a), Some(b)) if ports_ok(a, b) => (a, b),
        _ => DEFAULT_PORTS,
    }
}

fn ports_ok(from: i64, to: i64) -> bool {
    (1024..=65535).contains(&from) && (1024..=65535).contains(&to) && from <= to
}

/// Repos cloned over SSH, i.e. with the deploy key.
fn uses_key(repo: &str) -> bool {
    repo.starts_with("git@") || repo.starts_with("ssh://")
}

// ---------- server facts ----------

async fn hostname() -> String {
    Cmd::new("hostname")
        .output()
        .await
        .map(|o| o.stdout.trim().to_owned())
        .unwrap_or_default()
}

fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_owned()))
        })
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

fn timezone() -> String {
    std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .or_else(|| {
            let l = std::fs::read_link("/etc/localtime").ok()?;
            let s = l.to_string_lossy();
            s.split_once("zoneinfo/").map(|(_, z)| z.to_owned())
        })
        .unwrap_or_else(|| "UTC".into())
}

// ---------- GET ----------

async fn key_state(st: &AppState) -> ApiResult<Value> {
    let cur = apps::ensure_deploy_key(st).await?;
    let next = apps::next_key_path(st);
    let next = if next.exists() {
        Some(apps::key_info(&next).await?)
    } else {
        None
    };
    let repos: Vec<(String, String)> = sqlx::query_as("SELECT name, repo_url FROM apps ORDER BY name")
        .fetch_all(&st.db)
        .await?;
    let used_by: Vec<&String> = repos.iter().filter(|(_, r)| uses_key(r)).map(|(n, _)| n).collect();
    Ok(json!({ "current": apps::key_info(&cur).await?, "next": next, "usedBy": used_by }))
}

/// GET /settings
async fn show(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let (from, to) = port_range(&st).await;
    let public = st
        .cfg
        .public_url(headers.get(header::HOST).and_then(|v| v.to_str().ok()));
    deploys::webhook_secret(&st).await?;
    Ok(Json(json!({
        "server": {
            "host": hostname().await,
            "os": os_name(),
            "timezone": timezone(),
            "agent": format!("server-manager {}", env!("CARGO_PKG_VERSION")),
        },
        "appsDir": st.cfg.apps_dir,
        "portFrom": from,
        "portTo": to,
        "alerts": metrics::thresholds(&st.db).await,
        "httpsToken": apps::https_token_path(&st).exists(),
        "publicUrl": public,
        "webhookUrl": format!("{public}/hooks/<app>"),
        "deployKey": key_state(&st).await?,
    })))
}

// ---------- PUT defaults + alerts ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Alerts {
    disk_warn: f64,
    disk_stop: f64,
    cpu_pct: f64,
    cpu_minutes: f64,
    mem_warn: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveBody {
    port_from: i64,
    port_to: i64,
    alerts: Alerts,
}

fn check(b: &SaveBody) -> Result<(), AppError> {
    if !ports_ok(b.port_from, b.port_to) {
        return Err(AppError::bad_request(
            "Port range: two ports between 1024 and 65535, the first not above the second.",
        ));
    }
    let a = &b.alerts;
    let pct = |v: f64| v.is_finite() && (1.0..=100.0).contains(&v);
    if ![a.disk_warn, a.disk_stop, a.cpu_pct, a.mem_warn].into_iter().all(pct) {
        return Err(AppError::bad_request("Alert levels are percentages from 1 to 100."));
    }
    if a.disk_warn >= a.disk_stop {
        return Err(AppError::bad_request("Disk warning must be below the disk stop level."));
    }
    if !(a.cpu_minutes.is_finite() && (1.0..=60.0).contains(&a.cpu_minutes)) {
        return Err(AppError::bad_request("CPU high for: 1 to 60 minutes."));
    }
    Ok(())
}

/// PUT /settings {portFrom, portTo, alerts}
async fn save(
    State(st): State<AppState>,
    Extension(adm): Extension<Admin>,
    Json(b): Json<SaveBody>,
) -> ApiResult<Json<Value>> {
    check(&b)?;
    let a = &b.alerts;
    let pairs = [
        ("port_from", b.port_from.to_string()),
        ("port_to", b.port_to.to_string()),
        ("alert_disk_warn", a.disk_warn.to_string()),
        ("alert_disk_stop", a.disk_stop.to_string()),
        ("disk_stop_pct", a.disk_stop.to_string()),
        ("alert_cpu_pct", a.cpu_pct.to_string()),
        ("alert_cpu_minutes", a.cpu_minutes.to_string()),
        ("alert_mem_warn", a.mem_warn.to_string()),
    ];
    for (k, v) in &pairs {
        set_setting(&st, k, v).await?;
    }
    let detail = format!(
        "ports {}-{} · disk {}/{}% · cpu {}% for {} min · mem {}%",
        b.port_from, b.port_to, a.disk_warn, a.disk_stop, a.cpu_pct, a.cpu_minutes, a.mem_warn
    );
    activity::log(&st.db, &adm.username, "settings.update", "server", &detail).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------- deploy key ----------

/// POST /settings/deploy-key/new → the new public key (the old key keeps working).
async fn key_new(State(st): State<AppState>, Extension(adm): Extension<Admin>) -> ApiResult<Json<Value>> {
    let existed = apps::next_key_path(&st).exists();
    let path = apps::create_next_key(&st).await?;
    let info = apps::key_info(&path).await?;
    if !existed {
        let fp = info["fingerprint"].as_str().unwrap_or_default().to_owned();
        activity::log(&st.db, &adm.username, "settings.deploy_key_created", "deploy key", &fp).await;
    }
    Ok(Json(json!({ "key": info })))
}

/// POST /settings/deploy-key/check → `git ls-remote` per SSH repo with the NEW key alone
/// (the current key when no rotation is running).
async fn key_check(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let next = apps::next_key_path(&st);
    let (key, which) = if next.exists() {
        (next, "new")
    } else {
        (apps::ensure_deploy_key(&st).await?, "current")
    };
    let repos: Vec<(String, String)> = sqlx::query_as("SELECT name, repo_url FROM apps ORDER BY name")
        .fetch_all(&st.db)
        .await?;
    let mut set = tokio::task::JoinSet::new();
    for (i, (name, repo)) in repos.into_iter().filter(|(_, r)| uses_key(r)).enumerate() {
        let cmd = apps::git_with_keys(&st, std::slice::from_ref(&key))
            .args(["ls-remote", "--heads", "--", &repo])
            .timeout(Duration::from_secs(20));
        set.spawn(async move {
            let (ok, message) = match cmd.output().await {
                Ok(o) if o.exit.success() => (true, "Key works".to_owned()),
                Ok(o) if o.exit == Exit::TimedOut => (false, "No answer from the git host after 20 s.".to_owned()),
                Ok(o) => (false, apps::git_error(&o.stderr)),
                Err(e) => (false, format!("could not run git: {e}")),
            };
            (i, json!({ "app": name, "repo": repo, "ok": ok, "message": message }))
        });
    }
    let mut rows = set.join_all().await;
    rows.sort_by_key(|(i, _)| *i);
    let rows: Vec<Value> = rows.into_iter().map(|(_, v)| v).collect();
    Ok(Json(json!({ "key": which, "repos": rows })))
}

/// POST /settings/deploy-key/remove-old → the new key becomes the only key.
async fn key_remove_old(State(st): State<AppState>, Extension(adm): Extension<Admin>) -> ApiResult<Json<Value>> {
    let old = apps::key_info(&apps::deploy_key_path(&st)).await?;
    apps::promote_next_key(&st).await?;
    let fp = old["fingerprint"].as_str().unwrap_or_default().to_owned();
    activity::log(
        &st.db,
        &adm.username,
        "settings.deploy_key_removed",
        "deploy key",
        &format!("removed {fp}"),
    )
    .await;
    Ok(Json(json!({ "deployKey": key_state(&st).await? })))
}

// ---------- HTTPS token ----------

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

/// PUT /settings/https-token {token}: written to `<data>/https_token` (0600), never returned.
async fn token_set(
    State(st): State<AppState>,
    Extension(adm): Extension<Admin>,
    Json(b): Json<TokenBody>,
) -> ApiResult<Json<Value>> {
    let t = b.token.trim();
    if !(20..=255).contains(&t.len()) || !t.bytes().all(|c| c.is_ascii_graphic() && c != b'\'') {
        return Err(AppError::bad_request(
            "Token: paste the whole GitHub token (20-255 characters, no spaces).",
        ));
    }
    write_private(&apps::https_token_path(&st), t)?;
    activity::log(&st.db, &adm.username, "settings.https_token_set", "https token", "").await;
    Ok(Json(json!({ "httpsToken": true })))
}

/// DELETE /settings/https-token
async fn token_clear(State(st): State<AppState>, Extension(adm): Extension<Admin>) -> ApiResult<Json<Value>> {
    let p = apps::https_token_path(&st);
    if p.exists() {
        std::fs::remove_file(p)?;
        activity::log(&st.db, &adm.username, "settings.https_token_removed", "https token", "").await;
    }
    Ok(Json(json!({ "httpsToken": false })))
}

// ---------- webhook secret ----------

fn random_hex(n: usize) -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut b = vec![0u8; n];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

/// POST /settings/webhook-secret/rotate → the new secret, shown once. Apps with their own
/// secret keep it.
async fn secret_rotate(State(st): State<AppState>, Extension(adm): Extension<Admin>) -> ApiResult<Json<Value>> {
    let s = random_hex(32);
    set_setting(&st, "webhook_secret", &s).await?;
    activity::log(&st.db, &adm.username, "settings.webhook_secret_rotated", "webhook", "").await;
    Ok(Json(json!({ "secret": s })))
}

/// POST /settings/webhook-secret/reveal → the secret (logged, like env reveal).
async fn secret_reveal(State(st): State<AppState>, Extension(adm): Extension<Admin>) -> ApiResult<Json<Value>> {
    let s = deploys::webhook_secret(&st).await?;
    activity::log(&st.db, &adm.username, "settings.webhook_secret_revealed", "webhook", "").await;
    Ok(Json(json!({ "secret": s })))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/settings", get(show).put(save))
        .route("/settings/deploy-key/new", post(key_new))
        .route("/settings/deploy-key/check", post(key_check))
        .route("/settings/deploy-key/remove-old", post(key_remove_old))
        .route("/settings/https-token", put(token_set).delete(token_clear))
        .route("/settings/webhook-secret/rotate", post(secret_rotate))
        .route("/settings/webhook-secret/reveal", post(secret_reveal))
}

pub fn spawn(_st: &AppState) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(from: i64, to: i64, warn: f64, stop: f64, mins: f64) -> SaveBody {
        SaveBody {
            port_from: from,
            port_to: to,
            alerts: Alerts {
                disk_warn: warn,
                disk_stop: stop,
                cpu_pct: 80.0,
                cpu_minutes: mins,
                mem_warn: 90.0,
            },
        }
    }

    #[test]
    fn settings_validation() {
        assert!(check(&body(3010, 3099, 85.0, 95.0, 5.0)).is_ok());
        assert!(check(&body(3010, 3010, 85.0, 95.0, 5.0)).is_ok(), "one port is a range");
        for bad in [
            body(3099, 3010, 85.0, 95.0, 5.0),  // reversed
            body(80, 3010, 85.0, 95.0, 5.0),    // privileged
            body(3010, 70000, 85.0, 95.0, 5.0), // out of range
            body(3010, 3099, 95.0, 95.0, 5.0),  // warn not below stop
            body(3010, 3099, 85.0, 101.0, 5.0), // > 100 %
            body(3010, 3099, 0.0, 95.0, 5.0),   // 0 %
            body(3010, 3099, f64::NAN, 95.0, 5.0),
            body(3010, 3099, 85.0, 95.0, 0.0),  // 0 min
            body(3010, 3099, 85.0, 95.0, 61.0), // > 1 h
        ] {
            assert!(
                check(&bad).is_err(),
                "{} {} {}",
                bad.port_from,
                bad.port_to,
                bad.alerts.disk_warn
            );
        }
    }

    #[tokio::test]
    async fn port_range_defaults_and_saved_values() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        let st = AppState::new(
            db,
            crate::config::Config::test("/tmp".into(), "/tmp".into()),
            vec![0; 32],
        );
        assert_eq!(port_range(&st).await, DEFAULT_PORTS);
        set_setting(&st, "port_from", "3010").await.unwrap();
        set_setting(&st, "port_to", "3099").await.unwrap();
        assert_eq!(port_range(&st).await, (3010, 3099));
        set_setting(&st, "port_to", "10").await.unwrap();
        assert_eq!(port_range(&st).await, DEFAULT_PORTS, "a broken row falls back");
        assert!(uses_key("git@github.com:a/b.git") && uses_key("ssh://git@h/x") && !uses_key("https://github.com/a/b"));
    }
}
