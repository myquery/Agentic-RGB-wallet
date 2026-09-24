use super::*;
use async_trait::async_trait;
use axum::{body::to_bytes, http::Request};
use rgb402_agent::wallet_agent::{Message, ModelError, ModelResponse, ToolCall, ToolDefinition};
use rgb402_core::{
    wallet::{PaymentRequest, WalletBalance, WalletPolicy},
    AssetId, PaymentStatus,
};
use rgb402_payment::{
    rgb::RgbNode,
    wallet::{ApprovedPayment, PaymentResult, WalletError, WalletService},
};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tower::ServiceExt;
struct Model(VecDeque<ModelResponse>, Arc<AtomicU64>);
#[async_trait]
impl AgentModel for Model {
    async fn respond(
        &mut self,
        _: &[Message],
        _: &[ToolDefinition],
    ) -> Result<ModelResponse, ModelError> {
        self.1.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .0
            .pop_front()
            .unwrap_or(ModelResponse::Text("Check your wallet result.".into())))
    }
}
struct Node {
    fail_send: AtomicBool,
    sends: AtomicU64,
    amount: AtomicU64,
}
#[async_trait]
impl RgbNode for Node {
    async fn create_invoice(
        &self,
        _: &rgb402_payment::rgb::CreateInvoice,
    ) -> Result<rgb402_payment::rgb::CreatedInvoice, WalletError> {
        Ok(rgb402_payment::rgb::CreatedInvoice {
            invoice: "test-invoice".into(),
        })
    }

    async fn list_payments(&self) -> Result<Vec<rgb402_payment::rgb::NodePayment>, WalletError> {
        Ok(vec![serde_json::from_value(serde_json::json!({"payment_hash":"incoming","inbound":true,"status":"Succeeded","asset_id":"rgb:demo","asset_amount":5,"amt_msat":3000000,"created_at":1,"updated_at":2,"preimage":"must-not-leak"})).unwrap()])
    }

    async fn list_assets(&self) -> Result<Vec<Asset>, WalletError> {
        Ok(vec![Asset {
            asset_id: AssetId::new("rgb:demo")?,
            name: "Demo".into(),
            ticker: "R402USD".into(),
            precision: 0,
        }])
    }
    async fn asset_balance(&self, id: &AssetId) -> Result<WalletBalance, WalletError> {
        Ok(WalletBalance {
            asset_id: id.clone(),
            onchain_spendable: 400,
            offchain_outbound: 490,
        })
    }
    async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError> {
        Ok(PaymentRequest {
            asset_id: AssetId::new("rgb:demo")?,
            amount: self.amount.load(Ordering::SeqCst),
            invoice: invoice.into(),
            payment_hash: PaymentId::new("hash")?,
            expires_at: u64::MAX,
            network: "Regtest".into(),
            carrier_msat: 3000000,
        })
    }
    async fn send_payment(&self, p: &ApprovedPayment) -> Result<PaymentResult, WalletError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.fail_send.load(Ordering::SeqCst) {
            return Err(WalletError::Node("submission outcome unknown"));
        }
        Ok(PaymentResult {
            payment_id: p.request().payment_hash.clone(),
            payment_hash: p.request().payment_hash.clone(),
            status: PaymentStatus::Pending,
        })
    }
    async fn payment_status(&self, _: &PaymentId) -> Result<PaymentStatus, WalletError> {
        Ok(PaymentStatus::Settled)
    }
}
fn call(name: &str, args: serde_json::Value) -> ModelResponse {
    ModelResponse::Tool(ToolCall {
        id: "call".into(),
        name: name.into(),
        arguments: args.to_string(),
    })
}
static COUNT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    state: AppState<Model>,
    app: Router,
    node: Arc<Node>,
    model_calls: Arc<AtomicU64>,
    path: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn fixture() -> Fixture {
    let node = Arc::new(Node {
        fail_send: AtomicBool::new(false),
        sends: AtomicU64::new(0),
        amount: AtomicU64::new(5),
    });
    let path = std::env::temp_dir().join(format!(
        "rgb-web-{}-{}.jsonl",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::SeqCst)
    ));
    let wallet = WalletService::open(
        node.clone(),
        WalletPolicy {
            allowed_assets: [AssetId::new("rgb:demo").unwrap()].into(),
            auto_approve_below: 1,
            max_single_payment: 100,
            max_daily_spend: 500,
            max_carrier_msat: 3000000,
        },
        &path,
    )
    .unwrap();
    let model_calls = Arc::new(AtomicU64::new(0));
    let model = Model(
        VecDeque::from([
            call(
                "wallet_prepare_payment",
                serde_json::json!({"invoice":"invoice"}),
            ),
            call(
                "wallet_execute_payment",
                serde_json::json!({"plan_id":"hash-1"}),
            ),
        ]),
        model_calls.clone(),
    );
    let state = AppState::new(WalletAgent::new(model, wallet)).unwrap();
    Fixture {
        app: router(state.clone()),
        state,
        node,
        model_calls,
        path,
    }
}
async fn send(f: &Fixture, method: &str, url: &str, body: &str, csrf: bool) -> Response {
    let token = f.state.0.session.lock().unwrap().csrf.clone();
    let mut req = Request::builder()
        .method(method)
        .uri(url)
        .header("host", "127.0.0.1:3030")
        .header("origin", "http://127.0.0.1:3030")
        .header("content-type", "application/json");
    if csrf {
        req = req.header("x-wallet-csrf", token);
    }
    f.app
        .clone()
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
}
async fn idle(f: &Fixture) {
    for _ in 0..10000 {
        if !f.state.0.session.lock().unwrap().busy {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("agent did not finish");
}
async fn prepare(f: &Fixture) {
    assert_eq!(
        send(
            f,
            "POST",
            "/api/agent/message",
            r#"{"message":"Pay invoice"}"#,
            true
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    idle(f).await;
    assert!(f.state.0.session.lock().unwrap().pending.is_some());
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn approval_rejects_unknown_plan_and_injected_details() {
    let f = fixture();
    assert_eq!(
        send(&f, "POST", "/api/approvals/missing/approve", "{}", true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    prepare(&f).await;
    for body in [
        r#"{"amount":50}"#,
        r#"{"invoice":"other"}"#,
        r#"{"asset_id":"other","approval":true}"#,
    ] {
        assert_eq!(
            send(&f, "POST", "/api/approvals/hash-1/approve", body, true)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn exact_approval_executes_once_and_activity_is_authoritative() {
    let f = fixture();
    prepare(&f).await;
    assert_eq!(
        send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true)
            .await
            .status(),
        StatusCode::ACCEPTED
    );
    idle(&f).await;
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let body = to_bytes(
        send(&f, "GET", "/api/activity", "", false)
            .await
            .into_body(),
        16384,
    )
    .await
    .unwrap();
    let activity: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(activity[0]["status"], "settled");
    assert_eq!(activity[0]["amount"], "5");
}
#[tokio::test]
async fn reject_consumes_prompt_and_does_not_submit() {
    let f = fixture();
    prepare(&f).await;
    assert_eq!(
        send(&f, "POST", "/api/approvals/hash-1/reject", "{}", true)
            .await
            .status(),
        StatusCode::ACCEPTED
    );
    idle(&f).await;
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn wallet_rechecks_invoice_after_web_approval() {
    let f = fixture();
    prepare(&f).await;
    f.node.amount.store(50, Ordering::SeqCst);
    send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true).await;
    idle(&f).await;
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    assert!(f
        .state
        .0
        .session
        .lock()
        .unwrap()
        .events
        .iter()
        .any(|e| e.kind == "error"));
}
#[tokio::test]
async fn csrf_origin_and_host_are_required() {
    let f = fixture();
    assert_eq!(
        send(
            &f,
            "POST",
            "/api/agent/message",
            r#"{"message":"pay"}"#,
            false
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    for (host, origin) in [
        ("attacker.test:3030", "http://127.0.0.1:3030"),
        ("127.0.0.1:3030", "https://attacker.test"),
    ] {
        let req = Request::builder()
            .uri("/api/session")
            .header("host", host)
            .header("origin", origin)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            f.app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}
#[tokio::test]
async fn wallet_response_contains_balances_not_credentials() {
    let f = fixture();
    let response = send(&f, "GET", "/api/wallet", "", false).await;
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(response.into_body(), 16384).await.unwrap();
    let data: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data["holdings"][0]["outbound"], "490");
    assert!(data["sats"].is_null());
    assert!(!String::from_utf8_lossy(&body).contains("token"));
}

#[test]
fn wallet_instances_have_distinct_origins() {
    let alice = WebConfig::parse("127.0.0.1:3030", "Alice Wallet".into()).unwrap();
    let bob = WebConfig::parse("127.0.0.1:3031", "Bob Wallet".into()).unwrap();
    assert!(alice.allows_origin("http://127.0.0.1:3030"));
    assert!(!bob.allows_origin("http://127.0.0.1:3030"));
    assert!(bob.allows_origin("http://127.0.0.1:3031"));
    assert!(!bob.allows_origin("http://localhost:5173"));
    assert!(WebConfig::parse("0.0.0.0:3031", "Bob".into()).is_err());
}
#[tokio::test]
async fn receive_validates_input_without_payment_authority() {
    let f = fixture();
    for body in [
        r#"{"asset_id":"rgb:demo","amount":"0"}"#,
        r#"{"asset_id":"rgb:demo","amount":"5.5"}"#,
        r#"{"asset_id":"rgb:demo","amount":5}"#,
        r#"{"asset_id":"rgb:other","amount":"5"}"#,
        r#"{"asset_id":"rgb:demo","amount":"5","approve":true}"#,
    ] {
        assert_eq!(
            send(&f, "POST", "/api/invoice", body, true).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        send(
            &f,
            "POST",
            "/api/invoice",
            r#"{"asset_id":"rgb:demo","amount":"5"}"#,
            false
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &f,
            "POST",
            "/api/invoice",
            r#"{"asset_id":"rgb:demo","amount":"5"}"#,
            true
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    assert!(!f.state.0.session.lock().unwrap().busy);
    assert!(f.state.0.session.lock().unwrap().pending.is_none());
}

#[tokio::test]
async fn native_inbound_activity_is_sanitized_and_directional() {
    let f = fixture();
    let response = send(&f, "GET", "/api/activity", "", false).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 8192).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value[0]["direction"], "received");
    assert_eq!(value[0]["status"], "settled");
    assert_eq!(value[0]["payment_hash"], "incoming");
    assert!(!String::from_utf8(body.to_vec())
        .unwrap()
        .contains("must-not-leak"));
}

#[test]
fn independent_wallet_states_and_sessions_do_not_share_authority() {
    let alice = fixture();
    let bob = fixture();
    assert_ne!(alice.path, bob.path);
    assert_ne!(
        alice.state.0.session.lock().unwrap().csrf,
        bob.state.0.session.lock().unwrap().csrf
    );
    assert!(alice.state.0.session.lock().unwrap().pending.is_none());
    assert!(bob.state.0.session.lock().unwrap().pending.is_none());
}

#[tokio::test]
async fn approval_continuation_keeps_bound_plan_and_rejects_duplicate_click() {
    let f = fixture();
    prepare(&f).await;
    // Wrong approval cannot consume the valid pending plan.
    f.state
        .0
        .agent
        .lock()
        .await
        .confirm_from_human("wrong-plan", true)
        .unwrap_err();
    assert_eq!(
        send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true)
            .await
            .status(),
        StatusCode::ACCEPTED
    );
    idle(&f).await;
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
}

async fn prepare_direct_test(f: &Fixture) {
    assert_eq!(
        send(
            f,
            "POST",
            "/api/send/prepare",
            r#"{"invoice":"invoice"}"#,
            true
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    idle(f).await;
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    assert_eq!(f.model_calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn direct_send_uses_no_model_and_approval_executes_once() {
    let f = fixture();
    prepare_direct_test(&f).await;
    assert!(f.state.0.session.lock().unwrap().direct_pending);
    assert_eq!(
        send(&f, "POST", "/api/approvals/wrong/approve", "{}", true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true)
            .await
            .status(),
        StatusCode::ACCEPTED
    );
    idle(&f).await;
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert_eq!(f.model_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn direct_cancel_and_changed_invoice_never_submit() {
    for cancel in [true, false] {
        let f = fixture();
        prepare_direct_test(&f).await;
        let action = if cancel {
            "reject"
        } else {
            f.node.amount.store(50, Ordering::SeqCst);
            "approve"
        };
        send(
            &f,
            "POST",
            &format!("/api/approvals/hash-1/{action}"),
            "{}",
            true,
        )
        .await;
        idle(&f).await;
        assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
        assert_eq!(f.model_calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn direct_prepare_rejects_injection_and_denied_policy() {
    let f = fixture();
    for body in [
        r#"{"invoice":""}"#,
        r#"{"invoice":"invoice","approve":true}"#,
    ] {
        assert_eq!(
            send(&f, "POST", "/api/send/prepare", body, true)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        send(
            &f,
            "POST",
            "/api/send/prepare",
            r#"{"invoice":"invoice"}"#,
            false
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    f.node.amount.store(101, Ordering::SeqCst);
    prepare_direct_test(&f).await;
    assert!(f.state.0.session.lock().unwrap().pending.is_none());
}

#[tokio::test]
async fn direct_uncertain_submission_preserves_reservation_and_blocks_retry() {
    let f = fixture();
    f.node.fail_send.store(true, Ordering::SeqCst);
    prepare_direct_test(&f).await;
    send(&f, "POST", "/api/approvals/hash-1/approve", "{}", true).await;
    idle(&f).await;
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert!(!std::fs::read_to_string(&f.path).unwrap().is_empty());
    send(
        &f,
        "POST",
        "/api/send/prepare",
        r#"{"invoice":"invoice"}"#,
        true,
    )
    .await;
    idle(&f).await;
    let retry_id = f
        .state
        .0
        .session
        .lock()
        .unwrap()
        .pending
        .as_ref()
        .unwrap()
        .plan_id
        .clone();
    send(
        &f,
        "POST",
        &format!("/api/approvals/{retry_id}/approve"),
        "{}",
        true,
    )
    .await;
    idle(&f).await;
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert_eq!(f.model_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn merchant_settings_require_owner_csrf_and_public_router_cannot_edit() {
    use rgb402_core::merchant::MerchantProfile;
    let mut f = fixture();
    f.app = Router::new();
    let path = f.path.with_extension("store.jsonl");
    let asset = AssetId::new("rgb:demo").unwrap();
    let store = rgb402_merchant::store::Store::open(
        MerchantProfile {
            enabled: false,
            public_catalog: false,
            merchant_id: "bob@example.com".into(),
            display_name: "Bob".into(),
            accepted_assets: vec![asset.clone()],
            catalog: "https://example.com/commerce/v1/wallets/bob/catalog".into(),
            orders: "https://example.com/commerce/v1/wallets/bob/orders".into(),
        },
        asset,
        f.node.clone(),
        path.clone(),
    )
    .unwrap();
    let store = Arc::new(AsyncMutex::new(store));
    Arc::get_mut(&mut f.state.0).unwrap().merchant = Some(store.clone());
    f.app = router(f.state.clone());
    let body = r#"{"enabled":true,"public_catalog":true,"display_name":"Bob's Store","accepted_assets":["rgb:demo"],"products":[{"id":"tea","name":"Tea","amount":"4","asset_id":"rgb:demo","available":true}]}"#;
    assert_eq!(
        send(&f, "POST", "/api/merchant", body, false)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let public = rgb402_merchant::store::public_router(store.clone());
    for route in ["/api/merchant", "/internal/merchant"] {
        let response = public
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(route)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    assert_eq!(
        send(&f, "POST", "/api/merchant", body, true).await.status(),
        StatusCode::OK
    );
    let response = public
        .oneshot(
            Request::builder()
                .uri("/commerce/v1/catalog")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let data: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    assert_eq!(data[0]["name"], "Tea");
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    assert_eq!(f.model_calls.load(Ordering::SeqCst), 0);
    drop(store);
    drop(f);
    std::fs::remove_file(path.with_extension("settings.json")).unwrap();
    std::fs::remove_file(path).unwrap();
}
