use axum::{
    routing::{get, post},
    http::StatusCode,
    Json, Router, extract::Path,
};
use serde::{Deserialize, Serialize};
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::{info_span, Span};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};


#[tokio::main]
async fn main() {
    // let _guard = ftlog::builder().try_init().unwrap();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // axum logs rejections from built-in extractors with the `axum::rejection`
                // target, at `TRACE` level. `axum::rejection=trace` enables showing those events
                format!(
                    "{}=debug,tower_http=debug,axum::rejection=trace",
                    env!("CARGO_CRATE_NAME")
                )
                .into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();



    // build our application with a route
    let app = Router::new()
        .route("/users/:uname/ping", post(user_ping))
        .layer(TraceLayer::new_for_http());

    let listener: tokio::net::TcpListener = tokio::net::TcpListener::bind("127.0.0.1:5000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}


async fn user_ping(
    // this argument tells axum to parse the request body
    // as JSON into a `CreateUser` type
    Path(uname): Path<String>,
) -> (StatusCode, Json<PingResult>) {
    ftlog::info!("got ping {}", uname);
    let ping_res = PingResult{ now_nanos: 0, trade_start_nanos: 0, balance: -1 };
    (StatusCode::CREATED, Json(ping_res))
}

// the input to our `create_user` handler
#[derive(Serialize)]
struct PingResult {
    pub now_nanos: i64,
    pub trade_start_nanos: i64,

    pub balance: i64,
}
