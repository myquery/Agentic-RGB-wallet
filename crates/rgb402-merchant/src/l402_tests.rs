use super::*;
use async_trait::async_trait;
use rgb402_core::{
    machine::{MachineDecision, MachinePolicy},
    PaymentId, PaymentStatus,
};
use rgb402_payment::{
    commerce::{CommerceConfig, CommerceService},
    lightning::{ApprovedMachinePayment, LightningPayment},
    wallet::PaymentResult,
};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

struct Node {
    invoices: std::sync::Mutex<HashMap<String, LightningInvoice>>,
    sends: AtomicUsize,
    polls: AtomicUsize,
    issues: AtomicUsize,
    balance: AtomicU64,
    mode: AtomicUsize,
    amount_offset: AtomicU64,
}
impl Node {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            invoices: Default::default(),
            sends: AtomicUsize::new(0),
            polls: AtomicUsize::new(0),
            issues: AtomicUsize::new(0),
            balance: AtomicU64::new(1000),
            mode: AtomicUsize::new(0),
            amount_offset: AtomicU64::new(0),
        })
    }
    fn proof() -> String {
        hex::encode([7; 32])
    }
}
#[async_trait]
impl LightningNode for Node {
    async fn create_invoice(
        &self,
        amount: u64,
        _expiry: u32,
    ) -> Result<LightningInvoice, WalletError> {
        self.issues.fetch_add(1, Ordering::SeqCst);
        let hash = hex::encode(Sha256::digest([7; 32]));
        let invoice = LightningInvoice {
            invoice: format!("lnbcrt{amount}{hash}"),
            payment_hash: PaymentId::new(hash)?,
            amount_sats: amount,
            expires_at: now() + 600,
        };
        self.invoices
            .lock()
            .unwrap()
            .insert(invoice.invoice.clone(), invoice.clone());
        Ok(invoice)
    }
    async fn decode_btc_invoice(&self, invoice: &str) -> Result<LightningInvoice, WalletError> {
        let mut d = self
            .invoices
            .lock()
            .unwrap()
            .get(invoice)
            .cloned()
            .ok_or(WalletError::Invalid("unknown invoice"))?;
        d.amount_sats += self.amount_offset.load(Ordering::SeqCst);
        Ok(d)
    }
    async fn outbound_sats(&self) -> Result<u64, WalletError> {
        Ok(self.balance.load(Ordering::SeqCst))
    }
    async fn send_btc(&self, p: &ApprovedMachinePayment) -> Result<PaymentResult, WalletError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.mode.load(Ordering::SeqCst) == 3 {
            return Err(WalletError::Node("transport uncertain"));
        }
        Ok(PaymentResult {
            payment_id: p.invoice().payment_hash.clone(),
            payment_hash: p.invoice().payment_hash.clone(),
            status: PaymentStatus::Pending,
        })
    }
    async fn btc_payment(&self, _: &PaymentId) -> Result<LightningPayment, WalletError> {
        let poll = self.polls.fetch_add(1, Ordering::SeqCst);
        Ok(match self.mode.load(Ordering::SeqCst) {
            5 if poll < 15 => LightningPayment::new(PaymentStatus::Pending, None),
            1 => LightningPayment::new(PaymentStatus::Pending, None),
            2 => LightningPayment::new(PaymentStatus::Failed, None),
            3 => return Err(WalletError::Node("unavailable")),
            4 => LightningPayment::new(PaymentStatus::Settled, Some(hex::encode([8; 32]))),
            _ => LightningPayment::new(PaymentStatus::Settled, Some(Self::proof())),
        })
    }
}
struct Fixture {
    node: Arc<Node>,
    dir: std::path::PathBuf,
    origin: String,
    task: tokio::task::JoinHandle<()>,
    retry_fail: Arc<AtomicUsize>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
impl Fixture {
    async fn new() -> Self {
        static ID: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rgb402-commerce-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&dir).unwrap();
        let node = Node::new();
        let merchant = Merchant::open(node.clone(), &dir.join("key")).unwrap();
        let retry_fail = Arc::new(AtomicUsize::new(0));
        let flag = retry_fail.clone();
        let app = router(merchant).layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let flag = flag.clone();
                async move {
                    if request.headers().contains_key(header::AUTHORIZATION) {
                        match flag.load(Ordering::SeqCst) {
                            1 => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                            2 => tokio::time::sleep(std::time::Duration::from_secs(11)).await,
                            _ => (),
                        }
                    }
                    next.run(request).await
                }
            },
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            node,
            dir,
            origin,
            task,
            retry_fail,
        }
    }
    fn buyer(&self) -> CommerceService {
        CommerceService::open(
            self.node.clone(),
            CommerceConfig {
                origins: vec![self.origin.clone()],
                policy: MachinePolicy {
                    auto_approve_below_sats: 10,
                    max_single_payment_sats: 100,
                    max_daily_spend_sats: 500,
                },
                state_path: self.dir.join("buyer.jsonl"),
            },
        )
        .unwrap()
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
}
#[tokio::test]
async fn merchant_challenge_cached_and_valid_proof_resource_bound_and_expiring() {
    let f = Fixture::new().await;
    let client = reqwest::Client::new();
    let response = client.get(f.url("/premium/report")).send().await.unwrap();
    assert_eq!(response.status(), 402);
    let auth = response.headers()[header::WWW_AUTHENTICATE]
        .to_str()
        .unwrap()
        .to_owned();
    let challenge: L402Challenge = response.json().await.unwrap();
    assert_eq!(challenge.amount_sats, 3);
    assert!(challenge.invoice.starts_with("lnbcrt"));
    client.get(f.url("/premium/report")).send().await.unwrap();
    assert_eq!(f.node.issues.load(Ordering::SeqCst), 1);
    let token = auth
        .strip_prefix("L402 macaroon=\"")
        .unwrap()
        .split("\", invoice=")
        .next()
        .unwrap();
    let credential = format!("L402 {token}:{}", Node::proof());
    assert_eq!(
        client
            .get(f.url("/premium/report"))
            .header(header::AUTHORIZATION, &credential)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(f.url("/premium/extended"))
            .header(header::AUTHORIZATION, &credential)
            .send()
            .await
            .unwrap()
            .status(),
        402
    );
    assert_eq!(
        client
            .get(f.url("/premium/report"))
            .header(
                header::AUTHORIZATION,
                format!("L402 {token}:{}", hex::encode([0; 32]))
            )
            .send()
            .await
            .unwrap()
            .status(),
        402
    );
    let merchant = Merchant::open(f.node.clone(), &f.dir.join("key")).unwrap();
    assert!(merchant.authorized("/premium/report", &credential, now()));
    assert!(!merchant.authorized("/premium/report", &credential, now() + 3601));
    assert!(!merchant.authorized("/premium/report", "L402 invalid:00", now()));
}
#[tokio::test]
async fn free_resource_does_not_pay() {
    let f = Fixture::new().await;
    let r = f.buyer().fetch(&f.url("/public")).await.unwrap();
    assert_eq!(r.resource_status, "free");
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn auto_purchase_unlocks_and_duplicate_and_restart_do_not_repay() {
    let f = Fixture::new().await;
    let mut buyer = f.buyer();
    for _ in 0..2 {
        let r = buyer.fetch(&f.url("/premium/report")).await.unwrap();
        assert_eq!(r.resource_status, "purchased");
        assert_eq!(
            buyer.task(&f.url("/premium/report")).unwrap().state,
            rgb402_payment::harness::State::Complete
        );
        assert!(r.auto_approved);
        assert_eq!(r.cost_sats, 3);
        assert_eq!(r.http_status, Some(200));
        assert_eq!(r.resource.unwrap()["tier"], "premium");
    }
    drop(buyer);
    assert_eq!(
        f.buyer()
            .fetch(&f.url("/premium/report"))
            .await
            .unwrap()
            .resource_status,
        "purchased"
    );
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn above_threshold_requires_exact_application_approval() {
    let f = Fixture::new().await;
    let mut buyer = f.buyer();
    let r = buyer.fetch(&f.url("/premium/extended")).await.unwrap();
    let id = r.plan.unwrap().plan_id;
    assert_eq!(r.policy, MachineDecision::RequireApproval);
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    assert!(buyer.confirm_from_human("manufactured", true).is_err());
    buyer.fetch(&f.url("/premium/extended")).await.unwrap();
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
    buyer.confirm_from_human(&id, true).unwrap();
    let paid = buyer.fetch(&f.url("/premium/extended")).await.unwrap();
    assert_eq!(paid.resource_status, "purchased");
    assert!(!paid.auto_approved);
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn insufficient_balance_and_maximum_deny() {
    let f = Fixture::new().await;
    let mut buyer = f.buyer();
    f.node.balance.store(2, Ordering::SeqCst);
    assert_eq!(
        buyer
            .fetch(&f.url("/premium/report"))
            .await
            .unwrap()
            .resource_status,
        "denied"
    );
    drop(buyer);
    let mut buyer = CommerceService::open(
        f.node.clone(),
        CommerceConfig {
            origins: vec![f.origin.clone()],
            policy: MachinePolicy {
                auto_approve_below_sats: 10,
                max_single_payment_sats: 2,
                max_daily_spend_sats: 500,
            },
            state_path: f.dir.join("buyer.jsonl"),
        },
    )
    .unwrap();
    f.node.balance.store(1000, Ordering::SeqCst);
    assert_eq!(
        buyer
            .fetch(&f.url("/premium/report"))
            .await
            .unwrap()
            .resource_status,
        "denied"
    );
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn pending_failed_uncertain_and_bad_proof_never_unlock_or_repay() {
    for mode in 1..=4 {
        let f = Fixture::new().await;
        f.node.mode.store(mode, Ordering::SeqCst);
        let mut buyer = f.buyer();
        for _ in 0..2 {
            let r = buyer.fetch(&f.url("/premium/report")).await;
            if mode == 4 {
                assert!(r.is_err());
            } else {
                let r = r.unwrap();
                assert_ne!(r.resource_status, "purchased");
                assert!(r.resource.is_none());
            }
        }
        assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn resource_timeout_after_settlement_recovers_proof_without_second_payment() {
    let f = Fixture::new().await;
    f.retry_fail.store(2, Ordering::SeqCst);
    let mut buyer = f.buyer();
    let r = buyer.fetch(&f.url("/premium/report")).await.unwrap();
    assert_eq!(r.resource_status, "paid_resource_unavailable");
    assert_eq!(r.payment_status.as_deref(), Some("settled"));
    drop(buyer);
    f.retry_fail.store(0, Ordering::SeqCst);
    assert_eq!(
        f.buyer()
            .fetch(&f.url("/premium/report"))
            .await
            .unwrap()
            .resource_status,
        "purchased"
    );
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn amount_mismatch_and_denied_urls_do_not_pay() {
    let f = Fixture::new().await;
    let mut buyer = f.buyer();
    f.node.amount_offset.store(1, Ordering::SeqCst);
    assert!(buyer.fetch(&f.url("/premium/report")).await.is_err());
    for url in [
        "file:///etc/passwd",
        "http://169.254.169.254/latest/meta-data",
        "http://127.0.0.1:1/",
        "http://localhost:3040/",
        "https://example.com/",
    ] {
        assert!(buyer.fetch(url).await.is_err());
    }
    for url in [
        f.url("/premium/report?redirect=evil"),
        f.url("/premium/report#fragment"),
        f.origin.replace("http://", "http://user:password@"),
    ] {
        assert!(buyer.fetch(&url).await.is_err());
    }
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn approval_revalidates_balance_and_invoice() {
    let f = Fixture::new().await;
    let mut buyer = f.buyer();
    let id = buyer
        .fetch(&f.url("/premium/extended"))
        .await
        .unwrap()
        .plan
        .unwrap()
        .plan_id;
    buyer.confirm_from_human(&id, true).unwrap();
    f.node.amount_offset.store(1, Ordering::SeqCst);
    assert!(buyer.fetch(&f.url("/premium/extended")).await.is_err());
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn delayed_settlement_unlocks_in_the_original_fetch_without_another_prompt() {
    let f = Fixture::new().await;
    f.node.mode.store(5, Ordering::SeqCst);
    let r = f.buyer().fetch(&f.url("/premium/report")).await.unwrap();
    assert_eq!(r.resource_status, "purchased");
    assert_eq!(r.payment_status.as_deref(), Some("settled"));
    assert_eq!(r.http_status, Some(200));
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelled_machine_plan_cannot_reuse_approval() {
    let f = Fixture::new().await;
    let mut buyer = f.buyer();
    let url = f.url("/premium/extended");
    let plan = buyer.fetch(&url).await.unwrap().plan.unwrap();
    buyer.confirm_from_human(&plan.plan_id, false).unwrap();
    assert_eq!(
        buyer.task(&url).unwrap().state,
        rgb402_payment::harness::State::Cancelled
    );
    assert!(buyer.confirm_from_human(&plan.plan_id, true).is_err());
    assert_eq!(
        buyer.fetch(&url).await.unwrap().resource_status,
        "approval_required"
    );
    assert_eq!(f.node.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn approval_for_same_invoice_at_one_origin_cannot_authorize_another() {
    let first = Fixture::new().await;
    let second = Fixture::new().await;
    let mut buyer = CommerceService::open(
        first.node.clone(),
        CommerceConfig {
            origins: vec![first.origin.clone(), second.origin.clone()],
            policy: MachinePolicy {
                auto_approve_below_sats: 10,
                max_single_payment_sats: 100,
                max_daily_spend_sats: 500,
            },
            state_path: first.dir.join("buyer.jsonl"),
        },
    )
    .unwrap();
    let a = buyer
        .fetch(&first.url("/premium/extended"))
        .await
        .unwrap()
        .plan
        .unwrap();
    let b = buyer
        .fetch(&second.url("/premium/extended"))
        .await
        .unwrap()
        .plan
        .unwrap();
    assert_ne!(a.plan_id, b.plan_id);
    buyer.confirm_from_human(&a.plan_id, true).unwrap();
    assert_eq!(
        buyer
            .fetch(&second.url("/premium/extended"))
            .await
            .unwrap()
            .resource_status,
        "approval_required"
    );
    assert_eq!(first.node.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        buyer
            .fetch(&first.url("/premium/extended"))
            .await
            .unwrap()
            .resource_status,
        "purchased"
    );
    assert_eq!(first.node.sends.load(Ordering::SeqCst), 1);
}
