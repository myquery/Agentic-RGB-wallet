//! Application-only adapter into the ordinary RGB payment workflow.
use super::acquisition::{RecipientInvoiceContract, ValidatedRecipientInvoice};
use rgb402_payment::wallet::{PaymentPlan, RecipientProvenance, WalletError, WalletService};

/// Prepare the acquired candidate against the application's original intent.
/// No discovery, acquisition, approval or execution occurs in this adapter.
pub async fn prepare_recipient_payment(
    wallet: &mut WalletService,
    candidate: &ValidatedRecipientInvoice,
    expected_contract: &RecipientInvoiceContract,
) -> Result<PaymentPlan, WalletError> {
    if !candidate.matches_contract(expected_contract) {
        return Err(WalletError::Invalid(
            "recipient acquisition contract mismatch",
        ));
    }
    let contract = candidate.contract();
    let provenance = RecipientProvenance {
        identifier: contract.recipient().identifier().into(),
        authoritative_domain: contract.recipient().authoritative_domain().into(),
        recipient_contract_digest: contract.digest().into(),
        invoice_id: candidate.invoice_id().into(),
        payment_hash: candidate.request().payment_hash.clone(),
    };
    wallet
        .prepare_recipient_invoice(candidate.request(), provenance)
        .await
}
