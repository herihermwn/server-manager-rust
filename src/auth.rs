//! Owner login: argon2id hashes in `admin_users`, HS256 JWT in an HttpOnly SameSite=Strict cookie,
//! per-IP lockout, and the `require_auth` middleware that guards every /api route except login.

use crate::{
    activity,
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
use std::{collections::HashMap, net::SocketAddr, path::Path, sync::LazyLock};

pub const SESSION_SECS: i64 = 12 * 60 * 60;
const COOKIE: &str = "sm_session";
const MAX_FAILS: u32 = 5;
const LOCK_SECS: i64 = 15 * 60;
pub const MIN_PASSWORD: usize = 10;

// ---------- lockout ----------

#[derive(Default)]
struct Entry {
    fails: u32,
    last: i64,
    locked_until: i64,
}

/// Failed sign-ins per IP. The attempt is counted *before* the password is checked, so parallel
/// requests can't sneak past the limit.
// ponytail: in memory, so a restart clears locks; persist to SQLite if that ever matters.
#[derive(Default)]
pub struct Lockout(HashMap<String, Entry>);

impl Lockout {
    /// `Err(seconds)` while locked. `Ok(tries_left)` = tries left if *this* attempt fails.
    pub fn attempt(&mut self, ip: &str, now: i64) -> Result<u32, i64> {
        self.0.retain(|_, e| e.locked_until > now || now - e.last < LOCK_SECS);
        let e = self.0.entry(ip.to_owned()).or_default();
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
        self.0.remove(ip);
    }
}

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

/// `Secure` only when nginx says the browser came over https; the SSH tunnel is plain http on localhost.
fn session_cookie(token: &str, max_age: i64, h: &HeaderMap) -> HeaderValue {
    let secure = h.get("x-forwarded-proto").is_some_and(|v| v == "https");
    let v = format!(
        "{COOKIE}={token}; Path=/api; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
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
    let attempt = st.lockout.lock().unwrap_or_else(|e| e.into_inner()).attempt(&ip, now());
    let tries_left = match attempt {
        Ok(n) => n,
        Err(secs) => return Ok(locked(secs)),
    };

    let row: Option<(i64, String, i64)> =
        sqlx::query_as("SELECT id, password_hash, token_version FROM admin_users WHERE username = ?")
            .bind(&b.username)
            .fetch_optional(&st.db)
            .await?;
    let ok = check_password(b.password, row.as_ref().map(|r| r.1.clone())).await?;
    let Some((id, _, ver)) = row.filter(|_| ok) else {
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
    let t = now();
    sqlx::query("UPDATE admin_users SET last_login_at = ? WHERE id = ?")
        .bind(t)
        .bind(id)
        .execute(&st.db)
        .await?;
    activity::log(&st.db, &b.username, "auth.login", &ip, "").await;
    let token = sign(&st.jwt_secret, id, ver, t)?;
    let body = json!({ "user": { "username": b.username }, "expiresAt": (t + SESSION_SECS) * 1000 });
    Ok(with_cookie(
        Json(body).into_response(),
        session_cookie(&token, SESSION_SECS, &headers),
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

async fn logout(headers: HeaderMap) -> Response {
    with_cookie(
        Json(json!({ "ok": true })).into_response(),
        session_cookie("", 0, &headers),
    )
}

async fn me(Extension(a): Extension<Admin>) -> Json<serde_json::Value> {
    Json(json!({ "user": { "username": a.username } }))
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
    headers: HeaderMap,
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
        session_cookie(&token, SESSION_SECS, &headers),
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

/// 401 unless the cookie holds a valid, unexpired JWT whose token_version matches the user row.
pub async fn require_auth(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let claims = cookie_token(req.headers()).and_then(|t| verify(&st.jwt_secret, t));
    let user = match claims {
        Some(c) => sqlx::query_as::<_, (String, i64)>("SELECT username, token_version FROM admin_users WHERE id = ?")
            .bind(c.sub)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten()
            .filter(|(_, ver)| *ver == c.ver)
            .map(|(username, _)| Admin { id: c.sub, username }),
        None => None,
    };
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
    fn proxy_ip_only_when_trusted() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(client_ip(&h, peer, false), "127.0.0.1");
        assert_eq!(client_ip(&h, peer, true), "9.9.9.9");
    }
}
