use rgb402_core::{merchant::MerchantProfile, AssetId};
use rgb402_payment::rgb::RgbLightningClient;
use std::sync::Arc;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let domain = std::env::var("RECIPIENT_DOMAIN")?;
    if domain.is_empty()
        || !domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
    {
        return Err("invalid recipient domain".into());
    }
    let asset = AssetId::new(std::env::var("RECIPIENT_ASSET_ID")?)?;
    let profile = MerchantProfile {
        enabled: std::env::var("MERCHANT_ENABLED").as_deref() == Ok("true"),
        public_catalog: std::env::var("PUBLIC_CATALOG").as_deref() == Ok("true"),
        merchant_id: format!("carol@{domain}"),
        display_name: "Carol's Store".into(),
        accepted_assets: vec![asset.clone()],
        catalog: format!("https://{domain}/commerce/v1/catalog"),
        orders: format!("https://{domain}/commerce/v1/orders"),
    };
    let node = Arc::new(RgbLightningClient::new("http://127.0.0.1:3103", None)?);
    let store = rgb402_merchant::store::Store::open(
        profile,
        asset,
        node,
        ".var/regtest/carol-orders.jsonl".into(),
    )?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3051").await?;
    axum::serve(listener, rgb402_merchant::store::router(store))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
