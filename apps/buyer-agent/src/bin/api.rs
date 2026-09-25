#[path = "../agent_config.rs"]
mod agent_config;
use buyer_agent::web::{router, AppState, WebConfig};
use rgb402_agent::wallet_agent::{
    openai::OpenAiModel, AgentModel, Message, ModelError, ModelResponse, ToolDefinition,
    WalletAgent,
};
use rgb402_payment::{config::WalletConfig, rgb::RgbLightningClient, wallet::WalletService};
use std::sync::Arc;
fn main() {
    if let Err(e) = launch() {
        eprintln!("Wallet API startup failed: {e}");
        std::process::exit(1);
    }
}
fn launch() -> Result<(), Box<dyn std::error::Error>> {
    agent_config::load_defaults(std::path::Path::new(".env"))?;
    agent_config::load_defaults(std::path::Path::new(".env.example"))?;
    tokio::runtime::Runtime::new()?.block_on(start())
}
async fn start() -> Result<(), Box<dyn std::error::Error>> {
    let model = match std::env::var("WALLET_AGENT_ENABLED").as_deref() {
        Ok("false") => ApiModel(None),
        Ok("true") | Err(std::env::VarError::NotPresent) => {
            ApiModel(Some(OpenAiModel::from_env()?))
        }
        _ => return Err("WALLET_AGENT_ENABLED must be true or false".into()),
    };
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
    let btc = rgb402_payment::btc::BtcService::from_env(node.clone(), &config.state_path)?;
    let mut agent = WalletAgent::new(model, wallet).with_btc(btc);
    if let Some(config) = rgb402_payment::commerce::CommerceConfig::from_env()? {
        agent = agent.with_commerce(rgb402_payment::commerce::CommerceService::open(
            node.clone(),
            config,
        )?);
    }
    let web_config = WebConfig::from_env()?;
    let bind = web_config.bind;
    let merchant = if let Ok(bind) = std::env::var("MERCHANT_BIND") {
        use rgb402_core::merchant::MerchantProfile;
        use rgb402_payment::rgb::RgbNode;
        let bind: std::net::SocketAddr = bind.parse()?;
        if bind.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) || bind.port() == 0 {
            return Err("merchant listener must bind to IPv4 loopback".into());
        }
        let identity = web_config
            .recipient_address
            .clone()
            .ok_or("merchant requires recipient identity")?;
        let (account, domain) = identity
            .split_once('@')
            .ok_or("invalid merchant identity")?;
        if !["alice", "bob", "carol"].contains(&account) {
            return Err("unsupported demo merchant identity".into());
        }
        let asset = node
            .list_assets()
            .await?
            .first()
            .ok_or("merchant requires a wallet RGB asset")?
            .asset_id
            .clone();
        let base = if account == "carol" {
            format!("https://{domain}/commerce/v1")
        } else {
            format!("https://{domain}/commerce/v1/wallets/{account}")
        };
        let profile = MerchantProfile {
            enabled: web_config.merchant_enabled,
            public_catalog: web_config.merchant_enabled,
            merchant_id: identity.clone(),
            display_name: format!("{}'s Store", account[..1].to_uppercase() + &account[1..]),
            accepted_assets: vec![asset.clone()],
            catalog: format!("{base}/catalog"),
            orders: format!("{base}/orders"),
        };
        let path = std::env::var("MERCHANT_STATE_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| config.state_path.with_extension("merchant-orders.jsonl"));
        let store = rgb402_merchant::store::Store::open(profile, asset, node, path)?;
        let shared = Arc::new(tokio::sync::Mutex::new(store));
        let listener = tokio::net::TcpListener::bind(bind).await?;
        Some((shared, listener))
    } else {
        None
    };
    let mut state = AppState::with_config(agent, web_config)?;
    if let Some((store, listener)) = merchant {
        state = state.with_merchant(store.clone());
        tokio::spawn(async move {
            if let Err(error) =
                axum::serve(listener, rgb402_merchant::store::public_router(store)).await
            {
                eprintln!("Merchant listener stopped: {error}");
            }
        });
    }
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

// Explicitly disabled mode supports deterministic wallet operations without credentials.
struct ApiModel(Option<OpenAiModel>);
#[async_trait::async_trait]
impl AgentModel for ApiModel {
    async fn respond(
        &mut self,
        conversation: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<ModelResponse, ModelError> {
        match &mut self.0 {
            Some(model) => model.respond(conversation, tools).await,
            None => Ok(ModelResponse::Text(
                "The AI agent is disabled. Use Send to pay an invoice, or Receive to create one."
                    .into(),
            )),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disabled_model_needs_no_credentials_or_network() {
        let response = ApiModel(None).respond(&[], &[]).await.unwrap();
        assert!(matches!(response, ModelResponse::Text(text) if text.contains("disabled")));
    }
}
