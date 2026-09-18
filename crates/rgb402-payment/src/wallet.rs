use crate::harness::{Action, ReservationAuthority, TaskKind, TaskSnapshot};
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
fn authorization_scope(
    economic_action_id: &str,
    provenance: Option<&RecipientProvenance>,
) -> Result<String, WalletError> {
    use sha2::{Digest, Sha256};
    match provenance {
        Some(recipient) => Ok(hex::encode(Sha256::digest(serde_json::to_vec(&(
            "rgb402-recipient-authorization-v1",
            economic_action_id,
            &recipient.recipient_contract_digest,
        ))?))),
        None => Ok(economic_action_id.to_owned()),
    }
}
/// Bounded application provenance, never an authorization capability.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecipientProvenance {
    pub identifier: String,
    pub authoritative_domain: String,
    pub recipient_contract_digest: String,
    pub invoice_id: String,
    pub payment_hash: PaymentId,
}
impl RecipientProvenance {
    fn validate(&self, request: &PaymentRequest) -> Result<(), WalletError> {
        use sha2::{Digest, Sha256};
        let hex_digest = |s: &str| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        };
        if self.identifier.len() > 318
            || self.authoritative_domain.is_empty()
            || self.authoritative_domain.len() > 253
            || !self
                .identifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"@._+-".contains(&c))
            || !self
                .authoritative_domain
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
            || self.identifier.split_once('@').map(|(_, domain)| domain)
                != Some(self.authoritative_domain.as_str())
            || !hex_digest(self.payment_hash.as_str())
            || !hex_digest(&self.recipient_contract_digest)
            || !hex_digest(&self.invoice_id)
            || self.invoice_id != format!("{:x}", Sha256::digest(request.invoice.as_bytes()))
            || self.payment_hash != request.payment_hash
        {
            return Err(WalletError::Invalid("recipient provenance mismatch"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct PaymentPlan {
    plan_id: String,
    request: PaymentRequest,
    available_balance: u64,
    policy: PolicyDecision,
    #[serde(skip)]
    action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    recipient_provenance: Option<RecipientProvenance>,
    authorization_scope: String,
}
impl PaymentPlan {
    pub fn authorization_scope(&self) -> &str {
        &self.authorization_scope
    }
    pub fn recipient_provenance(&self) -> Option<&RecipientProvenance> {
        self.recipient_provenance.as_ref()
    }
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
    #[serde(default)]
    authority: Option<ReservationAuthority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recipient_provenance: Option<RecipientProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recipient_authorization_scope: Option<String>,
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
    #[cfg(test)]
    fn reserve(&mut self, request: PaymentRequest, now: u64) -> Result<(), WalletError> {
        self.reserve_authorized(request, now, None, None)
    }
    fn reserve_authorized(
        &mut self,
        request: PaymentRequest,
        now: u64,
        authority: Option<ReservationAuthority>,
        recipient_provenance: Option<RecipientProvenance>,
    ) -> Result<(), WalletError> {
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
        let recipient_authorization_scope = match &recipient_provenance {
            Some(_) => Some(authorization_scope(
                &authority
                    .as_ref()
                    .ok_or(WalletError::Invalid("missing recipient authority"))?
                    .economic_action_id,
                recipient_provenance.as_ref(),
            )?),
            None => None,
        };
        let entry = Reservation {
            request,
            at: now,
            authority,
            recipient_provenance,
            recipient_authorization_scope,
        };
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
    approvals: HashMap<String, String>,
    journal: Journal,
    sequence: u64,
    tasks: std::sync::Mutex<HashMap<PaymentId, Action>>,
}
impl WalletService {
    /// Application-owned carrier ceiling for recipient acquisition.
    pub fn max_carrier_msat(&self) -> u64 {
        self.policy.max_carrier_msat
    }
    /// Read-only reserved payments for activity; settlement must still be queried.
    pub fn payment_history(&self) -> Vec<(PaymentRequest, u64)> {
        self.journal
            .entries
            .iter()
            .map(|entry| (entry.request.clone(), entry.at))
            .collect()
    }
    /// Read the authoritative, unconsumed plan without permitting mutation.
    pub fn plan(&self, plan_id: &str) -> Result<&PaymentPlan, WalletError> {
        self.plans.get(plan_id).ok_or(WalletError::UnknownPlan)
    }
    pub fn open(
        node: Arc<dyn RgbNode>,
        policy: WalletPolicy,
        path: &Path,
    ) -> Result<Self, WalletError> {
        let journal = Journal::open(path)?;
        let mut tasks = HashMap::new();
        for entry in &journal.entries {
            if let Some(scope) = &entry.recipient_authorization_scope {
                let authority = entry
                    .authority
                    .as_ref()
                    .ok_or(WalletError::Invalid("missing recipient authority"))?;
                if entry.recipient_provenance.is_none()
                    || *scope
                        != authorization_scope(
                            &authority.economic_action_id,
                            entry.recipient_provenance.as_ref(),
                        )?
                {
                    return Err(WalletError::Invalid(
                        "recipient authorization scope mismatch",
                    ));
                }
            }
            if let Some(provenance) = &entry.recipient_provenance {
                provenance.validate(&entry.request)?;
            }
            tasks.insert(
                entry.request.payment_hash.clone(),
                Action::recover(
                    TaskKind::RgbPayment,
                    &entry.request,
                    entry.authority.as_ref(),
                )?,
            );
        }
        Ok(Self {
            node,
            policy,
            plans: HashMap::new(),
            approvals: Default::default(),
            journal,
            tasks: std::sync::Mutex::new(tasks),
            sequence: 0,
        })
    }
    /// Durable provenance for a reserved payment, including failed/uncertain submissions.
    pub fn recipient_provenance(&self, hash: &PaymentId) -> Option<&RecipientProvenance> {
        self.journal
            .entries
            .iter()
            .find(|entry| &entry.request.payment_hash == hash)
            .and_then(|entry| entry.recipient_provenance.as_ref())
    }
    /// Node-native receiving capability; creates no outgoing reservation.
    pub fn policy_limits(&self) -> &rgb402_core::wallet::WalletPolicy {
        &self.policy
    }
    pub async fn create_invoice(
        &self,
        request: &crate::rgb::CreateInvoice,
    ) -> Result<crate::rgb::CreatedInvoice, WalletError> {
        self.node.create_invoice(request).await
    }
    pub async fn node_payments(&self) -> Result<Vec<crate::rgb::NodePayment>, WalletError> {
        self.node.list_payments().await
    }
    /// Recorded audit binding only; recovered reservations never restore approval.
    pub fn recipient_authorization_scope(&self, hash: &PaymentId) -> Option<&str> {
        self.journal
            .entries
            .iter()
            .find(|entry| &entry.request.payment_hash == hash)
            .and_then(|entry| entry.recipient_authorization_scope.as_deref())
    }
    pub fn plan_task(&self, plan_id: &str) -> Option<TaskSnapshot> {
        self.plans.get(plan_id).map(|plan| plan.action.snapshot())
    }
    pub fn task(&self, hash: &PaymentId) -> Option<TaskSnapshot> {
        self.tasks
            .lock()
            .expect("task lock")
            .get(hash)
            .map(Action::snapshot)
            .or_else(|| {
                self.plans
                    .values()
                    .find(|p| &p.request.payment_hash == hash)
                    .map(|p| p.action.snapshot())
            })
    }
    pub fn cancel(&mut self, plan_id: &str) -> Result<(), WalletError> {
        let plan = self
            .plans
            .get_mut(plan_id)
            .ok_or(WalletError::UnknownPlan)?;
        plan.action.cancel()?;
        self.approvals.remove(plan_id);
        if !self
            .journal
            .entries
            .iter()
            .any(|entry| entry.request.payment_hash == plan.request.payment_hash)
        {
            self.tasks
                .lock()
                .expect("task lock")
                .insert(plan.request.payment_hash.clone(), plan.action.clone());
        }
        Ok(())
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
        self.prepare_inner(invoice, None).await
    }
    /// Application bridge: metadata cannot change the decoded economic request or policy.
    pub async fn prepare_recipient_invoice(
        &mut self,
        expected: &PaymentRequest,
        provenance: RecipientProvenance,
    ) -> Result<PaymentPlan, WalletError> {
        provenance.validate(expected)?;
        self.prepare_inner(&expected.invoice, Some((expected, provenance)))
            .await
    }
    async fn prepare_inner(
        &mut self,
        invoice: &str,
        bound: Option<(&PaymentRequest, RecipientProvenance)>,
    ) -> Result<PaymentPlan, WalletError> {
        tracing::info!(event = "payment_requested");
        let request = self.decode(invoice).await?;
        let recipient_provenance = match bound {
            Some((expected, provenance)) => {
                if &request != expected {
                    return Err(WalletError::Invalid("acquired invoice details changed"));
                }
                Some(provenance)
            }
            None => None,
        };
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
        let mut action = Action::new(TaskKind::RgbPayment, &request)?;
        action.decoded()?;
        action.validated()?;
        action.policy(
            matches!(policy, PolicyDecision::Allow),
            matches!(policy, PolicyDecision::Deny { .. }),
        )?;
        let authorization_scope = authorization_scope(
            &action.snapshot().economic_action_id,
            recipient_provenance.as_ref(),
        )?;
        let plan = PaymentPlan {
            plan_id: plan_id.clone(),
            request,
            available_balance: balance.offchain_outbound,
            policy,
            action,
            recipient_provenance,
            authorization_scope,
        };
        self.plans.insert(plan_id, plan.clone());
        Ok(plan)
    }
    /// Trusted human interface only. Do not expose this method as an agent tool.
    pub fn approve_from_human(&mut self, plan_id: &str) -> Result<(), WalletError> {
        if !self.plans.contains_key(plan_id) {
            return Err(WalletError::UnknownPlan);
        }
        let plan = self
            .plans
            .get_mut(plan_id)
            .ok_or(WalletError::UnknownPlan)?;
        if !matches!(plan.policy, PolicyDecision::Deny { .. }) {
            plan.action.human()?;
        }
        self.approvals
            .insert(plan_id.into(), plan.authorization_scope.clone());
        tracing::info!(event = "approval_received", %plan_id);
        Ok(())
    }
    pub async fn execute_payment(&mut self, plan_id: &str) -> Result<PaymentResult, WalletError> {
        let mut plan = self
            .plans
            .get(plan_id)
            .ok_or(WalletError::UnknownPlan)?
            .clone();
        if plan.authorization_scope
            != authorization_scope(
                &plan.action.snapshot().economic_action_id,
                plan.recipient_provenance.as_ref(),
            )?
        {
            return Err(WalletError::Invalid("plan authorization scope mismatch"));
        }
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
            PolicyDecision::RequireApproval { .. }
                if self.approvals.get(plan_id) != Some(&plan.authorization_scope) =>
            {
                return Err(WalletError::ApprovalRequired)
            }
            _ => {}
        }
        let authority = plan.action.authority(&request)?;
        self.journal.reserve_authorized(
            request.clone(),
            now,
            Some(authority),
            plan.recipient_provenance.clone(),
        )?;
        plan.action.reserved()?;
        plan.action.submit()?;
        self.tasks
            .lock()
            .expect("task lock")
            .insert(request.payment_hash.clone(), plan.action);
        self.plans.remove(plan_id);
        self.approvals.remove(plan_id);
        let hash = request.payment_hash.clone();
        let result = self.node.send_payment(&ApprovedPayment { request }).await;
        if let Some(task) = self.tasks.lock().expect("task lock").get_mut(&hash) {
            task.status(result.as_ref().ok().map(|r| &r.status))?;
        }
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
        if let Some(task) = self.tasks.lock().expect("task lock").get_mut(hash) {
            task.status(Some(&status))?;
        }
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
            tasks: Default::default(),
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
        w.cancel(p.plan_id()).unwrap();
        assert_eq!(
            w.task(&p.request.payment_hash).unwrap().state,
            crate::harness::State::Submitted
        );
        assert_eq!(node.calls.load(Ordering::SeqCst), 1);
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
        let recorded = std::fs::read_to_string(&path).unwrap();
        let entry: serde_json::Value = serde_json::from_str(recorded.trim()).unwrap();
        assert_eq!(entry["authority"]["source"], "human");
        assert_eq!(
            entry["authority"]["economic_action_id"],
            persistent
                .task(&p.request.payment_hash)
                .unwrap()
                .economic_action_id
        );
        drop(persistent);
        let mut reopened = WalletService::open(node, w.policy, &path).unwrap();
        assert_eq!(
            reopened.task(&p.request.payment_hash).unwrap().state,
            crate::harness::State::Uncertain
        );
        assert_eq!(
            reopened
                .task(&p.request.payment_hash)
                .unwrap()
                .authorization,
            Some(crate::harness::Authorization::Human)
        );
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
    #[tokio::test]
    async fn cancellation_consumes_wallet_plan_without_submission() {
        let (node, mut wallet) = setup();
        let p = wallet.prepare_payment("cancelled").await.unwrap();
        wallet.cancel(p.plan_id()).unwrap();
        assert!(wallet.approve_from_human(p.plan_id()).is_err());
        assert!(wallet.execute_payment(p.plan_id()).await.is_err());
        assert_eq!(
            wallet.task(&p.request.payment_hash).unwrap().state,
            crate::harness::State::Cancelled
        );
        assert_eq!(node.calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn legacy_reservation_recovers_without_inventing_authorization() {
        let (node, wallet) = setup();
        let request = wallet.decode("legacy").await.unwrap();
        let path =
            std::env::temp_dir().join(format!("harness-legacy-{}.jsonl", std::process::id()));
        std::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::json!({"request":request,"at":UnixTimestamp::now().seconds()})
            ),
        )
        .unwrap();
        let mut policy = wallet.policy.clone();
        policy.max_daily_spend = 100;
        let mut recovered = WalletService::open(node.clone(), policy, &path).unwrap();
        assert_eq!(
            recovered.task(&request.payment_hash).unwrap().authorization,
            Some(crate::harness::Authorization::LegacyUnknown)
        );
        recovered
            .payment_status(&request.payment_hash)
            .await
            .unwrap();
        let plan = recovered.prepare_payment("legacy").await.unwrap();
        recovered.approve_from_human(plan.plan_id()).unwrap();
        assert!(matches!(
            recovered.execute_payment(plan.plan_id()).await,
            Err(WalletError::Duplicate)
        ));
        assert_eq!(node.calls.load(Ordering::SeqCst), 0);
        drop(recovered);
        std::fs::remove_file(path).unwrap();
    }
}
