#[path = "../agent_config.rs"]
mod agent_config;
use buyer_agent::web::{router, AppState, WebConfig};
use rgb402_agent::wallet_agent::{openai::OpenAiModel, WalletAgent};
use rgb402_payment::{config::WalletConfig, rgb::RgbLightningClient, wallet::WalletService};
use std::sync::Arc;
fn main() {
    if let Err(e) = launch() {
        eprintln!("Wallet API startup failed: {e}");
        std::process::exit(1);
    }
}
fn launch() -> Result<(), Box<dyn std::error::Error>> {
    agent_config::load_defaults(std::path::Path::new(".env.example"))?;
    tokio::runtime::Runtime::new()?.block_on(start())
}
async fn start() -> Result<(), Box<dyn std::error::Error>> {
    let model = OpenAiModel::from_env()?;
    let config = WalletConfig::from_env()?;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("rgb402_agent=info,rgb402_payment=info,buyer_agent=info")
        .init();
    let node = Arc::new(RgbLightningClient::new(
        &config.node_url,
        config.node_token.as_deref(),
    )?);
    let wallet = WalletService::open(node.clone(), config.policy, &config.state_path)?;
    let mut agent = WalletAgent::new(model, wallet);
    if let Some(config) = rgb402_payment::commerce::CommerceConfig::from_env()? {
        agent = agent.with_commerce(rgb402_payment::commerce::CommerceService::open(
            node, config,
        )?);
    }
    let web_config = WebConfig::from_env()?;
    let bind = web_config.bind;
    let state = AppState::with_config(agent, web_config)?;
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    eprintln!("Wallet PWA/API: http://{bind} (local session only)");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
