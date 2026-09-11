use rgb402_merchant::MerchantConfig;
use rgb402_payment::{SettlementVerifier, SimulatedRgbPaymentProvider};
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            env::var("RUST_LOG")
                .unwrap_or_else(|_| "merchant_server=info,rgb402_merchant=info".to_owned()),
        )
        .init();

    let bind_addr = env::var("RGB402_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_owned());
    let ledger_path =
        env::var("RGB402_SIM_LEDGER_PATH").unwrap_or_else(|_| ".rgb402-sim-ledger.json".to_owned());
    let addr: SocketAddr = bind_addr.parse()?;

    let settlement: Arc<dyn SettlementVerifier> =
        Arc::new(SimulatedRgbPaymentProvider::with_json_file(&ledger_path));
    let app = rgb402_merchant::router(MerchantConfig::premium_analysis_default(), settlement);

    println!("RGB402 merchant listening on http://{addr}");
    println!("Protected resource: http://{addr}/premium-analysis");
    println!("Simulated settlement ledger: {ledger_path}");
    println!("Future real-node backend target: Polar");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
