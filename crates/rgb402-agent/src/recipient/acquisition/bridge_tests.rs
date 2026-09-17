use super::*;
use crate::recipient::bridge::prepare_recipient_payment;
use rgb402_core::{
    wallet::{PolicyDecision, WalletBalance, WalletPolicy},
    PaymentStatus,
};
use rgb402_payment::{
    harness::State,
    rgb::RgbNode,
    wallet::{ApprovedPayment, PaymentResult, WalletError, WalletService},
};
use std::sync::{Arc, Mutex};

struct Node {
    request: Mutex<PaymentRequest>,
    sends: AtomicUsize,
    decodes: AtomicUsize,
    fail: bool,
}
#[async_trait]
impl RgbNode for Node {
    async fn list_assets(&self) -> Result<Vec<rgb402_core::wallet::Asset>, WalletError> {
        panic!("not needed")
    }
    async fn asset_balance(&self, asset: &AssetId) -> Result<WalletBalance, WalletError> {
        Ok(WalletBalance {
            asset_id: asset.clone(),
            onchain_spendable: 0,
            offchain_outbound: 100,
        })
    }
    async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError> {
        self.decodes.fetch_add(1, Ordering::SeqCst);
        let request = self.request.lock().unwrap().clone();
        assert_eq!(request.invoice, invoice);
        Ok(request)
    }
    async fn send_payment(&self, payment: &ApprovedPayment) -> Result<PaymentResult, WalletError> {
        assert_eq!(payment.request(), &*self.request.lock().unwrap());
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(WalletError::Node("uncertain"));
        }
        Ok(PaymentResult {
            payment_id: payment.request().payment_hash.clone(),
            payment_hash: payment.request().payment_hash.clone(),
            status: PaymentStatus::Pending,
        })
    }
    async fn payment_status(&self, _: &PaymentId) -> Result<PaymentStatus, WalletError> {
        Ok(PaymentStatus::Settled)
    }
}
fn policy() -> WalletPolicy {
    WalletPolicy {
        allowed_assets: [AssetId::new(ASSET).unwrap()].into(),
        auto_approve_below: 5,
        max_single_payment: 10,
        max_daily_spend: 100,
        max_carrier_msat: 3_000_000,
    }
}
fn path() -> std::path::PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "recipient-bridge-{}-{}-{}.jsonl",
        std::process::id(),
        UnixTimestamp::now().seconds(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ))
}
async fn candidate(fresh: bool) -> ValidatedRecipientInvoice {
    let issued = if fresh {
        UnixTimestamp::now().seconds()
    } else {
        100
    };
    acquire_with(
        &contract(),
        &Service::new(invoice_at(5, "rgb", issued)),
        || issued,
        TIMEOUT,
    )
    .await
    .unwrap()
}
fn node(candidate: &ValidatedRecipientInvoice, fail: bool) -> Arc<Node> {
    Arc::new(Node {
        request: Mutex::new(candidate.request().clone()),
        sends: AtomicUsize::new(0),
        decodes: AtomicUsize::new(0),
        fail,
    })
}

#[tokio::test]
async fn bridge_equals_direct_economics_and_preserves_provenance_without_authority() {
    let candidate = candidate(true).await;
    let node = node(&candidate, false);
    let path = path();
    let mut wallet = WalletService::open(node.clone(), policy(), &path).unwrap();
    let direct = wallet
        .prepare_payment(&candidate.request().invoice)
        .await
        .unwrap();
    let bridged = prepare_recipient_payment(&mut wallet, &candidate, &contract())
        .await
        .unwrap();
    assert_eq!(direct.request(), bridged.request());
    assert_eq!(direct.policy(), bridged.policy());
    assert_eq!(direct.available_balance(), bridged.available_balance());
    assert_eq!(
        wallet
            .plan_task(direct.plan_id())
            .unwrap()
            .economic_action_id,
        wallet
            .plan_task(bridged.plan_id())
            .unwrap()
            .economic_action_id
    );
    assert_eq!(
        wallet.plan_task(bridged.plan_id()).unwrap().state,
        State::AwaitingAuthorization
    );
    assert_eq!(node.decodes.load(Ordering::SeqCst), 2); // One decode per preparation, no second acquisition.
    assert!(matches!(
        wallet.execute_payment(bridged.plan_id()).await,
        Err(WalletError::ApprovalRequired)
    ));
    assert_eq!(node.sends.load(Ordering::SeqCst), 0);
    let provenance = bridged.recipient_provenance().unwrap().clone();
    assert_eq!(provenance.identifier, "alice@example.com");
    assert_eq!(provenance.authoritative_domain, "example.com");
    assert_eq!(provenance.recipient_contract_digest, contract().digest());
    assert_eq!(provenance.invoice_id, candidate.invoice_id());
    wallet.approve_from_human(bridged.plan_id()).unwrap();
    let result = wallet.execute_payment(bridged.plan_id()).await.unwrap();
    assert_eq!(
        wallet.recipient_provenance(&result.payment_hash),
        Some(&provenance)
    );
    assert_eq!(
        wallet.payment_status(&result.payment_hash).await.unwrap(),
        PaymentStatus::Settled
    );
    assert!(matches!(
        wallet.execute_payment(bridged.plan_id()).await,
        Err(WalletError::UnknownPlan)
    ));
    wallet.approve_from_human(direct.plan_id()).unwrap();
    assert!(matches!(
        wallet.execute_payment(direct.plan_id()).await,
        Err(WalletError::Duplicate)
    ));
    assert_eq!(node.sends.load(Ordering::SeqCst), 1);
    drop(wallet);
    let row: Value = serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
    assert_eq!(
        row["recipient_provenance"]["payment_hash"],
        serde_json::to_value(&result.payment_hash).unwrap()
    );
    assert_eq!(
        row["authority"]["economic_action_id"],
        serde_json::to_value(
            WalletService::open(node.clone(), policy(), &path)
                .unwrap()
                .task(&result.payment_hash)
                .unwrap()
                .economic_action_id
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(&path)
            .unwrap()
            .matches(&candidate.request().invoice)
            .count(),
        1
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn uncertain_submission_restart_keeps_provenance_and_cannot_resubmit() {
    let candidate = candidate(true).await;
    let node = node(&candidate, true);
    let path = path();
    let mut wallet = WalletService::open(node.clone(), policy(), &path).unwrap();
    let plan = prepare_recipient_payment(&mut wallet, &candidate, &contract())
        .await
        .unwrap();
    let provenance = plan.recipient_provenance().unwrap().clone();
    wallet.approve_from_human(plan.plan_id()).unwrap();
    assert!(wallet.execute_payment(plan.plan_id()).await.is_err());
    drop(wallet);
    let mut recovered = WalletService::open(node.clone(), policy(), &path).unwrap();
    assert_eq!(
        recovered.recipient_provenance(&candidate.request().payment_hash),
        Some(&provenance)
    );
    assert_eq!(
        recovered
            .task(&candidate.request().payment_hash)
            .unwrap()
            .state,
        State::Uncertain
    );
    assert!(matches!(
        recovered.execute_payment(plan.plan_id()).await,
        Err(WalletError::UnknownPlan)
    ));
    let replay = prepare_recipient_payment(&mut recovered, &candidate, &contract())
        .await
        .unwrap();
    assert!(matches!(
        recovered.execute_payment(replay.plan_id()).await,
        Err(WalletError::ApprovalRequired)
    ));
    recovered.approve_from_human(replay.plan_id()).unwrap();
    assert!(matches!(
        recovered.execute_payment(replay.plan_id()).await,
        Err(WalletError::Duplicate)
    ));
    assert_eq!(
        recovered
            .payment_status(&candidate.request().payment_hash)
            .await
            .unwrap(),
        PaymentStatus::Settled
    );
    assert_eq!(node.sends.load(Ordering::SeqCst), 1);
    drop(recovered);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn expired_acquisition_and_changed_decode_cannot_execute() {
    let expired = candidate(false).await;
    let expired_node = node(&expired, false);
    let p = path();
    let mut wallet = WalletService::open(expired_node.clone(), policy(), &p).unwrap();
    let plan = prepare_recipient_payment(&mut wallet, &expired, &contract())
        .await
        .unwrap();
    assert!(matches!(plan.policy(), PolicyDecision::Deny { .. }));
    wallet.approve_from_human(plan.plan_id()).unwrap();
    assert!(matches!(
        wallet.execute_payment(plan.plan_id()).await,
        Err(WalletError::Denied(_))
    ));
    assert_eq!(expired_node.sends.load(Ordering::SeqCst), 0);
    drop(wallet);
    std::fs::remove_file(p).unwrap();
    let candidate = candidate(true).await;
    let node = node(&candidate, false);
    let p = path();
    let mut wallet = WalletService::open(node.clone(), policy(), &p).unwrap();
    node.request.lock().unwrap().amount = 6;
    assert!(matches!(
        prepare_recipient_payment(&mut wallet, &candidate, &contract()).await,
        Err(WalletError::Invalid(_))
    ));
    *node.request.lock().unwrap() = candidate.request().clone();
    let plan = prepare_recipient_payment(&mut wallet, &candidate, &contract())
        .await
        .unwrap();
    wallet.approve_from_human(plan.plan_id()).unwrap();
    node.request.lock().unwrap().amount = 6;
    assert!(matches!(
        wallet.execute_payment(plan.plan_id()).await,
        Err(WalletError::Invalid(_))
    ));
    assert_eq!(node.sends.load(Ordering::SeqCst), 0);
    drop(wallet);
    std::fs::remove_file(p).unwrap();
}

#[tokio::test]
async fn mismatched_or_forged_digest_stops_before_harness_and_legacy_stays_readable() {
    let candidate = candidate(true).await;
    let node = node(&candidate, false);
    let p = path();
    let mut wallet = WalletService::open(node.clone(), policy(), &p).unwrap();
    let mut other = RecipientInvoiceContract::new(
        contract().recipient.clone(),
        AssetId::new(ASSET).unwrap(),
        6,
        3_000_000,
    )
    .unwrap();
    assert!(prepare_recipient_payment(&mut wallet, &candidate, &other)
        .await
        .is_err());
    other.contract_digest = contract().digest().into();
    assert!(prepare_recipient_payment(&mut wallet, &candidate, &other)
        .await
        .is_err());
    assert_eq!(node.decodes.load(Ordering::SeqCst), 0);
    assert!(wallet.payment_history().is_empty());
    drop(wallet);
    std::fs::write(
        &p,
        format!(
            "{}\n",
            json!({"request":candidate.request(),"at":UnixTimestamp::now().seconds()})
        ),
    )
    .unwrap();
    let mut legacy = WalletService::open(node.clone(), policy(), &p).unwrap();
    assert!(legacy
        .recipient_provenance(&candidate.request().payment_hash)
        .is_none());
    let plan = prepare_recipient_payment(&mut legacy, &candidate, &contract())
        .await
        .unwrap();
    legacy.approve_from_human(plan.plan_id()).unwrap();
    assert!(matches!(
        legacy.execute_payment(plan.plan_id()).await,
        Err(WalletError::Duplicate)
    ));
    assert_eq!(node.sends.load(Ordering::SeqCst), 0);
    drop(legacy);
    std::fs::remove_file(p).unwrap();
}

#[tokio::test]
async fn invoice_expiring_after_approval_is_rejected_at_execution() {
    let issued = UnixTimestamp::now().seconds() - 95;
    let candidate = acquire_with(
        &contract(),
        &Service::new(invoice_at(5, "rgb", issued)),
        || issued,
        TIMEOUT,
    )
    .await
    .unwrap();
    let node = node(&candidate, false);
    let p = path();
    let mut wallet = WalletService::open(node.clone(), policy(), &p).unwrap();
    let plan = prepare_recipient_payment(&mut wallet, &candidate, &contract())
        .await
        .unwrap();
    assert!(matches!(
        plan.policy(),
        PolicyDecision::RequireApproval { .. }
    ));
    wallet.approve_from_human(plan.plan_id()).unwrap();
    tokio::time::sleep(Duration::from_secs(
        candidate
            .request()
            .expires_at
            .saturating_sub(UnixTimestamp::now().seconds())
            + 1,
    ))
    .await;
    assert!(matches!(
        wallet.execute_payment(plan.plan_id()).await,
        Err(WalletError::Denied(_))
    ));
    assert_eq!(node.sends.load(Ordering::SeqCst), 0);
    assert!(wallet.payment_history().is_empty());
    drop(wallet);
    std::fs::remove_file(p).unwrap();
}

async fn alternate_candidate(
    alice: &ValidatedRecipientInvoice,
    change: &str,
) -> ValidatedRecipientInvoice {
    let mut recipient = contract().recipient.clone();
    match change {
        "bob" => {
            recipient.identifier = "bob@example.com".into();
            recipient.subject = "acct:bob@example.com".into();
        }
        "domain" => {
            recipient.identifier = "alice@other.example".into();
            recipient.subject = "acct:alice@other.example".into();
            recipient.authoritative_domain = "other.example".into();
            recipient.service.url = "https://other.example/invoice/alice".into();
        }
        "service" => recipient.service.url = "https://example.com/another-service".into(),
        _ => unreachable!(),
    }
    let intent =
        RecipientInvoiceContract::new(recipient, contract().requested_asset.clone(), 5, 3_000_000)
            .unwrap();
    acquire_with(
        &intent,
        &Service::new(alice.request().invoice.clone()),
        || alice.validated_at(),
        TIMEOUT,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn same_invoice_distinct_recipient_scopes_require_separate_human_approval() {
    for approve_alice in [true, false] {
        let alice = candidate(true).await;
        let bob = alternate_candidate(&alice, "bob").await;
        let node = node(&alice, false);
        let p = path();
        let mut wallet = WalletService::open(node.clone(), policy(), &p).unwrap();
        let a = prepare_recipient_payment(&mut wallet, &alice, alice.contract())
            .await
            .unwrap();
        let b = prepare_recipient_payment(&mut wallet, &bob, bob.contract())
            .await
            .unwrap();
        let direct = wallet
            .prepare_payment(&alice.request().invoice)
            .await
            .unwrap();
        let economic = wallet.plan_task(a.plan_id()).unwrap().economic_action_id;
        assert_eq!(
            economic,
            wallet.plan_task(b.plan_id()).unwrap().economic_action_id
        );
        assert_eq!(
            economic,
            wallet
                .plan_task(direct.plan_id())
                .unwrap()
                .economic_action_id
        );
        assert_eq!(a.request(), b.request());
        assert_eq!(a.request().payment_hash, b.request().payment_hash);
        assert_eq!(direct.authorization_scope(), economic);
        assert_ne!(a.authorization_scope(), b.authorization_scope());
        assert_ne!(a.authorization_scope(), direct.authorization_scope());
        let expected = digest(
            &serde_json::to_vec(&(
                "rgb402-recipient-authorization-v1",
                &economic,
                alice.contract().digest(),
            ))
            .unwrap(),
        );
        assert_eq!(a.authorization_scope(), expected);
        for change in ["domain", "service"] {
            let alternate = alternate_candidate(&alice, change).await;
            let plan = prepare_recipient_payment(&mut wallet, &alternate, alternate.contract())
                .await
                .unwrap();
            assert_ne!(a.authorization_scope(), plan.authorization_scope());
            assert_eq!(
                economic,
                wallet.plan_task(plan.plan_id()).unwrap().economic_action_id
            );
        }
        let (approved, other) = if approve_alice { (&a, &b) } else { (&b, &a) };
        wallet.approve_from_human(approved.plan_id()).unwrap();
        assert!(matches!(
            wallet.execute_payment(other.plan_id()).await,
            Err(WalletError::ApprovalRequired)
        ));
        assert_eq!(node.sends.load(Ordering::SeqCst), 0);
        let result = wallet.execute_payment(approved.plan_id()).await.unwrap();
        assert_eq!(
            wallet.recipient_authorization_scope(&result.payment_hash),
            Some(approved.authorization_scope())
        );
        wallet.approve_from_human(other.plan_id()).unwrap();
        assert!(matches!(
            wallet.execute_payment(other.plan_id()).await,
            Err(WalletError::Duplicate)
        ));
        wallet.approve_from_human(direct.plan_id()).unwrap();
        assert!(matches!(
            wallet.execute_payment(direct.plan_id()).await,
            Err(WalletError::Duplicate)
        ));
        assert_eq!(node.sends.load(Ordering::SeqCst), 1);
        drop(wallet);
        let row: Value = serde_json::from_str(std::fs::read_to_string(&p).unwrap().trim()).unwrap();
        assert_eq!(row["authority"]["source"], "human");
        let mut recovered = WalletService::open(node.clone(), policy(), &p).unwrap();
        assert_eq!(
            recovered.recipient_authorization_scope(&result.payment_hash),
            Some(approved.authorization_scope())
        );
        assert!(matches!(
            recovered.execute_payment(approved.plan_id()).await,
            Err(WalletError::UnknownPlan)
        ));
        let replay = prepare_recipient_payment(&mut recovered, &bob, bob.contract())
            .await
            .unwrap();
        assert!(matches!(
            recovered.execute_payment(replay.plan_id()).await,
            Err(WalletError::ApprovalRequired)
        ));
        assert_eq!(node.sends.load(Ordering::SeqCst), 1);
        drop(recovered);
        std::fs::remove_file(p).unwrap();
    }
}

#[tokio::test]
async fn automatic_recipient_scope_is_durable_and_recovery_rejects_mismatch() {
    let alice = candidate(true).await;
    let node = node(&alice, false);
    let p = path();
    let mut automatic = policy();
    automatic.auto_approve_below = 6;
    let mut wallet = WalletService::open(node.clone(), automatic.clone(), &p).unwrap();
    let plan = prepare_recipient_payment(&mut wallet, &alice, alice.contract())
        .await
        .unwrap();
    assert_eq!(plan.policy(), &PolicyDecision::Allow);
    let result = wallet.execute_payment(plan.plan_id()).await.unwrap();
    assert_eq!(
        wallet.recipient_authorization_scope(&result.payment_hash),
        Some(plan.authorization_scope())
    );
    drop(wallet);
    let mut row: Value = serde_json::from_str(std::fs::read_to_string(&p).unwrap().trim()).unwrap();
    assert_eq!(row["authority"]["source"], "automatic");
    assert_eq!(
        row["recipient_authorization_scope"],
        plan.authorization_scope()
    );
    let mut recovered = WalletService::open(node.clone(), automatic.clone(), &p).unwrap();
    assert_eq!(
        recovered.recipient_authorization_scope(&result.payment_hash),
        Some(plan.authorization_scope())
    );
    let bob = alternate_candidate(&alice, "bob").await;
    let replay = prepare_recipient_payment(&mut recovered, &bob, bob.contract())
        .await
        .unwrap();
    assert_eq!(replay.policy(), &PolicyDecision::Allow);
    assert!(matches!(
        recovered.execute_payment(replay.plan_id()).await,
        Err(WalletError::Duplicate)
    ));
    assert_eq!(node.sends.load(Ordering::SeqCst), 1);
    drop(recovered);
    row["recipient_authorization_scope"] = json!("0".repeat(64));
    std::fs::write(&p, format!("{row}\n")).unwrap();
    assert!(WalletService::open(node.clone(), automatic.clone(), &p).is_err());
    // Pre-scope recipient rows remain readable, with no invented historical scope.
    row.as_object_mut()
        .unwrap()
        .remove("recipient_authorization_scope");
    std::fs::write(&p, format!("{row}\n")).unwrap();
    let legacy = WalletService::open(node, automatic, &p).unwrap();
    assert!(legacy
        .recipient_authorization_scope(&result.payment_hash)
        .is_none());
    assert_eq!(
        legacy.task(&result.payment_hash).unwrap().state,
        State::Uncertain
    );
    drop(legacy);
    std::fs::remove_file(p).unwrap();
}

#[path = "agent_tests.rs"]
mod agent_tests;
