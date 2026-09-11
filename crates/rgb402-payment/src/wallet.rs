use crate::rgb::RgbNode;
use rgb402_core::{
    wallet::{PaymentRequest, PolicyDecision, WalletBalance, WalletPolicy},
    AssetId, PaymentId, PaymentStatus, UnixTimestamp,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WalletError {
    #[error("configuration: {0}")]
    Config(String),
    #[error("{0}")]
    Invalid(&'static str),
    #[error("node: {0}")]
    Node(&'static str),
    #[error("node HTTP status {0}")]
    Http(u16),
    #[error("policy denied: {0}")]
    Denied(String),
    #[error("human approval required for this plan")]
    ApprovalRequired,
    #[error("unknown or consumed plan")]
    UnknownPlan,
    #[error("payment already reserved; query its status before taking further action")]
    Duplicate,
    #[error("wallet state I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("wallet state invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid identifier: {0}")]
    Domain(#[from] rgb402_core::DomainError),
}
#[derive(Clone, Debug, Serialize)]
pub struct PaymentPlan {
    plan_id: String,
    request: PaymentRequest,
    available_balance: u64,
    policy: PolicyDecision,
}
impl PaymentPlan {
    pub fn available_balance(&self) -> u64 {
        self.available_balance
    }
    pub fn plan_id(&self) -> &str {
        &self.plan_id
    }
    pub fn request(&self) -> &PaymentRequest {
        &self.request
    }
    pub fn policy(&self) -> &PolicyDecision {
        &self.policy
    }
}
// Only WalletService can mint this capability. Never deserialize it from tool input.
pub struct ApprovedPayment {
    request: PaymentRequest,
}
impl ApprovedPayment {
    pub fn request(&self) -> &PaymentRequest {
        &self.request
    }
}
#[derive(Debug, Serialize)]
pub struct PaymentResult {
    pub payment_id: PaymentId,
    pub payment_hash: PaymentId,
    pub status: PaymentStatus,
}
#[derive(Serialize, Deserialize)]
struct Reservation {
    request: PaymentRequest,
    at: u64,
}
struct Journal {
    entries: Vec<Reservation>,
    file: Option<File>,
    lock: Option<PathBuf>,
    poisoned: bool,
}
impl Drop for Journal {
    fn drop(&mut self) {
        if let Some(path) = &self.lock {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl Journal {
    fn open(path: &Path) -> Result<Self, WalletError> {
        let lock = path.with_extension("lock");
        let lock_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)?;
        lock_file.sync_all()?;
        let mut journal = Self {
            entries: vec![],
            file: None,
            lock: Some(lock),
            poisoned: false,
        };
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)?;
        for line in BufReader::new(&file).lines() {
            journal.entries.push(serde_json::from_str(&line?)?);
        }
        file.sync_all()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        File::open(parent)?.sync_all()?;
        journal.file = Some(file);
        Ok(journal)
    }
    fn spent(&self, asset: &AssetId, now: u64) -> Result<u64, WalletError> {
        self.entries
            .iter()
            .filter(|r| &r.request.asset_id == asset && r.at / 86400 >= now / 86400)
            .try_fold(0u64, |sum, r| {
                sum.checked_add(r.request.amount)
                    .ok_or(WalletError::Invalid("daily spend overflow"))
            })
    }
    fn reserve(&mut self, request: PaymentRequest, now: u64) -> Result<(), WalletError> {
        if self.poisoned {
            return Err(WalletError::Invalid(
                "journal write failed; restart and inspect state",
            ));
        }
        if self
            .entries
            .iter()
            .any(|r| r.request.payment_hash == request.payment_hash)
        {
            return Err(WalletError::Duplicate);
        }
        let entry = Reservation { request, at: now };
        if let Some(file) = &mut self.file {
            self.poisoned = true;
            let mut bytes = serde_json::to_vec(&entry)?;
            bytes.push(b'\n');
            file.write_all(&bytes)?;
            file.sync_all()?;
            self.poisoned = false;
        }
        self.entries.push(entry);
        Ok(())
    }
}
pub struct WalletService {
    node: Arc<dyn RgbNode>,
    policy: WalletPolicy,
    plans: HashMap<String, PaymentPlan>,
    approvals: std::collections::HashSet<String>,
    journal: Journal,
    sequence: u64,
}
impl WalletService {
    /// Read the authoritative, unconsumed plan without permitting mutation.
    pub fn plan(&self, plan_id: &str) -> Result<&PaymentPlan, WalletError> {
        self.plans.get(plan_id).ok_or(WalletError::UnknownPlan)
    }
    pub fn open(
        node: Arc<dyn RgbNode>,
        policy: WalletPolicy,
        path: &Path,
    ) -> Result<Self, WalletError> {
        Ok(Self {
            node,
            policy,
            plans: HashMap::new(),
            approvals: Default::default(),
            journal: Journal::open(path)?,
            sequence: 0,
        })
    }
    pub async fn assets(&self) -> Result<Vec<rgb402_core::wallet::Asset>, WalletError> {
        self.node.list_assets().await
    }
    pub async fn balance(&self, asset: &AssetId) -> Result<WalletBalance, WalletError> {
        self.node.asset_balance(asset).await
    }
    pub async fn decode(&self, invoice: &str) -> Result<PaymentRequest, WalletError> {
        let request = self.node.decode_invoice(invoice).await?;
        tracing::info!(event = "invoice_decoded", payment_hash = %request.payment_hash, asset = %request.asset_id, amount = request.amount);
        Ok(request)
    }
    pub async fn prepare_payment(&mut self, invoice: &str) -> Result<PaymentPlan, WalletError> {
        tracing::info!(event = "payment_requested");
        let request = self.decode(invoice).await?;
        let balance = self.balance(&request.asset_id).await?;
        let now = UnixTimestamp::now().seconds();
        let policy = self.policy.evaluate(
            &request,
            balance.offchain_outbound,
            self.journal.spent(&request.asset_id, now)?,
            now,
        );
        self.sequence += 1;
        let plan_id = format!("{}-{}", request.payment_hash, self.sequence);
        tracing::info!(event = "policy_evaluated", %plan_id, decision = ?policy);
        if matches!(policy, PolicyDecision::RequireApproval { .. }) {
            tracing::info!(event = "approval_requested", %plan_id);
        }
        let plan = PaymentPlan {
            plan_id: plan_id.clone(),
            request,
            available_balance: balance.offchain_outbound,
            policy,
        };
        self.plans.insert(plan_id, plan.clone());
        Ok(plan)
    }
    /// Trusted human interface only. Do not expose this method as an agent tool.
    pub fn approve_from_human(&mut self, plan_id: &str) -> Result<(), WalletError> {
        if !self.plans.contains_key(plan_id) {
            return Err(WalletError::UnknownPlan);
        }
        self.approvals.insert(plan_id.into());
        tracing::info!(event = "approval_received", %plan_id);
        Ok(())
    }
    pub async fn execute_payment(&mut self, plan_id: &str) -> Result<PaymentResult, WalletError> {
        let plan = self
            .plans
            .get(plan_id)
            .ok_or(WalletError::UnknownPlan)?
            .clone();
        let request = self.decode(&plan.request.invoice).await?;
        if request != plan.request {
            return Err(WalletError::Invalid(
                "invoice details changed; prepare again",
            ));
        }
        let balance = self.balance(&request.asset_id).await?;
        let now = UnixTimestamp::now().seconds();
        let decision = self.policy.evaluate(
            &request,
            balance.offchain_outbound,
            self.journal.spent(&request.asset_id, now)?,
            now,
        );
        tracing::info!(event = "policy_evaluated", %plan_id, decision = ?decision);
        match decision {
            PolicyDecision::Deny { reason } => return Err(WalletError::Denied(reason)),
            PolicyDecision::RequireApproval { .. } if !self.approvals.contains(plan_id) => {
                return Err(WalletError::ApprovalRequired)
            }
            _ => {}
        }
        self.journal.reserve(request.clone(), now)?;
        self.plans.remove(plan_id);
        self.approvals.remove(plan_id);
        let result = self.node.send_payment(&ApprovedPayment { request }).await;
        match &result {
            Ok(result) => {
                tracing::info!(event = "payment_submitted", %plan_id, payment_hash = %result.payment_hash);
                audit_status(&result.payment_hash, &result.status);
            }
            Err(_) => {
                tracing::error!(event = "payment_failed", %plan_id, outcome = "uncertain; reservation retained")
            }
        }
        result
    }
    pub async fn payment_status(&self, hash: &PaymentId) -> Result<PaymentStatus, WalletError> {
        let status = self.node.payment_status(hash).await?;
        audit_status(hash, &status);
        Ok(status)
    }
}
fn audit_status(hash: &PaymentId, status: &PaymentStatus) {
    match status {
        PaymentStatus::Settled => tracing::info!(event = "payment_settled", payment_hash = %hash),
        PaymentStatus::Failed => tracing::warn!(event = "payment_failed", payment_hash = %hash),
        PaymentStatus::Pending => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    struct MockRgbNode {
        balance: AtomicU64,
        amount: AtomicU64,
        calls: AtomicU64,
        fail: AtomicBool,
    }
    #[async_trait]
    impl RgbNode for MockRgbNode {
        async fn list_assets(&self) -> Result<Vec<rgb402_core::wallet::Asset>, WalletError> {
            Ok(vec![])
        }
        async fn asset_balance(&self, asset: &AssetId) -> Result<WalletBalance, WalletError> {
            Ok(WalletBalance {
                asset_id: asset.clone(),
                onchain_spendable: 999,
                offchain_outbound: self.balance.load(Ordering::SeqCst),
            })
        }
        async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError> {
            Ok(PaymentRequest {
                asset_id: AssetId::new("rgb:demo")?,
                amount: self.amount.load(Ordering::SeqCst),
                invoice: invoice.into(),
                payment_hash: PaymentId::new(invoice)?,
                expires_at: u64::MAX,
                network: "Regtest".into(),
                carrier_msat: 1000,
            })
        }
        async fn send_payment(&self, p: &ApprovedPayment) -> Result<PaymentResult, WalletError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(WalletError::Node("timeout"));
            }
            Ok(PaymentResult {
                payment_id: p.request.payment_hash.clone(),
                payment_hash: p.request.payment_hash.clone(),
                status: PaymentStatus::Pending,
            })
        }
        async fn payment_status(&self, _: &PaymentId) -> Result<PaymentStatus, WalletError> {
            Ok(PaymentStatus::Settled)
        }
    }
    fn setup() -> (Arc<MockRgbNode>, WalletService) {
        let node = Arc::new(MockRgbNode {
            balance: AtomicU64::new(100),
            amount: AtomicU64::new(10),
            calls: AtomicU64::new(0),
            fail: AtomicBool::new(false),
        });
        let policy = WalletPolicy {
            allowed_assets: [AssetId::new("rgb:demo").unwrap()].into(),
            auto_approve_below: 10,
            max_single_payment: 50,
            max_daily_spend: 15,
            max_carrier_msat: 1000,
        };
        let service = WalletService {
            node: node.clone(),
            policy,
            plans: HashMap::new(),
            approvals: Default::default(),
            journal: Journal {
                entries: vec![],
                file: None,
                lock: None,
                poisoned: false,
            },
            sequence: 0,
        };
        (node, service)
    }
    #[tokio::test]
    async fn prepare_approve_execute_status_and_replay() {
        let (node, mut w) = setup();
        let p = w.prepare_payment("one").await.unwrap();
        assert!(matches!(p.policy(), PolicyDecision::RequireApproval { .. }));
        assert!(matches!(
            w.execute_payment(p.plan_id()).await,
            Err(WalletError::ApprovalRequired)
        ));
        w.approve_from_human(p.plan_id()).unwrap();
        assert_eq!(
            w.execute_payment(p.plan_id()).await.unwrap().status,
            PaymentStatus::Pending
        );
        assert_eq!(
            w.payment_status(&p.request.payment_hash).await.unwrap(),
            PaymentStatus::Settled
        );
        assert!(matches!(
            w.execute_payment(p.plan_id()).await,
            Err(WalletError::UnknownPlan)
        ));
        assert_eq!(node.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn changed_invoice_or_balance_cannot_use_approval() {
        let (node, mut w) = setup();
        let p = w.prepare_payment("one").await.unwrap();
        w.approve_from_human(p.plan_id()).unwrap();
        node.amount.store(11, Ordering::SeqCst);
        assert!(matches!(
            w.execute_payment(p.plan_id()).await,
            Err(WalletError::Invalid(_))
        ));
        node.amount.store(10, Ordering::SeqCst);
        node.balance.store(0, Ordering::SeqCst);
        assert!(matches!(
            w.execute_payment(p.plan_id()).await,
            Err(WalletError::Denied(_))
        ));
        assert_eq!(node.calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn daily_limit_rechecked_and_failure_keeps_reservation() {
        let (node, mut w) = setup();
        let p = w.prepare_payment("one").await.unwrap();
        let q = w.prepare_payment("two").await.unwrap();
        w.approve_from_human(p.plan_id()).unwrap();
        w.approve_from_human(q.plan_id()).unwrap();
        node.fail.store(true, Ordering::SeqCst);
        assert!(w.execute_payment(p.plan_id()).await.is_err());
        assert!(matches!(
            w.execute_payment(q.plan_id()).await,
            Err(WalletError::Denied(_))
        ));
        assert_eq!(node.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn insufficient_balance_and_asset_denial() {
        let (node, mut w) = setup();
        node.balance.store(0, Ordering::SeqCst);
        assert!(matches!(
            w.prepare_payment("one").await.unwrap().policy(),
            PolicyDecision::Deny { .. }
        ));
        node.balance.store(100, Ordering::SeqCst);
        w.policy.allowed_assets.clear();
        let p = w.prepare_payment("two").await.unwrap();
        w.approve_from_human(p.plan_id()).unwrap();
        assert!(matches!(
            w.execute_payment(p.plan_id()).await,
            Err(WalletError::Denied(_))
        ));
        assert_eq!(node.calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn automatic_payment_and_duplicate_hash() {
        let (node, mut w) = setup();
        node.amount.store(5, Ordering::SeqCst);
        let p = w.prepare_payment("one").await.unwrap();
        w.execute_payment(p.plan_id()).await.unwrap();
        let p = w.prepare_payment("one").await.unwrap();
        assert!(matches!(
            w.execute_payment(p.plan_id()).await,
            Err(WalletError::Duplicate)
        ));
    }
    #[tokio::test]
    async fn journal_survives_restart_and_locks() {
        let (node, w) = setup();
        let path = std::env::temp_dir().join(format!(
            "rgb-wallet-test-{}-{}.jsonl",
            std::process::id(),
            UnixTimestamp::now().seconds()
        ));
        let mut persistent = WalletService::open(node.clone(), w.policy.clone(), &path).unwrap();
        assert!(WalletService::open(node.clone(), w.policy.clone(), &path).is_err());
        let p = persistent.prepare_payment("one").await.unwrap();
        persistent.approve_from_human(p.plan_id()).unwrap();
        persistent.execute_payment(p.plan_id()).await.unwrap();
        drop(persistent);
        let mut reopened = WalletService::open(node, w.policy, &path).unwrap();
        assert!(matches!(
            reopened.prepare_payment("two").await.unwrap().policy(),
            PolicyDecision::Deny { .. }
        ));
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn approval_is_bound_to_one_plan() {
        let (node, mut wallet) = setup();
        let first = wallet.prepare_payment("first").await.unwrap();
        let second = wallet.prepare_payment("second").await.unwrap();
        wallet.approve_from_human(first.plan_id()).unwrap();
        assert!(matches!(
            wallet.execute_payment(second.plan_id()).await,
            Err(WalletError::ApprovalRequired)
        ));
        assert_eq!(node.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn utc_day_accounting_and_overflow_fail_closed() {
        let (_, mut wallet) = setup();
        let request = wallet.decode("first").await.unwrap();
        wallet.journal.reserve(request.clone(), 86_399).unwrap();
        assert_eq!(wallet.journal.spent(&request.asset_id, 86_399).unwrap(), 10);
        assert_eq!(wallet.journal.spent(&request.asset_id, 86_400).unwrap(), 0);
        // A backwards wall clock must not erase reservations.
        assert_eq!(wallet.journal.spent(&request.asset_id, 0).unwrap(), 10);
        let mut large = wallet.decode("second").await.unwrap();
        large.amount = u64::MAX;
        wallet.journal.reserve(large, 86_399).unwrap();
        assert!(wallet.journal.spent(&request.asset_id, 86_399).is_err());
    }
}
