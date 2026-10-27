//! Phase 6 · Web server: site config generator, `nginx -t` before reload, certbot, enable/disable.
//! Routes are relative to /api and already behind auth.
//!
//! Files: `<SM_NGINX_DIR>/<domain>.conf` (live) or `<domain>.conf.disabled` (not matched by the
//! `include .../*.conf` in nginx.conf). Every change snapshots the previous files, runs
//! `sudo -n nginx -t`, and puts the snapshot back if the test (or the reload) fails.
//!
//! Certbot mode: `certonly --webroot -w /var/www/html`. The generated port-80 server always has
//! `location /.well-known/acme-challenge/ { root /var/www/html; }`, so issuing and renewing never
//! depend on the SSL block existing yet and certbot never rewrites our files (the `--nginx` plugin
//! edits config in place, which would fight the generator and break the "restore on failure" rule).
//!
//! Static folders: nginx runs as www-data, /home/ubuntu is 0750 and the manager's UMask 0077 makes
//! release files 0600, so saving a static site grants www-data access with POSIX ACLs
//! (`grant_web_read`, needs the `acl` package): traverse-only (x) on each folder from the apps
//! folder's parent down, read (rX) + the same default ACL on the served tree so new files inherit
//! it. A folder under `<app>/current` or `<app>/releases` is granted on `<app>/releases`, so the
//! next deploy's release is readable too. `<app>/shared` (.env) is never granted.

use crate::{
    activity,
    auth::Admin,
    exec::{Cmd, Line, Stream},
    http::{ApiResult, AppError, now},
    sse::line_event,
    state::AppState,
    validate,
};
use axum::{
    Extension, Json, Router,
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, sse::Event},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::{sync::MutexGuard, sync::mpsc, sync::watch};

const WEBROOT: &str = "/var/www/html";
const EXPIRY_WARN_DAYS: i64 = 30;

fn nginx_dir() -> PathBuf {
    std::env::var("SM_NGINX_DIR")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| "/etc/nginx/sites-manager".into())
}

/// `SM_PUBLIC_IP`, else the IPv4 this host sends from (UDP connect, no packet leaves). Behind NAT
/// (most clouds) that is a private address, so set SM_PUBLIC_IP there.
fn public_ip() -> String {
    std::env::var("SM_PUBLIC_IP")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
            s.connect("1.1.1.1:53").ok()?;
            Some(s.local_addr().ok()?.ip().to_string())
        })
        .unwrap_or_else(|| "127.0.0.1".into())
}

// ======================================================================
// Validation
// ======================================================================

/// Lowercased hostname: dot-separated labels of a-z 0-9 -, each 1-63 chars not starting/ending
/// with -, at least two labels, TLD with a letter (so a bare IP is refused). No spaces, `;`, braces
/// or quotes can get through, so the value is safe inside `server_name` and a file name.
pub fn hostname(s: &str) -> Result<String, AppError> {
    let d = s.trim().to_ascii_lowercase();
    let labels_ok = d.split('.').all(|l| {
        (1..=63).contains(&l.len())
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let tld_ok = d
        .rsplit('.')
        .next()
        .is_some_and(|t| t.bytes().any(|b| b.is_ascii_lowercase()));
    if d.len() <= 253 && d.contains('.') && labels_ok && tld_ok {
        Ok(d)
    } else {
        Err(AppError::bad_request(format!(
            "Invalid domain \"{}\": use letters, digits and - in dot-separated labels, like app.example.com.",
            s.trim().chars().take(80).collect::<String>()
        )))
    }
}

/// True when every `{` has its `}` and nothing closes the surrounding `server { }`.
/// Follows nginx's own tokenizer (`ngx_conf_read_token`): `#` comments, quotes and `}` only count at the
/// start of a token, `\` escapes the next character, and a `{` right after `$` is part of the word.
pub fn braces_balanced(s: &str) -> bool {
    let (mut depth, mut quote, mut comment) = (0i32, None::<char>, false);
    let mut start = true; // at the start of a token
    let mut var = false; // after `$` (or `${`): nginx reads a `{` here as part of the word
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if comment {
            if c == '\n' {
                comment = false;
                start = true;
            }
            continue;
        }
        if c == '\\' {
            chars.next();
            start = false;
            var = false;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
                start = false;
            }
            continue;
        }
        match c {
            '#' if start => comment = true,
            '\'' | '"' if start => quote = Some(c),
            '{' if var => {}
            '{' => {
                depth += 1;
                start = true;
            }
            // Inside a word (`${y}`, `a}`) nginx keeps `}` as a plain character.
            '}' if !start => {}
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
                start = true;
            }
            // nginx's whitespace only: a vertical tab or NBSP is a word character to it.
            ';' | ' ' | '\t' | '\r' | '\n' => start = true,
            _ => start = false,
        }
        var = c == '$' || (var && c == '{');
    }
    depth == 0 && quote.is_none()
}

/// Folder path that is going into `root <path>;`: plain path characters only.
fn plain_path(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+@".contains(&b))
}

/// `http(s)://host[/path][?query]` with nothing nginx could read as syntax.
fn redirect_url(s: &str) -> Result<String, AppError> {
    let u = s.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"));
    let bad = u.len() > 2000
        || u.bytes()
            .any(|b| b.is_ascii_control() || b" ;{}'\"\\$`<>^|".contains(&b) || b >= 0x80);
    match rest {
        Some(r) if !bad && r.split(['/', '?', '#']).next().is_some_and(|h| !h.is_empty()) => Ok(u.to_owned()),
        _ => Err(AppError::bad_request(
            "Redirect URL must start with http:// or https:// and contain no spaces, quotes, ; { } or $.",
        )),
    }
}

fn email(s: &str) -> Result<String, AppError> {
    let e = s.trim();
    let ok = e.len() <= 254
        && !e.starts_with('-')
        && e.matches('@').count() == 1
        && !e.starts_with('@')
        && !e.ends_with('@')
        && e.bytes().all(|b| b.is_ascii_alphanumeric() || b"._%+-@".contains(&b));
    if ok {
        Ok(e.to_owned())
    } else {
        Err(AppError::bad_request("Expiry email is not a valid address."))
    }
}

// ======================================================================
// Draft + generator
// ======================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Proxy,
    Static,
    Redirect,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Proxy => "proxy",
            Kind::Static => "static",
            Kind::Redirect => "redirect",
        }
    }
}

/// A validated site, everything the generator needs. Build with `Draft::build`.
#[derive(Clone, Debug)]
pub struct Draft {
    pub domain: String,
    pub aliases: Vec<String>,
    pub kind: Kind,
    pub app_id: Option<i64>,
    /// `127.0.0.1:<port>` for proxy.
    pub upstream: Option<String>,
    pub root_path: Option<String>,
    pub redirect_url: Option<String>,
    pub max_upload_mb: i64,
    pub websocket: bool,
    pub gzip: bool,
    pub cache_assets: bool,
    pub custom: String,
    pub ssl: bool,
    pub force_https: bool,
    pub ssl_email: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Input {
    /// Only for `POST /sites/preview`: lets the draft know whether that site already has a certificate.
    id: Option<i64>,
    domain: String,
    aliases: Vec<String>,
    kind: String,
    app_id: Option<i64>,
    root_path: String,
    redirect_url: String,
    max_upload_mb: i64,
    websocket: bool,
    gzip: bool,
    cache_assets: bool,
    custom_directives: String,
    ssl: bool,
    force_https: bool,
    ssl_email: String,
}

impl Default for Input {
    fn default() -> Self {
        Input {
            id: None,
            domain: String::new(),
            aliases: Vec::new(),
            kind: "proxy".into(),
            app_id: None,
            root_path: String::new(),
            redirect_url: String::new(),
            max_upload_mb: 10,
            websocket: true,
            gzip: true,
            cache_assets: false,
            custom_directives: String::new(),
            ssl: false,
            force_https: true,
            ssl_email: String::new(),
        }
    }
}

impl Draft {
    /// `app_port` is the port of `input.app_id` from the apps table (None = no such app / no port).
    fn build(i: &Input, apps_dir: &Path, app_port: Option<i64>) -> Result<Draft, AppError> {
        let domain = hostname(&i.domain)?;
        if i.aliases.len() > 20 {
            return Err(AppError::bad_request("At most 20 aliases."));
        }
        let mut aliases: Vec<String> = Vec::new();
        for a in &i.aliases {
            let a = hostname(a)?;
            if a == domain || aliases.contains(&a) {
                return Err(AppError::bad_request(format!("{a} is listed twice.")));
            }
            aliases.push(a);
        }
        let kind = match i.kind.as_str() {
            "proxy" => Kind::Proxy,
            "static" => Kind::Static,
            "redirect" => Kind::Redirect,
            _ => return Err(AppError::bad_request("Type must be proxy, static or redirect.")),
        };
        let (mut app_id, mut upstream, mut root_path, mut redirect) = (None, None, None, None);
        match kind {
            Kind::Proxy => {
                let id = i
                    .app_id
                    .ok_or_else(|| AppError::bad_request("Pick the app to proxy to."))?;
                let port = app_port
                    .filter(|p| (1..=65535).contains(p))
                    .ok_or_else(|| AppError::bad_request("That app has no port yet. Set one in the app's settings."))?;
                app_id = Some(id);
                upstream = Some(format!("127.0.0.1:{port}"));
            }
            Kind::Static => {
                let raw = i.root_path.trim();
                if !plain_path(raw) {
                    return Err(AppError::bad_request(
                        "Folder: use an absolute path with letters, digits and . _ - + @ only.",
                    ));
                }
                let p = validate::path_under(apps_dir, Path::new(raw))?;
                // Keep `<app>/current/...` as typed: resolving the link would pin the site to
                // today's release, which a later clean-up deletes.
                let p = if Path::new(raw).starts_with(apps_dir) {
                    raw.trim_end_matches('/').to_owned()
                } else {
                    p.to_string_lossy().into_owned()
                };
                if !plain_path(&p) {
                    return Err(AppError::bad_request(
                        "Folder path has characters nginx config cannot hold.",
                    ));
                }
                root_path = Some(p);
            }
            Kind::Redirect => redirect = Some(redirect_url(&i.redirect_url)?),
        }
        if !(1..=10240).contains(&i.max_upload_mb) {
            return Err(AppError::bad_request("Max upload must be 1-10240 MB."));
        }
        let custom = i.custom_directives.replace('\r', "");
        if custom.len() > 8000 || custom.contains('\0') {
            return Err(AppError::bad_request("Custom directives: at most 8,000 characters."));
        }
        if !braces_balanced(&custom) {
            return Err(AppError::bad_request(
                "Custom directives: braces { } do not balance (or a quote is left open). They must stay inside server { }.",
            ));
        }
        let ssl_email = match i.ssl_email.trim() {
            "" => None,
            e => Some(email(e)?),
        };
        Ok(Draft {
            domain,
            aliases,
            kind,
            app_id,
            upstream,
            root_path,
            redirect_url: redirect,
            max_upload_mb: i.max_upload_mb,
            websocket: i.websocket,
            gzip: i.gzip,
            cache_assets: i.cache_assets,
            custom: custom.trim_matches('\n').to_owned(),
            ssl: i.ssl,
            force_https: i.force_https,
            ssl_email,
        })
    }
}

fn proxy_block(d: &Draft, extra: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = vec![format!(
        "        proxy_pass http://{};",
        d.upstream.as_deref().unwrap_or_default()
    )];
    v.extend(
        [
            "        proxy_set_header Host $host;",
            "        proxy_set_header X-Real-IP $remote_addr;",
            "        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;",
            "        proxy_set_header X-Forwarded-Proto $scheme;",
            "        proxy_http_version 1.1;",
        ]
        .map(String::from),
    );
    if d.websocket {
        v.push("        proxy_set_header Upgrade $http_upgrade;".into());
        v.push("        proxy_set_header Connection \"upgrade\";".into());
    }
    v.extend(extra.iter().map(|s| format!("        {s}")));
    v
}

/// Custom directives with their own `location / { … }` replace the generated one (nginx refuses
/// two), e.g. for headers that must differ from proxy_block's.
fn custom_has_root(custom: &str) -> bool {
    custom.lines().any(|l| {
        let mut w = l.split_whitespace();
        w.next() == Some("location") && matches!(w.next(), Some("/" | "/{"))
    })
}

/// The lines every content-serving server block shares (below listen/server_name/ssl_*).
/// `acme`: add the webroot challenge location (the block that listens on port 80).
fn body(d: &Draft, acme: bool) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let own_root = custom_has_root(&d.custom);
    let section = |v: &mut Vec<String>, lines: Vec<String>| {
        if !v.is_empty() {
            v.push(String::new());
        }
        v.extend(lines);
    };
    if acme {
        section(
            &mut v,
            vec![
                "    location /.well-known/acme-challenge/ {".into(),
                format!("        root {WEBROOT};"),
                "    }".into(),
            ],
        );
    }
    if d.kind != Kind::Redirect {
        let mut l = vec![format!("    client_max_body_size {}m;", d.max_upload_mb)];
        if d.gzip {
            l.push("    gzip on;".into());
        }
        section(&mut v, l);
    }
    match d.kind {
        Kind::Proxy => {
            if !own_root {
                let mut l = vec!["    location / {".to_owned()];
                l.extend(proxy_block(d, &[]));
                l.push("    }".into());
                section(&mut v, l);
            }
            if d.cache_assets {
                let mut l = vec!["    location /assets/ {".to_owned()];
                l.extend(proxy_block(d, &["expires 7d;", "add_header Cache-Control \"public\";"]));
                l.push("    }".into());
                section(&mut v, l);
            }
        }
        Kind::Static => {
            section(
                &mut v,
                vec![
                    format!("    root {};", d.root_path.as_deref().unwrap_or_default()),
                    "    index index.html;".into(),
                ],
            );
            if !own_root {
                section(
                    &mut v,
                    vec![
                        "    location / {".into(),
                        "        try_files $uri $uri/ =404;".into(),
                        "    }".into(),
                    ],
                );
            }
            if d.cache_assets {
                section(
                    &mut v,
                    vec![
                        "    location /assets/ {".into(),
                        "        expires 7d;".into(),
                        "        add_header Cache-Control \"public\";".into(),
                        "    }".into(),
                    ],
                );
            }
        }
        Kind::Redirect => {
            // A URL without a path keeps the request path (old.example.com/a -> new.example.com/a).
            let u = d.redirect_url.as_deref().unwrap_or_default();
            let host_only = u.split_once("://").is_some_and(|(_, r)| !r.contains(['/', '?']));
            let to = if host_only {
                format!("{u}$request_uri")
            } else {
                u.to_owned()
            };
            section(
                &mut v,
                vec![
                    "    location / {".into(),
                    format!("        return 301 {to};"),
                    "    }".into(),
                ],
            );
        }
    }
    if !d.custom.trim().is_empty() {
        section(&mut v, vec!["    # custom".into()]);
        v.extend(d.custom.lines().map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                format!("    {l}")
            }
        }));
    }
    v
}

/// The nginx config for one site. Pure: same input, same text. `ssl_active` = the site wants SSL
/// *and* its certificate exists (nginx -t fails on a missing certificate file).
pub fn generate(d: &Draft, ssl_active: bool) -> String {
    let names = std::iter::once(d.domain.as_str())
        .chain(d.aliases.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let mut o: Vec<String> = Vec::new();
    let ssl_lines = |o: &mut Vec<String>| {
        o.push(format!(
            "    ssl_certificate     /etc/letsencrypt/live/{}/fullchain.pem;",
            d.domain
        ));
        o.push(format!(
            "    ssl_certificate_key /etc/letsencrypt/live/{}/privkey.pem;",
            d.domain
        ));
        o.push(String::new());
    };
    if ssl_active && d.force_https {
        o.extend([
            "server {".into(),
            "    listen 80;".into(),
            format!("    server_name {names};"),
            String::new(),
            "    location /.well-known/acme-challenge/ {".into(),
            format!("        root {WEBROOT};"),
            "    }".into(),
            String::new(),
            "    location / {".into(),
            "        return 301 https://$host$request_uri;".into(),
            "    }".into(),
            "}".into(),
            String::new(),
            "server {".into(),
            "    listen 443 ssl http2;".into(),
            format!("    server_name {names};"),
            String::new(),
        ]);
        ssl_lines(&mut o);
        o.extend(body(d, false));
    } else if ssl_active {
        o.extend([
            "server {".into(),
            "    listen 80;".into(),
            "    listen 443 ssl http2;".into(),
            format!("    server_name {names};"),
            String::new(),
        ]);
        ssl_lines(&mut o);
        o.extend(body(d, true));
    } else {
        o.extend([
            "server {".into(),
            "    listen 80;".into(),
            format!("    server_name {names};"),
            String::new(),
        ]);
        o.extend(body(d, true));
    }
    o.push("}".into());
    o.push(String::new());
    o.join("\n")
}

// ======================================================================
// nginx -t, apply with restore
// ======================================================================

#[derive(Clone, Debug, Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub output: String,
    /// File named by the first `[emerg]` line, and the line number in it.
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// `nginx: [emerg] unknown directive "x" in /etc/nginx/sites-manager/a.conf:31` -> (path, 31).
pub fn parse_test_error(out: &str) -> Option<(String, u32)> {
    out.lines()
        .filter(|l| ["[emerg]", "[crit]", "[alert]"].iter().any(|t| l.contains(t)))
        .find_map(|l| {
            let (head, n) = l.trim_end().rsplit_once(':')?;
            let n: u32 = n.trim().parse().ok()?;
            let (_, path) = head.rsplit_once(" in ")?;
            Some((path.trim().to_owned(), n))
        })
}

impl TestResult {
    fn from_output(ok: bool, output: String) -> Self {
        let (file, line) = match parse_test_error(&output) {
            Some((f, l)) if !ok => (Some(f), Some(l)),
            _ => (None, None),
        };
        TestResult { ok, output, file, line }
    }

    /// The line number only when the error is inside this site's own file (so the UI can highlight
    /// the matching line of the generated config).
    fn line_in(&self, domain: &str) -> Option<u32> {
        let f = self.file.as_deref()?;
        let name = f.rsplit('/').next()?;
        (name == format!("{domain}.conf") || name == format!("{domain}.conf.disabled")).then_some(self.line?)
    }
}

struct TestRecord {
    at: i64,
    ok: bool,
    output: String,
}
static LAST_TEST: Mutex<Option<TestRecord>> = Mutex::new(None);

/// One change at a time: files, `nginx -t` and the sites rows are shared state. Handlers hold it over
/// the whole read-modify-write (load, apply, DB write); `apply` takes the guard to prove it.
static APPLY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn sudo(args: &[&str]) -> Cmd {
    Cmd::new("sudo").arg("-n").args(args.iter().copied())
}

fn joined(o: &crate::exec::Output) -> String {
    format!("{}{}", o.stderr, o.stdout).trim().to_owned()
}

async fn nginx_test() -> TestResult {
    let r = match sudo(&["/usr/sbin/nginx", "-t"])
        .timeout(Duration::from_secs(30))
        .output()
        .await
    {
        Ok(o) => TestResult::from_output(o.exit.success(), joined(&o)),
        Err(e) => TestResult::from_output(false, format!("Could not run nginx -t: {e}")),
    };
    *LAST_TEST.lock().unwrap() = Some(TestRecord {
        at: now(),
        ok: r.ok,
        output: r.output.clone(),
    });
    r
}

async fn nginx_reload() -> Result<(), String> {
    match sudo(&["/usr/bin/systemctl", "reload", "nginx"])
        .timeout(Duration::from_secs(30))
        .output()
        .await
    {
        Ok(o) if o.exit.success() => Ok(()),
        Ok(o) => Err(joined(&o)),
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Debug)]
enum ApplyError {
    Io(String),
    Test(TestResult),
    Reload(String),
}

impl From<std::io::Error> for ApplyError {
    fn from(e: std::io::Error) -> Self {
        ApplyError::Io(e.to_string())
    }
}

/// Production check: `nginx -t`, then reload.
async fn test_and_reload() -> Result<(), ApplyError> {
    let t = nginx_test().await;
    if !t.ok {
        return Err(ApplyError::Test(t));
    }
    nginx_reload().await.map_err(ApplyError::Reload)
}

fn conf_file(dir: &Path, domain: &str, enabled: bool) -> PathBuf {
    dir.join(if enabled {
        format!("{domain}.conf")
    } else {
        format!("{domain}.conf.disabled")
    })
}

/// Remove the files of `remove`, write `write = (domain, enabled, content)`, and when anything live
/// changed run `check` (nginx -t + reload). If `check` fails every touched file is put back as it was.
/// The caller holds `APPLY_LOCK`.
async fn apply_with(
    dir: &Path,
    remove: &[&str],
    write: Option<(&str, bool, &str)>,
    check: impl AsyncFn() -> Result<(), ApplyError>,
) -> Result<(), ApplyError> {
    let mut domains: Vec<&str> = remove.to_vec();
    if let Some((d, ..)) = write {
        domains.push(d);
    }
    let mut snap: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    for d in &domains {
        for en in [true, false] {
            let p = conf_file(dir, d, en);
            // Only a missing file is "nothing to restore"; any other read error stops before a change.
            let old = match std::fs::read(&p) {
                Ok(b) => Some(b),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            snap.push((p, old));
        }
    }
    let was_live = snap
        .iter()
        .any(|(p, old)| old.is_some() && p.extension().is_some_and(|e| e == "conf"));
    let restore = |snap: &[(PathBuf, Option<Vec<u8>>)]| {
        for (p, old) in snap {
            match old {
                Some(c) => {
                    std::fs::write(p, c).ok();
                }
                None => {
                    std::fs::remove_file(p).ok();
                }
            }
        }
    };

    let changed = (|| -> std::io::Result<()> {
        for d in remove {
            for en in [true, false] {
                let p = conf_file(dir, d, en);
                if p.exists() {
                    std::fs::remove_file(p)?;
                }
            }
        }
        if let Some((d, enabled, content)) = write {
            let target = conf_file(dir, d, enabled);
            let tmp = dir.join(format!(".{d}.tmp"));
            std::fs::write(&tmp, content)?;
            std::fs::rename(&tmp, &target)?;
            let other = conf_file(dir, d, !enabled);
            if other.exists() {
                std::fs::remove_file(other)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = changed {
        restore(&snap);
        return Err(e.into());
    }
    let now_live = write.is_some_and(|(_, en, _)| en);
    if !(was_live || now_live) {
        return Ok(());
    }
    if let Err(e) = check().await {
        restore(&snap);
        return Err(e);
    }
    Ok(())
}

async fn apply(
    _g: &MutexGuard<'_, ()>,
    remove: &[&str],
    write: Option<(&str, bool, &str)>,
) -> Result<(), ApplyError> {
    apply_with(&nginx_dir(), remove, write, test_and_reload).await
}

/// 500 for I/O, 422 with the nginx output and failing line for a config that did not pass.
fn apply_error(e: ApplyError, domain: &str) -> Response {
    match e {
        ApplyError::Io(m) => AppError::Internal(format!("nginx files: {m}")).into_response(),
        ApplyError::Test(t) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "nginx -t failed, nothing was reloaded. The live site still uses the previous config.",
                "nginx": { "output": t.output, "line": t.line_in(domain) },
            })),
        )
            .into_response(),
        ApplyError::Reload(m) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": "nginx -t passed but the reload failed. The previous config was restored.", "nginx": { "output": m, "line": null } })),
        )
            .into_response(),
    }
}

// ======================================================================
// DNS + certificates
// ======================================================================

#[derive(Serialize)]
struct DnsResult {
    name: String,
    ips: Vec<String>,
    ok: bool,
    error: Option<String>,
}

async fn dns_check(name: &str, want: &str) -> DnsResult {
    let r = tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((name, 80))).await;
    match r {
        Ok(Ok(addrs)) => {
            let mut ips: Vec<String> = addrs.filter(|a| a.is_ipv4()).map(|a| a.ip().to_string()).collect();
            ips.sort();
            ips.dedup();
            let ok = ips.iter().any(|i| i == want);
            let error = (!ok).then(|| {
                if ips.is_empty() {
                    "No A record found.".to_owned()
                } else {
                    format!("A record points to {}.", ips.join(", "))
                }
            });
            DnsResult {
                name: name.into(),
                ips,
                ok,
                error,
            }
        }
        Ok(Err(_)) => DnsResult {
            name: name.into(),
            ips: vec![],
            ok: false,
            error: Some("No A record found.".into()),
        },
        Err(_) => DnsResult {
            name: name.into(),
            ips: vec![],
            ok: false,
            error: Some("DNS lookup timed out.".into()),
        },
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cert {
    pub name: String,
    pub domains: Vec<String>,
    pub expires_at: i64,
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "2026-12-19 10:00:00+00:00 (VALID: 89 days)" -> unix seconds (offset applied).
fn parse_expiry(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(' ')?;
    let time = rest.split([' ', '+']).next()?;
    let off = rest.get(time.len()..).unwrap_or_default();
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|x| x.parse::<i64>().ok());
    let (h, mi, sec) = (t.next()??, t.next()??, t.next()??);
    let mut secs = days_from_civil(y, m, day) * 86400 + h * 3600 + mi * 60 + sec;
    if let Some(o) = off.strip_prefix('+').and_then(|o| o.split_whitespace().next()) {
        let (oh, om) = o.split_once(':')?;
        secs -= oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60;
    }
    Some(secs)
}

/// `certbot certificates` output -> certs.
pub fn parse_certs(out: &str) -> Vec<Cert> {
    let mut certs = Vec::new();
    let (mut name, mut domains) = (None::<String>, Vec::new());
    for l in out.lines().map(str::trim) {
        if let Some(n) = l.strip_prefix("Certificate Name:") {
            name = Some(n.trim().to_owned());
            domains.clear();
        } else if let Some(d) = l.strip_prefix("Domains:") {
            domains = d.split_whitespace().map(str::to_owned).collect();
        } else if let Some(e) = l.strip_prefix("Expiry Date:")
            && let (Some(n), Some(expires_at)) = (name.take(), parse_expiry(e))
        {
            certs.push(Cert {
                name: n,
                domains: std::mem::take(&mut domains),
                expires_at,
            });
        }
    }
    certs
}

static CERTS: Mutex<Option<(Instant, Vec<Cert>)>> = Mutex::new(None);

/// `sudo -n certbot certificates`, cached 5 minutes. None when certbot cannot be run.
async fn certs(force: bool) -> Option<Vec<Cert>> {
    if !force
        && let Some((at, c)) = CERTS.lock().unwrap().as_ref()
        && at.elapsed() < Duration::from_secs(300)
    {
        return Some(c.clone());
    }
    let o = sudo(&["/usr/bin/certbot", "certificates"])
        .timeout(Duration::from_secs(60))
        .output()
        .await
        .ok()?;
    if !o.exit.success() {
        return None;
    }
    let c = parse_certs(&o.stdout);
    *CERTS.lock().unwrap() = Some((Instant::now(), c.clone()));
    Some(c)
}

/// Copy certificate expiry into `sites.ssl_expires_at`. A site whose certificate is gone from
/// certbot goes back to NULL (the config is then generated without SSL).
async fn sync_certs(st: &AppState, force: bool) {
    let Some(list) = certs(force).await else { return };
    let rows: Vec<(i64, String, Option<i64>)> = sqlx::query_as("SELECT id, domain, ssl_expires_at FROM sites")
        .fetch_all(&st.db)
        .await
        .unwrap_or_default();
    for (id, domain, cur) in rows {
        let want = list.iter().find(|c| c.name == domain).map(|c| c.expires_at);
        if want != cur {
            sqlx::query("UPDATE sites SET ssl_expires_at = ? WHERE id = ?")
                .bind(want)
                .bind(id)
                .execute(&st.db)
                .await
                .ok();
        }
    }
}

// ======================================================================
// Rows
// ======================================================================

#[derive(sqlx::FromRow, Clone)]
struct Row {
    id: i64,
    domain: String,
    aliases: String,
    app_id: Option<i64>,
    kind: String,
    upstream: Option<String>,
    root_path: Option<String>,
    redirect_url: Option<String>,
    max_upload_mb: i64,
    websocket: bool,
    gzip: bool,
    cache_assets: bool,
    custom_directives: String,
    enabled: bool,
    ssl: bool,
    force_https: bool,
    ssl_email: Option<String>,
    ssl_expires_at: Option<i64>,
    updated_at: i64,
    app_name: Option<String>,
    app_port: Option<i64>,
}

const SELECT: &str = "SELECT s.id, s.domain, s.aliases, s.app_id, s.kind, s.upstream, s.root_path, s.redirect_url, \
    s.max_upload_mb, s.websocket, s.gzip, s.cache_assets, s.custom_directives, s.enabled, s.ssl, s.force_https, \
    s.ssl_email, s.ssl_expires_at, s.updated_at, a.name AS app_name, a.port AS app_port \
    FROM sites s LEFT JOIN apps a ON a.id = s.app_id";

impl Row {
    fn aliases(&self) -> Vec<String> {
        serde_json::from_str(&self.aliases).unwrap_or_default()
    }

    fn cert_ready(&self) -> bool {
        self.ssl_expires_at.is_some()
    }

    fn input(&self) -> Input {
        Input {
            id: Some(self.id),
            domain: self.domain.clone(),
            aliases: self.aliases(),
            kind: self.kind.clone(),
            app_id: self.app_id,
            root_path: self.root_path.clone().unwrap_or_default(),
            redirect_url: self.redirect_url.clone().unwrap_or_default(),
            max_upload_mb: self.max_upload_mb,
            websocket: self.websocket,
            gzip: self.gzip,
            cache_assets: self.cache_assets,
            custom_directives: self.custom_directives.clone(),
            ssl: self.ssl,
            force_https: self.force_https,
            ssl_email: self.ssl_email.clone().unwrap_or_default(),
        }
    }

    fn draft(&self, st: &AppState) -> Result<Draft, AppError> {
        Draft::build(&self.input(), &st.cfg.apps_dir, self.app_port)
    }

    fn json(&self) -> Value {
        let upstream = match (self.kind.as_str(), self.app_port) {
            ("proxy", Some(p)) => Some(format!("127.0.0.1:{p}")),
            _ => self.upstream.clone(),
        };
        let days_left = self.ssl_expires_at.map(|e| (e - now()).div_euclid(86400));
        json!({
            "id": self.id,
            "domain": self.domain,
            "aliases": self.aliases(),
            "kind": self.kind,
            "appId": self.app_id,
            "appName": self.app_name,
            "upstream": upstream,
            "rootPath": self.root_path,
            "redirectUrl": self.redirect_url,
            "maxUploadMb": self.max_upload_mb,
            "websocket": self.websocket,
            "gzip": self.gzip,
            "cacheAssets": self.cache_assets,
            "customDirectives": self.custom_directives,
            "enabled": self.enabled,
            "ssl": self.ssl,
            "forceHttps": self.force_https,
            "sslEmail": self.ssl_email,
            "sslExpiresAt": self.ssl_expires_at,
            "sslDaysLeft": days_left,
            "certReady": self.cert_ready(),
            "file": conf_file(&nginx_dir(), &self.domain, self.enabled).to_string_lossy(),
            "updatedAt": self.updated_at,
        })
    }
}

async fn load(st: &AppState, id: i64) -> ApiResult<Row> {
    sqlx::query_as::<_, Row>(&format!("{SELECT} WHERE s.id = ?"))
        .bind(id)
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| AppError::not_found("Site not found."))
}

async fn app_port(st: &AppState, app_id: Option<i64>) -> ApiResult<Option<i64>> {
    let Some(id) = app_id else { return Ok(None) };
    let r: Option<(Option<i64>,)> = sqlx::query_as("SELECT port FROM apps WHERE id = ?")
        .bind(id)
        .fetch_optional(&st.db)
        .await?;
    match r {
        Some((p,)) => Ok(p),
        None => Err(AppError::bad_request("That app does not exist.")),
    }
}

async fn taken(st: &AppState, domain: &str, except: Option<i64>) -> ApiResult<()> {
    let r: Option<(i64,)> = sqlx::query_as("SELECT id FROM sites WHERE domain = ? AND id != ?")
        .bind(domain)
        .bind(except.unwrap_or(-1))
        .fetch_optional(&st.db)
        .await?;
    if r.is_some() {
        return Err(AppError::Status(
            StatusCode::CONFLICT,
            format!("A site for {domain} already exists."),
        ));
    }
    Ok(())
}

// ======================================================================
// Handlers: sites
// ======================================================================

async fn list(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    sync_certs(&st, false).await;
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} ORDER BY s.domain"))
        .fetch_all(&st.db)
        .await?;
    Ok(Json(rows.iter().map(Row::json).collect::<Vec<_>>()))
}

async fn get_one(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    Ok(Json(load(&st, id).await?.json()))
}

#[derive(Serialize)]
struct AppOption {
    id: i64,
    name: String,
    port: Option<i64>,
}

/// Apps to pick from in the editor (own query, so this page does not depend on the apps module's API).
async fn apps_list(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows: Vec<(i64, String, Option<i64>)> = sqlx::query_as("SELECT id, name, port FROM apps ORDER BY name")
        .fetch_all(&st.db)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, port)| AppOption { id, name, port })
            .collect::<Vec<_>>(),
    ))
}

#[derive(Deserialize)]
struct DnsQuery {
    names: String,
}

async fn dns(Query(q): Query<DnsQuery>) -> ApiResult<impl IntoResponse> {
    let names: Vec<String> = q
        .names
        .split(',')
        .filter(|n| !n.trim().is_empty())
        .take(21)
        .map(hostname)
        .collect::<Result<_, _>>()?;
    let want = public_ip();
    let mut set = tokio::task::JoinSet::new();
    for (i, n) in names.into_iter().enumerate() {
        let want = want.clone();
        set.spawn(async move { (i, dns_check(&n, &want).await) });
    }
    let mut out = Vec::new();
    while let Some(Ok(r)) = set.join_next().await {
        out.push(r);
    }
    out.sort_by_key(|(i, _)| *i);
    Ok(Json(
        json!({ "serverIp": want, "results": out.into_iter().map(|(_, r)| r).collect::<Vec<_>>() }),
    ))
}

/// Draft + the SSL state it would be generated with.
async fn draft_for(st: &AppState, input: &Input) -> ApiResult<(Draft, bool)> {
    let draft = Draft::build(input, &st.cfg.apps_dir, app_port(st, input.app_id).await?)?;
    let cert = match input.id {
        Some(id) => load(st, id)
            .await
            .ok()
            .is_some_and(|r| r.cert_ready() && r.domain == draft.domain),
        None => false,
    };
    Ok((draft, cert))
}

async fn preview_draft(State(st): State<AppState>, Json(input): Json<Input>) -> ApiResult<impl IntoResponse> {
    let (d, cert) = draft_for(&st, &input).await?;
    let ssl_active = d.ssl && cert;
    Ok(Json(
        json!({ "config": generate(&d, ssl_active), "sslActive": ssl_active }),
    ))
}

async fn preview(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    let r = load(&st, id).await?;
    let ssl_active = r.ssl && r.cert_ready();
    Ok(Json(
        json!({ "config": generate(&r.draft(&st)?, ssl_active), "sslActive": ssl_active }),
    ))
}

/// (folders that get traverse-only, tree that gets read + default ACL) for a static root.
fn acl_plan(apps_dir: &Path, root: &Path) -> Result<(Vec<PathBuf>, PathBuf), String> {
    let rel = root.strip_prefix(apps_dir).map_err(|_| "Folder is outside the apps folder.".to_owned())?;
    let app = rel.components().next().ok_or("Pick a folder inside an app.")?;
    let app_dir = apps_dir.join(app);
    let in_app = root.strip_prefix(&app_dir).unwrap_or(Path::new(""));
    let releases = app_dir.join("releases");
    let tree = if (in_app.starts_with("current") || in_app.starts_with("releases")) && releases.is_dir() {
        releases
    } else {
        root.to_path_buf()
    };
    if app_dir.join("shared").starts_with(&tree) || tree.starts_with(app_dir.join("shared")) {
        return Err("Not shared to nginx: the folder holds or is inside shared/ (the app's .env). Serve a build folder such as current/dist.".into());
    }
    let top = apps_dir.parent().unwrap_or(apps_dir);
    let mut walk: Vec<PathBuf> = tree
        .ancestors()
        .skip(1)
        .take_while(|a| a.starts_with(top) && *a != Path::new("/"))
        .map(Path::to_path_buf)
        .collect();
    walk.reverse();
    Ok((walk, tree))
}

/// Let www-data read a static site's folder (see the module docs). Err = a warning for the owner;
/// the site itself is saved. No-op when the manager isn't root (macOS dev).
async fn grant_web_read(apps_dir: &Path, root: &str) -> Result<(), String> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        return Ok(());
    }
    let (walk, tree) = acl_plan(apps_dir, Path::new(root))?;
    if !tree.exists() {
        return Err(format!("{} doesn't exist yet. Save the site again after the first deploy so nginx may read it.", tree.display()));
    }
    let run = |args: Vec<String>| async move {
        match Cmd::new("setfacl").args(args).timeout(Duration::from_secs(120)).output().await {
            Ok(o) if o.exit.success() => Ok(()),
            Ok(o) => Err(format!("setfacl: {}", o.stderr.trim())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err("setfacl is missing, so nginx can't read this folder. Install it with: sudo apt install acl".into())
            }
            Err(e) => Err(format!("setfacl: {e}")),
        }
    };
    let mut a = vec!["-m".to_owned(), "u:www-data:x".to_owned(), "--".to_owned()];
    a.extend(walk.iter().map(|p| p.to_string_lossy().into_owned()));
    if !walk.is_empty() {
        run(a).await?;
    }
    run(vec![
        "-R".into(),
        "-m".into(),
        "u:www-data:rX,d:u:www-data:rX".into(),
        "--".into(),
        tree.to_string_lossy().into_owned(),
    ])
    .await
}

/// After a static site is saved: ACLs for nginx; a problem comes back as `warning`.
async fn web_read_warning(st: &AppState, d: &Draft) -> Option<String> {
    let root = d.root_path.as_deref().filter(|_| d.kind == Kind::Static)?;
    let r = grant_web_read(&st.cfg.apps_dir, root).await.err();
    if let Some(w) = &r {
        eprintln!("[nginx] {}: {w}", d.domain);
    }
    r
}

async fn create(State(st): State<AppState>, Extension(a): Extension<Admin>, Json(input): Json<Input>) -> ApiResult {
    let d = Draft::build(&input, &st.cfg.apps_dir, app_port(&st, input.app_id).await?)?;
    let g = APPLY_LOCK.lock().await;
    taken(&st, &d.domain, None).await?;
    if let Err(e) = apply(&g, &[], Some((&d.domain, true, &generate(&d, false)))).await {
        return Ok(apply_error(e, &d.domain));
    }
    let id = insert_or_update(&st, None, &d, true, None).await?;
    drop(g);
    let warning = web_read_warning(&st, &d).await;
    activity::log(
        &st.db,
        &a.username,
        "site.create",
        &d.domain,
        &format!("{} · ssl {}", d.kind.as_str(), if d.ssl { "on" } else { "off" }),
    )
    .await;
    let site = load(&st, id).await?;
    Ok(Json(json!({ "site": site.json(), "needsCert": d.ssl && !site.cert_ready(), "warning": warning })).into_response())
}

async fn insert_or_update(
    st: &AppState,
    id: Option<i64>,
    d: &Draft,
    enabled: bool,
    expires: Option<i64>,
) -> ApiResult<i64> {
    let aliases = serde_json::to_string(&d.aliases).unwrap_or_else(|_| "[]".into());
    let sql = if id.is_some() {
        "UPDATE sites SET domain=?, aliases=?, app_id=?, kind=?, upstream=?, root_path=?, redirect_url=?, max_upload_mb=?, \
         websocket=?, gzip=?, cache_assets=?, custom_directives=?, enabled=?, ssl=?, force_https=?, ssl_email=?, \
         ssl_expires_at=?, updated_at=unixepoch() WHERE id=?"
    } else {
        "INSERT INTO sites (domain, aliases, app_id, kind, upstream, root_path, redirect_url, max_upload_mb, websocket, gzip, \
         cache_assets, custom_directives, enabled, ssl, force_https, ssl_email, ssl_expires_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
    };
    let q = sqlx::query(sql)
        .bind(&d.domain)
        .bind(aliases)
        .bind(d.app_id)
        .bind(d.kind.as_str())
        .bind(&d.upstream)
        .bind(&d.root_path)
        .bind(&d.redirect_url)
        .bind(d.max_upload_mb)
        .bind(d.websocket)
        .bind(d.gzip)
        .bind(d.cache_assets)
        .bind(&d.custom)
        .bind(enabled)
        .bind(d.ssl)
        .bind(d.force_https)
        .bind(&d.ssl_email)
        .bind(expires);
    let r = match id {
        Some(id) => q.bind(id).execute(&st.db).await?,
        None => q.execute(&st.db).await?,
    };
    Ok(id.unwrap_or(r.last_insert_rowid()))
}

async fn update(
    State(st): State<AppState>,
    Extension(a): Extension<Admin>,
    UrlPath(id): UrlPath<i64>,
    Json(input): Json<Input>,
) -> ApiResult {
    let g = APPLY_LOCK.lock().await;
    let old = load(&st, id).await?;
    let d = Draft::build(&input, &st.cfg.apps_dir, app_port(&st, input.app_id).await?)?;
    taken(&st, &d.domain, Some(id)).await?;
    // A certificate belongs to the domain it was issued for; a rename starts without one.
    let expires = if old.domain == d.domain {
        old.ssl_expires_at
    } else {
        None
    };
    let ssl_active = d.ssl && expires.is_some();
    let rename = old.domain != d.domain;
    let remove: Vec<&str> = if rename { vec![&old.domain] } else { vec![] };
    if let Err(e) = apply(&g, &remove, Some((&d.domain, old.enabled, &generate(&d, ssl_active)))).await {
        return Ok(apply_error(e, &d.domain));
    }
    insert_or_update(&st, Some(id), &d, old.enabled, expires).await?;
    drop(g);
    let warning = web_read_warning(&st, &d).await;
    activity::log(
        &st.db,
        &a.username,
        "site.update",
        &d.domain,
        if rename { &old.domain } else { "" },
    )
    .await;
    let site = load(&st, id).await?;
    Ok(Json(json!({ "site": site.json(), "needsCert": d.ssl && !site.cert_ready(), "warning": warning })).into_response())
}

#[derive(Deserialize)]
struct DeleteQuery {
    #[serde(default, rename = "deleteCert")]
    delete_cert: bool,
}

async fn remove(
    State(st): State<AppState>,
    Extension(a): Extension<Admin>,
    UrlPath(id): UrlPath<i64>,
    Query(q): Query<DeleteQuery>,
) -> ApiResult {
    let g = APPLY_LOCK.lock().await;
    let r = load(&st, id).await?;
    if let Err(e) = apply(&g, &[&r.domain], None).await {
        return Ok(apply_error(e, &r.domain));
    }
    sqlx::query("DELETE FROM sites WHERE id = ?")
        .bind(id)
        .execute(&st.db)
        .await?;
    drop(g);
    let mut cert_error = None;
    if q.delete_cert && r.cert_ready() {
        let o = sudo(&[
            "/usr/bin/certbot",
            "delete",
            "--non-interactive",
            "--cert-name",
            &r.domain,
        ])
        .timeout(Duration::from_secs(60))
        .output()
        .await;
        match o {
            Ok(o) if o.exit.success() => {}
            Ok(o) => cert_error = Some(joined(&o)),
            Err(e) => cert_error = Some(e.to_string()),
        }
        sync_certs(&st, true).await;
    }
    activity::log(
        &st.db,
        &a.username,
        "site.delete",
        &r.domain,
        if q.delete_cert { "with certificate" } else { "" },
    )
    .await;
    Ok(Json(json!({ "ok": true, "certError": cert_error })).into_response())
}

async fn set_enabled(st: &AppState, a: &Admin, id: i64, enabled: bool) -> ApiResult {
    let g = APPLY_LOCK.lock().await;
    let r = load(st, id).await?;
    if r.enabled == enabled {
        return Ok(Json(json!({ "site": r.json() })).into_response());
    }
    // Move the file as it is: regenerating needs the app, which may have been deleted.
    // Only a missing file is rebuilt from the row.
    let content = match std::fs::read_to_string(conf_file(&nginx_dir(), &r.domain, r.enabled)) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let d = r.draft(st)?;
            generate(&d, d.ssl && r.cert_ready())
        }
        Err(e) => return Err(e.into()),
    };
    if let Err(e) = apply(&g, &[], Some((&r.domain, enabled, &content))).await {
        return Ok(apply_error(e, &r.domain));
    }
    sqlx::query("UPDATE sites SET enabled = ?, updated_at = unixepoch() WHERE id = ?")
        .bind(enabled)
        .bind(id)
        .execute(&st.db)
        .await?;
    drop(g);
    activity::log(
        &st.db,
        &a.username,
        if enabled { "site.enable" } else { "site.disable" },
        &r.domain,
        "",
    )
    .await;
    Ok(Json(json!({ "site": load(st, id).await?.json() })).into_response())
}

async fn enable(State(st): State<AppState>, Extension(a): Extension<Admin>, UrlPath(id): UrlPath<i64>) -> ApiResult {
    set_enabled(&st, &a, id, true).await
}

async fn disable(State(st): State<AppState>, Extension(a): Extension<Admin>, UrlPath(id): UrlPath<i64>) -> ApiResult {
    set_enabled(&st, &a, id, false).await
}

// ======================================================================
// Handlers: nginx
// ======================================================================

async fn status(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    sync_certs(&st, false).await;
    let active = Cmd::new("systemctl")
        .args(["is-active", "nginx"])
        .output()
        .await
        .is_ok_and(|o| o.stdout.trim() == "active");
    let version = Cmd::new("/usr/sbin/nginx").arg("-v").output().await.ok().and_then(|o| {
        let t = joined(&o);
        t.split_once("nginx/")
            .map(|(_, v)| v.split_whitespace().next().unwrap_or_default().to_owned())
    });
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} ORDER BY s.domain"))
        .fetch_all(&st.db)
        .await?;
    let expiring: Vec<Value> = rows
        .iter()
        .filter_map(|r| {
            let days = (r.ssl_expires_at? - now()).div_euclid(86400);
            (days <= EXPIRY_WARN_DAYS).then(|| json!({ "id": r.id, "domain": r.domain, "daysLeft": days }))
        })
        .collect();
    let last = LAST_TEST
        .lock()
        .unwrap()
        .as_ref()
        .map(|t| json!({ "at": t.at, "ok": t.ok, "output": t.output }));
    Ok(Json(json!({
        "active": active,
        "version": version,
        "lastTest": last,
        "sites": { "total": rows.len(), "enabled": rows.iter().filter(|r| r.enabled).count() },
        "certs": { "total": rows.iter().filter(|r| r.cert_ready()).count(), "expiring": expiring },
        "publicIp": public_ip(),
        "dir": nginx_dir().to_string_lossy(),
    })))
}

async fn test(Extension(a): Extension<Admin>, State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    // Never test a half-applied change.
    let t = {
        let _g = APPLY_LOCK.lock().await;
        nginx_test().await
    };
    activity::log(
        &st.db,
        &a.username,
        "nginx.test",
        "",
        if t.ok { "ok" } else { "failed" },
    )
    .await;
    Ok(Json(t))
}

async fn reload(Extension(a): Extension<Admin>, State(st): State<AppState>) -> ApiResult {
    let _g = APPLY_LOCK.lock().await;
    let r = test_and_reload().await;
    activity::log(
        &st.db,
        &a.username,
        "nginx.reload",
        "",
        if r.is_ok() { "ok" } else { "failed" },
    )
    .await;
    Ok(match r {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => apply_error(e, ""),
    })
}

// ======================================================================
// SSL issuing: POST starts a job, GET .../ssl/log replays + follows it as SSE
// ======================================================================

struct Job {
    events: Mutex<Vec<(&'static str, String)>>,
    tx: watch::Sender<usize>,
}

impl Job {
    fn push(&self, name: &'static str, data: String) {
        let mut e = self.events.lock().unwrap();
        e.push((name, data));
        self.tx.send_replace(e.len());
    }
    fn line(&self, stream: &'static str, text: impl Into<String>) {
        self.push(stream, text.into());
    }
    fn step(&self, idx: usize, state: &str, meta: &str) {
        self.push("step", json!({ "idx": idx, "state": state, "meta": meta }).to_string());
    }
    fn finished(&self) -> bool {
        self.events.lock().unwrap().last().is_some_and(|(n, _)| *n == "done")
    }
}

static JOBS: LazyLock<Mutex<HashMap<i64, Arc<Job>>>> = LazyLock::new(Default::default);

#[derive(Deserialize, Default)]
#[serde(default)]
struct SslBody {
    force: bool,
}

async fn ssl_start(
    State(st): State<AppState>,
    Extension(a): Extension<Admin>,
    UrlPath(id): UrlPath<i64>,
    Query(body): Query<SslBody>,
) -> ApiResult {
    let site = load(&st, id).await?;
    let d = site.draft(&st)?;
    if d.ssl_email.is_none() {
        return Err(AppError::bad_request(
            "Add an expiry email before requesting a certificate.",
        ));
    }
    let job = Arc::new(Job {
        events: Mutex::new(Vec::new()),
        tx: watch::channel(0).0,
    });
    {
        let mut jobs = JOBS.lock().unwrap();
        if jobs.get(&id).is_some_and(|j| !j.finished()) {
            return Err(AppError::Status(
                StatusCode::CONFLICT,
                "A certificate request for this site is already running.".into(),
            ));
        }
        jobs.insert(id, job.clone());
    }
    let force = body.force;
    activity::log(
        &st.db,
        &a.username,
        "site.ssl",
        &site.domain,
        if force { "force renewal" } else { "issue" },
    )
    .await;
    tokio::spawn(async move {
        match run_ssl(&st, &job, &site, d, force).await {
            Ok(()) => job.push("done", json!({ "code": 0 }).to_string()),
            Err(m) => {
                job.line("stderr", m.clone());
                job.push("done", json!({ "code": 1, "error": m }).to_string());
            }
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "started": true }))).into_response())
}

async fn run_ssl(st: &AppState, job: &Job, site: &Row, d: Draft, force: bool) -> Result<(), String> {
    let email = d.ssl_email.clone().unwrap_or_default();
    let names: Vec<&str> = std::iter::once(d.domain.as_str())
        .chain(d.aliases.iter().map(String::as_str))
        .collect();

    // 1. DNS
    job.step(0, "run", "");
    let ip = public_ip();
    let mut bad = Vec::new();
    for n in &names {
        let r = dns_check(n, &ip).await;
        job.line(
            "stdout",
            format!(
                "{n} -> {}",
                if r.ips.is_empty() {
                    "no A record".into()
                } else {
                    r.ips.join(", ")
                }
            ),
        );
        if !r.ok {
            bad.push(*n);
        }
    }
    if !bad.is_empty() {
        job.step(0, "fail", &format!("{} does not point to {ip}", bad.join(", ")));
        return Err(format!(
            "DNS: {} must have an A record pointing to {ip} before a certificate can be issued.",
            bad.join(", ")
        ));
    }
    job.step(0, "done", &format!("{} -> {ip}", d.domain));

    // 2. certbot (webroot)
    job.step(1, "run", "");
    let mut cmd = sudo(&[
        "/usr/bin/certbot",
        "certonly",
        "--webroot",
        "-w",
        WEBROOT,
        "--cert-name",
        &d.domain,
    ]);
    for n in &names {
        cmd = cmd.arg("-d").arg(*n);
    }
    cmd = cmd
        .args([
            "--email",
            &email,
            "--agree-tos",
            "--non-interactive",
            "--expand",
            "--deploy-hook",
            "systemctl reload nginx",
        ])
        .timeout(Duration::from_secs(300));
    if force {
        cmd = cmd.arg("--force-renewal");
    }
    job.line("stdout", format!("$ {}", cmd.display()));
    let (tx, mut rx) = mpsc::channel::<Line>(256);
    let fwd = async {
        while let Some(l) = rx.recv().await {
            job.line(if l.stream == Stream::Out { "stdout" } else { "stderr" }, l.text);
        }
    };
    let (exit, ()) = tokio::join!(cmd.stream(tx), fwd);
    match exit {
        Ok(e) if e.success() => {}
        Ok(e) => {
            job.step(1, "fail", "");
            return Err(format!(
                "certbot did not finish ({e:?}). The site config was not changed."
            ));
        }
        Err(e) => {
            job.step(1, "fail", "");
            return Err(format!("Could not run certbot: {e}"));
        }
    }
    job.step(1, "done", "");

    // 3. regenerate with SSL, nginx -t, reload
    job.step(2, "run", "");
    let list = certs(true).await.unwrap_or_default();
    let Some(cert) = list.iter().find(|c| c.name == d.domain) else {
        job.step(2, "fail", "");
        return Err("certbot succeeded but the certificate is not listed. Check `certbot certificates`.".into());
    };
    // certbot can take minutes: build from the row as it is now, so edits made meanwhile are kept.
    let g = APPLY_LOCK.lock().await;
    let text = |e: AppError| match e {
        AppError::Status(_, m) | AppError::Internal(m) => m,
    };
    let site = load(st, site.id).await.map_err(text)?;
    let mut d = site.draft(st).map_err(text)?;
    if d.domain != cert.name {
        job.step(2, "fail", "");
        return Err("The site was renamed while the certificate was issued. The config was not changed.".into());
    }
    d.ssl = true;
    if let Err(e) = apply(&g, &[], Some((&d.domain, site.enabled, &generate(&d, true)))).await {
        job.step(2, "fail", "");
        let msg = match e {
            ApplyError::Test(t) => {
                for l in t.output.lines() {
                    job.line("stderr", l);
                }
                "The certificate was issued, but nginx -t rejected the SSL config. The previous config was kept."
                    .to_owned()
            }
            ApplyError::Reload(m) => format!("The certificate was issued, but the reload failed: {m}"),
            ApplyError::Io(m) => format!("Could not write the config: {m}"),
        };
        return Err(msg);
    }
    // Only now: a config nginx rejected must not leave ssl = 1 behind.
    sqlx::query("UPDATE sites SET ssl = 1, ssl_expires_at = ?, updated_at = unixepoch() WHERE id = ?")
        .bind(cert.expires_at)
        .bind(site.id)
        .execute(&st.db)
        .await
        .map_err(|e| e.to_string())?;
    drop(g);
    job.step(2, "done", "nginx -t && systemctl reload nginx");
    activity::log(
        &st.db,
        "system",
        "site.ssl.done",
        &d.domain,
        &format!("expires {}", cert.expires_at),
    )
    .await;
    Ok(())
}

async fn ssl_log(UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    let job = JOBS
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| AppError::not_found("No certificate request has run for this site."))?;
    let (etx, erx) = mpsc::channel::<Event>(256);
    tokio::spawn(async move {
        let mut rx = job.tx.subscribe();
        let mut i = 0;
        loop {
            let batch: Vec<_> = job.events.lock().unwrap()[i..].to_vec();
            for (name, data) in batch {
                let ev = match name {
                    "stdout" | "stderr" => line_event(&Line {
                        stream: if name == "stdout" { Stream::Out } else { Stream::Err },
                        text: data,
                    }),
                    other => Event::default().event(other).data(data),
                };
                i += 1;
                let last = name == "done";
                if etx.send(ev).await.is_err() || last {
                    return;
                }
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    });
    Ok(crate::sse::from_channel(erx))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/sites", get(list).post(create))
        .route("/sites/apps", get(apps_list))
        .route("/sites/dns", get(dns))
        .route("/sites/preview", post(preview_draft))
        .route("/sites/{id}", get(get_one).put(update).delete(remove))
        .route("/sites/{id}/enable", post(enable))
        .route("/sites/{id}/disable", post(disable))
        .route("/sites/{id}/preview", get(preview))
        .route("/sites/{id}/ssl", post(ssl_start))
        .route("/sites/{id}/ssl/log", get(ssl_log))
        .route("/nginx/status", get(status))
        .route("/nginx/test", post(test))
        .route("/nginx/reload", post(reload))
}

pub fn spawn(_st: &AppState) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: &str) -> Input {
        Input {
            domain: "radio.herani.id".into(),
            kind: kind.into(),
            ..Input::default()
        }
    }

    fn proxy() -> Draft {
        Draft::build(
            &Input {
                app_id: Some(1),
                ..input("proxy")
            },
            Path::new("/nonexistent"),
            Some(3010),
        )
        .unwrap()
    }

    #[test]
    fn proxy_with_ssl_matches_design() {
        let mut d = proxy();
        d.custom = "location /docs {\n    proxy_pass http://127.0.0.1:3010;\n    proxy_buffering off;\n}".into();
        let want = r#"server {
    listen 80;
    server_name radio.herani.id;

    location /.well-known/acme-challenge/ {
        root /var/www/html;
    }

    location / {
        return 301 https://$host$request_uri;
    }
}

server {
    listen 443 ssl http2;
    server_name radio.herani.id;

    ssl_certificate     /etc/letsencrypt/live/radio.herani.id/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/radio.herani.id/privkey.pem;

    client_max_body_size 10m;
    gzip on;

    location / {
        proxy_pass http://127.0.0.1:3010;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }

    # custom
    location /docs {
        proxy_pass http://127.0.0.1:3010;
        proxy_buffering off;
    }
}
"#;
        assert_eq!(generate(&d, true), want);
    }

    #[test]
    fn custom_root_location_replaces_the_generated_one() {
        let mut d = proxy();
        d.custom = "location / {\n    proxy_pass http://127.0.0.1:3010;\n    proxy_set_header X-Legacy \"\";\n}".into();
        let out = generate(&d, false);
        assert_eq!(out.matches("location / {").count(), 1, "{out}");
        assert!(out.contains("X-Legacy"));
        assert!(out.contains("acme-challenge"));
        assert!(!custom_has_root("location /docs {"));
        assert!(custom_has_root("  location /{"));
    }

    #[test]
    fn proxy_without_ssl_and_with_aliases() {
        let mut d = proxy();
        d.aliases = vec!["www.radio.herani.id".into()];
        d.websocket = false;
        d.gzip = false;
        d.max_upload_mb = 50;
        d.cache_assets = true;
        let want = r#"server {
    listen 80;
    server_name radio.herani.id www.radio.herani.id;

    location /.well-known/acme-challenge/ {
        root /var/www/html;
    }

    client_max_body_size 50m;

    location / {
        proxy_pass http://127.0.0.1:3010;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_http_version 1.1;
    }

    location /assets/ {
        proxy_pass http://127.0.0.1:3010;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_http_version 1.1;
        expires 7d;
        add_header Cache-Control "public";
    }
}
"#;
        assert_eq!(generate(&d, false), want);
    }

    #[test]
    fn ssl_without_forced_redirect_is_one_server() {
        let mut d = proxy();
        d.force_https = false;
        let c = generate(&d, true);
        assert_eq!(c.matches("server {").count(), 1);
        assert!(c.contains("    listen 80;\n    listen 443 ssl http2;\n"));
        assert!(c.contains("acme-challenge"));
        assert!(!c.contains("return 301"));
    }

    #[test]
    fn static_site() {
        let tmp = std::env::temp_dir().join(format!("sm-nginx-static-{}", std::process::id()));
        let apps = tmp.join("apps");
        std::fs::create_dir_all(apps.join("landing/dist")).unwrap();
        let root = apps.join("landing/dist");
        let d = Draft::build(
            &Input {
                root_path: root.to_string_lossy().into(),
                ..input("static")
            },
            &apps,
            None,
        )
        .unwrap();
        let root = d.root_path.clone().unwrap();
        let want = format!(
            r#"server {{
    listen 80;
    server_name radio.herani.id;

    location /.well-known/acme-challenge/ {{
        root /var/www/html;
    }}

    client_max_body_size 10m;
    gzip on;

    root {root};
    index index.html;

    location / {{
        try_files $uri $uri/ =404;
    }}
}}
"#
        );
        assert_eq!(generate(&d, false), want);
        let outside = Input {
            root_path: "/etc".into(),
            ..input("static")
        };
        assert!(Draft::build(&outside, &apps, None).is_err());
        let spaced = Input {
            root_path: format!("{root} x;"),
            ..input("static")
        };
        assert!(Draft::build(&spaced, &apps, None).is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn static_acl_plan() {
        let tmp = std::env::temp_dir().join(format!("sm-nginx-acl-{}", std::process::id()));
        let apps = tmp.join("home/ubuntu/apps");
        std::fs::create_dir_all(apps.join("web/releases/3/dist")).unwrap();
        std::fs::create_dir_all(apps.join("plain/public")).unwrap();
        let home = tmp.join("home/ubuntu");
        // release layout → the whole releases/ tree, traverse on home, apps, app
        let (walk, tree) = acl_plan(&apps, &apps.join("web/current/dist")).unwrap();
        assert_eq!(tree, apps.join("web/releases"));
        assert_eq!(walk, vec![home.clone(), apps.clone(), apps.join("web")]);
        // plain folder → just that folder
        let (walk, tree) = acl_plan(&apps, &apps.join("plain/public")).unwrap();
        assert_eq!(tree, apps.join("plain/public"));
        assert_eq!(walk, vec![home, apps.clone(), apps.join("plain")]);
        // never the app folder itself or shared/ (.env)
        assert!(acl_plan(&apps, &apps.join("web")).is_err());
        assert!(acl_plan(&apps, &apps.join("web/shared")).is_err());
        assert!(acl_plan(&apps, &apps.join("web/shared/public")).is_err());
        assert!(acl_plan(&apps, Path::new("/etc")).is_err());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn redirect_site() {
        let r = |u: &str| {
            Draft::build(
                &Input {
                    redirect_url: u.into(),
                    ..input("redirect")
                },
                Path::new("/x"),
                None,
            )
            .unwrap()
        };
        let want = r#"server {
    listen 80;
    server_name radio.herani.id;

    location /.well-known/acme-challenge/ {
        root /var/www/html;
    }

    location / {
        return 301 https://new.example.com$request_uri;
    }
}
"#;
        assert_eq!(generate(&r("https://new.example.com"), false), want);
        assert!(
            generate(&r("https://new.example.com/landing"), false)
                .contains("return 301 https://new.example.com/landing;")
        );
        for bad in [
            "ftp://x.com",
            "https://",
            "https://a.com;rm",
            "https://a.com/{x}",
            "https://a b.com",
            "https://a.com/$x",
            "javascript:alert(1)",
        ] {
            let i = Input {
                redirect_url: bad.into(),
                ..input("redirect")
            };
            assert!(Draft::build(&i, Path::new("/x"), None).is_err(), "{bad}");
        }
    }

    #[test]
    fn domains() {
        for ok in ["a.io", "Radio.Herani.ID", "x-1.sub.example.com", "sm-test.invalid"] {
            assert!(hostname(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "localhost",
            "a b.com",
            "a.com;",
            "a.com{",
            "a.com}",
            "a\".com",
            "a'.com",
            "-a.com",
            "a-.com",
            "a..com",
            ".a.com",
            "a.com/x",
            "1.2.3.4",
            "a_b.com",
            "ä.com",
            "a.com\nserver",
            "$host.com",
            &format!("{}.com", "x".repeat(64)),
        ] {
            assert!(hostname(bad).is_err(), "{bad:?}");
        }
        assert_eq!(hostname(" Radio.Herani.ID ").unwrap(), "radio.herani.id");
        let dup = Input {
            aliases: vec!["radio.herani.id".into()],
            ..input("redirect")
        };
        assert!(Draft::build(&dup, Path::new("/x"), None).is_err());
        let bad_alias = Input {
            aliases: vec!["www.a.com; deny all".into()],
            ..input("redirect")
        };
        assert!(Draft::build(&bad_alias, Path::new("/x"), None).is_err());
    }

    #[test]
    fn proxy_needs_app_and_port() {
        assert!(Draft::build(&input("proxy"), Path::new("/x"), None).is_err());
        let i = Input {
            app_id: Some(1),
            ..input("proxy")
        };
        assert!(Draft::build(&i, Path::new("/x"), None).is_err());
        assert!(Draft::build(&i, Path::new("/x"), Some(0)).is_err());
        assert!(Draft::build(&Input { max_upload_mb: 0, ..i }, Path::new("/x"), Some(3000)).is_err());
    }

    #[test]
    fn brace_balance() {
        for ok in [
            "",
            "location /a { return 200; }",
            "location /a {\n  location /b { }\n}",
            "# a } in a comment\nproxy_buffering off;",
            "add_header X \"}\";",
            "add_header X '{';",
            "set $x ${y};",
            "return 200 \"a \\\" }\";",
            // nginx ends the word `${y` at `;` (nginx -t then rejects the variable), no block opens or closes.
            "set $x ${y;",
            "add_header X a};",
        ] {
            assert!(braces_balanced(ok), "{ok:?}");
        }
        for bad in [
            "}",
            "location / { }\n}\nserver {",
            "location /a {",
            "return 200 \"x;",
            "} server { listen 1;",
            "add_header X a#b; }",
            "a } # {",
            // `${` is part of the word, but `;` still ends it, so these `}` close server { }.
            "default_type ${;}",
            "upstream u${ { # }\n",
            "set $x ${{;}",
            // A vertical tab or NBSP is not whitespace to nginx: `#` here is not a comment.
            "default_type a\u{b}#;}",
            "default_type a\u{a0}#;}",
        ] {
            assert!(!braces_balanced(bad), "{bad:?}");
        }
        let long = Input {
            custom_directives: "}".into(),
            ..Input {
                app_id: Some(1),
                ..input("proxy")
            }
        };
        assert!(Draft::build(&long, Path::new("/x"), Some(3000)).is_err());
    }

    #[test]
    fn parses_nginx_test_output() {
        let out = "nginx: [emerg] unknown directive \"proxy_buffer\" in /etc/nginx/sites-manager/radio.herani.id.conf:31\n\
                   nginx: configuration file /etc/nginx/nginx.conf test failed\n";
        assert_eq!(
            parse_test_error(out),
            Some(("/etc/nginx/sites-manager/radio.herani.id.conf".into(), 31))
        );
        let t = TestResult::from_output(false, out.into());
        assert_eq!(t.line_in("radio.herani.id"), Some(31));
        assert_eq!(t.line_in("other.herani.id"), None);
        // Real nginx 1.24 output: timestamp and pid instead of the "nginx:" prefix.
        assert_eq!(
            parse_test_error(
                "2026/10/18 15:40:53 [emerg] 35323#35323: unknown directive \"proxy_buffer\" in /etc/nginx/sites-manager/sm-test.invalid.conf:25"
            ),
            Some(("/etc/nginx/sites-manager/sm-test.invalid.conf".into(), 25))
        );
        let ok = "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\nnginx: configuration file /etc/nginx/nginx.conf test is successful\n";
        assert_eq!(parse_test_error(ok), None);
        assert!(TestResult::from_output(true, ok.into()).line.is_none());
        assert_eq!(
            parse_test_error("nginx: [emerg] cannot load certificate \"/x/y.pem\": BIO_new_file() failed"),
            None
        );
        assert_eq!(
            parse_test_error(
                "nginx: [warn] conflicting server name \"a\" on 0.0.0.0:80, ignored in /x/a.conf:3\nnginx: [emerg] bad in /x/b.conf:9"
            ),
            Some(("/x/b.conf".into(), 9))
        );
    }

    #[test]
    fn parses_certbot_certificates() {
        let out = "Found the following certs:\n  Certificate Name: radio.herani.id\n    Serial Number: 1\n    Key Type: ECDSA\n    \
                   Domains: radio.herani.id www.radio.herani.id\n    Expiry Date: 2026-12-19 10:00:00+00:00 (VALID: 61 days)\n    \
                   Certificate Path: /etc/letsencrypt/live/radio.herani.id/fullchain.pem\n  Certificate Name: b.example.com\n    \
                   Domains: b.example.com\n    Expiry Date: 2027-01-01 00:00:00+00:00 (VALID: 90 days)\n";
        let c = parse_certs(out);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].name, "radio.herani.id");
        assert_eq!(c[0].domains, ["radio.herani.id", "www.radio.herani.id"]);
        assert_eq!(c[0].expires_at, 1_797_674_400); // 2026-12-19T10:00:00Z
        assert_eq!(c[1].expires_at, 1_798_761_600); // 2027-01-01T00:00:00Z
        assert!(parse_certs("No certificates found.").is_empty());
    }

    #[tokio::test]
    async fn failed_check_restores_the_previous_file() {
        let dir = std::env::temp_dir().join(format!("sm-nginx-apply-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("a.example.com.conf");

        // New site, check fails: the new file is removed again.
        let fail = async || {
            Err(ApplyError::Test(TestResult::from_output(
                false,
                "nginx: [emerg] boom in x.conf:2".into(),
            )))
        };
        let r = apply_with(&dir, &[], Some(("a.example.com", true, "v1")), fail).await;
        assert!(matches!(r, Err(ApplyError::Test(_))));
        assert!(!live.exists());

        // Check passes: file is there.
        apply_with(&dir, &[], Some(("a.example.com", true, "v1")), async || Ok(()))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "v1");

        // Update fails: v1 is back, the temp file is gone.
        let r = apply_with(&dir, &[], Some(("a.example.com", true, "v2")), fail).await;
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "v1");

        // Rename fails: old file is back, new one is gone.
        let r = apply_with(&dir, &["a.example.com"], Some(("b.example.com", true, "v3")), fail).await;
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "v1");
        assert!(!dir.join("b.example.com.conf").exists());

        // Disable = renamed to .disabled; the check runs because a live file went away.
        apply_with(&dir, &[], Some(("a.example.com", false, "v1")), async || Ok(()))
            .await
            .unwrap();
        assert!(!live.exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("a.example.com.conf.disabled")).unwrap(),
            "v1"
        );

        // Disabled -> disabled touches nothing live, so the check must not run.
        let must_not_run = async || -> Result<(), ApplyError> { panic!("check ran for a disabled site") };
        apply_with(&dir, &[], Some(("a.example.com", false, "v2")), must_not_run)
            .await
            .unwrap();

        // Delete.
        apply_with(&dir, &["a.example.com"], None, async || Ok(()))
            .await
            .unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn snapshot_keeps_bytes_and_stops_on_read_errors() {
        let dir = std::env::temp_dir().join(format!("sm-nginx-snap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fail = async || Err(ApplyError::Reload("boom".into()));
        // Callers hold the lock; apply_with must not take it again (tokio's Mutex is not reentrant).
        let g = APPLY_LOCK.lock().await;

        // Non-UTF-8 content is restored byte for byte, not dropped as "no file".
        let live = dir.join("a.example.com.conf");
        std::fs::write(&live, b"v1 \xff").unwrap();
        let r = tokio::time::timeout(
            Duration::from_secs(5),
            apply_with(&dir, &[], Some(("a.example.com", true, "v2")), fail),
        )
        .await
        .expect("apply_with waited for APPLY_LOCK");
        assert!(matches!(r, Err(ApplyError::Reload(_))));
        assert_eq!(std::fs::read(&live).unwrap(), b"v1 \xff");

        // A file that cannot be read stops before anything is touched (it used to count as missing,
        // so a failed check deleted it). Root reads it anyway, so the case only runs as a normal user.
        use std::os::unix::fs::PermissionsExt;
        let b = dir.join("b.example.com.conf");
        std::fs::write(&b, "v1").unwrap();
        std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&b).is_err() {
            let r = apply_with(&dir, &[], Some(("b.example.com", true, "v2")), fail).await;
            assert!(matches!(r, Err(ApplyError::Io(_))), "{r:?}");
            std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(std::fs::read_to_string(&b).unwrap(), "v1");
        }
        drop(g);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
