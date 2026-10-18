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
}

fn var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
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
            trust_proxy: var("SM_TRUST_PROXY").is_some_and(|v| v == "1" || v == "true"),
        }
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }
}
