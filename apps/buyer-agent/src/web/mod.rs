//! Single local wallet session. No raw node API or model-controlled approval.
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rgb402_agent::wallet_agent::{
    AgentModel, OutcomeStatus, PlanView, ToolOutput, TurnOutcome, WalletAgent,
};
use rgb402_core::{
    wallet::{Asset, PolicyDecision},
    PaymentId,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Clone)]
pub struct WebConfig {
    pub merchant_enabled: bool,
    pub bind: std::net::SocketAddr,
    pub wallet_name: String,
    pub recipient_address: Option<String>,
}
impl Default for WebConfig {
    fn default() -> Self {
        Self {
            merchant_enabled: false,
            bind: "127.0.0.1:3030".parse().unwrap(),
            wallet_name: "RGB Wallet".into(),
            recipient_address: None,
        }
    }
}
impl WebConfig {
    pub fn from_env() -> Result<Self, &'static str> {
        let mut config = Self::parse(
            &std::env::var("WALLET_API_BIND").unwrap_or_else(|_| "127.0.0.1:3030".into()),
            std::env::var("WALLET_NAME").unwrap_or_else(|_| "RGB Wallet".into()),
        )?;
        config.merchant_enabled = std::env::var("MERCHANT_ENABLED").as_deref() == Ok("true");
        if let Ok(address) = std::env::var("WALLET_RECIPIENT_ADDRESS") {
            if address.is_empty()
                || address.len() > 318
                || !address.is_ascii()
                || address.chars().any(|c| c.is_control() || c.is_whitespace())
                || address.split('@').count() != 2
            {
                return Err("invalid WALLET_RECIPIENT_ADDRESS");
            }
            config.recipient_address = Some(address);
        }
        Ok(config)
    }
    pub fn parse(bind: &str, wallet_name: String) -> Result<Self, &'static str> {
        let bind: std::net::SocketAddr = bind.parse().map_err(|_| "invalid WALLET_API_BIND")?;
        if bind.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) || bind.port() == 0 {
            return Err("wallet API must bind to IPv4 loopback with a fixed port");
        }
        if wallet_name.is_empty()
            || wallet_name.len() > 64
            || wallet_name.chars().any(char::is_control)
        {
            return Err("invalid WALLET_NAME");
        }
        Ok(Self {
            merchant_enabled: false,
            bind,
            wallet_name,
            recipient_address: None,
        })
    }
    fn hosts(&self) -> [String; 2] {
        [
            format!("127.0.0.1:{}", self.bind.port()),
            format!("localhost:{}", self.bind.port()),
        ]
    }
    fn allows_origin(&self, origin: &str) -> bool {
        self.hosts()
            .iter()
            .any(|host| origin == format!("http://{host}"))
            || (self.bind.port() == 3030
                && ["http://localhost:5173", "http://127.0.0.1:5173"].contains(&origin))
    }
}
#[derive(Clone, Serialize)]
pub struct Holding {
    pub asset: Asset,
    pub outbound: String,
    pub onchain: String,
}
#[derive(Clone, Serialize)]
pub struct Activity {
    pub payment_hash: String,
    pub asset_id: String,
    pub amount: String,
    pub timestamp: u64,
    pub kind: &'static str,
    pub resource: Option<String>,
    pub auto_approved: Option<bool>,
    pub direction: &'static str,
    pub status: OutcomeStatus,
}
#[derive(Clone, Serialize)]
pub struct Event {
    pub id: u64,
    pub kind: &'static str,
    pub text: String,
}
#[derive(Clone, Serialize)]
pub struct Session {
    pub csrf: String,
    pub busy: bool,
    #[serde(skip)]
    pub direct_pending: bool,
    #[serde(serialize_with = "web_plan")]
    pub pending: Option<PlanView>,
    pub btc_pending: Option<rgb402_payment::btc::BtcPlanView>,
    pub machine_pending: Option<rgb402_payment::commerce::MachinePlanView>,
    pub machine_result: Option<rgb402_payment::commerce::PurchaseView>,
    pub events: Vec<Event>,
}
// JSON numbers cannot represent every u64 in JavaScript. Preserve exact plan amounts.
fn web_plan<S: serde::Serializer>(
    plan: &Option<PlanView>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut value = serde_json::to_value(plan).map_err(serde::ser::Error::custom)?;
    if let Some(plan) = plan {
        value["request"]["amount"] = serde_json::Value::String(plan.request.amount.to_string());
        value["request"]["carrier_msat"] =
            serde_json::Value::String(plan.request.carrier_msat.to_string());
    }
    value.serialize(serializer)
}
struct Inner<M> {
    config: WebConfig,
    merchant: Option<rgb402_merchant::store::Shared>,
    agent: AsyncMutex<WalletAgent<M>>,
    session: Mutex<Session>,
}
pub struct AppState<M>(Arc<Inner<M>>);
impl<M> Clone for AppState<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<M: AgentModel + Sync + 'static> AppState<M> {
    pub fn new(agent: WalletAgent<M>) -> std::io::Result<Self> {
        Self::with_config(agent, WebConfig::default())
    }
    pub fn with_config(agent: WalletAgent<M>, config: WebConfig) -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let csrf = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self(Arc::new(Inner {
            config,
            merchant: None,
            agent: AsyncMutex::new(agent),
            session: Mutex::new(Session {
                csrf,
                busy: false,
                direct_pending: false,
                pending: None,
                btc_pending: None,
                machine_pending: None,
                machine_result: None,
                events: vec![],
            }),
        })))
    }
    pub fn with_merchant(mut self, store: rgb402_merchant::store::Shared) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("unshared startup state")
            .merchant = Some(store);
        self
    }
    fn event(&self, kind: &'static str, text: String) {
        let mut session = self.0.session.lock().expect("session lock");
        let id = session.events.last().map_or(1, |e| e.id + 1);
        session.events.push(Event { id, kind, text });
        if session.events.len() > 100 {
            session.events.remove(0);
        }
    }
    fn observe(&self, result: &ToolOutput) {
        let (kind, text) = match result {
            ToolOutput::Merchant { data } => (
                "wallet",
                if let Some(products) = data.get("products").and_then(serde_json::Value::as_array) {
                    products
                        .iter()
                        .map(|p| {
                            format!(
                                "{} — {} base units",
                                p["name"].as_str().unwrap_or("Product"),
                                p["amount"].as_str().unwrap_or("?")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    format!(
                        "Order {} · {}",
                        data["order_id"].as_str().unwrap_or(""),
                        data["status"].as_str().unwrap_or("unavailable")
                    )
                },
            ),
            ToolOutput::BtcPlan { plan } => match &plan.policy {
                PolicyDecision::Deny { reason } => {
                    ("error", format!("BTC payment denied: {reason}"))
                }
                _ => (
                    "wallet",
                    format!(
                        "BTC payment prepared: {} sats to {}. Application approval required.",
                        plan.amount_sats, plan.recipient.identifier
                    ),
                ),
            },
            ToolOutput::BtcPayment {
                payment_hash,
                status,
            } => (
                "payment",
                format!("BTC payment {} · {}", status_name(status), payment_hash),
            ),
            ToolOutput::Machine { purchase } => {
                self.0.session.lock().expect("session lock").machine_result =
                    Some(purchase.as_ref().clone());
                (
                    "machine",
                    format!(
                        "Machine service · {} sats · {} · payment {}",
                        purchase.cost_sats,
                        purchase.resource_status,
                        purchase
                            .payment_status
                            .as_deref()
                            .unwrap_or("not submitted")
                    ),
                )
            }
            ToolOutput::Invoice { request } => (
                "wallet",
                format!(
                    "Invoice decoded · {} base units over RGB Lightning",
                    request.amount
                ),
            ),
            ToolOutput::Assets { .. } => ("wallet", "RGB assets identified".into()),
            ToolOutput::Balance { balance } => (
                "wallet",
                format!(
                    "Balance checked · {} base units available",
                    balance.offchain_outbound
                ),
            ),
            ToolOutput::Plan { plan } => match &plan.policy {
                PolicyDecision::Deny { reason } => ("error", format!("Payment denied: {reason}")),
                _ => (
                    "wallet",
                    "Payment prepared. Review the exact details before approving.".into(),
                ),
            },
            ToolOutput::Payment {
                payment_hash,
                status,
                ..
            } => (
                "payment",
                format!("Payment {} · {}", status_name(status), payment_hash),
            ),
            ToolOutput::Error { message, .. } => ("error", message.clone()),
        };
        self.event(kind, text);
    }
}
fn status_name(status: &OutcomeStatus) -> &'static str {
    match status {
        OutcomeStatus::Pending => "pending",
        OutcomeStatus::Settled => "settled",
        OutcomeStatus::Failed => "failed",
        OutcomeStatus::Uncertain => "uncertain",
    }
}
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({"error":message}))).into_response()
}

pub fn router<M: AgentModel + Sync + 'static>(state: AppState<M>) -> Router {
    Router::new()
        .route("/api/session", get(session::<M>))
        .route("/api/invoice", post(create_invoice::<M>))
        .route("/api/send/prepare", post(prepare_direct::<M>))
        .route("/api/wallet", get(wallet::<M>))
        .route(
            "/api/merchant",
            get(merchant::<M>).post(configure_merchant::<M>),
        )
        .route("/api/assets", get(assets::<M>))
        .route("/api/activity", get(activity::<M>))
        .route("/api/payments/:id", get(payment::<M>))
        .route("/api/agent/message", post(message::<M>))
        .route("/api/agent/conversation", post(new_conversation::<M>))
        .route("/api/approvals/:id/approve", post(approve::<M>))
        .route("/api/approvals/:id/reject", post(reject::<M>))
        .fallback(static_file)
        .layer(DefaultBodyLimit::max(20 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard::<M>))
        .with_state(state)
}
async fn guard<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
    request: Request,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !state.0.config.hosts().iter().any(|allowed| allowed == host) {
        return error(StatusCode::FORBIDDEN, "Local wallet host required");
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        if !origin
            .to_str()
            .is_ok_and(|value| state.0.config.allows_origin(value))
        {
            return error(StatusCode::FORBIDDEN, "Origin not allowed");
        }
    }
    if request
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
    {
        return error(StatusCode::FORBIDDEN, "Cross-site request rejected");
    }
    if request.method() != axum::http::Method::GET && request.method() != axum::http::Method::HEAD {
        let token = state.0.session.lock().expect("session lock").csrf.clone();
        if request
            .headers()
            .get("x-wallet-csrf")
            .and_then(|h| h.to_str().ok())
            != Some(token.as_str())
        {
            return error(
                StatusCode::FORBIDDEN,
                "Wallet session expired; reload before approving",
            );
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY,"default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    response
}
async fn session<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
) -> Json<Session> {
    Json(state.0.session.lock().expect("session lock").clone())
}
async fn wallet<M: AgentModel + Sync + 'static>(State(state): State<AppState<M>>) -> Response {
    let Ok(agent) = state.0.agent.try_lock() else {
        return error(StatusCode::CONFLICT, "Wallet is busy; refreshing shortly");
    };
    let Ok(assets) = agent.assets().await else {
        return error(
            StatusCode::BAD_GATEWAY,
            "RGB node unavailable; balances could not be refreshed",
        );
    };
    let mut holdings = vec![];
    for asset in assets {
        let Ok(balance) = agent.balance(&asset.asset_id).await else {
            return error(StatusCode::BAD_GATEWAY, "RGB balance unavailable");
        };
        holdings.push(Holding {
            asset,
            outbound: balance.offchain_outbound.to_string(),
            onchain: balance.onchain_spendable.to_string(),
        });
    }
    let merchant_enabled = match &state.0.merchant {
        Some(store) => store.lock().await.enabled(),
        None => false,
    };
    Json(serde_json::json!({"holdings":holdings,"sats":null,"network":"regtest","wallet_name":state.0.config.wallet_name,"recipient_address":state.0.config.recipient_address,
        "merchant_available":state.0.merchant.is_some(),"merchant_enabled":merchant_enabled,"commerce_enabled":agent.commerce_enabled(),"btc_policy":agent.btc_policy(),"btc_outbound_sats":agent.btc_balance().await.map(|n|n.to_string()),
        "policy":{"auto_approve_below":agent.policy_limits().auto_approve_below.to_string(),"max_single_payment":agent.policy_limits().max_single_payment.to_string(),"max_daily_spend":agent.policy_limits().max_daily_spend.to_string(),"max_carrier_msat":agent.policy_limits().max_carrier_msat.to_string()}})).into_response()
}
async fn merchant<M: AgentModel + Sync + 'static>(State(state): State<AppState<M>>) -> Response {
    let Some(store) = &state.0.merchant else {
        return error(
            StatusCode::NOT_FOUND,
            "Merchant setup requires a configured recipient address and listener",
        );
    };
    Json(store.lock().await.overview().await).into_response()
}
async fn configure_merchant<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
    Json(settings): Json<rgb402_core::merchant::MerchantSettings>,
) -> Response {
    let Some(store) = &state.0.merchant else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut store = store.lock().await;
    match store.configure(settings).await {
        Ok(()) => Json(store.overview().await).into_response(),
        Err(status) => error(status, "Store settings could not be saved. Check product IDs, positive integer prices and wallet assets."),
    }
}
async fn assets<M: AgentModel + Sync + 'static>(State(state): State<AppState<M>>) -> Response {
    let Ok(agent) = state.0.agent.try_lock() else {
        return error(StatusCode::CONFLICT, "Wallet is busy");
    };
    match agent.assets().await {
        Ok(assets) => Json(assets).into_response(),
        Err(_) => error(StatusCode::BAD_GATEWAY, "RGB assets unavailable"),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvoiceInput {
    asset_id: rgb402_core::AssetId,
    amount: String,
}
async fn create_invoice<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
    body: Bytes,
) -> Response {
    let Ok(input) = serde_json::from_slice::<InvoiceInput>(&body) else {
        return error(
            StatusCode::BAD_REQUEST,
            "Expected asset ID and integer base-unit amount string",
        );
    };
    let Ok(amount) = input.amount.parse::<u64>() else {
        return error(StatusCode::BAD_REQUEST, "Invalid integer amount");
    };
    if amount == 0 || !input.amount.bytes().all(|c| c.is_ascii_digit()) {
        return error(
            StatusCode::BAD_REQUEST,
            "Amount must be positive integer base units",
        );
    }
    let Ok(agent) = state.0.agent.try_lock() else {
        return error(StatusCode::CONFLICT, "Wallet busy");
    };
    let Ok(assets) = agent.assets().await else {
        return error(StatusCode::BAD_GATEWAY, "Asset lookup unavailable");
    };
    if !assets.iter().any(|a| a.asset_id == input.asset_id) {
        return error(StatusCode::BAD_REQUEST, "Unknown RGB asset");
    }
    let request = rgb402_payment::rgb::CreateInvoice {
        asset_id: input.asset_id,
        asset_amount: amount,
        amt_msat: 3_000_000,
        expiry_sec: 3600,
        description: None,
        description_hash: None,
    };
    match agent.create_invoice(&request).await {
        Ok(invoice) => Json(invoice).into_response(),
        Err(_) => error(
            StatusCode::BAD_GATEWAY,
            "Invoice creation failed; no automatic retry",
        ),
    }
}
async fn activity<M: AgentModel + Sync + 'static>(State(state): State<AppState<M>>) -> Response {
    let Ok(mut agent) = state.0.agent.try_lock() else {
        return error(StatusCode::CONFLICT, "Wallet is busy");
    };
    let mut entries = vec![];
    for (request, timestamp) in agent.payment_history().into_iter().rev().take(50) {
        let status = agent
            .payment_status(&request.payment_hash)
            .await
            .map(OutcomeStatus::from)
            .unwrap_or(OutcomeStatus::Uncertain);
        entries.push(Activity {
            payment_hash: request.payment_hash.to_string(),
            asset_id: request.asset_id.to_string(),
            amount: request.amount.to_string(),
            timestamp,
            direction: "sent",
            kind: "rgb_transfer",
            resource: None,
            auto_approved: None,
            status,
        });
    }
    for (hash, amount, timestamp, status) in agent.btc_activity().await {
        entries.push(Activity {
            payment_hash: hash.to_string(),
            asset_id: "BTC".into(),
            amount: amount.to_string(),
            timestamp,
            kind: "btc_transfer",
            resource: None,
            auto_approved: Some(false),
            direction: "sent",
            status,
        });
    }
    // Preserve journal-backed outgoing entries, including uncertain status. Merge by hash.
    let history = match agent.node_payments().await {
        Ok(history) => history,
        Err(_) => return error(StatusCode::BAD_GATEWAY, "Node payment history unavailable"),
    };
    for payment in history {
        let (asset, amount, kind) = match (payment.asset_id, payment.asset_amount) {
            (Some(asset), Some(amount)) => (asset.to_string(), amount, "rgb_transfer"),
            (None, None)
                if payment.inbound && payment.amt_msat.is_some_and(|n| n > 0 && n % 1000 == 0) =>
            {
                (
                    "BTC".into(),
                    payment.amt_msat.unwrap() / 1000,
                    "btc_transfer",
                )
            }
            _ => continue,
        };
        if entries
            .iter()
            .any(|entry| entry.payment_hash == payment.payment_hash.as_str())
        {
            continue;
        }
        entries.push(Activity {
            payment_hash: payment.payment_hash.to_string(),
            asset_id: asset.to_string(),
            amount: amount.to_string(),
            timestamp: payment.created_at,
            kind,
            resource: None,
            auto_approved: None,
            direction: if payment.inbound { "received" } else { "sent" },
            status: OutcomeStatus::from(rgb402_core::PaymentStatus::from(payment.status)),
        });
    }
    for entry in agent.machine_activity().await {
        entries.push(Activity {
            payment_hash: entry.payment_hash.to_string(),
            asset_id: "BTC".into(),
            amount: entry.amount_sats.to_string(),
            timestamp: entry.timestamp,
            direction: "sent",
            kind: "machine_purchase",
            resource: Some(entry.url),
            auto_approved: Some(entry.auto_approved),
            status: match entry.status.as_str() {
                "settled" => OutcomeStatus::Settled,
                "pending" => OutcomeStatus::Pending,
                "failed" => OutcomeStatus::Failed,
                _ => OutcomeStatus::Uncertain,
            },
        });
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    Json(entries).into_response()
}
async fn payment<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
    Path(id): Path<String>,
) -> Response {
    let Ok(hash) = PaymentId::new(id) else {
        return error(StatusCode::BAD_REQUEST, "Invalid payment identifier");
    };
    let Ok(agent) = state.0.agent.try_lock() else {
        return error(StatusCode::CONFLICT, "Wallet is busy");
    };
    match agent.payment_status(&hash).await {
        Ok(status) => {
            Json(serde_json::json!({"payment_hash":hash,"status":OutcomeStatus::from(status)}))
                .into_response()
        }
        Err(_) => error(
            StatusCode::BAD_GATEWAY,
            "Payment status unavailable; outcome is uncertain",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageInput {
    message: String,
}
async fn message<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
    body: Bytes,
) -> Response {
    let Ok(input) = serde_json::from_slice::<MessageInput>(&body) else {
        return error(StatusCode::BAD_REQUEST, "Expected a message only");
    };
    if input.message.trim().is_empty() || input.message.len() > 16_384 {
        return error(StatusCode::BAD_REQUEST, "Enter a message of at most 16 KiB");
    }
    {
        let mut s = state.0.session.lock().expect("session lock");
        if s.busy || s.pending.is_some() || s.machine_pending.is_some() || s.btc_pending.is_some() {
            return error(
                StatusCode::CONFLICT,
                "Finish the current payment review first",
            );
        }
        s.busy = true;
    }
    state.event("user", input.message.clone());
    launch(state, input.message, None, false);
    StatusCode::ACCEPTED.into_response()
}
async fn new_conversation<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
) -> Response {
    {
        let session = state.0.session.lock().expect("session lock");
        if session.busy
            || session.pending.is_some()
            || session.machine_pending.is_some()
            || session.btc_pending.is_some()
        {
            return error(
                StatusCode::CONFLICT,
                "Finish the current payment review first",
            );
        }
    }
    let Ok(mut agent) = state.0.agent.try_lock() else {
        return error(StatusCode::CONFLICT, "Wallet is busy; try again shortly");
    };
    if agent.start_new_conversation().is_err() {
        return error(
            StatusCode::CONFLICT,
            "Finish the current payment review first",
        );
    }
    let mut session = state.0.session.lock().expect("session lock");
    session.events.clear();
    session.machine_result = None;
    session.direct_pending = false;
    Json(session.clone()).into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectInput {
    invoice: String,
}
async fn prepare_direct<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
    body: Bytes,
) -> Response {
    let Ok(input) = serde_json::from_slice::<DirectInput>(&body) else {
        return error(StatusCode::BAD_REQUEST, "Expected an invoice only");
    };
    if input.invoice.trim().is_empty() || input.invoice.len() > 16_384 {
        return error(StatusCode::BAD_REQUEST, "Invalid invoice size");
    }
    {
        let mut s = state.0.session.lock().expect("session lock");
        if s.busy || s.pending.is_some() || s.machine_pending.is_some() || s.btc_pending.is_some() {
            return error(StatusCode::CONFLICT, "Finish the current review first");
        }
        s.busy = true;
    }
    tokio::spawn(async move {
        let mut agent = state.0.agent.lock().await;
        match agent.prepare_from_application(input.invoice.trim()).await {
            Ok(plan) => {
                state.observe(&ToolOutput::Plan { plan: plan.clone() });
                if !matches!(plan.policy, PolicyDecision::Deny { .. }) {
                    let mut s = state.0.session.lock().expect("session lock");
                    s.pending = Some(plan);
                    s.direct_pending = true;
                }
            }
            Err(_) => state.event(
                "error",
                "Invoice preparation failed. No payment was submitted.".into(),
            ),
        }
        state.0.session.lock().expect("session lock").busy = false;
    });
    StatusCode::ACCEPTED.into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
async fn approve<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    approval(state, id, body, true).await
}
async fn reject<M: AgentModel + Sync + 'static>(
    State(state): State<AppState<M>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    approval(state, id, body, false).await
}
async fn approval<M: AgentModel + Sync + 'static>(
    state: AppState<M>,
    id: String,
    body: Bytes,
    yes: bool,
) -> Response {
    if !body.is_empty() && serde_json::from_slice::<Empty>(&body).is_err() {
        return error(
            StatusCode::BAD_REQUEST,
            "Approval accepts plan identity only; payment details cannot be changed",
        );
    }
    let direct = {
        let mut s = state.0.session.lock().expect("session lock");
        if s.busy {
            return error(StatusCode::CONFLICT, "Wallet action already in progress");
        }
        if s.pending
            .as_ref()
            .map(|p| p.plan_id.as_str())
            .or_else(|| s.machine_pending.as_ref().map(|p| p.plan_id.as_str()))
            .or_else(|| s.btc_pending.as_ref().map(|p| p.plan_id.as_str()))
            != Some(id.as_str())
        {
            return error(
                StatusCode::NOT_FOUND,
                "Unknown, consumed or cancelled payment plan",
            );
        }
        s.busy = true;
        s.pending = None;
        s.machine_pending = None;
        s.btc_pending = None;
        let direct = s.direct_pending;
        s.direct_pending = false;
        direct
    };
    launch(state, String::new(), Some((id, yes)), direct);
    StatusCode::ACCEPTED.into_response()
}
fn launch<M: AgentModel + Sync + 'static>(
    state: AppState<M>,
    message: String,
    approval: Option<(String, bool)>,
    direct: bool,
) {
    tokio::spawn(async move {
        let mut agent = state.0.agent.lock().await;
        if let Some((id, yes)) = approval {
            if agent.confirm_from_human(&id, yes).is_err() {
                state.event(
                    "error",
                    "The payment plan is no longer available. Prepare again.".into(),
                );
                state.0.session.lock().expect("session lock").busy = false;
                return;
            }
            state.event(
                "approval",
                if yes {
                    "You approved this exact payment."
                } else {
                    "Payment cancelled. Nothing was submitted."
                }
                .into(),
            );
            if !yes {
                state.0.session.lock().expect("session lock").busy = false;
                return;
            }
        }
        if direct {
            match agent.execute_from_application().await {
                Ok(output) => state.observe(&output),
                Err(_) => state.event(
                    "error",
                    "Execution unavailable. Check activity before retrying.".into(),
                ),
            }
            state.0.session.lock().expect("session lock").busy = false;
            return;
        }
        state.event("progress", "Working with your wallet…".into());
        let observer = state.clone();
        let outcome = agent
            .turn_with_observer(&message, move |o| observer.observe(o))
            .await;
        match outcome {
            Ok(TurnOutcome::Reply(text)) => state.event("assistant", text),
            Ok(TurnOutcome::BtcPrepared(plan)) => {
                state.0.session.lock().expect("session lock").btc_pending = Some(plan);
            }
            Ok(TurnOutcome::MachinePrepared(plan)) => {
                state
                    .0
                    .session
                    .lock()
                    .expect("session lock")
                    .machine_pending = Some(plan);
            }
            Ok(TurnOutcome::Prepared(plan)) => {
                state.0.session.lock().expect("session lock").pending = Some(plan)
            }
            Ok(TurnOutcome::LimitReached) => state.event(
                "error",
                "The agent reached its eight-step limit. Check activity before trying again."
                    .into(),
            ),
            Err(e) => {
                tracing::warn!(event = "web_model_failed");
                state.event("error",format!("OpenAI unavailable: {e}. Check activity for any submitted payment; do not resubmit an uncertain payment."));
            }
        }
        state.0.session.lock().expect("session lock").busy = false;
    });
}
async fn static_file(request: Request) -> Response {
    let path = request.uri().path();
    if path.starts_with("/api/") {
        return error(StatusCode::NOT_FOUND, "Unknown wallet operation");
    }
    let relative = if path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };
    if relative.contains("..") || relative.starts_with('.') || relative.contains('\\') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../wallet-ui/dist");
    let Ok(root) = root.canonicalize() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Build the wallet UI with npm run build in apps/wallet-ui",
        );
    };
    let Ok(file) = root.join(relative).canonicalize() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !file.starts_with(&root) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mime = match file.extension().and_then(|s| s.to_str()) {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("webmanifest") => "application/manifest+json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    match std::fs::read(file) {
        Ok(bytes) => ([(header::CONTENT_TYPE, mime)], Body::from(bytes)).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
#[cfg(test)]
mod tests;
