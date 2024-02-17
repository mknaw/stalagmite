use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::response::{sse, Sse};
use axum::routing::get;
use axum::Router;
use futures::stream::Stream;
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer_opt;
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

use crate::{cache, Config};

// TODO this is async-colored from `generate`, but doesn't actually exploit async otherwise
// TODO could do better to only reload when the content of the current url has changed.
async fn watch<P: AsRef<Path>>(path: P, reload_tx: broadcast::Sender<()>) -> notify::Result<()> {
    let config = Arc::new(Config::init(None).map_or_else(|e| panic!("{}", e), |c| c));
    let (tx, rx) = std::sync::mpsc::channel();

    let backend_config = notify::Config::default().with_poll_interval(Duration::from_millis(100));
    let debouncer_config = notify_debouncer_mini::Config::default()
        .with_timeout(Duration::from_millis(100))
        .with_notify_config(backend_config);
    let mut debouncer = new_debouncer_opt::<_, notify::PollWatcher>(debouncer_config, tx).unwrap();

    debouncer
        .watcher()
        .watch(path.as_ref(), RecursiveMode::Recursive)
        .unwrap();

    let pool = Arc::new(cache::bootstrap().unwrap());

    for res in rx {
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
                    println!("got events: {:?}", events);
                    tracing::info!("regenerating...");
                    crate::generate(config, pool).await.unwrap();
                    reload_tx.send(()).ok();
                }
            }
            Err(error) => println!("Error: {error:?}"),
        }
    }

    Ok(())
}

pub async fn run() {
    let (reload_tx, _) = broadcast::channel(1);
    {
        let reload_tx = reload_tx.clone();
        tokio::spawn(async move {
            watch(".", reload_tx).await.unwrap();
        });
    }

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on {}", addr);
    let app = Router::new()
        .fallback_service(ServeDir::new("public"))
        .route(
            "/__dev_reload",
            get(move || {
                let tx = reload_tx.clone();
                async move { reload_signaler(tx).await }
            }),
        );
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn reload_signaler(
    reload_tx: broadcast::Sender<()>,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    // A `Stream` that repeats an event every second
    //
    // You can also create streams from tokio channels using the wrappers in
    // https://docs.rs/tokio-stream
    // let stream = stream::repeat_with(|| sse::Event::default().data("hi!"))
    //     .map(Ok)
    //     .throttle(Duration::from_secs(1));
    //
    let mut reload_rx = reload_tx.subscribe();

    let stream = async_stream::stream! {
        while reload_rx.recv().await.ok().is_some() {
            println!("sending reload event...");
            yield Ok(sse::Event::default().data("hi!"));
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(1))
            .text("keep-alive-text"),
    )
}
