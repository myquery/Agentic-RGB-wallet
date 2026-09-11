use super::*;
use async_trait::async_trait;
use rgb402_agent::wallet_agent::{Message, ModelError, ModelResponse, ToolCall, ToolDefinition};
use rgb402_core::{
    wallet::{Asset, PaymentRequest, WalletBalance, WalletPolicy},
    AssetId, PaymentId, PaymentStatus,
};
use rgb402_payment::{
    rgb::RgbNode,
    wallet::{ApprovedPayment, PaymentResult, WalletError},
};
use std::{
    collections::VecDeque,
    io::Cursor,
    sync::atomic::{AtomicU64, Ordering},
};
struct FakeModel(VecDeque<Result<ModelResponse, ModelError>>);
#[async_trait]
impl AgentModel for FakeModel {
    async fn respond(
        &mut self,
        _: &[Message],
        _: &[ToolDefinition],
    ) -> Result<ModelResponse, ModelError> {
        self.0
            .pop_front()
            .unwrap_or(Ok(ModelResponse::Text("Review wallet result.".into())))
    }
}
struct Node {
    sends: AtomicU64,
}
#[async_trait]
impl RgbNode for Node {
    async fn list_assets(&self) -> Result<Vec<Asset>, WalletError> {
        Ok(vec![])
    }
    async fn asset_balance(&self, id: &AssetId) -> Result<WalletBalance, WalletError> {
        Ok(WalletBalance {
            asset_id: id.clone(),
            onchain_spendable: 400,
            offchain_outbound: 495,
        })
    }
    async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError> {
        Ok(PaymentRequest {
            asset_id: AssetId::new("rgb:demo")?,
            amount: 5,
            invoice: invoice.into(),
            payment_hash: PaymentId::new("hash")?,
            expires_at: u64::MAX,
            network: "Regtest".into(),
            carrier_msat: 3_000_000,
        })
    }
    async fn send_payment(&self, p: &ApprovedPayment) -> Result<PaymentResult, WalletError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
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
fn tool(name: &str, args: serde_json::Value) -> Result<ModelResponse, ModelError> {
    Ok(ModelResponse::Tool(ToolCall {
        id: "call".into(),
        name: name.into(),
        arguments: args.to_string(),
    }))
}
static COUNTER: AtomicU64 = AtomicU64::new(0);
async fn session(input: &str, fail_after_payment: bool) -> (String, u64) {
    let node = Arc::new(Node {
        sends: AtomicU64::new(0),
    });
    let path = std::env::temp_dir().join(format!(
        "rgb-agent-cli-{}-{}.jsonl",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let wallet = WalletService::open(
        node.clone(),
        WalletPolicy {
            allowed_assets: [AssetId::new("rgb:demo").unwrap()].into(),
            auto_approve_below: 1,
            max_single_payment: 100,
            max_daily_spend: 500,
            max_carrier_msat: 3_000_000,
        },
        &path,
    )
    .unwrap();
    let mut script = VecDeque::from([
        tool(
            "wallet_prepare_payment",
            serde_json::json!({"invoice":"lnbcrt-demo"}),
        ),
        tool(
            "wallet_execute_payment",
            serde_json::json!({"plan_id":"hash-1"}),
        ),
    ]);
    if fail_after_payment {
        script.push_back(Err(ModelError::Transport));
    }
    let mut agent = WalletAgent::new(FakeModel(script), wallet);
    let mut output = vec![];
    run(&mut agent, &mut Cursor::new(input), &mut output)
        .await
        .unwrap();
    drop(agent);
    std::fs::remove_file(path).unwrap();
    (
        String::from_utf8(output).unwrap(),
        node.sends.load(Ordering::SeqCst),
    )
}
#[tokio::test]
async fn cli_requires_explicit_application_input_and_shows_bound_details() {
    let (text, sends) = session("pay invoice\ny\n/quit\n", false).await;
    assert_eq!(sends, 1);
    let prompt = text.find("Approve this exact demo payment?").unwrap();
    for detail in [
        "Asset ID: rgb:demo",
        "Amount: 5 base units",
        "Available outbound: 495 base units",
        "Invoice/destination: lnbcrt-demo",
        "Payment hash: hash",
        "Carrier: 3000000 msat",
        "require_approval",
    ] {
        assert!(text[..prompt].contains(detail));
    }
    assert!(text.contains("\"status\": \"settled\""));
}
#[tokio::test]
async fn cli_blank_eof_and_conversational_assent_cancel() {
    for input in [
        "pay\n",
        "pay\n\n/quit\n",
        "pay\nyeah sure\n/quit\n",
        "pay\ngo ahead\n/quit\n",
        "pay\nn\n/quit\n",
    ] {
        let (text, sends) = session(input, false).await;
        assert_eq!(sends, 0);
        assert!(text.contains("Payment cancelled."));
    }
}
#[tokio::test]
async fn settlement_is_printed_even_when_model_transport_then_fails() {
    let (text, sends) = session("pay\ny\n/quit\n", true).await;
    assert_eq!(sends, 1);
    assert!(text.find("\"status\": \"settled\"").unwrap() < text.find("Model error:").unwrap());
}
#[test]
fn terminal_controls_cannot_rewrite_approval_display() {
    let text = terminal_text("x\u{1b}[2J\ry\u{202e}\n");
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains('\r'));
    assert!(!text.contains('\u{202e}'));
    assert!(text.ends_with('\n'));
}
