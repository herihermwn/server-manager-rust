//! Server-sent events for live logs. The browser side is `web/dashboard/lib/useEventStream.ts`.
//!
//! Events: `stdout` / `stderr` (data = one redacted line), `done` (data = `{"code":0}` or
//! `{"code":null,"reason":"timeout"|"cancelled"|"signal"}`), plus whatever a phase sends itself.

use crate::exec::{Cmd, Exit, Line, Stream};
use axum::response::{
    IntoResponse,
    sse::{Event, KeepAlive, Sse},
};
use std::convert::Infallible;
use tokio::sync::mpsc;
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

/// Wrap a channel of events as an SSE response (15 s keep-alive comments). The stream ends when
/// every sender is dropped.
pub fn from_channel(rx: mpsc::Receiver<Event>) -> impl IntoResponse {
    Sse::new(ReceiverStream::new(rx).map(Ok::<_, Infallible>)).keep_alive(KeepAlive::default())
}

pub fn line_event(l: &Line) -> Event {
    let name = if l.stream == Stream::Out { "stdout" } else { "stderr" };
    // axum panics on `\r` in data; exec already strips it, this keeps hand-made lines safe too.
    Event::default().event(name).data(l.text.replace('\r', ""))
}

pub fn done_event(exit: &std::io::Result<Exit>) -> Event {
    let data = match exit {
        Ok(Exit::Code(c)) => serde_json::json!({ "code": c }),
        Ok(Exit::TimedOut) => serde_json::json!({ "code": null, "reason": "timeout" }),
        Ok(Exit::Cancelled) => serde_json::json!({ "code": null, "reason": "cancelled" }),
        Ok(Exit::Signal(s)) => serde_json::json!({ "code": null, "reason": "signal", "signal": s }),
        Err(e) => serde_json::json!({ "code": null, "reason": "spawn", "error": e.to_string() }),
    };
    Event::default().event("done").data(data.to_string())
}

/// Run `cmd` and stream its output to the browser, ending with a `done` event.
/// The command keeps running if the browser disconnects (it is not tied to the request).
pub fn run_command(cmd: Cmd) -> impl IntoResponse {
    let (etx, erx) = mpsc::channel(256);
    tokio::spawn(async move {
        let (ltx, mut lrx) = mpsc::channel(256);
        let forward = async {
            while let Some(l) = lrx.recv().await {
                etx.send(line_event(&l)).await.ok();
            }
        };
        let (exit, ()) = tokio::join!(cmd.stream(ltx), forward);
        etx.send(done_event(&exit)).await.ok();
    });
    from_channel(erx)
}
