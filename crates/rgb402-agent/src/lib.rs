pub mod wallet_agent;

use async_trait::async_trait;
use rgb402_core::{
    DomainError, PaymentAmount, PaymentChallenge, PaymentId, PaymentReceipt, PaymentRequest,
    PolicyDecision, PolicyRejectionReason, SpendingPolicy, SpendingSession, UnixTimestamp,
    RGB402_PAYMENT_ID_HEADER,
};
use rgb402_payment::{PaymentError, PaymentProvider};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("domain error: {0}")]
    Domain(#[from] DomainError),
    #[error("payment error: {0}")]
    Payment(#[from] PaymentError),
    #[error("unexpected merchant status {status}: {body}")]
    UnexpectedStatus { status: u16, body: String },
}

pub struct BuyerAgent {
    merchant_client: Arc<dyn MerchantClient>,
    payment_provider: Arc<dyn PaymentProvider>,
    session: SpendingSession,
    now: Arc<dyn Fn() -> UnixTimestamp + Send + Sync>,
}

impl BuyerAgent {
    pub fn new(
        base_url: impl Into<String>,
        policy: SpendingPolicy,
        payment_provider: Arc<dyn PaymentProvider>,
    ) -> Result<Self, AgentError> {
        let merchant_client: Arc<dyn MerchantClient> =
            Arc::new(HttpMerchantClient::new(base_url.into()));
        Self::with_merchant_client(merchant_client, policy, payment_provider)
    }

    pub fn with_merchant_client(
        merchant_client: Arc<dyn MerchantClient>,
        policy: SpendingPolicy,
        payment_provider: Arc<dyn PaymentProvider>,
    ) -> Result<Self, AgentError> {
        Ok(Self {
            merchant_client,
            payment_provider,
            session: SpendingSession::new(policy)?,
            now: Arc::new(UnixTimestamp::now),
        })
    }

    pub fn with_now(mut self, now: impl Fn() -> UnixTimestamp + Send + Sync + 'static) -> Self {
        self.now = Arc::new(now);
        self
    }

    pub fn session_spend(&self) -> PaymentAmount {
        self.session.spent()
    }

    pub async fn fetch_paid_resource(
        &mut self,
        resource_path: &str,
    ) -> Result<AgentOutcome, AgentError> {
        let challenge = match self
            .merchant_client
            .get_resource(resource_path, None)
            .await?
        {
            MerchantResponse::Ok { body } => return Ok(AgentOutcome::AlreadyAccessible { body }),
            MerchantResponse::PaymentRequired { challenge } => challenge,
        };

        let decision = self
            .session
            .evaluate_challenge(&challenge, (self.now.as_ref())());

        if let PolicyDecision::Rejected { .. } = decision {
            return Ok(AgentOutcome::Rejected {
                challenge,
                decision,
            });
        }

        let payment_request = PaymentRequest::try_from(challenge.clone())?;
        let receipt = self.payment_provider.pay(&payment_request).await?;
        self.session.record_receipt(&receipt)?;

        match self
            .merchant_client
            .get_resource(resource_path, Some(&receipt.payment_id))
            .await?
        {
            MerchantResponse::Ok { body } => Ok(AgentOutcome::Purchased {
                challenge,
                decision,
                receipt,
                body,
                session_spend: self.session.spent(),
            }),
            MerchantResponse::PaymentRequired { .. } => Err(AgentError::UnexpectedStatus {
                status: 402,
                body: "merchant still requires payment after settlement".to_owned(),
            }),
        }
    }
}

#[async_trait]
pub trait MerchantClient: Send + Sync {
    async fn get_resource(
        &self,
        resource_path: &str,
        payment_id: Option<&PaymentId>,
    ) -> Result<MerchantResponse, AgentError>;
}

pub struct HttpMerchantClient {
    base_url: String,
    http: reqwest::Client,
}

impl HttpMerchantClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
        }
    }

    fn resource_url(&self, resource_path: &str) -> String {
        if resource_path.starts_with('/') {
            format!("{}{}", self.base_url, resource_path)
        } else {
            format!("{}/{}", self.base_url, resource_path)
        }
    }
}

#[async_trait]
impl MerchantClient for HttpMerchantClient {
    async fn get_resource(
        &self,
        resource_path: &str,
        payment_id: Option<&PaymentId>,
    ) -> Result<MerchantResponse, AgentError> {
        let mut request = self.http.get(self.resource_url(resource_path));
        if let Some(payment_id) = payment_id {
            request = request.header(RGB402_PAYMENT_ID_HEADER, payment_id.as_str());
        }

        let response = request.send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::OK {
            return Ok(MerchantResponse::Ok {
                body: response.text().await?,
            });
        }

        if status == reqwest::StatusCode::PAYMENT_REQUIRED {
            return Ok(MerchantResponse::PaymentRequired {
                challenge: response.json::<PaymentChallenge>().await?,
            });
        }

        Err(AgentError::UnexpectedStatus {
            status: status.as_u16(),
            body: response.text().await?,
        })
    }
}

#[derive(Clone, Debug)]
pub enum MerchantResponse {
    Ok { body: String },
    PaymentRequired { challenge: PaymentChallenge },
}

#[derive(Clone, Debug)]
pub enum AgentOutcome {
    AlreadyAccessible {
        body: String,
    },
    Purchased {
        challenge: PaymentChallenge,
        decision: PolicyDecision,
        receipt: PaymentReceipt,
        body: String,
        session_spend: PaymentAmount,
    },
    Rejected {
        challenge: PaymentChallenge,
        decision: PolicyDecision,
    },
}

impl AgentOutcome {
    pub fn rejection_reason(&self) -> Option<&PolicyRejectionReason> {
        match self {
            Self::Rejected {
                decision: PolicyDecision::Rejected { reason },
                ..
            } => Some(reason),
            _ => None,
        }
    }
}
