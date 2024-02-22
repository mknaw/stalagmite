use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{sse, IntoResponse, Sse};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer_opt, DebounceEventHandler, DebounceEventResult};
use tokio::sync::Notify;
use tower_http::services::ServeDir;

use crate::{cache, Config};

lazy_static! {
    static ref DEV_RELOAD_SCRIPT: Vec<u8> = {
        let mut vec = Vec::new();
        vec.extend_from_slice(b"\n<script>\n");
        vec.extend_from_slice(include_bytes!("../assets/dev-reload.js"));
        vec.extend_from_slice(b"\n</script>");
        vec
    };
}

/// Allow using an async channel in the notifier debouncer.
struct AsyncishEventHandler {
    tx: tokio::sync::mpsc::Sender<DebounceEventResult>,
}

impl DebounceEventHandler for AsyncishEventHandler {
    /// Handles an event.
    fn handle_event(&mut self, event: DebounceEventResult) {
        futures::executor::block_on(async {
            self.tx.send(event).await.unwrap();
        });
    }
}

/// Watch for changes in the filesystem.
async fn watch(config: Arc<Config>, notify: Arc<Notify>) -> notify::Result<()> {
    // TODO this is async-colored from `generate`, but doesn't actually exploit async otherwise
    // TODO could do better to only reload when the content of the current url has changed.
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    let backend_config = notify::Config::default().with_poll_interval(Duration::from_millis(100));
    let debouncer_config = notify_debouncer_mini::Config::default()
        .with_timeout(Duration::from_millis(100))
        .with_notify_config(backend_config);
    let event_handler = AsyncishEventHandler { tx: tx.clone() };
    let mut debouncer =
        new_debouncer_opt::<_, notify::PollWatcher>(debouncer_config, event_handler).unwrap();

    let path = Path::new(".");
    debouncer
        .watcher()
        .watch(path, RecursiveMode::Recursive)
        .unwrap();

    let pool = Arc::new(cache::bootstrap().unwrap());
    while let Some(res) = rx.recv().await {
        let pool = pool.clone();
        let config = config.clone();
        match res {
            Ok(events) => {
                let should_regenerate = events
                    .iter()
                    // TODO may want to specify this path a bit better...
                    .any(|event| {
                        event.path.is_file()
                            && ["./pages", "./layouts", "./blocks", "./assets"]
                                .iter()
                                .any(|&folder| event.path.starts_with(folder))
                    });
                // TODO should just use events as an input instead of collecting everything.
                if should_regenerate {
                    tracing::info!("regenerating...");
                    crate::generate(config, pool).await.unwrap();
                    notify.notify_one();
                }
            }
            Err(error) => println!("Error: {error:?}"),
        }
    }

    Ok(())
}

/// Emit a SSE when the files have changed.
async fn signal_reload(
    notify: Arc<Notify>,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let stream = async_stream::stream! {
        loop {
            notify.notified().await;
            tracing::debug!("sending reload SSE");
            yield Ok(sse::Event::default().data("x"));
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(1))
            .text("keep-alive-text"),
    )
}

/// Append a script that reloads the page when the server sends a message.
async fn append_reload_script(request: Request, next: Next) -> impl IntoResponse {
    let response = next.run(request).await;
    // Nothing too important in the `parts` for our purposes.
    let (_, body) = response.into_parts();
    let body_stream = body.into_data_stream();
    let append_stream =
        futures::stream::iter(std::iter::once(Ok(Bytes::from_static(&DEV_RELOAD_SCRIPT))));
    axum::body::Body::from_stream(body_stream.chain(append_stream))
}

/// Watch for changes & emit SSEs when they happen.
fn reloader<S>(config: Arc<Config>) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let notify = Arc::new(Notify::new());
    {
        let notify = notify.clone();
        tokio::spawn(async move {
            watch(config, notify).await.unwrap();
        });
    }

    Router::new().route(
        "/__dev_reload",
        get(move || {
            let tx = notify.clone();
            async move { signal_reload(tx).await }
        }),
    )
}

/// Run the static-file server.
pub async fn run(config: Arc<Config>, dev_mode: bool) {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("starting server on {}, dev_mode={:?}", addr, dev_mode);

    let app = Router::new().nest_service("/", ServeDir::new("public"));

    let app = if dev_mode {
        app.layer(axum::middleware::from_fn(append_reload_script))
            .merge(reloader(config))
    } else {
        app
    };

    axum::serve(listener, app).await.unwrap();
}
