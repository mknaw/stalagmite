use std::path::Path;
use std::time::Duration;

use axum::Router;
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer_opt;
use tower_http::services::ServeDir;

fn watch<P: AsRef<Path>>(path: P) -> notify::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();

    let backend_config = notify::Config::default().with_poll_interval(Duration::from_secs(1));
    // debouncer configuration
    let debouncer_config = notify_debouncer_mini::Config::default()
        .with_timeout(Duration::from_millis(1000))
        .with_notify_config(backend_config);
    // select backend via fish operator, here PollWatcher backend
    let mut debouncer = new_debouncer_opt::<_, notify::PollWatcher>(debouncer_config, tx).unwrap();

    debouncer
        .watcher()
        .watch(path.as_ref(), RecursiveMode::Recursive)
        .unwrap();

    for res in rx {
        match res {
            Ok(events) => {
                let should_regenerate = events
                    .iter()
                    // TODO may want to specify this path a bit better...
                    .any(|event| !event.path.starts_with("./public/"));
                if should_regenerate {
                    tracing::info!("regenerating...");
                    crate::generate();
                }
            }
            Err(error) => println!("Error: {error:?}"),
        }
    }

    Ok(())
}

pub async fn run() {
    tracing_subscriber::fmt::init();

    // build our application with a route
    let app = Router::new().nest_service("/", ServeDir::new("public"));

    tokio::spawn(async move {
        watch(".").unwrap();
    });

    // run our app with hyper, listening globally on port 3000
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
