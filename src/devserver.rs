use axum::Router;
use tower_http::services::ServeDir;

pub async fn run() {
    tracing_subscriber::fmt::init();

    // build our application with a route
    let app = Router::new().nest_service("/", ServeDir::new("public"));

    // run our app with hyper, listening globally on port 3000
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
