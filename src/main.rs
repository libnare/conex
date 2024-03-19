use actix_web::{App, HttpServer, web};
use tracing::{info, Level};

use conex::{AppState, Bind};
use conex::handlers::config_routes;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    let state = AppState::new().await;

    let bind = Bind::default();
    let addr = format!("{}:{}", bind.host.unwrap(), bind.port.unwrap());
    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(config_routes)
    })
        .bind(&addr)?;

    info!("listening on {}", addr);
    server.run().await
}