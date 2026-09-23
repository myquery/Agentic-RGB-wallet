//! Optional wallet merchant capability; invoice issuance and node-verified receipts only.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use fs2::FileExt;
use rgb402_core::{merchant::*, AssetId, PaymentStatus};
use rgb402_payment::rgb::{CreateInvoice, RgbNode};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub struct Store {
    profile: MerchantProfile,
    products: Vec<Product>,
    node: Arc<dyn RgbNode>,
    orders: BTreeMap<String, Order>,
    journal: File,
    // Held for the store lifetime; the kernel releases it after a crash or reboot.
    _lock: File,
    settings_path: PathBuf,
}
impl Store {
    pub fn open(
        profile: MerchantProfile,
        asset: AssetId,
        node: Arc<dyn RgbNode>,
        path: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let settings_path = path.with_extension("settings.json");
        let lock = path.with_extension("lock");
        let guard = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)?;
        guard.try_lock_exclusive()?;
        let result = (|| {
            let mut options = OpenOptions::new();
            options.create(true).append(true).read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let journal = options.open(path)?;
            let mut orders = BTreeMap::new();
            for line in BufReader::new(journal.try_clone()?).lines() {
                let order: Order = serde_json::from_str(&line?)?;
                if order.merchant_id != profile.merchant_id {
                    return Err("merchant journal configuration mismatch".into());
                }
                orders.insert(order.id.clone(), order);
            }
            let products = [("coffee", "Coffee", "5"), ("sandwich", "Sandwich", "8")]
                .into_iter()
                .map(|(id, name, amount)| Product {
                    id: id.into(),
                    name: name.into(),
                    amount: amount.into(),
                    asset_id: asset.clone(),
                    available: true,
                })
                .collect();
            let mut store = Self {
                profile,
                products,
                node,
                orders,
                journal,
                _lock: guard,
                settings_path,
            };
            if store.settings_path.exists() {
                let settings = serde_json::from_reader(File::open(&store.settings_path)?)?;
                store.apply(settings);
            } else if !store.profile.enabled {
                store.products.clear();
            }
            Ok(store)
        })();
        result
    }
    pub fn enabled(&self) -> bool {
        self.profile.enabled
    }
    fn apply(&mut self, settings: MerchantSettings) {
        self.profile.enabled = settings.enabled;
        self.profile.public_catalog = settings.public_catalog;
        self.profile.display_name = settings.display_name;
        self.profile.accepted_assets = settings.accepted_assets;
        self.products = settings.products;
    }
    pub async fn configure(&mut self, settings: MerchantSettings) -> Result<(), StatusCode> {
        let assets = self
            .node
            .list_assets()
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        validate_settings(
            &settings,
            &assets
                .iter()
                .map(|a| a.asset_id.clone())
                .collect::<Vec<_>>(),
        )?;
        let bytes = serde_json::to_vec(&settings).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let tmp = self.settings_path.with_extension("tmp");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let persisted = (|| -> std::io::Result<()> {
            let mut file = options.open(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&tmp, &self.settings_path)?;
            File::open(
                self.settings_path
                    .parent()
                    .unwrap_or(std::path::Path::new(".")),
            )?
            .sync_all()
        })();
        persisted.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        self.apply(settings);
        Ok(())
    }
    pub async fn overview(&mut self) -> serde_json::Value {
        let mut stale = false;
        for id in self.orders.keys().cloned().collect::<Vec<_>>() {
            if self.status(&id).await.is_err() {
                stale = true;
            }
        }
        serde_json::json!({"profile":self.profile,"products":self.products,"orders":self.orders.values().collect::<Vec<_>>(),"status_refresh_failed":stale})
    }
    fn save(&mut self, order: Order) -> Result<Order, StatusCode> {
        let mut bytes =
            serde_json::to_vec(&order).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        bytes.push(b'\n');
        self.journal
            .write_all(&bytes)
            .and_then(|_| self.journal.sync_all())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        self.orders.insert(order.id.clone(), order.clone());
        Ok(order)
    }
    async fn create(&mut self, input: OrderInput) -> Result<Order, StatusCode> {
        if !self.profile.enabled || !self.profile.public_catalog {
            return Err(StatusCode::NOT_FOUND);
        }
        let product = self
            .products
            .iter()
            .find(|p| p.id == input.product_id && p.available)
            .cloned()
            .ok_or(StatusCode::NOT_FOUND)?;
        let amount = price(&product, input.quantity)?;
        let invoice = self
            .node
            .create_invoice(&CreateInvoice {
                asset_id: product.asset_id.clone(),
                asset_amount: amount,
                amt_msat: 3_000_000,
                expiry_sec: 3600,
                description: None,
                description_hash: None,
            })
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        let payment = self
            .node
            .decode_invoice(&invoice.invoice)
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        if payment.asset_id != product.asset_id
            || payment.amount != amount
            || payment.expires_at <= now()
            || !payment.network.eq_ignore_ascii_case("regtest")
            || payment.carrier_msat != 3_000_000
        {
            return Err(StatusCode::BAD_GATEWAY);
        }
        // Ownership is verified against this wallet's inbound invoice registry, not model metadata.
        let owned = self
            .node
            .list_payments()
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?
            .iter()
            .any(|p| {
                p.inbound
                    && p.payment_hash == payment.payment_hash
                    && p.amt_msat == Some(payment.carrier_msat)
            });
        if !owned {
            return Err(StatusCode::BAD_GATEWAY);
        }
        let id = format!("ord_{}", payment.payment_hash.as_str());
        if self.orders.contains_key(&id) {
            return Err(StatusCode::CONFLICT);
        }
        self.save(Order {
            id,
            merchant_id: self.profile.merchant_id.clone(),
            product,
            quantity: input.quantity,
            payment,
            status: OrderStatus::AwaitingPayment,
            payment_status: PaymentStatus::Pending,
            created_at: now(),
        })
    }
    async fn status(&mut self, id: &str) -> Result<Order, StatusCode> {
        let mut order = self.orders.get(id).cloned().ok_or(StatusCode::NOT_FOUND)?;
        let state = self
            .node
            .payment_status(&order.payment.payment_hash)
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        order.status = match state {
            PaymentStatus::Settled => OrderStatus::Paid,
            PaymentStatus::Failed => OrderStatus::Failed,
            PaymentStatus::Pending if order.payment.expires_at <= now() => OrderStatus::Expired,
            _ => OrderStatus::AwaitingPayment,
        };
        order.payment_status = state;
        if self.orders[id].status != order.status {
            self.save(order)
        } else {
            Ok(order)
        }
    }
}
fn price(product: &Product, quantity: u64) -> Result<u64, StatusCode> {
    if quantity == 0 || quantity > 100 {
        return Err(StatusCode::BAD_REQUEST);
    }
    product
        .amount
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(quantity))
        .filter(|n| *n > 0)
        .ok_or(StatusCode::BAD_REQUEST)
}
pub type Shared = Arc<Mutex<Store>>;
pub fn router(store: Store) -> Router {
    public_router(Arc::new(Mutex::new(store)))
}
pub fn public_router(store: Shared) -> Router {
    Router::new()
        .route("/commerce/v1/merchant", get(profile))
        .route("/commerce/v1/catalog", get(catalog))
        .route("/commerce/v1/orders", post(create))
        .route("/commerce/v1/orders/:id", get(status))
        .layer(axum::extract::DefaultBodyLimit::max(2048))
        .with_state(store)
}
async fn profile(State(s): State<Shared>) -> Result<Json<MerchantProfile>, StatusCode> {
    let s = s.lock().await;
    if !s.profile.enabled {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(s.profile.clone()))
}
async fn catalog(State(s): State<Shared>) -> Result<Json<Vec<Product>>, StatusCode> {
    let s = s.lock().await;
    if !s.profile.enabled || !s.profile.public_catalog {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(
        s.products.iter().filter(|p| p.available).cloned().collect(),
    ))
}
async fn create(
    State(s): State<Shared>,
    Json(input): Json<OrderInput>,
) -> Result<Json<Order>, StatusCode> {
    s.lock().await.create(input).await.map(Json)
}
async fn status(
    State(s): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Order>, StatusCode> {
    s.lock().await.status(&id).await.map(Json)
}
fn validate_settings(s: &MerchantSettings, assets: &[AssetId]) -> Result<(), StatusCode> {
    let text = |v: &str| !v.trim().is_empty() && v.len() <= 100 && !v.chars().any(char::is_control);
    let mut ids = std::collections::HashSet::new();
    let mut accepted = std::collections::HashSet::new();
    if !text(&s.display_name)
        || s.accepted_assets.len() > 8
        || (s.enabled && s.accepted_assets.is_empty())
        || s.accepted_assets
            .iter()
            .any(|a| !assets.contains(a) || !accepted.insert(a))
        || s.products.len() > 20
        || s.products.iter().any(|p| {
            p.id.is_empty()
                || p.id.len() > 64
                || !p
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || !ids.insert(&p.id)
                || !text(&p.name)
                || !s.accepted_assets.contains(&p.asset_id)
                || p.amount.is_empty()
                || p.amount.len() > 20
                || !p.amount.bytes().all(|b| b.is_ascii_digit())
                || price(p, 1).is_err()
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgb402_core::{
        wallet::{Asset, PaymentRequest, WalletBalance},
        PaymentId,
    };
    use rgb402_payment::{
        rgb::{CreatedInvoice, NodePayment, NodePaymentStatus},
        wallet::{ApprovedPayment, PaymentResult, WalletError},
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Node {
        paid: AtomicBool,
        wrong: AtomicBool,
        owned: AtomicBool,
    }
    #[async_trait::async_trait]
    impl RgbNode for Node {
        async fn create_invoice(&self, _: &CreateInvoice) -> Result<CreatedInvoice, WalletError> {
            Ok(CreatedInvoice {
                invoice: "lnbcrt-test".into(),
            })
        }
        async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError> {
            Ok(PaymentRequest {
                invoice: invoice.into(),
                asset_id: AssetId::new("rgb:test").unwrap(),
                amount: if self.wrong.load(Ordering::SeqCst) {
                    8
                } else {
                    5
                },
                payment_hash: PaymentId::new("a".repeat(64)).unwrap(),
                network: "Regtest".into(),
                carrier_msat: 3_000_000,
                expires_at: u64::MAX,
            })
        }
        async fn list_payments(&self) -> Result<Vec<NodePayment>, WalletError> {
            Ok(vec![NodePayment {
                payment_hash: PaymentId::new("a".repeat(64)).unwrap(),
                inbound: self.owned.load(Ordering::SeqCst),
                status: NodePaymentStatus::Pending,
                // The pinned node populates RGB history fields only after settlement.
                asset_id: None,
                asset_amount: None,
                amt_msat: Some(3_000_000),
                created_at: 1,
                updated_at: 1,
            }])
        }
        async fn list_assets(&self) -> Result<Vec<Asset>, WalletError> {
            Ok(vec![Asset {
                asset_id: AssetId::new("rgb:test").unwrap(),
                name: "Test".into(),
                ticker: "TEST".into(),
                precision: 0,
            }])
        }
        async fn asset_balance(&self, _: &AssetId) -> Result<WalletBalance, WalletError> {
            panic!("merchant must not spend")
        }
        async fn send_payment(&self, _: &ApprovedPayment) -> Result<PaymentResult, WalletError> {
            panic!("merchant must not send")
        }
        async fn payment_status(&self, _: &PaymentId) -> Result<PaymentStatus, WalletError> {
            Ok(if self.paid.load(Ordering::SeqCst) {
                PaymentStatus::Settled
            } else {
                PaymentStatus::Pending
            })
        }
    }
    #[tokio::test]
    async fn order_is_owned_validated_durable_and_only_paid_by_node() {
        let node = Arc::new(Node {
            paid: AtomicBool::new(false),
            wrong: AtomicBool::new(false),
            owned: AtomicBool::new(true),
        });
        let asset = AssetId::new("rgb:test").unwrap();
        let profile = MerchantProfile {
            enabled: true,
            public_catalog: true,
            merchant_id: "carol@example.com".into(),
            display_name: "Carol".into(),
            accepted_assets: vec![asset.clone()],
            catalog: "https://example.com/commerce/v1/catalog".into(),
            orders: "https://example.com/commerce/v1/orders".into(),
        };
        let path = std::env::temp_dir().join(format!(
            "luma-store-{}-{}.jsonl",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut store =
            Store::open(profile.clone(), asset.clone(), node.clone(), path.clone()).unwrap();
        assert!(Store::open(profile.clone(), asset.clone(), node.clone(), path.clone()).is_err());
        node.wrong.store(true, Ordering::SeqCst);
        assert!(store
            .create(OrderInput {
                product_id: "coffee".into(),
                quantity: 1
            })
            .await
            .is_err());
        node.wrong.store(false, Ordering::SeqCst);
        node.owned.store(false, Ordering::SeqCst);
        assert!(store
            .create(OrderInput {
                product_id: "coffee".into(),
                quantity: 1
            })
            .await
            .is_err());
        node.owned.store(true, Ordering::SeqCst);
        let order = store
            .create(OrderInput {
                product_id: "coffee".into(),
                quantity: 1,
            })
            .await
            .unwrap();
        assert_eq!(order.status, OrderStatus::AwaitingPayment);
        assert!(store
            .create(OrderInput {
                product_id: "coffee".into(),
                quantity: 1
            })
            .await
            .is_err());
        drop(store);
        let mut store = Store::open(profile, asset, node.clone(), path.clone()).unwrap();
        assert_eq!(
            store.status(&order.id).await.unwrap().status,
            OrderStatus::AwaitingPayment
        );
        node.paid.store(true, Ordering::SeqCst);
        assert_eq!(
            store.status(&order.id).await.unwrap().status,
            OrderStatus::Paid
        );
        assert_eq!(store.orders.len(), 1);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn owner_settings_are_validated_persisted_and_do_not_reprice_orders() {
        let node = Arc::new(Node {
            paid: AtomicBool::new(false),
            wrong: AtomicBool::new(false),
            owned: AtomicBool::new(true),
        });
        let asset = AssetId::new("rgb:test").unwrap();
        let profile = MerchantProfile {
            enabled: true,
            public_catalog: true,
            merchant_id: "bob@example.com".into(),
            display_name: "Bob".into(),
            accepted_assets: vec![asset.clone()],
            catalog: "https://example.com/commerce/v1/wallets/bob/catalog".into(),
            orders: "https://example.com/commerce/v1/wallets/bob/orders".into(),
        };
        let path = std::env::temp_dir().join(format!(
            "merchant-settings-{}-{}.jsonl",
            std::process::id(),
            now()
        ));
        let mut store =
            Store::open(profile.clone(), asset.clone(), node.clone(), path.clone()).unwrap();
        let order = store
            .create(OrderInput {
                product_id: "coffee".into(),
                quantity: 1,
            })
            .await
            .unwrap();
        let mut settings = MerchantSettings {
            enabled: false,
            public_catalog: false,
            display_name: "Bob's Bakery".into(),
            accepted_assets: vec![asset.clone()],
            products: store.products.clone(),
        };
        settings.products[0].amount = "10".into();
        store.configure(settings.clone()).await.unwrap();
        assert!(store
            .create(OrderInput {
                product_id: "coffee".into(),
                quantity: 1
            })
            .await
            .is_err());
        assert_eq!(store.orders[&order.id].product.amount, "5");
        let mut bad = settings.clone();
        bad.products[0].asset_id = AssetId::new("rgb:foreign").unwrap();
        assert_eq!(
            store.configure(bad).await.unwrap_err(),
            StatusCode::BAD_REQUEST
        );
        let mut bad = settings.clone();
        bad.products.push(bad.products[0].clone());
        assert!(store.configure(bad).await.is_err());
        let mut bad = settings.clone();
        bad.products[0].amount = "0".into();
        assert!(store.configure(bad).await.is_err());
        drop(store);
        let mut store = Store::open(profile, asset, node.clone(), path.clone()).unwrap();
        assert!(!store.profile.enabled);
        assert_eq!(store.profile.display_name, "Bob's Bakery");
        assert_eq!(store.products[0].amount, "10");
        node.paid.store(true, Ordering::SeqCst);
        assert_eq!(
            store.status(&order.id).await.unwrap().status,
            OrderStatus::Paid
        );
        settings.enabled = true;
        settings.public_catalog = true;
        store.configure(settings).await.unwrap();
        assert!(store.enabled());
        drop(store);
        std::fs::remove_file(path.with_extension("settings.json")).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn prices_are_server_owned_and_overflow_checked() {
        let mut p = Product {
            id: "coffee".into(),
            name: "Coffee".into(),
            amount: "5".into(),
            asset_id: AssetId::new("rgb:test").unwrap(),
            available: true,
        };
        assert_eq!(price(&p, 2).unwrap(), 10);
        assert!(price(&p, 0).is_err());
        assert!(price(&p, 101).is_err());
        p.amount = u64::MAX.to_string();
        assert!(price(&p, 2).is_err());
        assert!(serde_json::from_str::<OrderInput>(
            r#"{"product_id":"coffee","quantity":1,"amount":"1"}"#
        )
        .is_err());
    }
}
