mod activity;
mod auth;
mod config;
// Building blocks for phases 2-7; drop the allow once a route uses them.
#[allow(dead_code)]
mod exec;
mod http;
mod routes;
#[allow(dead_code)]
mod sse;
mod state;
#[allow(dead_code)]
mod validate;
mod web;

use axum::{Router, middleware::from_fn_with_state};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use state::AppState;
use std::{net::SocketAddr, os::unix::fs::PermissionsExt, time::Duration};

const USAGE: &str = "\
server-manager                         run the web server (HOST/PORT, default 127.0.0.1:3900)
server-manager create-admin <username> create the owner account or reset its password (read from stdin)";

#[tokio::main]
async fn main() {
    let cfg = config::Config::from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => {}
        Some("create-admin") => {
            let Some(name) = args.get(1) else {
                eprintln!("usage: server-manager create-admin <username>");
                std::process::exit(2);
            };
            let db = open_db(&cfg).await;
            auth::create_admin_cli(&db, name).await;
            return;
        }
        Some("-h" | "--help" | "help") => return println!("{USAGE}"),
        Some(other) => {
            eprintln!("unknown command: {other}\n\n{USAGE}");
            std::process::exit(2);
        }
    }

    let db = open_db(&cfg).await;
    let secret = auth::load_or_create_secret(&cfg.data_dir).expect("jwt secret");
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM admin_users")
        .fetch_one(&db)
        .await
        .unwrap_or(0);
    if users == 0 {
        eprintln!("[auth] no admin yet: run `server-manager create-admin <username>`");
    }

    let st = AppState::new(db, cfg, secret);
    routes::spawn_all(&st);
    let addr = format!("{}:{}", st.cfg.host, st.cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("bind {addr}: {e}"));
    println!(
        "server-manager on http://{addr} (data {}, apps {})",
        st.cfg.data_dir.display(),
        st.cfg.apps_dir.display()
    );
    axum::serve(listener, app(st).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown())
        .await
        .expect("server");
}

/// `/api/auth/login` open; every other `/api/*` (incl. unknown paths) behind `require_auth`;
/// `/hooks/*` checks its own signature; anything else is the SPA.
fn app(st: AppState) -> Router {
    let api = routes::api()
        .fallback(http::api_not_found)
        .layer(from_fn_with_state(st.clone(), auth::require_auth));
    let [h1, h2, h3, h4] = web::security_headers();
    Router::new()
        .merge(auth::login_route())
        .nest("/api", api)
        .merge(routes::public())
        .fallback_service(web::service(&st.cfg.web_dist))
        .layer(h1)
        .layer(h2)
        .layer(h3)
        .layer(h4)
        .with_state(st)
}

/// Data dir (0700) + SQLite (WAL, foreign keys) + migrations.
async fn open_db(cfg: &config::Config) -> SqlitePool {
    for d in [&cfg.data_dir, &cfg.logs_dir()] {
        std::fs::create_dir_all(d).unwrap_or_else(|e| panic!("create {}: {e}", d.display()));
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).ok();
    }
    let opts = SqliteConnectOptions::new()
        .filename(cfg.data_dir.join("state.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .expect("open state.db");
    sqlx::migrate!().run(&db).await.expect("migrations failed");
    db
}

async fn shutdown() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
}
