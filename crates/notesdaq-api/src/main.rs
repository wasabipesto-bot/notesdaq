mod routes;

use anyhow::Result;
use axum::Router;
use sqlx::PgPool;
use tower_http::cors::CorsLayer;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "notesdaq_api=info".parse().unwrap()),
        )
        .init();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or("postgres://notesdaq:notesdaq@localhost/notesdaq".into());

    info!("Connecting to database...");
    let pool = notesdaq_db::connect(&database_url).await?;

    let state = AppState { pool };

    let app = Router::new()
        .nest("/api/v1", routes::router())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind = std::env::var("BIND").unwrap_or("0.0.0.0:3001".into());
    info!("Listening on {bind}");
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
