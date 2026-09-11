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
    assert!(has_error(&f, "unknown_plan"));
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
    f.agent.turn("").await.unwrap();
    f.node.amount.store(50, Ordering::SeqCst);
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
