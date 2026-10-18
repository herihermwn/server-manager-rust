//! Config from env. Everything the manager owns lives in the data dir (default `~/.server-manager`).

use std::{env, path::PathBuf};

pub struct Config {
    pub host: String,
    pub port: u16,
    /// `SM_DATA_DIR`: state.db, jwt.secret, logs/.
    pub data_dir: PathBuf,
    /// `SM_APPS_DIR`: every app folder must be under it (validate::path_under).
    pub apps_dir: PathBuf,
    /// `SM_WEB_DIST`: built frontend (web/build.sh output).
    pub web_dist: PathBuf,
    /// `SM_TRUST_PROXY=1`: take the client IP from X-Real-IP (only behind our own nginx).
    pub trust_proxy: bool,
    /// `SM_BASE_PATH` ("" or "/server-manager"): the prefix nginx strips before proxying here.
    /// Routes don't change; only the cookie path and public URLs carry it.
    pub base_path: String,
    /// `SM_PUBLIC_URL` (e.g. https://herani.id/server-manager, no trailing slash): shown for webhooks.
    pub public_url: Option<String>,
    /// `SM_COOKIE_SECURE=1`: session cookie gets `Secure` (browser is on https, e.g. behind Cloudflare).
    pub cookie_secure: bool,
}

fn var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn flag(key: &str) -> bool {
    var(key).is_some_and(|v| v == "1" || v == "true")
}

/// "" or "/a/b": leading slash, no trailing slash. Anything outside [A-Za-z0-9/_.-] is refused at
/// boot (it ends up in a cookie attribute).
fn base_path(raw: &str) -> String {
    let p = raw.trim().trim_matches('/');
    if p.is_empty() {
        return String::new();
    }
    assert!(
        p.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(&b)) && !p.contains(".."),
        "SM_BASE_PATH may only hold letters, digits and / _ . - (got {raw:?})"
    );
    format!("/{p}")
}

fn home() -> PathBuf {
    var("HOME").map(PathBuf::from).expect("HOME is not set")
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            host: var("HOST").unwrap_or_else(|| "127.0.0.1".into()),
            port: var("PORT").and_then(|p| p.parse().ok()).unwrap_or(3900),
            data_dir: var("SM_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home().join(".server-manager")),
            apps_dir: var("SM_APPS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home().join("apps")),
            web_dist: var("SM_WEB_DIST")
                .map(PathBuf::from)
                .unwrap_or_else(|| "web/dist".into()),
            trust_proxy: flag("SM_TRUST_PROXY"),
            base_path: base_path(var("SM_BASE_PATH").as_deref().unwrap_or("")),
            public_url: var("SM_PUBLIC_URL").map(|u| u.trim().trim_end_matches('/').to_owned()),
            cookie_secure: flag("SM_COOKIE_SECURE"),
        }
    }

    /// Config for tests: everything off, no prefix.
    #[cfg(test)]
    pub fn test(data_dir: PathBuf, apps_dir: PathBuf) -> Self {
        Config {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir,
            apps_dir,
            web_dist: "x".into(),
            trust_proxy: false,
            base_path: String::new(),
            public_url: None,
            cookie_secure: false,
        }
    }

    /// `SM_PUBLIC_URL`, else `http(s)://<Host header><base path>`. A Host with odd characters
    /// becomes `localhost`, so this never echoes markup or spaces back.
    pub fn public_url(&self, host: Option<&str>) -> String {
        if let Some(u) = &self.public_url {
            return u.clone();
        }
        let host = host
            .filter(|h| {
                !h.is_empty() && h.len() <= 255 && h.bytes().all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
            })
            .unwrap_or("localhost");
        let scheme = if self.cookie_secure { "https" } else { "http" };
        format!("{scheme}://{host}{}", self.base_path)
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_path_is_normalised() {
        assert_eq!(base_path(""), "");
        assert_eq!(base_path("/"), "");
        assert_eq!(base_path("server-manager"), "/server-manager");
        assert_eq!(base_path("/server-manager/"), "/server-manager");
        assert!(std::panic::catch_unwind(|| base_path("/a;b")).is_err());
    }

    #[test]
    fn public_url_falls_back_to_host() {
        let mut c = Config::test("/d".into(), "/a".into());
        assert_eq!(c.public_url(Some("localhost:3900")), "http://localhost:3900");
        assert_eq!(c.public_url(Some("evil\"<x>")), "http://localhost");
        assert_eq!(c.public_url(None), "http://localhost");
        c.base_path = "/server-manager".into();
        c.cookie_secure = true;
        assert_eq!(c.public_url(Some("herani.id")), "https://herani.id/server-manager");
        c.public_url = Some("https://herani.id/server-manager".into());
        assert_eq!(c.public_url(Some("127.0.0.1:3900")), "https://herani.id/server-manager");
    }
}
