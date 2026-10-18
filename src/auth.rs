//! Owner login: argon2id hashes in `admin_users`, HS256 JWT in an HttpOnly SameSite=Strict cookie,
//! per-IP lockout (kept in SQLite `login_fails`, so a restart doesn't unlock), a global soft limit
//! against distributed guessing, and the `require_auth` middleware that guards every /api route
//! except login. Logout bumps `token_version`, so the token is dead server-side too.

use crate::{
    activity,
    config::Config,
    http::{ApiResult, AppError, now},
    state::AppState,
};
use argon2::{
    Argon2,
    password_hash::{
        PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
        rand_core::{OsRng, RngCore},
    },
};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    path::Path,
    sync::LazyLock,
    time::Duration,
};

pub const SESSION_SECS: i64 = 12 * 60 * 60;
const COOKIE: &str = "sm_session";
const MAX_FAILS: u32 = 5;
const LOCK_SECS: i64 = 15 * 60;
/// More failed sign-ins than this in a minute, across all IPs, turns on the soft limit.
const GLOBAL_FAILS: usize = 30;
const SLOW_DELAY: Duration = Duration::from_secs(2);
pub const MIN_PASSWORD: usize = 10;

// ---------- lockout ----------

#[derive(Default)]
struct Entry {
    fails: u32,
    last: i64,
    locked_until: i64,
}

/// Failed sign-ins per IP. The attempt is counted *before* the password is checked, so parallel
/// requests can't sneak past the limit. Written through to `login_fails` (see `attempt` below).
/// `recent` = times of failed passwords from any IP, for the global soft limit (memory only).
#[derive(Default)]
pub struct Lockout {
    ips: HashMap<String, Entry>,
    recent: VecDeque<i64>,
}

impl Lockout {
    /// `Err(seconds)` while locked. `Ok(tries_left)` = tries left if *this* attempt fails.
    pub fn attempt(&mut self, ip: &str, now: i64) -> Result<u32, i64> {
        self.ips.retain(|_, e| e.locked_until > now || now - e.last < LOCK_SECS);
        let e = self.ips.entry(ip.to_owned()).or_default();
        if e.locked_until > now {
            return Err(e.locked_until - now);
        }
        e.fails += 1;
        e.last = now;
        if e.fails >= MAX_FAILS {
            e.locked_until = now + LOCK_SECS;
        }
        Ok(MAX_FAILS.saturating_sub(e.fails))
    }

    pub fn success(&mut self, ip: &str) {
        self.ips.remove(ip);
    }

    /// (fails, last, locked_until) for SQLite.
    fn get(&self, ip: &str) -> Option<(u32, i64, i64)> {
        self.ips.get(ip).map(|e| (e.fails, e.last, e.locked_until))
    }

    fn restore(&mut self, ip: String, fails: u32, last: i64, locked_until: i64) {
        self.ips.insert(ip, Entry { fails, last, locked_until });
    }

    /// A wrong password from any IP.
    fn note_fail(&mut self, now: i64) {
        self.recent.push_back(now);
        if self.recent.len() > 10 * GLOBAL_FAILS {
            self.recent.pop_front();
        }
    }

    /// More than GLOBAL_FAILS wrong passwords in the last minute, all IPs together.
    fn busy(&mut self, now: i64) -> bool {
        while self.recent.front().is_some_and(|t| now - t >= 60) {
            self.recent.pop_front();
        }
        self.recent.len() > GLOBAL_FAILS
    }
}

/// Count a sign-in attempt for `ip` and write the entry to `login_fails`.
async fn attempt(st: &AppState, ip: &str) -> ApiResult<Result<u32, i64>> {
    let t = now();
    let (res, snap) = {
        let mut l = st.lockout.lock().unwrap_or_else(|e| e.into_inner());
        let r = l.attempt(ip, t);
        (r, l.get(ip))
    };
    if let Some((fails, last, until)) = snap {
        sqlx::query(
            "INSERT INTO login_fails (ip, fails, last, locked_until) VALUES (?, ?, ?, ?) \
             ON CONFLICT (ip) DO UPDATE SET fails = excluded.fails, last = excluded.last, locked_until = excluded.locked_until",
        )
        .bind(ip)
        .bind(fails)
        .bind(last)
        .bind(until)
        .execute(&st.db)
        .await?;
    }
    sqlx::query("DELETE FROM login_fails WHERE locked_until <= ?1 AND ?1 - last >= ?2")
        .bind(t)
        .bind(LOCK_SECS)
        .execute(&st.db)
        .await?;
    Ok(res)
}

/// Boot: load the lockouts that are still running from `login_fails`.
pub async fn restore_lockouts(st: &AppState) -> Result<(), sqlx::Error> {
    let rows: Vec<(String, i64, i64, i64)> =
        sqlx::query_as("SELECT ip, fails, last, locked_until FROM login_fails").fetch_all(&st.db).await?;
    let mut l = st.lockout.lock().unwrap_or_else(|e| e.into_inner());
    for (ip, fails, last, until) in rows {
        l.restore(ip, fails.clamp(0, MAX_FAILS as i64) as u32, last, until);
    }
    Ok(())
}

/// Serialises the delayed attempts while the soft limit is on, so parallel requests can't
/// overlap their delays: at most one password check per SLOW_DELAY server-wide.
static SLOW: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------- tokens ----------

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Claims {
    sub: i64,
    ver: i64,
    exp: i64,
}

fn sign(secret: &[u8], user_id: i64, ver: i64, now: i64) -> Result<String, AppError> {
    let c = Claims {
        sub: user_id,
        ver,
        exp: now + SESSION_SECS,
    };
    jsonwebtoken::encode(&Header::new(Algorithm::HS256), &c, &EncodingKey::from_secret(secret))
        .map_err(|e| AppError::Internal(format!("jwt: {e}")))
}

fn verify(secret: &[u8], token: &str) -> Option<Claims> {
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 0;
    jsonwebtoken::decode::<Claims>(token, &DecodingKey::from_secret(secret), &v)
        .ok()
        .map(|d| d.claims)
}

/// `SM_JWT_SECRET`, else `<data>/jwt.secret` (created with 32 random bytes, mode 600).
pub fn load_or_create_secret(data_dir: &Path) -> std::io::Result<Vec<u8>> {
    if let Ok(s) = std::env::var("SM_JWT_SECRET")
        && s.len() >= 32
    {
        return Ok(s.into_bytes());
    }
    let path = data_dir.join("jwt.secret");
    if let Ok(s) = std::fs::read_to_string(&path)
        && s.trim().len() >= 32
    {
        return Ok(s.trim().as_bytes().to_vec());
    }
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    write_private(&path, &hex)?;
    Ok(hex.into_bytes())
}

/// Write a file readable only by the owner (0600).
pub fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents.as_bytes())
}

// ---------- passwords ----------

pub fn hash_password(pw: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|p| Argon2::default().verify_password(pw.as_bytes(), &p).is_ok())
}

/// Checked when the username does not exist, so a wrong name costs as long as a wrong password.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| hash_password("not-a-real-password").unwrap_or_default());

async fn check_password(pw: String, hash: Option<String>) -> Result<bool, AppError> {
    tokio::task::spawn_blocking(move || match hash {
        Some(h) => verify_password(&pw, &h),
        None => {
            verify_password(&pw, &DUMMY_HASH);
            false
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("join: {e}")))
}

async fn hash_async(pw: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || hash_password(&pw))
        .await
        .map_err(|e| AppError::Internal(format!("join: {e}")))?
        .map_err(|e| AppError::Internal(format!("argon2: {e}")))
}

// ---------- request helpers ----------

/// Peer address; with `SM_TRUST_PROXY=1` the X-Real-IP set by our nginx.
fn client_ip(h: &HeaderMap, peer: SocketAddr, trust_proxy: bool) -> String {
    if trust_proxy
        && let Some(ip) = h
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
    {
        return ip.trim().to_owned();
    }
    peer.ip().to_string()
}

fn cookie_token(h: &HeaderMap) -> Option<&str> {
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| kv.trim().strip_prefix(COOKIE)?.strip_prefix('='))
}

/// Path `<SM_BASE_PATH>/api` (nginx strips the prefix, the browser still sees it). `Secure` with
/// `SM_COOKIE_SECURE=1`: behind Cloudflare Flexible nginx itself only sees http, so it is config,
/// not a header. The SSH tunnel is plain http on localhost and leaves it off.
fn session_cookie(token: &str, max_age: i64, cfg: &Config) -> HeaderValue {
    let v = format!(
        "{COOKIE}={token}; Path={}/api; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
        cfg.base_path,
        if cfg.cookie_secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&v).expect("cookie is ascii")
}

fn with_cookie(mut res: Response, cookie: HeaderValue) -> Response {
    res.headers_mut().insert(header::SET_COOKIE, cookie);
    res
}

/// The signed-in owner, inserted by `require_auth`.
#[derive(Clone)]
pub struct Admin {
    pub id: i64,
    pub username: String,
}

// ---------- handlers ----------

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(b): Json<LoginBody>,
) -> ApiResult {
    let ip = client_ip(&headers, peer, st.cfg.trust_proxy);
    let tries_left = match attempt(&st, &ip).await? {
        Ok(n) => n,
        Err(secs) => return Ok(locked(secs)),
    };
    // Soft limit: many wrong passwords server-wide → every attempt waits its turn for 2 s.
    let busy = st.lockout.lock().unwrap_or_else(|e| e.into_inner()).busy(now());
    if busy {
        match tokio::time::timeout(Duration::from_secs(10), SLOW.lock()).await {
            Ok(_turn) => tokio::time::sleep(SLOW_DELAY).await,
            Err(_) => {
                let body = json!({ "error": "Too many sign-in attempts on this server right now. Try again in a minute.", "retryAfter": 60 });
                return Ok((StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response());
            }
        }
    }

    let row: Option<(i64, String, i64)> =
        sqlx::query_as("SELECT id, password_hash, token_version FROM admin_users WHERE username = ?")
            .bind(&b.username)
            .fetch_optional(&st.db)
            .await?;
    let ok = check_password(b.password, row.as_ref().map(|r| r.1.clone())).await?;
    let Some((id, _, ver)) = row.filter(|_| ok) else {
        st.lockout.lock().unwrap_or_else(|e| e.into_inner()).note_fail(now());
        if tries_left == 0 {
            activity::log(
                &st.db,
                &b.username,
                "auth.locked",
                &ip,
                "5 failed sign-ins, locked 15 min",
            )
            .await;
            return Ok(locked(LOCK_SECS));
        }
        let body = json!({ "error": "Wrong username or password.", "triesLeft": tries_left });
        return Ok((StatusCode::UNAUTHORIZED, Json(body)).into_response());
    };

    st.lockout.lock().unwrap_or_else(|e| e.into_inner()).success(&ip);
    sqlx::query("DELETE FROM login_fails WHERE ip = ?").bind(&ip).execute(&st.db).await?;
    let t = now();
    sqlx::query("UPDATE admin_users SET last_login_at = ? WHERE id = ?")
        .bind(t)
        .bind(id)
        .execute(&st.db)
        .await?;
    activity::log(&st.db, &b.username, "auth.login", &ip, "").await;
    let token = sign(&st.jwt_secret, id, ver, t)?;
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let body = json!({
        "user": { "username": b.username },
        "expiresAt": (t + SESSION_SECS) * 1000,
        "publicUrl": st.cfg.public_url(host),
    });
    Ok(with_cookie(
        Json(body).into_response(),
        session_cookie(&token, SESSION_SECS, &st.cfg),
    ))
}

fn locked(secs: i64) -> Response {
    let mins = (secs + 59) / 60;
    let body = json!({
        "error": format!("Too many failed sign-ins. Try again in {mins} minute{}.", if mins == 1 { "" } else { "s" }),
        "retryAfter": secs,
    });
    let mut res = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
    res.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(secs));
    res
}

/// Bump token_version: this token (and every other session of the owner) stops working now,
/// not when it expires. Single owner, so "sign out" = sign out everywhere.
async fn revoke_sessions(db: &SqlitePool, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE admin_users SET token_version = token_version + 1 WHERE id = ?")
        .bind(user_id)
        .execute(db)
        .await
        .map(|_| ())
}

async fn logout(State(st): State<AppState>, Extension(a): Extension<Admin>) -> ApiResult {
    revoke_sessions(&st.db, a.id).await?;
    activity::log(&st.db, &a.username, "auth.logout", &a.username, "sessions revoked").await;
    Ok(with_cookie(
        Json(json!({ "ok": true })).into_response(),
        session_cookie("", 0, &st.cfg),
    ))
}

/// The owner + the public base URL of this dashboard (webhook links are built from it).
async fn me(State(st): State<AppState>, Extension(a): Extension<Admin>, headers: HeaderMap) -> Json<serde_json::Value> {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    Json(json!({ "user": { "username": a.username }, "publicUrl": st.cfg.public_url(host) }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PasswordBody {
    current_password: String,
    new_password: String,
}

/// Bumps token_version (every other session is signed out) and re-issues this session's cookie.
async fn change_password(
    State(st): State<AppState>,
    Extension(a): Extension<Admin>,
    Json(b): Json<PasswordBody>,
) -> ApiResult {
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM admin_users WHERE id = ?")
        .bind(a.id)
        .fetch_optional(&st.db)
        .await?;
    if !check_password(b.current_password, hash).await? {
        return Err(AppError::bad_request("Current password is wrong."));
    }
    if b.new_password.chars().count() < MIN_PASSWORD {
        return Err(AppError::bad_request(format!(
            "New password needs at least {MIN_PASSWORD} characters."
        )));
    }
    let new_hash = hash_async(b.new_password).await?;
    let ver: i64 = sqlx::query_scalar(
        "UPDATE admin_users SET password_hash = ?, token_version = token_version + 1, updated_at = unixepoch() \
         WHERE id = ? RETURNING token_version",
    )
    .bind(new_hash)
    .bind(a.id)
    .fetch_one(&st.db)
    .await?;
    activity::log(
        &st.db,
        &a.username,
        "auth.password_changed",
        &a.username,
        "other sessions signed out",
    )
    .await;
    let token = sign(&st.jwt_secret, a.id, ver, now())?;
    Ok(with_cookie(
        Json(json!({ "ok": true })).into_response(),
        session_cookie(&token, SESSION_SECS, &st.cfg),
    ))
}

/// Open route: POST /api/auth/login.
pub fn login_route() -> Router<AppState> {
    LazyLock::force(&DUMMY_HASH); // hash once at boot, so the first unknown-user login isn't slower
    Router::new().route("/api/auth/login", post(login))
}

/// Mounted inside the authenticated /api router.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/auth/password", post(change_password))
}

/// The owner, if `token` is a valid, unexpired JWT whose token_version matches the user row.
async fn session_user(st: &AppState, token: Option<&str>) -> Option<Admin> {
    let c = verify(&st.jwt_secret, token?)?;
    sqlx::query_as::<_, (String, i64)>("SELECT username, token_version FROM admin_users WHERE id = ?")
        .bind(c.sub)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten()
        .filter(|(_, ver)| *ver == c.ver)
        .map(|(username, _)| Admin { id: c.sub, username })
}

/// 401 unless the cookie holds a live session (`session_user`).
pub async fn require_auth(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let user = session_user(&st, cookie_token(req.headers())).await;
    match user {
        Some(a) => {
            req.extensions_mut().insert(a);
            next.run(req).await
        }
        None => AppError::Status(StatusCode::UNAUTHORIZED, "Authentication required".into()).into_response(),
    }
}

// ---------- CLI ----------

fn read_password(prompt: &str) -> String {
    use std::io::{BufRead, IsTerminal, Write};
    let tty = std::io::stdin().is_terminal();
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    // Hide typing on a terminal; piped stdin (`echo … |`) is read as-is.
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    if tty && unsafe { libc::tcgetattr(0, &mut old) } == 0 {
        let mut t = old;
        t.c_lflag &= !libc::ECHO;
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &t) };
    }
    let mut line = String::new();
    let r = std::io::stdin().lock().read_line(&mut line);
    if tty {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &old) };
        eprintln!();
    }
    if r.is_err() {
        die("could not read password from stdin");
    }
    line.trim_end_matches(['\r', '\n']).to_owned()
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}

/// `server-manager create-admin <username>`: password from stdin (never argv, it would show in `ps`).
/// Existing user = password reset; that also signs out every session.
pub async fn create_admin_cli(db: &SqlitePool, username: &str) {
    if username.is_empty()
        || username.len() > 64
        || !username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        die("username: 1-64 characters, letters, digits, - _ .");
    }
    let pw = read_password(&format!("Password for '{username}' (min {MIN_PASSWORD} characters): "));
    if pw.chars().count() < MIN_PASSWORD {
        die(&format!("password needs at least {MIN_PASSWORD} characters"));
    }
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) && read_password("Repeat password: ") != pw {
        die("passwords do not match");
    }
    let hash = hash_password(&pw).unwrap_or_else(|e| die(&format!("argon2: {e}")));
    let r = sqlx::query(
        "INSERT INTO admin_users (username, password_hash) VALUES (?, ?) \
         ON CONFLICT (username) DO UPDATE SET password_hash = excluded.password_hash, \
         token_version = token_version + 1, updated_at = unixepoch()",
    )
    .bind(username)
    .bind(hash)
    .execute(db)
    .await;
    if let Err(e) = r {
        die(&format!("database: {e}"));
    }
    activity::log(db, "system", "auth.admin_set", username, "create-admin CLI").await;
    println!("Admin '{username}' is ready.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockout_after_five_fails() {
        let mut l = Lockout::default();
        for left in [4, 3, 2, 1, 0] {
            assert_eq!(l.attempt("1.1.1.1", 100), Ok(left));
        }
        // 6th try is refused without checking the password, other IPs unaffected
        assert_eq!(l.attempt("1.1.1.1", 160), Err(LOCK_SECS - 60));
        assert_eq!(l.attempt("2.2.2.2", 160), Ok(4));
        // lock over → fresh count
        assert_eq!(l.attempt("1.1.1.1", 100 + LOCK_SECS), Ok(4));
    }

    #[test]
    fn success_resets_and_old_fails_expire() {
        let mut l = Lockout::default();
        l.attempt("a", 0).unwrap();
        l.attempt("a", 0).unwrap();
        l.success("a");
        assert_eq!(l.attempt("a", 1), Ok(4));
        // a failure older than the window is forgotten
        assert_eq!(l.attempt("a", 1 + LOCK_SECS), Ok(4));
    }

    #[test]
    fn jwt_roundtrip_expiry_and_secret() {
        let t = sign(b"k1", 7, 3, now()).unwrap();
        assert_eq!(
            verify(b"k1", &t),
            Some(Claims {
                sub: 7,
                ver: 3,
                exp: now() + SESSION_SECS
            })
        );
        assert_eq!(verify(b"k2", &t), None);
        let old = sign(b"k1", 7, 3, now() - SESSION_SECS - 1).unwrap();
        assert_eq!(verify(b"k1", &old), None);
    }

    #[test]
    fn argon2id() {
        let h = hash_password("correct horse").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("correct horsf", &h));
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        assert_eq!(cookie_token(&h), None);
        h.insert(header::COOKIE, "a=1; sm_session=abc.def; b=2".parse().unwrap());
        assert_eq!(cookie_token(&h), Some("abc.def"));
        h.insert(header::COOKIE, "sm_sessionx=nope".parse().unwrap());
        assert_eq!(cookie_token(&h), None);
    }

    #[test]
    fn global_soft_limit() {
        let mut l = Lockout::default();
        for _ in 0..GLOBAL_FAILS {
            l.note_fail(100);
        }
        assert!(!l.busy(100), "30 a minute is still fine");
        l.note_fail(120);
        assert!(l.busy(120), "31st failure in a minute turns it on");
        assert!(!l.busy(160), "the first 30 are older than a minute now");
    }

    #[test]
    fn cookie_path_follows_base_path() {
        let mut c = Config::test("/d".into(), "/a".into());
        let v = session_cookie("t", 60, &c);
        assert_eq!(v, "sm_session=t; Path=/api; HttpOnly; SameSite=Strict; Max-Age=60");
        c.base_path = "/server-manager".into();
        c.cookie_secure = true;
        let v = session_cookie("t", 0, &c);
        assert_eq!(v, "sm_session=t; Path=/server-manager/api; HttpOnly; SameSite=Strict; Max-Age=0; Secure");
    }

    async fn test_state(db: SqlitePool) -> AppState {
        AppState::new(db, Config::test("/tmp".into(), "/tmp".into()), vec![7; 32])
    }

    async fn memory_db() -> SqlitePool {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn lockout_survives_restart() {
        let db = memory_db().await;
        let st = test_state(db.clone()).await;
        for left in [4, 3, 2, 1, 0] {
            assert_eq!(attempt(&st, "5.5.5.5").await.unwrap(), Ok(left));
        }
        // "restart": a fresh state on the same database
        let st2 = test_state(db).await;
        assert_eq!(attempt(&st2, "6.6.6.6").await.unwrap(), Ok(4));
        restore_lockouts(&st2).await.unwrap();
        assert!(attempt(&st2, "5.5.5.5").await.unwrap().is_err(), "still locked after restart");
    }

    #[tokio::test]
    async fn logout_revokes_the_token() {
        let db = memory_db().await;
        let st = test_state(db.clone()).await;
        sqlx::query("INSERT INTO admin_users (id, username, password_hash) VALUES (1, 'heri', 'x')")
            .execute(&db)
            .await
            .unwrap();
        let t = sign(&st.jwt_secret, 1, 0, now()).unwrap();
        assert_eq!(session_user(&st, Some(&t)).await.map(|a| a.username).as_deref(), Some("heri"));
        revoke_sessions(&db, 1).await.unwrap();
        assert!(session_user(&st, Some(&t)).await.is_none(), "old token is dead after logout");
    }

    #[test]
    fn proxy_ip_only_when_trusted() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(client_ip(&h, peer, false), "127.0.0.1");
        assert_eq!(client_ip(&h, peer, true), "9.9.9.9");
    }
}
