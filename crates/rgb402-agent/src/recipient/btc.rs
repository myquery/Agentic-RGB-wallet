//! BTC-specific address discovery/acquisition. Public HTTPS only, local decoding, no authority.
use super::*;
use lightning_invoice::{Bolt11Invoice, Currency, TaggedField};
use rgb402_core::{PaymentId, UnixTimestamp};
use rgb402_payment::{btc::BtcRecipient, lightning::LightningInvoice, wallet::WalletError};

pub struct BtcCandidate {
    pub recipient: BtcRecipient,
    pub invoice: LightningInvoice,
}
#[async_trait]
pub trait BtcRecipientServices: Send + Sync {
    async fn acquire(
        &self,
        identifier: &str,
        amount_sats: u64,
    ) -> Result<BtcCandidate, WalletError>;
}
pub struct PublicBtcRecipientServices;
#[async_trait]
impl BtcRecipientServices for PublicBtcRecipientServices {
    async fn acquire(
        &self,
        identifier: &str,
        amount_sats: u64,
    ) -> Result<BtcCandidate, WalletError> {
        if amount_sats == 0 || amount_sats.checked_mul(1000).is_none() {
            return Err(WalletError::Invalid("invalid satoshi amount"));
        }
        tokio::time::timeout(TIMEOUT, async {
            let descriptor =
                match resolve_relation_with(identifier, &HttpsTransport, TIMEOUT, BTC_INVOICE_REL)
                    .await
                {
                    DiscoveryResult::Resolved { recipient } => recipient,
                    _ => {
                        return Err(WalletError::Invalid(
                            "BTC recipient discovery unavailable or rejected",
                        ))
                    }
                };
            let response = https::exchange(
                Url::parse(descriptor.service_url())
                    .map_err(|_| WalletError::Invalid("invalid recipient endpoint"))?,
                Some(&json!({"subject":descriptor.subject(),"amount_sats":amount_sats})),
                "application/json",
            )
            .await
            .map_err(|_| WalletError::Node("BTC invoice acquisition unavailable or rejected"))?;
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Candidate {
                invoice: String,
            }
            let candidate: Candidate = serde_json::from_slice(&response.body)
                .map_err(|_| WalletError::Invalid("invalid BTC invoice response"))?;
            let invoice = validate_invoice(
                &candidate.invoice,
                amount_sats,
                UnixTimestamp::now().seconds(),
            )?;
            Ok(BtcCandidate {
                recipient: BtcRecipient {
                    identifier: descriptor.identifier().into(),
                    authoritative_domain: descriptor.authoritative_domain().into(),
                    service_url: descriptor.service_url().into(),
                },
                invoice,
            })
        })
        .await
        .map_err(|_| WalletError::Node("BTC acquisition timed out"))?
    }
}
fn validate_invoice(invoice: &str, amount: u64, now: u64) -> Result<LightningInvoice, WalletError> {
    if invoice.len() > 8192 {
        return Err(WalletError::Invalid("invoice too large"));
    }
    let decoded: Bolt11Invoice = invoice
        .parse()
        .map_err(|_| WalletError::Invalid("invalid BTC invoice"))?;
    if decoded.currency() != Currency::Regtest
        || decoded
            .tagged_fields()
            .any(|f| matches!(f, TaggedField::RgbContractId(_) | TaggedField::RgbAmount(_)))
        || decoded
            .tagged_fields()
            .filter(|f| matches!(f, TaggedField::ExpiryTime(_)))
            .count()
            > 1
    {
        return Err(WalletError::Invalid("BTC-only regtest invoice required"));
    }
    let msat = amount
        .checked_mul(1000)
        .filter(|n| *n > 0)
        .ok_or(WalletError::Invalid("invalid BTC amount"))?;
    if decoded.amount_milli_satoshis() != Some(msat) {
        return Err(WalletError::Invalid("BTC invoice amount mismatch"));
    }
    let issued = decoded.duration_since_epoch().as_secs();
    let expires = issued
        .checked_add(decoded.expiry_time().as_secs())
        .ok_or(WalletError::Invalid("expiry overflow"))?;
    if issued > now || expires <= now {
        return Err(WalletError::Invalid("BTC invoice expired or future dated"));
    }
    Ok(LightningInvoice {
        invoice: invoice.into(),
        payment_hash: PaymentId::new(decoded.payment_hash().to_string())?,
        amount_sats: amount,
        expires_at: expires,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        hashes::{sha256, Hash},
        secp256k1::{Secp256k1, SecretKey},
    };
    use lightning_invoice::{InvoiceBuilder, PaymentSecret};
    fn invoice(msat: Option<u64>, currency: Currency, rgb: bool) -> String {
        let mut b = InvoiceBuilder::new(currency)
            .description("public test fixture".into())
            .payment_hash(sha256::Hash::from_slice(&[1; 32]).unwrap())
            .payment_secret(PaymentSecret([2; 32]))
            .duration_since_epoch(Duration::from_secs(100))
            .min_final_cltv_expiry_delta(18)
            .expiry_time(Duration::from_secs(100));
        if let Some(msat) = msat {
            b = b.amount_milli_satoshis(msat)
        }
        if rgb {
            b = b.rgb_amount(5)
        }
        b.build_signed(|h| {
            Secp256k1::new().sign_ecdsa_recoverable(h, &SecretKey::from_slice(&[1; 32]).unwrap())
        })
        .unwrap()
        .to_string()
    }
    #[test]
    fn btc_local_decoder_enforces_exact_amount_currency_expiry_and_no_rgb() {
        let valid = invoice(Some(5000), Currency::Regtest, false);
        assert_eq!(validate_invoice(&valid, 5, 150).unwrap().amount_sats, 5);
        assert!(validate_invoice(&valid, 6, 150).is_err());
        assert!(validate_invoice(&valid, 5, 200).is_err());
        assert!(validate_invoice(&valid, 5, 99).is_err());
        for candidate in [
            invoice(None, Currency::Regtest, false),
            invoice(Some(5001), Currency::Regtest, false),
            invoice(Some(5000), Currency::Bitcoin, false),
            invoice(Some(5000), Currency::Regtest, true),
        ] {
            assert!(validate_invoice(&candidate, 5, 150).is_err())
        }
    }
    #[test]
    fn discovery_requires_btc_relation_and_same_origin_subject() {
        let account = Account::parse("alice@example.com").unwrap();
        for (relation, subject, url, ok) in [
            (
                BTC_INVOICE_REL,
                "acct:alice@example.com",
                "https://example.com/btc/alice",
                true,
            ),
            (
                RGB_INVOICE_REL,
                "acct:alice@example.com",
                "https://example.com/rgb/alice",
                false,
            ),
            (
                BTC_INVOICE_REL,
                "acct:bob@example.com",
                "https://example.com/btc/alice",
                false,
            ),
            (
                BTC_INVOICE_REL,
                "acct:alice@example.com",
                "https://evil.example/btc/alice",
                false,
            ),
        ] {
            let response = Response {
                status: 200,
                content_type: "application/jrd+json".into(),
                body: serde_json::to_vec(
                    &json!({"subject":subject,"links":[{"rel":relation,"href":url}]}),
                )
                .unwrap(),
            };
            assert_eq!(
                validate_relation_response(&account, response, BTC_INVOICE_REL).is_ok(),
                ok
            );
        }
    }
}
