//! API modules, one per phase. Each file owns its routes (paths relative to /api, all behind
//! `auth::require_auth`) and its background tasks. Later phases edit only their own file.
//!
//! | file         | phase | owns                                                        |
//! |--------------|-------|-------------------------------------------------------------|
//! | metrics.rs   | 2     | /api/metrics/*, /api/services, sampler + 5 min rollup        |
//! | apps.rs      | 3     | /api/apps, wizard checks, start/stop/restart, systemd units  |
//! | deploys.rs   | 3     | /api/apps/{name}/deploys*, pipeline, log SSE, /hooks webhook |
//! | env.rs       | 4     | /api/apps/{name}/env*                                        |
//! | databases.rs | 5     | /api/databases*, daily pg_dump                               |
//! | nginx.rs     | 6     | /api/sites*, nginx -t, certbot                               |
//! | settings.rs  | 7     | /api/settings*, deploy key, tokens, thresholds               |

pub mod apps;
pub mod databases;
pub mod deploys;
pub mod env;
pub mod metrics;
pub mod nginx;
pub mod settings;

use crate::{activity, auth, state::AppState};
use axum::Router;

/// Everything under /api except login. Mounted behind `require_auth` in main.rs.
pub fn api() -> Router<AppState> {
    Router::new()
        .merge(auth::routes())
        .merge(activity::routes())
        .merge(metrics::routes())
        .merge(apps::routes())
        .merge(deploys::routes())
        .merge(env::routes())
        .merge(databases::routes())
        .merge(nginx::routes())
        .merge(settings::routes())
}

/// Unauthenticated routes outside /api (they check their own signature, e.g. the GitHub webhook).
pub fn public() -> Router<AppState> {
    deploys::public_routes()
}

/// Background tasks, started once at boot.
pub fn spawn_all(st: &AppState) {
    metrics::spawn(st);
    apps::spawn(st);
    deploys::spawn(st);
    env::spawn(st);
    databases::spawn(st);
    nginx::spawn(st);
    settings::spawn(st);
}
