//! Composition only: protocol validation stays in the existing recipient layers.
use super::*;
use crate::recipient::{
    self,
    acquisition::{self, AcquisitionError, RecipientInvoiceContract, ValidatedRecipientInvoice},
    DiscoveryError, DiscoveryResult,
};

#[async_trait]
pub(crate) trait RecipientServices: Send + Sync {
    async fn resolve(&self, identifier: &str) -> DiscoveryResult;
    async fn acquire(
        &self,
        contract: &RecipientInvoiceContract,
    ) -> Result<ValidatedRecipientInvoice, AcquisitionError>;
}
pub(crate) struct PublicRecipientServices;
#[async_trait]
impl RecipientServices for PublicRecipientServices {
    async fn resolve(&self, identifier: &str) -> DiscoveryResult {
        recipient::resolve_recipient(identifier).await
    }
    async fn acquire(
        &self,
        contract: &RecipientInvoiceContract,
    ) -> Result<ValidatedRecipientInvoice, AcquisitionError> {
        acquisition::acquire_recipient_invoice(contract).await
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecipientInput {
    pub identifier: String,
    pub asset_id: AssetId,
    pub amount: u64,
}

pub(super) async fn prepare(
    wallet: &mut WalletService,
    services: &dyn RecipientServices,
    input: RecipientInput,
) -> Result<ToolOutput, WalletError> {
    if input.amount == 0 || input.identifier.len() > 318 || input.asset_id.as_str().len() > 256 {
        return Ok(error(
            "invalid_arguments",
            "Provide a recipient, asset ID and positive integer base-unit amount",
        ));
    }
    let descriptor = match services.resolve(&input.identifier).await {
        DiscoveryResult::Resolved { recipient } => recipient,
        DiscoveryResult::Failed { code, .. } => {
            let code = match code {
                DiscoveryError::MalformedIdentifier => "invalid_recipient",
                DiscoveryError::UnknownAccount => "recipient_not_found",
                DiscoveryError::DnsUnavailable
                | DiscoveryError::Timeout
                | DiscoveryError::ServiceUnavailable => "discovery_unavailable",
                DiscoveryError::MissingRelation | DiscoveryError::UnsupportedMediaType => {
                    "recipient_unsupported"
                }
                _ => "discovery_security_rejected",
            };
            return Ok(error(
                code,
                "Recipient preparation stopped; verify the recipient before preparing again",
            ));
        }
    };
    let contract = match RecipientInvoiceContract::new(
        descriptor,
        input.asset_id,
        input.amount,
        wallet.max_carrier_msat(),
    ) {
        Ok(contract) => contract,
        Err(_) => {
            return Ok(error(
                "invalid_arguments",
                "Provide a supported asset ID and positive integer amount",
            ))
        }
    };
    let candidate = match services.acquire(&contract).await {
        Ok(candidate) => candidate,
        Err(failure) => {
            let code = match failure {
                AcquisitionError::AssetMismatch => "asset_mismatch",
                AcquisitionError::AmountMismatch => "amount_mismatch",
                AcquisitionError::ExpiredInvoice => "invoice_expired",
                AcquisitionError::Unavailable | AcquisitionError::Timeout => {
                    "acquisition_unavailable"
                }
                AcquisitionError::SecurityRejected
                | AcquisitionError::HttpsFailure
                | AcquisitionError::ResponseTooLarge => "acquisition_security_rejected",
                _ => "invoice_invalid",
            };
            return Ok(error(
                code,
                "Invoice preparation stopped; no payment was submitted",
            ));
        }
    };
    let plan = recipient::bridge::prepare_recipient_payment(wallet, &candidate, &contract).await?;
    Ok(ToolOutput::Plan {
        plan: PlanView::from(&plan),
    })
}
