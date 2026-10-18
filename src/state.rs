use crate::{auth::Lockout, config::Config};
use sqlx::SqlitePool;
use std::{ops::Deref, sync::Arc, sync::Mutex};

/// Shared by every handler. Later phases keep their own runtime state (deploy locks, sampler
/// caches) in a `static` inside their module instead of growing this struct.
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub db: SqlitePool,
    pub cfg: Config,
    pub jwt_secret: Vec<u8>,
    pub lockout: Mutex<Lockout>,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(db: SqlitePool, cfg: Config, jwt_secret: Vec<u8>) -> Self {
        AppState(Arc::new(Inner {
            db,
            cfg,
            jwt_secret,
            lockout: Mutex::default(),
        }))
    }
}
