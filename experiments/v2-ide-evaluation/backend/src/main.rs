mod app;
mod files;
mod protocol;

#[allow(dead_code)]
#[path = "../../../../crates/factory-api/src/security.rs"]
mod security;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let config = args
        .get(1)
        .ok_or("usage: asf-p01-ide-probe CONFIG_JSON [PORT]")?;
    let port: u16 = args.get(2).map(String::as_str).unwrap_or("0").parse()?;
    let state = app::load(config)?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!("P01_READY http://{}", listener.local_addr()?);
    let shutdown_state = state.clone();
    let app = app::router(state.clone());
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown_state.shutdown.notified().await })
        .await?;
    state.cleanup();
    Ok(())
}
