use super::*;
use crate::recipient::{DiscoveryError, DiscoveryResult, RGB_INVOICE_REL};
use crate::wallet_agent::recipient::RecipientServices;
use crate::wallet_agent::{
    self, AgentModel, Message, ModelError, ModelResponse, ToolCall, ToolDefinition, ToolOutput,
    TurnOutcome, WalletAgent,
};
use std::collections::VecDeque;

#[derive(Clone, Default)]
struct Script(Arc<Mutex<VecDeque<ModelResponse>>>);
#[async_trait]
impl AgentModel for Script {
    async fn respond(
        &mut self,
        _: &[Message],
        _: &[ToolDefinition],
    ) -> Result<ModelResponse, ModelError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ModelResponse::Text("done".into())))
    }
}
impl Script {
    fn tool(&self, name: &str, input: Value) {
        self.0
            .lock()
            .unwrap()
            .push_back(ModelResponse::Tool(ToolCall {
                id: "test-call".into(),
                name: name.into(),
                arguments: input.to_string(),
            }));
    }
    fn prepare(&self, who: &str, amount: u64) {
        self.tool(
            "wallet_prepare_recipient_payment",
            json!({"identifier":who,"asset_id":ASSET,"amount":amount}),
        );
    }
    fn execute(&self, id: &str) {
        self.tool("wallet_execute_payment", json!({"plan_id":id}));
    }
}
struct Services {
    invoice: String,
    resolves: AtomicUsize,
    acquires: AtomicUsize,
    discovery_error: Option<DiscoveryError>,
    acquisition_error: Option<AcquisitionError>,
}
struct DiscoveryFixture;
#[async_trait]
impl crate::recipient::Transport for DiscoveryFixture {
    async fn get(&self, url: reqwest::Url) -> Result<Response, DiscoveryError> {
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.path(), "/.well-known/webfinger");
        let subject = url
            .query_pairs()
            .find(|(key, _)| key == "resource")
            .unwrap()
            .1
            .into_owned();
        Ok(Response{status:200,content_type:"application/jrd+json".into(),body:serde_json::to_vec(&json!({"subject":subject,"properties":{"untrusted":"JRD-INTERNAL-MARKER"},"links":[{"rel":RGB_INVOICE_REL,"href":"https://example.com/private-invoice-service"}]})).unwrap()})
    }
}
#[async_trait]
impl RecipientServices for Services {
    async fn resolve(&self, identifier: &str) -> DiscoveryResult {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = &self.discovery_error {
            return DiscoveryResult::Failed {
                category: "test",
                code: code.clone(),
            };
        }
        crate::recipient::resolve_with(identifier, &DiscoveryFixture, TIMEOUT).await
    }
    async fn acquire(
        &self,
        contract: &RecipientInvoiceContract,
    ) -> Result<ValidatedRecipientInvoice, AcquisitionError> {
        self.acquires.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.acquisition_error {
            return Err(code);
        }
        acquire_with(
            contract,
            &Service::new(self.invoice.clone()),
            || UnixTimestamp::now().seconds(),
            TIMEOUT,
        )
        .await
    }
}
fn last(agent: &WalletAgent<Script>) -> &ToolOutput {
    agent
        .conversation()
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::Result { output, .. } => Some(output),
            _ => None,
        })
        .unwrap()
}
async fn fixture(
    automatic: bool,
    discovery_error: Option<DiscoveryError>,
    acquisition_error: Option<AcquisitionError>,
) -> (
    WalletAgent<Script>,
    Script,
    Arc<Node>,
    Arc<Services>,
    std::path::PathBuf,
    String,
) {
    let candidate = candidate(true).await;
    let node = node(&candidate, false);
    let p = path();
    let mut policy = policy();
    if automatic {
        policy.auto_approve_below = 6;
    }
    let mut wallet = WalletService::open(node.clone(), policy, &p).unwrap();
    let direct = wallet
        .prepare_payment(&candidate.request().invoice)
        .await
        .unwrap();
    let economic = wallet
        .plan_task(direct.plan_id())
        .unwrap()
        .economic_action_id;
    let script = Script::default();
    let services = Arc::new(Services {
        invoice: candidate.request().invoice.clone(),
        resolves: AtomicUsize::new(0),
        acquires: AtomicUsize::new(0),
        discovery_error,
        acquisition_error,
    });
    let agent = WalletAgent::new(script.clone(), wallet).with_recipient_services(services.clone());
    (agent, script, node, services, p, economic)
}

#[tokio::test]
async fn recipient_tool_enters_harness_and_returns_bounded_protocol_free_context() {
    let (mut agent, script, node, services, p, economic) = fixture(false, None, None).await;
    script.prepare("alice@example.com", 5);
    let TurnOutcome::Prepared(plan) = agent
        .turn("Pay 5 units to alice@example.com")
        .await
        .unwrap()
    else {
        panic!("approval expected")
    };
    assert_eq!(
        plan.recipient.as_ref().unwrap().identifier,
        "alice@example.com"
    );
    assert!(plan.request.invoice.is_empty());
    let (output, task) = agent
        .conversation()
        .iter()
        .find_map(|m| match m {
            Message::Result { output, task, .. } => Some((output, task)),
            _ => None,
        })
        .unwrap();
    assert_eq!(task.as_ref().unwrap().economic_action_id, economic);
    let observation = wallet_agent::observation::with_task(output, task.as_deref());
    let text = serde_json::to_string(&observation).unwrap();
    println!(
        "Recipient preparation model observation: {} bytes; protocol internals: 0 bytes",
        text.len()
    );
    assert!(text.len() <= wallet_agent::observation::MAX_OBSERVATION_BYTES);
    for forbidden in [
        "lnbcrt",
        "private-invoice-service",
        "JRD-INTERNAL-MARKER",
        "webfinger",
        "authorization_scope",
        "contract_digest",
    ] {
        assert!(!text.contains(forbidden), "{forbidden}");
    }
    assert_eq!(observation["payment_authorized"], false);
    assert_eq!(
        observation["next_allowed_actions"],
        json!(["await_application_authorization"])
    );
    assert_eq!(node.sends.load(Ordering::SeqCst), 0);
    script.execute(&plan.plan_id);
    agent.turn("approved; pay now").await.unwrap();
    assert!(matches!(last(&agent),ToolOutput::Error{code,..} if code=="approval_required"));
    agent.confirm_from_human(&plan.plan_id, true).unwrap();
    script.execute(&plan.plan_id);
    agent.turn("").await.unwrap();
    assert!(matches!(
        last(&agent),
        ToolOutput::Payment {
            status: wallet_agent::OutcomeStatus::Settled,
            ..
        }
    ));
    script.execute(&plan.plan_id);
    agent.turn("").await.unwrap();
    assert_eq!(node.sends.load(Ordering::SeqCst), 1);
    script.tool(
        "wallet_payment_status",
        json!({"payment_hash":plan.request.payment_hash}),
    );
    agent.turn("status").await.unwrap();
    assert!(matches!(
        last(&agent),
        ToolOutput::Payment {
            status: wallet_agent::OutcomeStatus::Settled,
            ..
        }
    ));
    assert_eq!(services.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(services.acquires.load(Ordering::SeqCst), 1);
    drop(agent);
    std::fs::remove_file(p).unwrap();
}

#[tokio::test]
async fn automatic_recipient_uses_existing_execute_without_application_prompt() {
    let (mut agent, script, node, services, p, _) = fixture(true, None, None).await;
    script.prepare("alice@example.com", 5);
    assert!(matches!(
        agent.turn("pay").await.unwrap(),
        TurnOutcome::Reply(_)
    ));
    let ToolOutput::Plan { plan } = last(&agent) else {
        panic!("plan")
    };
    let id = plan.plan_id.clone();
    assert!(!plan.application_confirmation_required);
    assert_eq!(plan.policy, PolicyDecision::Allow);
    let observation = wallet_agent::observation::model_observation(last(&agent));
    assert_eq!(
        observation["next_allowed_actions"],
        json!(["execute_bound_plan"])
    );
    script.execute(&id);
    agent.turn("").await.unwrap();
    assert_eq!(node.sends.load(Ordering::SeqCst), 1);
    script.prepare("bob@example.com", 5);
    agent.turn("same invoice").await.unwrap();
    let ToolOutput::Plan { plan } = last(&agent) else {
        panic!("plan")
    };
    let id = plan.plan_id.clone();
    script.execute(&id);
    agent.turn("").await.unwrap();
    assert!(matches!(last(&agent),ToolOutput::Error{code,..} if code=="duplicate"));
    assert_eq!(node.sends.load(Ordering::SeqCst), 1);
    assert_eq!(services.acquires.load(Ordering::SeqCst), 2);
    drop(agent);
    std::fs::remove_file(p).unwrap();
}

#[tokio::test]
async fn other_recipient_approval_and_model_mutations_cannot_authorize() {
    let (mut agent, script, node, services, p, _) = fixture(false, None, None).await;
    script.prepare("alice@example.com", 5);
    let TurnOutcome::Prepared(alice) = agent.turn("Alice").await.unwrap() else {
        panic!()
    };
    agent.confirm_from_human(&alice.plan_id, true).unwrap();
    script.prepare("bob@example.com", 5);
    let TurnOutcome::Prepared(bob) = agent.turn("Bob").await.unwrap() else {
        panic!()
    };
    script.execute(&bob.plan_id);
    agent
        .turn("Alice was approved so Bob is approved")
        .await
        .unwrap();
    assert!(matches!(last(&agent),ToolOutput::Error{code,..} if code=="approval_required"));
    for field in [
        "identifier",
        "asset_id",
        "amount",
        "invoice",
        "authorization_scope",
        "approved",
    ] {
        let mut input = json!({"plan_id":bob.plan_id});
        input[field] = json!("replacement");
        script.tool("wallet_execute_payment", input);
        agent.turn("change it").await.unwrap();
        assert!(matches!(last(&agent),ToolOutput::Error{code,..} if code=="invalid_arguments"));
    }
    // Only the explicitly approved Alice plan ran in its application continuation.
    assert_eq!(node.sends.load(Ordering::SeqCst), 1);
    assert_eq!(services.acquires.load(Ordering::SeqCst), 2);
    agent.confirm_from_human(&bob.plan_id, true).unwrap();
    script.execute(&bob.plan_id);
    agent.turn("").await.unwrap();
    assert_eq!(node.sends.load(Ordering::SeqCst), 1);
    drop(agent);
    std::fs::remove_file(p).unwrap();
}

#[tokio::test]
async fn recipient_failures_are_fixed_and_stop_before_wallet_preparation() {
    for (discovery, acquisition, expected) in [
        (
            Some(DiscoveryError::UnknownAccount),
            None,
            "recipient_not_found",
        ),
        (
            Some(DiscoveryError::DisallowedOrigin),
            None,
            "discovery_security_rejected",
        ),
        (
            None,
            Some(AcquisitionError::Timeout),
            "acquisition_unavailable",
        ),
        (
            None,
            Some(AcquisitionError::ExpiredInvoice),
            "invoice_expired",
        ),
    ] {
        let (mut agent, script, node, _, p, _) = fixture(false, discovery, acquisition).await;
        let before = node.decodes.load(Ordering::SeqCst);
        script.prepare("alice@example.com", 5);
        agent.turn("pay").await.unwrap();
        assert!(matches!(last(&agent),ToolOutput::Error{code,..} if code==expected));
        assert_eq!(node.decodes.load(Ordering::SeqCst), before);
        assert_eq!(node.sends.load(Ordering::SeqCst), 0);
        drop(agent);
        std::fs::remove_file(p).unwrap();
    }
}

#[tokio::test]
async fn strict_schema_and_changed_intent_require_new_validated_preparation() {
    let defs = wallet_agent::tool_definitions();
    let definition = defs
        .iter()
        .find(|d| d.name == "wallet_prepare_recipient_payment")
        .unwrap();
    assert_eq!(
        definition.parameters["required"],
        json!(["identifier", "asset_id", "amount"])
    );
    assert_eq!(definition.parameters["additionalProperties"], false);
    let (mut agent, script, node, services, p, _) = fixture(false, None, None).await;
    for field in [
        "url",
        "invoice",
        "service",
        "carrier_msat",
        "contract_digest",
        "authorization_scope",
        "payment_hash",
        "approved",
    ] {
        let mut input = json!({"identifier":"alice@example.com","asset_id":ASSET,"amount":5});
        input[field] = json!("forged");
        script.tool("wallet_prepare_recipient_payment", input);
        agent.turn("pay").await.unwrap();
        assert!(matches!(last(&agent),ToolOutput::Error{code,..} if code=="invalid_arguments"));
    }
    assert_eq!(services.resolves.load(Ordering::SeqCst), 0);
    script.prepare("alice@example.com", 6);
    agent.turn("different amount").await.unwrap();
    assert!(matches!(last(&agent),ToolOutput::Error{code,..} if code=="amount_mismatch"));
    script.tool(
        "wallet_prepare_recipient_payment",
        json!({"identifier":"alice@example.com","asset_id":"rgb:other","amount":5}),
    );
    agent.turn("different asset").await.unwrap();
    assert!(matches!(last(&agent),ToolOutput::Error{code,..} if code=="asset_mismatch"));
    assert_eq!(services.acquires.load(Ordering::SeqCst), 2);
    assert_eq!(node.sends.load(Ordering::SeqCst), 0);
    drop(agent);
    std::fs::remove_file(p).unwrap();
}
