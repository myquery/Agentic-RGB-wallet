use rgb402_merchant::l402::{router, Merchant};
use rgb402_payment::rgb::RgbLightningClient;
use std::{path::PathBuf, sync::Arc};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("MERCHANT_LIGHTNING_NODE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3102".into());
    let token = std::env::var("MERCHANT_LIGHTNING_NODE_TOKEN").ok();
    let bind = std::env::var("MERCHANT_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3040".into());
    let bind: std::net::SocketAddr = bind.parse().map_err(|_| "invalid merchant bind address")?;
    if !bind.ip().is_loopback() {
        return Err("local regtest merchant must bind to loopback".into());
    }
    let key = PathBuf::from(
        std::env::var("MERCHANT_KEY_PATH").unwrap_or_else(|_| ".merchant-key".into()),
    );
    let merchant = Merchant::open(
        Arc::new(RgbLightningClient::new(&url, token.as_deref())?),
        &key,
    )?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    eprintln!(
        "L402 demo merchant: http://{bind}/premium/report (3 sats); /premium/extended (50 sats)"
    );
    axum::serve(listener, router(merchant)).await?;
    Ok(())
}
