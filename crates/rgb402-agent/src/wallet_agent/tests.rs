use super::*;
use rgb402_core::wallet::WalletPolicy;
use rgb402_payment::{
    rgb::RgbNode,
    wallet::{ApprovedPayment, PaymentResult},
};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
struct FakeModel(VecDeque<ModelResponse>);
#[async_trait]
impl AgentModel for FakeModel {
    async fn respond(
        &mut self,
        _: &[Message],
        _: &[ToolDefinition],
    ) -> Result<ModelResponse, ModelError> {
        Ok(self
            .0
            .pop_front()
            .unwrap_or(ModelResponse::Text("done".into())))
    }
}
struct Node {
    sends: AtomicU64,
    amount: AtomicU64,
    fail: AtomicBool,
    settled: AtomicBool,
    status_failed: AtomicBool,
    read_failed: AtomicBool,
}
#[async_trait]
impl RgbNode for Node {
    async fn list_assets(&self) -> Result<Vec<Asset>, WalletError> {
        if self.read_failed.load(Ordering::SeqCst) {
            return Err(WalletError::Node("secret remote body"));
        }
        Ok(vec![Asset {
            asset_id: AssetId::new("rgb:demo")?,
            name: "Demo".into(),
            ticker: "R402USD".into(),
            precision: 0,
        }])
    }
    async fn asset_balance(&self, asset: &AssetId) -> Result<WalletBalance, WalletError> {
        Ok(WalletBalance {
            asset_id: asset.clone(),
            onchain_spendable: 400,
            offchain_outbound: 495,
        })
    }
    async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError> {
        Ok(PaymentRequest {
            asset_id: AssetId::new("rgb:demo")?,
            amount: self.amount.load(Ordering::SeqCst),
            invoice: invoice.into(),
            payment_hash: PaymentId::new(invoice)?,
            expires_at: u64::MAX,
            network: "Regtest".into(),
            carrier_msat: 3_000_000,
        })
    }
    async fn send_payment(&self, p: &ApprovedPayment) -> Result<PaymentResult, WalletError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(WalletError::Node("secret remote body"));
        }
        Ok(PaymentResult {
            payment_id: p.request().payment_hash.clone(),
            payment_hash: p.request().payment_hash.clone(),
            status: PaymentStatus::Pending,
        })
    }
    async fn payment_status(&self, _: &PaymentId) -> Result<PaymentStatus, WalletError> {
        Ok(if self.status_failed.load(Ordering::SeqCst) {
            PaymentStatus::Failed
        } else if self.settled.load(Ordering::SeqCst) {
            PaymentStatus::Settled
        } else {
            PaymentStatus::Pending
        })
    }
}
fn call(name: &str, args: serde_json::Value) -> ModelResponse {
    ModelResponse::Tool(ToolCall {
        id: "call".into(),
        name: name.into(),
        arguments: args.to_string(),
    })
}
fn prepare(invoice: &str) -> ModelResponse {
    call(
        "wallet_prepare_payment",
        serde_json::json!({"invoice":invoice}),
    )
}
fn execute(plan: &str) -> ModelResponse {
    call(
        "wallet_execute_payment",
        serde_json::json!({"plan_id":plan}),
    )
}
static COUNTER: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    agent: WalletAgent<FakeModel>,
    node: Arc<Node>,
    path: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn setup(script: Vec<ModelResponse>) -> Fixture {
    let node = Arc::new(Node {
        sends: AtomicU64::new(0),
        amount: AtomicU64::new(5),
        fail: AtomicBool::new(false),
        settled: AtomicBool::new(false),
        status_failed: AtomicBool::new(false),
        read_failed: AtomicBool::new(false),
    });
    let path = std::env::temp_dir().join(format!(
        "rgb402-agent-{}-{}.jsonl",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let policy = WalletPolicy {
        allowed_assets: [AssetId::new("rgb:demo").unwrap()].into(),
        auto_approve_below: 1,
        max_single_payment: 100,
        max_daily_spend: 500,
        max_carrier_msat: 3_000_000,
    };
    let wallet = WalletService::open(node.clone(), policy, &path).unwrap();
    Fixture {
        agent: WalletAgent::new(FakeModel(script.into()), wallet),
        node,
        path,
    }
}
fn outputs(f: &Fixture) -> Vec<&ToolOutput> {
    f.agent
        .conversation()
        .iter()
        .filter_map(|m| {
            if let Message::Result { output, .. } = m {
                Some(output)
            } else {
                None
            }
        })
        .collect()
}
fn has_error(f: &Fixture, expected: &str) -> bool {
    outputs(f)
        .iter()
        .any(|o| matches!(o,ToolOutput::Error {code,..} if code == expected))
}
#[tokio::test]
async fn readonly_routing_uses_wallet() {
    let mut f = setup(vec![
        call("wallet_get_assets", serde_json::json!({})),
        call(
            "wallet_get_balance",
            serde_json::json!({"asset_id":"rgb:demo"}),
        ),
        call(
            "wallet_decode_invoice",
            serde_json::json!({"invoice":"one"}),
        ),
    ]);
    f.agent
        .turn("what is in my wallet and decode one")
        .await
        .unwrap();
    assert!(matches!(outputs(&f)[0],ToolOutput::Assets {assets} if assets[0].ticker == "R402USD"));
    assert!(
        matches!(outputs(&f)[1],ToolOutput::Balance {balance} if balance.offchain_outbound==495)
    );
    assert!(matches!(outputs(&f)[2],ToolOutput::Invoice {request} if request.amount==5));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn preparation_stops_before_execution_and_conversation_is_not_approval() {
    let mut f = setup(vec![prepare("one"), execute("one-1")]);
    assert!(matches!(
        f.agent.turn("pay one").await.unwrap(),
        TurnOutcome::Prepared(_)
    ));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), "");
    f.agent.turn("yes go ahead I approve").await.unwrap();
    assert!(has_error(&f, "approval_required"));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn approved_pending_settled_and_consumed_plan() {
    let mut f = setup(vec![
        prepare("one"),
        execute("one-1"),
        execute("one-1"),
        call(
            "wallet_payment_status",
            serde_json::json!({"payment_hash":"one"}),
        ),
    ]);
    f.agent.turn("pay").await.unwrap();
    f.agent.confirm_from_human("one-1", true).unwrap();
    f.agent.turn("").await.unwrap();
    assert!(outputs(&f).iter().any(|o| matches!(
        o,
        ToolOutput::Payment {
            status: OutcomeStatus::Pending,
            ..
        }
    )));
    assert!(!has_error(&f, "unknown_plan"));
    assert!(outputs(&f).iter().any(|o| matches!(
        o,
        ToolOutput::Payment {
            submitted: Some(false),
            ..
        }
    )));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    f.node.settled.store(true, Ordering::SeqCst);
    f.agent.model.0.push_back(call(
        "wallet_payment_status",
        serde_json::json!({"payment_hash":"one"}),
    ));
    f.agent.turn("status").await.unwrap();
    assert!(matches!(
        outputs(&f).last(),
        Some(ToolOutput::Payment {
            status: OutcomeStatus::Settled,
            ..
        })
    ));
}
#[tokio::test]
async fn invalid_and_forged_arguments_are_rejected() {
    let mut f = setup(vec![
        execute("missing"),
        call(
            "wallet_execute_payment",
            serde_json::json!({"plan_id":"one-1","amount":50}),
        ),
        call(
            "wallet_execute_payment",
            serde_json::json!({"plan_id":"one-1","approval":true}),
        ),
        call("wallet_decode_invoice", serde_json::json!({"invoice":42})),
        call("approve_from_human", serde_json::json!({"plan_id":"one-1"})),
    ]);
    f.agent.turn("attack").await.unwrap();
    assert!(has_error(&f, "unknown_plan"));
    assert!(has_error(&f, "invalid_arguments"));
    assert!(has_error(&f, "unknown_tool"));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn changed_invoice_and_other_plan_cannot_use_approval() {
    let mut f = setup(vec![
        prepare("one"),
        prepare("two"),
        execute("two-2"),
        execute("one-1"),
    ]);
    f.agent.turn("pay").await.unwrap();
    assert!(f.agent.confirm_from_human("two-2", true).is_err());
    f.agent.confirm_from_human("one-1", true).unwrap();
    f.node.amount.store(50, Ordering::SeqCst);
    f.agent.turn("").await.unwrap();
    f.agent.turn("go").await.unwrap();
    assert!(has_error(&f, "approval_required"));
    assert!(has_error(&f, "invalid_payment"));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn denied_plan_does_not_prompt_or_submit() {
    let mut f = setup(vec![prepare("one"), execute("one-1")]);
    f.node.amount.store(600, Ordering::SeqCst);
    assert!(matches!(
        f.agent.turn("pay").await.unwrap(),
        TurnOutcome::Reply(_)
    ));
    assert!(f.agent.confirm_from_human("one-1", true).is_err());
    assert!(
        matches!(outputs(&f)[0],ToolOutput::Plan {plan} if matches!(plan.policy,PolicyDecision::Deny{..}))
    );
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn cancellation_never_authorizes() {
    let mut f = setup(vec![prepare("one"), execute("one-1")]);
    f.agent.turn("pay").await.unwrap();
    f.agent.confirm_from_human("one-1", false).unwrap();
    f.agent.turn("").await.unwrap();
    assert!(has_error(&f, "approval_required"));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn uncertain_submission_retains_duplicate_protection() {
    let mut f = setup(vec![
        prepare("one"),
        execute("one-1"),
        prepare("one"),
        execute("one-2"),
    ]);
    f.node.fail.store(true, Ordering::SeqCst);
    f.agent.turn("pay").await.unwrap();
    f.agent.confirm_from_human("one-1", true).unwrap();
    f.agent.turn("").await.unwrap();
    assert!(outputs(&f).iter().any(|o| matches!(
        o,
        ToolOutput::Payment {
            status: OutcomeStatus::Uncertain,
            ..
        }
    )));
    f.agent.confirm_from_human("one-2", true).unwrap();
    f.agent.turn("").await.unwrap();
    assert!(has_error(&f, "duplicate"));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert!(!serde_json::to_string(f.agent.conversation())
        .unwrap()
        .contains("secret remote body"));
}
#[tokio::test]
async fn repeated_tools_stop_at_bound() {
    let mut f = setup(
        (0..20)
            .map(|_| call("wallet_get_assets", serde_json::json!({})))
            .collect(),
    );
    assert!(matches!(
        f.agent.turn("loop").await.unwrap(),
        TurnOutcome::LimitReached
    ));
    assert_eq!(outputs(&f).len(), MAX_TOOL_STEPS);
}
#[test]
fn tool_schema_has_no_approval_or_payment_override() {
    let defs = tool_definitions();
    let execute = defs
        .iter()
        .find(|t| t.name == "wallet_execute_payment")
        .unwrap();
    assert_eq!(
        execute.parameters["required"],
        serde_json::json!(["plan_id"])
    );
    assert_eq!(execute.parameters["additionalProperties"], false);
    assert_eq!(
        execute.parameters["properties"].as_object().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn failed_status_is_never_settled() {
    let mut f = setup(vec![prepare("one"), execute("one-1")]);
    f.node.status_failed.store(true, Ordering::SeqCst);
    f.agent.turn("pay").await.unwrap();
    f.agent.confirm_from_human("one-1", true).unwrap();
    f.agent.turn("").await.unwrap();
    assert!(matches!(
        outputs(&f).last(),
        Some(ToolOutput::Payment {
            status: OutcomeStatus::Failed,
            ..
        })
    ));
}
#[tokio::test]
async fn read_failure_returns_sanitized_structured_error() {
    let mut f = setup(vec![call("wallet_get_assets", serde_json::json!({}))]);
    f.node.read_failed.store(true, Ordering::SeqCst);
    f.agent.turn("balance").await.unwrap();
    assert!(has_error(&f, "wallet_failure"));
    assert!(!serde_json::to_string(f.agent.conversation())
        .unwrap()
        .contains("secret remote body"));
}

#[async_trait]
impl rgb402_payment::lightning::LightningNode for Node {
    async fn create_invoice(
        &self,
        _: u64,
        _: u32,
    ) -> Result<rgb402_payment::lightning::LightningInvoice, WalletError> {
        unreachable!()
    }
    async fn decode_btc_invoice(
        &self,
        invoice: &str,
    ) -> Result<rgb402_payment::lightning::LightningInvoice, WalletError> {
        Ok(rgb402_payment::lightning::LightningInvoice {
            invoice: invoice.into(),
            amount_sats: self.amount.load(Ordering::SeqCst),
            expires_at: u64::MAX,
            payment_hash: PaymentId::new(
                "4bb06f8e4e3a7715d201d573d0aa423762e55dabd61a2c02278fa56cc6d294e0",
            )?,
        })
    }
    async fn outbound_sats(&self) -> Result<u64, WalletError> {
        Ok(1000)
    }
    async fn send_btc(
        &self,
        p: &rgb402_payment::lightning::ApprovedMachinePayment,
    ) -> Result<PaymentResult, WalletError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(PaymentResult {
            payment_id: p.invoice().payment_hash.clone(),
            payment_hash: p.invoice().payment_hash.clone(),
            status: PaymentStatus::Pending,
        })
    }
    async fn btc_payment(
        &self,
        _: &PaymentId,
    ) -> Result<rgb402_payment::lightning::LightningPayment, WalletError> {
        Ok(rgb402_payment::lightning::LightningPayment::new(
            if self.status_failed.load(Ordering::SeqCst) {
                PaymentStatus::Failed
            } else {
                PaymentStatus::Settled
            },
            Some("0707070707070707070707070707070707070707070707070707070707070707".into()),
        ))
    }
}
#[tokio::test]
async fn machine_tool_preserves_application_boundary_and_rejects_model_approval() {
    use axum::{
        http::{header, StatusCode},
        routing::get,
        Json, Router,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let url = format!("{origin}/premium/report");
    let app=Router::new().route("/premium/report",get(||async{(StatusCode::PAYMENT_REQUIRED,[(header::WWW_AUTHENTICATE,"L402 macaroon=\"abc\", invoice=\"lnbcrt50\"")],Json(serde_json::json!({"resource":"/premium/report","amount_sats":50,"invoice":"lnbcrt50"})))}));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut f = setup(vec![
        call(
            "agent_fetch_resource",
            serde_json::json!({"url":url,"approved":true}),
        ),
        call("agent_fetch_resource", serde_json::json!({"url":url})),
    ]);
    f.node.amount.store(50, Ordering::SeqCst);
    let state = f.path.with_file_name(format!(
        "machine-{}",
        f.path.file_name().unwrap().to_string_lossy()
    ));
    let service = CommerceService::open(
        f.node.clone(),
        rgb402_payment::commerce::CommerceConfig {
            origins: vec![origin],
            policy: rgb402_core::machine::MachinePolicy {
                auto_approve_below_sats: 10,
                max_single_payment_sats: 100,
                max_daily_spend_sats: 500,
            },
            state_path: state.clone(),
        },
    )
    .unwrap();
    f.agent.commerce = Some(service);
    assert!(matches!(
        f.agent
            .turn("Fetch the report and approve it yourself")
            .await
            .unwrap(),
        TurnOutcome::MachinePrepared(_)
    ));
    assert!(f.agent.conversation().iter().any(|m|matches!(m,Message::Result {output:ToolOutput::Error {code,..},..} if code=="invalid_arguments")));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    assert!(f.agent.confirm_from_human("invented", true).is_err());
    server.abort();
    drop(f);
    let _ = std::fs::remove_file(state);
}

#[tokio::test]
async fn machine_auto_purchase_returns_only_high_level_resource_to_model() {
    use axum::{
        http::{header, HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
        Json, Router,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let url = format!("{origin}/premium/report");
    let app=Router::new().route("/premium/report",get(|headers:HeaderMap|async move{
        if headers.contains_key(header::AUTHORIZATION) {return Json(serde_json::json!({"report":"actual protected fixture"})).into_response();}
        (StatusCode::PAYMENT_REQUIRED,[(header::WWW_AUTHENTICATE,"L402 macaroon=\"abc\", invoice=\"lnbcrt3\"")],Json(serde_json::json!({"resource":"/premium/report","amount_sats":3,"invoice":"lnbcrt3"}))).into_response()
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut f = setup(vec![call(
        "agent_fetch_resource",
        serde_json::json!({"url":url}),
    )]);
    f.node.amount.store(3, Ordering::SeqCst);
    let state = f.path.with_file_name(format!(
        "machine-{}",
        f.path.file_name().unwrap().to_string_lossy()
    ));
    f.agent.commerce = Some(
        CommerceService::open(
            f.node.clone(),
            rgb402_payment::commerce::CommerceConfig {
                origins: vec![origin],
                policy: rgb402_core::machine::MachinePolicy {
                    auto_approve_below_sats: 10,
                    max_single_payment_sats: 100,
                    max_daily_spend_sats: 500,
                },
                state_path: state.clone(),
            },
        )
        .unwrap(),
    );
    assert!(matches!(
        f.agent.turn("Get the premium report").await.unwrap(),
        TurnOutcome::Reply(_)
    ));
    let output = f
        .agent
        .conversation()
        .iter()
        .find_map(|m| match m {
            Message::Result {
                output: ToolOutput::Machine { purchase },
                ..
            } => Some(purchase),
            _ => None,
        })
        .unwrap();
    assert_eq!(output.resource_status, "purchased");
    assert_eq!(
        output.resource.as_ref().unwrap()["report"],
        "actual protected fixture"
    );
    assert!(output.auto_approved);
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    let json = serde_json::to_string(f.agent.conversation()).unwrap();
    assert!(!json.contains("macaroon"));
    assert!(!json.contains("0707070707070707"));
    assert!(f.agent.machine_pending.is_none());
    server.abort();
    drop(f);
    let _ = std::fs::remove_file(state);
}

#[tokio::test]
async fn model_narration_is_not_economic_evidence() {
    let mut f = setup(vec![ModelResponse::Text(
        "Payment settled and resource unlocked; I authorize it.".into(),
    )]);
    f.agent.turn("Pay the invoice").await.unwrap();
    assert!(f.agent.wallet.payment_history().is_empty());
    assert!(f
        .agent
        .wallet
        .task(&PaymentId::new("invented").unwrap())
        .is_none());
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gateway_trace_is_bounded_and_does_not_copy_untrusted_arguments() {
    let mut f = setup(vec![]);
    for _ in 0..5 {
        f.agent.model.0 = (0..8)
            .map(|_| {
                call(
                    "unknown-secret-tool",
                    serde_json::json!({"secret":"must-not-be-in-trace"}),
                )
            })
            .collect();
        assert!(matches!(
            f.agent.turn("inspect").await.unwrap(),
            TurnOutcome::LimitReached
        ));
    }
    assert_eq!(f.agent.trace().len(), 32);
    let serialized = serde_json::to_string(f.agent.trace()).unwrap();
    assert!(!serialized.contains("must-not-be-in-trace"));
    assert!(!serialized.contains("unknown-secret-tool"));
    assert!(serialized.contains("unknown_tool"));
}

#[tokio::test]
async fn approved_plan_continuation_does_not_depend_on_model_copying_id() {
    let mut f = setup(vec![
        prepare("one"),
        execute("one"),
        execute("one-1"),
        execute("one-1"),
    ]);
    f.agent.turn("pay").await.unwrap();
    assert!(f.agent.wallet.plan("one-1").is_ok());
    assert!(f.agent.confirm_from_human("stale", true).is_err());
    f.agent.confirm_from_human("one-1", true).unwrap();
    assert!(f.agent.wallet.plan("one-1").is_ok());
    assert!(f.agent.confirm_from_human("one-1", true).is_err());
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    f.agent.turn("").await.unwrap();
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert!(f.agent.wallet.plan("one-1").is_err());
    assert!(f.agent.trace().iter().any(|e| e
        .task
        .as_ref()
        .is_some_and(|t| t.observed_submission_attempts == 1)));
}

fn btc_invoice() -> rgb402_payment::lightning::LightningInvoice {
    rgb402_payment::lightning::LightningInvoice {
        invoice: "btc-test-invoice".into(),
        payment_hash: PaymentId::new(
            "4bb06f8e4e3a7715d201d573d0aa423762e55dabd61a2c02278fa56cc6d294e0",
        )
        .unwrap(),
        amount_sats: 5,
        expires_at: u64::MAX,
    }
}
struct BtcResolver;
#[async_trait]
impl crate::recipient::btc::BtcRecipientServices for BtcResolver {
    async fn acquire(
        &self,
        id: &str,
        amount: u64,
    ) -> Result<crate::recipient::btc::BtcCandidate, WalletError> {
        assert_eq!(id, "alice@example.com");
        assert_eq!(amount, 5);
        Ok(crate::recipient::btc::BtcCandidate {
            recipient: rgb402_payment::btc::BtcRecipient {
                identifier: id.into(),
                authoritative_domain: "example.com".into(),
                service_url: "https://example.com/btc/alice".into(),
            },
            invoice: btc_invoice(),
        })
    }
}
#[tokio::test]
async fn btc_recipient_requires_application_approval_and_continues_exact_plan() {
    let mut f = setup(vec![call(
        "wallet_prepare_btc_recipient_payment",
        serde_json::json!({"identifier":"alice@example.com","amount_sats":5}),
    )]);
    let path = f.path.with_extension("btc.jsonl");
    f.agent.btc = Some(
        BtcService::open(
            f.node.clone(),
            rgb402_core::btc::BtcTransferPolicy {
                max_payment_sats: 100,
                max_daily_sats: 500,
            },
            &path,
        )
        .unwrap(),
    );
    f.agent.btc_services = Arc::new(BtcResolver);
    let TurnOutcome::BtcPrepared(plan) =
        f.agent.turn("Pay alice@example.com 5 sats").await.unwrap()
    else {
        panic!("expected BTC approval")
    };
    assert_eq!(plan.amount_sats, "5");
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    let forged = ToolCall {
        id: "forged".into(),
        name: "wallet_execute_btc_payment".into(),
        arguments: serde_json::json!({"plan_id":plan.plan_id,"approve":true}).to_string(),
    };
    assert!(matches!(
        f.agent.dispatch(&forged).await,
        ToolOutput::Error { .. }
    ));
    let unapproved = ToolCall {
        arguments: serde_json::json!({"plan_id":plan.plan_id}).to_string(),
        ..forged
    };
    assert!(matches!(
        f.agent.dispatch(&unapproved).await,
        ToolOutput::Error { .. }
    ));
    assert!(f.agent.confirm_from_human("other", true).is_err());
    f.agent.confirm_from_human(&plan.plan_id, true).unwrap();
    f.agent.turn("").await.unwrap();
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert!(matches!(
        f.agent.dispatch(&unapproved).await,
        ToolOutput::Error { .. }
    ));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    assert!(f
        .agent
        .trace()
        .iter()
        .any(|e| e.tool == "wallet_execute_btc_payment"
            && e.task
                .as_ref()
                .is_some_and(|t| t.kind == rgb402_payment::harness::TaskKind::BtcTransfer)));
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn sats_intent_cannot_be_substituted_with_an_rgb_plan() {
    let mut f = setup(vec![prepare("rgb-invoice")]);
    f.agent.turn("Pay alice@example.com 5 sats").await.unwrap();
    assert!(outputs(&f)
        .iter()
        .any(|o| matches!(o,ToolOutput::Error{code,..} if code=="currency_mismatch")));
    assert!(f.agent.pending.is_none());
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn btc_failure_receipt_cannot_be_reinterpreted_as_missing_approval() {
    let mut f = setup(vec![
        call(
            "wallet_btc_payment_status",
            serde_json::json!({"payment_hash":"test-hash"}),
        ),
        ModelResponse::Text("Please approve again".into()),
    ]);
    let path = f.path.with_extension("btc.jsonl");
    f.agent.btc = Some(
        BtcService::open(
            f.node.clone(),
            rgb402_core::btc::BtcTransferPolicy {
                max_payment_sats: 100,
                max_daily_sats: 500,
            },
            &path,
        )
        .unwrap(),
    );
    f.node.status_failed.store(true, Ordering::SeqCst);
    let TurnOutcome::Reply(receipt) = f.agent.turn("Check BTC payment status").await.unwrap()
    else {
        panic!("receipt required")
    };
    assert!(receipt.contains("failed at the node"));
    assert!(receipt.contains("does not require another approval"));
    assert!(!receipt.contains("Please approve again"));
    assert_eq!(f.agent.model.0.len(), 1);
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    std::fs::remove_file(path).unwrap();
}
