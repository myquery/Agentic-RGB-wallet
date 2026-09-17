//! Non-economic production discovery/acquisition only. Never opens wallet state.
use rgb402_agent::recipient::{
    acquisition::{acquire_recipient_invoice, RecipientInvoiceContract},
    resolve_recipient, DiscoveryResult,
};
use rgb402_core::AssetId;
use serde_json::json;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identifier = std::env::var("RECIPIENT_IDENTIFIER")?;
    let asset = AssetId::new(std::env::var("RECIPIENT_ASSET_ID")?)?;
    let recipient = match resolve_recipient(&identifier).await {
        DiscoveryResult::Resolved { recipient } => recipient,
        DiscoveryResult::Failed { code, .. } => {
            return Err(format!("recipient resolution failed: {code:?}").into())
        }
    };
    let contract = RecipientInvoiceContract::new(recipient, asset, 5, 3_000_000)
        .map_err(|e| format!("contract rejected: {e:?}"))?;
    let candidate = acquire_recipient_invoice(&contract)
        .await
        .map_err(|e| format!("acquisition failed: {e:?}"))?;
    let request = candidate.request();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "timestamp_unix":candidate.validated_at(),"recipient_identifier":identifier,
            "authoritative_domain":contract.recipient().authoritative_domain(),"webfinger_success":true,
            "invoice_service_origin":format!("https://{}",contract.recipient().authoritative_domain()),
            "requested_asset":contract.requested_asset(),"requested_amount":5,"invoice_validation_result":"passed",
            "invoice_id":candidate.invoice_id(),"payment_hash":request.payment_hash,
            "decoded_asset":request.asset_id,"decoded_amount":request.amount,"network":request.network,
            "expires_at":request.expires_at,"unexpired_at_validation":request.expires_at>candidate.validated_at(),
            "carrier_msat":request.carrier_msat,"carrier_ceiling_msat":3_000_000,
            "preparation_attempted":false,"payment_submissions":0
        }))?
    );
    Ok(())
}
