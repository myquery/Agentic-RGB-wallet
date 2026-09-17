use super::*;
use bitcoin::{
    hashes::{sha256, Hash},
    secp256k1::{Secp256k1, SecretKey},
};
use lightning_invoice::{InvoiceBuilder, PaymentSecret};
use std::sync::atomic::{AtomicUsize, Ordering};
const ASSET: &str = "rgb:a45R_GaI-6nlzwJO-b7Rg0bQ-XHDQVz0-Zh98TMR-LeuLP4M";
fn contract() -> RecipientInvoiceContract {
    let account = super::super::Account::parse("alice@example.com").unwrap();
    let recipient = super::super::validate_response(&account, Response { status:200, content_type:"application/jrd+json".into(), body:serde_json::to_vec(&json!({"subject":account.subject(),"links":[{"rel":super::super::RGB_INVOICE_REL,"href":"https://example.com/invoice/alice"}]})).unwrap() }).unwrap();
    RecipientInvoiceContract::new(recipient, AssetId::new(ASSET).unwrap(), 5, 3_000_000).unwrap()
}
fn invoice(amount: u64, form: &str) -> String {
    invoice_at(amount, form, 100)
}
fn invoice_at(amount: u64, form: &str, issued_at: u64) -> String {
    let currency = if form == "network" {
        Currency::Bitcoin
    } else {
        Currency::Regtest
    };
    let mut builder = InvoiceBuilder::new(currency)
        .description("offline test".into())
        .payment_hash(sha256::Hash::from_slice(&[1; 32]).unwrap())
        .payment_secret(PaymentSecret([2; 32]))
        .duration_since_epoch(Duration::from_secs(issued_at))
        .min_final_cltv_expiry_delta(18)
        .expiry_time(Duration::from_secs(100));
    if form != "amountless" {
        builder = builder.amount_milli_satoshis(if form == "carrier" {
            3_000_001
        } else {
            3_000_000
        });
    }
    if form != "btc" {
        builder = builder
            .rgb_contract_id(ASSET.parse().unwrap())
            .rgb_amount(amount);
    }
    if form == "duplicate_asset" {
        builder = builder.rgb_contract_id(ASSET.parse().unwrap());
    }
    if form == "duplicate_expiry" {
        builder = builder.expiry_time(Duration::from_secs(200));
    }
    if form == "ambiguous" {
        builder = builder.rgb_amount(amount + 1);
    }
    // Public deterministic fixture signing material, unrelated to any wallet/node key.
    let key = SecretKey::from_slice(&[1; 32]).unwrap();
    builder
        .build_signed(|hash| Secp256k1::new().sign_ecdsa_recoverable(hash, &key))
        .unwrap()
        .to_string()
}
struct Service {
    body: Vec<u8>,
    status: u16,
    media: &'static str,
    error: Option<DiscoveryError>,
    calls: AtomicUsize,
}
impl Service {
    fn new(invoice: String) -> Self {
        Self {
            body: serde_json::to_vec(&json!({"invoice":invoice})).unwrap(),
            status: 200,
            media: "application/json",
            error: None,
            calls: AtomicUsize::new(0),
        }
    }
}
#[async_trait]
impl InvoiceService for Service {
    async fn request(
        &self,
        contract: &RecipientInvoiceContract,
        payload: &Value,
    ) -> Result<Response, DiscoveryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            payload,
            &json!({"subject":contract.recipient.subject(),"asset_id":contract.requested_asset,"asset_amount":contract.requested_amount})
        );
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(Response {
            status: self.status,
            content_type: self.media.into(),
            body: self.body.clone(),
        })
    }
}
async fn run(service: &Service) -> Result<ValidatedRecipientInvoice, AcquisitionError> {
    acquire_with(&contract(), service, || 150, TIMEOUT).await
}
fn reject(result: Result<ValidatedRecipientInvoice, AcquisitionError>, error: AcquisitionError) {
    assert!(matches!(result, Err(e) if e == error));
}

#[tokio::test]
async fn successful_local_validation_requires_no_node_wallet_or_harness() {
    // Only capability supplied is an invoice service. No node/decoder mock or economic capability exists.
    let service = Service::new(invoice(5, "rgb"));
    let result = run(&service).await.unwrap();
    assert_eq!(result.request().amount, 5);
    assert_eq!(result.request().asset_id.as_str(), ASSET);
    assert_eq!(result.request().expires_at, 200);
    assert_eq!(result.issued_at(), 100);
    assert_eq!(result.validated_at(), 150);
    assert!(result.matches_contract(&contract()));
    assert_eq!(
        result.invoice_id(),
        digest(result.request().invoice.as_bytes())
    );
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn asset_amount_network_and_carrier_must_match_intent() {
    reject(
        run(&Service::new(invoice(6, "rgb"))).await,
        AcquisitionError::AmountMismatch,
    );
    reject(
        run(&Service::new(invoice(5, "network"))).await,
        AcquisitionError::NetworkMismatch,
    );
    reject(
        run(&Service::new(invoice(5, "carrier"))).await,
        AcquisitionError::CarrierLimit,
    );
    let c = contract();
    let other = RecipientInvoiceContract::new(
        c.recipient.clone(),
        AssetId::new("rgb:other").unwrap(),
        5,
        3_000_000,
    )
    .unwrap();
    reject(
        acquire_with(&other, &Service::new(invoice(5, "rgb")), || 150, TIMEOUT).await,
        AcquisitionError::AssetMismatch,
    );
}
#[tokio::test]
async fn malformed_and_unsupported_forms_fail_locally() {
    reject(
        run(&Service::new("not an invoice".into())).await,
        AcquisitionError::MalformedInvoice,
    );
    for form in [
        "btc",
        "amountless",
        "ambiguous",
        "duplicate_asset",
        "duplicate_expiry",
    ] {
        reject(
            run(&Service::new(invoice(5, form))).await,
            AcquisitionError::UnsupportedInvoice,
        );
    }
    reject(
        run(&Service::new(invoice(0, "rgb"))).await,
        AcquisitionError::UnsupportedInvoice,
    );
    let mut corrupt = invoice(5, "rgb").into_bytes();
    let last = corrupt.len() - 1;
    corrupt[last] = if corrupt[last] == b'q' { b'p' } else { b'q' };
    reject(
        run(&Service::new(String::from_utf8(corrupt).unwrap())).await,
        AcquisitionError::MalformedInvoice,
    );
}
#[tokio::test]
async fn expiry_is_enforced_but_replay_is_not_claimed_as_fresh() {
    let c = contract();
    let service = Service::new(invoice(5, "rgb"));
    reject(
        acquire_with(&c, &service, || 200, TIMEOUT).await,
        AcquisitionError::ExpiredInvoice,
    );
    reject(
        acquire_with(&c, &service, || 99, TIMEOUT).await,
        AcquisitionError::FutureTimestamp,
    );
    let a = acquire_with(&c, &service, || 150, TIMEOUT).await.unwrap();
    let b = acquire_with(&c, &service, || 151, TIMEOUT).await.unwrap();
    assert_eq!(a.invoice_id(), b.invoice_id());
    assert_eq!(
        b.model_observation()["freshness"],
        "unexpired_at_validation_only"
    );
}
#[tokio::test]
async fn result_cannot_be_rebound_to_changed_contract() {
    let result = run(&Service::new(invoice(5, "rgb"))).await.unwrap();
    let c = contract();
    for change in [
        "identifier",
        "domain",
        "service",
        "asset",
        "amount",
        "carrier",
    ] {
        let mut recipient = c.recipient.clone();
        match change {
            "identifier" => {
                recipient.identifier = "bob@example.com".into();
                recipient.subject = "acct:bob@example.com".into();
            }
            "domain" => recipient.authoritative_domain = "other.example".into(),
            "service" => recipient.service.url = "https://example.com/other".into(),
            _ => (),
        }
        let other = RecipientInvoiceContract::new(
            recipient,
            if change == "asset" {
                AssetId::new("rgb:other").unwrap()
            } else {
                c.requested_asset.clone()
            },
            if change == "amount" { 6 } else { 5 },
            if change == "carrier" {
                4_000_000
            } else {
                3_000_000
            },
        )
        .unwrap();
        assert!(!result.matches_contract(&other), "{change}");
    }
}
#[tokio::test]
async fn bounded_projection_omits_raw_invoice_endpoint_and_secrets() {
    let result = run(&Service::new(invoice(5, "rgb"))).await.unwrap();
    let observation = result.model_observation();
    let text = serde_json::to_string(&observation).unwrap();
    assert!(text.len() <= MAX_OBSERVATION_BYTES);
    assert!(!text.contains("lnbcrt") && !text.contains("/invoice/alice"));
    assert_eq!(observation["payment_authorized"], false);
    assert_eq!(observation["next_allowed_actions"], json!([]));
    assert_eq!(observation["amount"], "5");
}
#[tokio::test]
async fn malformed_media_size_redirect_and_https_fail_closed() {
    for (status, error) in [
        (302, AcquisitionError::SecurityRejected),
        (503, AcquisitionError::Unavailable),
    ] {
        let mut service = Service::new(invoice(5, "rgb"));
        service.status = status;
        reject(run(&service).await, error);
        assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    }
    for error in [
        DiscoveryError::HttpsFailure,
        DiscoveryError::DisallowedOrigin,
        DiscoveryError::Timeout,
    ] {
        let mut service = Service::new(invoice(5, "rgb"));
        service.error = Some(error.clone());
        reject(run(&service).await, error.into());
    }
    let mut service = Service::new(invoice(5, "rgb"));
    service.media = "text/html";
    reject(run(&service).await, AcquisitionError::WrongMediaType);
    service.media = "application/json";
    service.body = vec![b' '; MAX_RESPONSE_BYTES + 1];
    reject(run(&service).await, AcquisitionError::ResponseTooLarge);
    for body in [
        "{}",
        "null",
        "{\"invoice\":5}",
        "{\"invoice\":\"x\",\"asset_amount\":5}",
    ] {
        service.body = body.as_bytes().to_vec();
        reject(run(&service).await, AcquisitionError::MalformedResponse);
    }
}
struct Hanging;
#[async_trait]
impl InvoiceService for Hanging {
    async fn request(
        &self,
        _: &RecipientInvoiceContract,
        _: &Value,
    ) -> Result<Response, DiscoveryError> {
        std::future::pending().await
    }
}
#[tokio::test]
async fn one_total_deadline() {
    reject(
        acquire_with(&contract(), &Hanging, || 150, Duration::from_millis(1)).await,
        AcquisitionError::Timeout,
    );
}

#[tokio::test]
async fn maximum_identity_projection_and_invalid_intent_are_bounded() {
    let c = contract();
    for (amount, carrier) in [(0, 3_000_000), (5, 0)] {
        assert!(RecipientInvoiceContract::new(
            c.recipient.clone(),
            c.requested_asset.clone(),
            amount,
            carrier
        )
        .is_err());
    }
    assert!(RecipientInvoiceContract::new(
        c.recipient.clone(),
        AssetId::new(format!("rgb:{}", "a".repeat(256))).unwrap(),
        5,
        3_000_000
    )
    .is_err());
    let mut recipient = c.recipient.clone();
    let domain = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    recipient.identifier = format!("{}@{domain}", "a".repeat(64));
    recipient.subject = format!("acct:{}", recipient.identifier);
    recipient.authoritative_domain = domain.clone();
    recipient.service.url = format!("https://{domain}/invoice");
    let intent =
        RecipientInvoiceContract::new(recipient, c.requested_asset.clone(), 5, 3_000_000).unwrap();
    let result = acquire_with(&intent, &Service::new(invoice(5, "rgb")), || 150, TIMEOUT)
        .await
        .unwrap();
    assert!(
        serde_json::to_vec(&result.model_observation())
            .unwrap()
            .len()
            <= MAX_OBSERVATION_BYTES
    );
}

#[path = "bridge_tests.rs"]
mod bridge_tests;

#[test]
fn node_asset_with_tilde_passes_contract_and_local_invoice_validation() {
    let asset = "rgb:KigwNgFx-bh7pHa~-Q7gi49D-ncmlxS5-~~44UbJ-g0Ienok";
    let mut c = contract();
    c = RecipientInvoiceContract::new(
        c.recipient.clone(),
        AssetId::new(asset).unwrap(),
        5,
        3_000_000,
    )
    .unwrap();
    let key = SecretKey::from_slice(&[1; 32]).unwrap();
    let invoice = InvoiceBuilder::new(Currency::Regtest)
        .description("offline tilde regression".into())
        .payment_hash(sha256::Hash::from_slice(&[3; 32]).unwrap())
        .payment_secret(PaymentSecret([4; 32]))
        .duration_since_epoch(Duration::from_secs(100))
        .min_final_cltv_expiry_delta(18)
        .expiry_time(Duration::from_secs(100))
        .amount_milli_satoshis(3_000_000)
        .rgb_contract_id(asset.parse().unwrap())
        .rgb_amount(5)
        .build_signed(|hash| Secp256k1::new().sign_ecdsa_recoverable(hash, &key))
        .unwrap()
        .to_string();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::new(invoice);
    let validated = runtime
        .block_on(acquire_with(&c, &service, || 150, TIMEOUT))
        .unwrap();
    assert_eq!(validated.request().asset_id.as_str(), asset);
    for suffix in ["/", "?", "#", "%", " ", "é"] {
        assert!(RecipientInvoiceContract::new(
            c.recipient.clone(),
            AssetId::new(format!("{asset}{suffix}")).unwrap(),
            5,
            3_000_000
        )
        .is_err());
    }
}
