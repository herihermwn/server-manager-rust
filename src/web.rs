//! Static frontend from `web/dist` (output of `web/build.sh`). Any path that is not a file gets
//! `index.html`; the SPA routes by URL hash. /api has its own JSON 404, so it never lands here.

use axum::http::{HeaderName, HeaderValue, header};
use std::path::Path;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

pub fn service(dist: &Path) -> ServeDir<ServeFile> {
    ServeDir::new(dist).fallback(ServeFile::new(dist.join("index.html")))
}

/// Headers on every response. This UI can run commands on the server, so: no framing, no sniffing,
/// no referrer, scripts only from our own origin.
pub fn security_headers() -> [SetResponseHeaderLayer<HeaderValue>; 4] {
    let h =
        |name: HeaderName, v: &'static str| SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(v));
    [
        h(header::X_FRAME_OPTIONS, "DENY"),
        h(header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        h(header::REFERRER_POLICY, "no-referrer"),
        h(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
             font-src https://fonts.gstatic.com; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    ]
}
