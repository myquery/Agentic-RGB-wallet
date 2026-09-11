use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use rgb402_agent::{AgentError, AgentOutcome, BuyerAgent, MerchantClient, MerchantResponse};
use rgb402_core::{
    AssetId, PaymentAmount, PaymentChallenge, PaymentId, PaymentRequestId, PolicyDecision,
    PolicyRejectionReason, SpendingPolicy, UnixTimestamp, RGB402_PAYMENT_ID_HEADER,
};
use rgb402_merchant::MerchantConfig;
use rgb402_payment::{PaymentProvider, SettlementVerifier, SimulatedRgbPaymentProvider};
use std::sync::Arc;
use tower::ServiceExt;

const RESOURCE: &str = "/premium-analysis";

#[tokio::test]
async fn successful_purchase_unlocks_resource_after_payment_retry() {
    let provider = Arc::new(SimulatedRgbPaymentProvider::new());
    let merchant = in_process_merchant(default_config(), provider.clone());
    let mut agent = agent_with_policy(merchant, provider.clone(), boss_policy(5, 20));

    let outcome = agent.fetch_paid_resource(RESOURCE).await.unwrap();

    let AgentOutcome::Purchased {
        challenge,
        decision,
        receipt,
        body,
        session_spend,
    } = outcome
    else {
        panic!("expected successful purchase");
    };

    assert_eq!(challenge.amount, amount(4));
    assert!(decision.is_approved());
    assert_eq!(
        receipt.payment_id,
        SimulatedRgbPaymentProvider::deterministic_payment_id(&challenge.payment_request_id)
            .unwrap()
    );
    assert!(body.contains("RGB402 bootstrap demo"));
    assert_eq!(session_spend, amount(4));
    assert_eq!(provider.settled_count().unwrap(), 1);
}

#[tokio::test]
async fn expensive_payment_is_rejected_and_no_payment_is_performed() {
    let provider = Arc::new(SimulatedRgbPaymentProvider::new());
    let mut config = default_config();
    config.amount = amount(8);
    let merchant = in_process_merchant(config, provider.clone());
    let mut agent = agent_with_policy(merchant, provider.clone(), boss_policy(5, 20));

    let outcome = agent.fetch_paid_resource(RESOURCE).await.unwrap();

    assert_rejected(outcome, PolicyRejectionReason::ExceedsMaxPayment);
    assert_eq!(provider.settled_count().unwrap(), 0);
}

#[tokio::test]
async fn wrong_asset_is_rejected_and_no_payment_is_performed() {
    let provider = Arc::new(SimulatedRgbPaymentProvider::new());
    let mut config = default_config();
    config.asset_id = AssetId::new("rgb:unknown").unwrap();
    let merchant = in_process_merchant(config, provider.clone());
    let mut agent = agent_with_policy(merchant, provider.clone(), boss_policy(5, 20));

    let outcome = agent.fetch_paid_resource(RESOURCE).await.unwrap();

    assert_rejected(outcome, PolicyRejectionReason::AssetNotAllowed);
    assert_eq!(provider.settled_count().unwrap(), 0);
}

#[tokio::test]
async fn session_budget_blocks_second_payment() {
    let provider = Arc::new(SimulatedRgbPaymentProvider::new());
    let merchant = in_process_merchant(default_config(), provider.clone());
    let mut agent = agent_with_policy(merchant, provider.clone(), boss_policy(5, 5));

    let first = agent.fetch_paid_resource(RESOURCE).await.unwrap();
    assert!(matches!(first, AgentOutcome::Purchased { .. }));

    let second = agent.fetch_paid_resource(RESOURCE).await.unwrap();
    assert_rejected(second, PolicyRejectionReason::ExceedsSessionBudget);
    assert_eq!(provider.settled_count().unwrap(), 1);
}

#[tokio::test]
async fn fake_receipt_does_not_unlock_resource() {
    let provider = Arc::new(SimulatedRgbPaymentProvider::new());
    let merchant = in_process_merchant(default_config(), provider.clone());
    let fake_payment_id = PaymentId::new("sim-rgb-payment-req-premium-analysis").unwrap();

    let response = merchant
        .get_resource(RESOURCE, Some(&fake_payment_id))
        .await
        .unwrap();

    assert!(matches!(response, MerchantResponse::PaymentRequired { .. }));
    assert_eq!(provider.settled_count().unwrap(), 0);
}

#[tokio::test]
async fn expired_challenge_is_rejected_and_no_payment_is_performed() {
    let provider = Arc::new(SimulatedRgbPaymentProvider::new());
    let mut config = default_config();
    config.expires_at = UnixTimestamp::new(99);
    let merchant = in_process_merchant(config, provider.clone());
    let mut agent = agent_with_policy(merchant, provider.clone(), boss_policy(5, 20));

    let outcome = agent.fetch_paid_resource(RESOURCE).await.unwrap();

    assert_rejected(outcome, PolicyRejectionReason::ChallengeExpired);
    assert_eq!(provider.settled_count().unwrap(), 0);
}

fn in_process_merchant(
    config: MerchantConfig,
    provider: Arc<SimulatedRgbPaymentProvider>,
) -> Arc<InProcessMerchantClient> {
    let settlement: Arc<dyn SettlementVerifier> = provider;
    let app = rgb402_merchant::router(config, settlement);
    Arc::new(InProcessMerchantClient { app })
}

fn agent_with_policy(
    merchant: Arc<dyn MerchantClient>,
    provider: Arc<SimulatedRgbPaymentProvider>,
    policy: SpendingPolicy,
) -> BuyerAgent {
    let payment_provider: Arc<dyn PaymentProvider> = provider;
    BuyerAgent::with_merchant_client(merchant, policy, payment_provider)
        .unwrap()
        .with_now(|| UnixTimestamp::new(100))
}

fn boss_policy(max_payment: u64, session_budget: u64) -> SpendingPolicy {
    SpendingPolicy::new(
        vec![AssetId::boss()],
        amount(max_payment),
        amount(session_budget),
    )
}

fn default_config() -> MerchantConfig {
    let mut config = MerchantConfig::premium_analysis_default();
    config.expires_at = UnixTimestamp::new(1_000);
    config.payment_request_id = PaymentRequestId::new("req-premium-analysis").unwrap();
    config
}

fn amount(major_units: u64) -> PaymentAmount {
    PaymentAmount::from_major_minor(major_units, 0, 2).unwrap()
}

fn assert_rejected(outcome: AgentOutcome, expected_reason: PolicyRejectionReason) {
    let AgentOutcome::Rejected { decision, .. } = outcome else {
        panic!("expected rejected outcome");
    };
    assert_eq!(
        decision,
        PolicyDecision::Rejected {
            reason: expected_reason
        }
    );
}

#[derive(Clone)]
struct InProcessMerchantClient {
    app: axum::Router,
}

#[async_trait]
impl MerchantClient for InProcessMerchantClient {
    async fn get_resource(
        &self,
        resource_path: &str,
        payment_id: Option<&PaymentId>,
    ) -> Result<MerchantResponse, AgentError> {
        let mut request = Request::builder().method("GET").uri(resource_path);
        if let Some(payment_id) = payment_id {
            request = request.header(RGB402_PAYMENT_ID_HEADER, payment_id.as_str());
        }

        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

        if status == StatusCode::OK {
            return Ok(MerchantResponse::Ok {
                body: String::from_utf8(bytes.to_vec()).unwrap(),
            });
        }

        if status == StatusCode::PAYMENT_REQUIRED {
            return Ok(MerchantResponse::PaymentRequired {
                challenge: serde_json::from_slice::<PaymentChallenge>(&bytes).unwrap(),
            });
        }

        Err(AgentError::UnexpectedStatus {
            status: status.as_u16(),
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        })
    }
}
