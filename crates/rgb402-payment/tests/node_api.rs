//! HTTP contract tests: these exercise the real client, not a fake node trait.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use rgb402_core::{wallet::WalletPolicy, AssetId, PaymentStatus};
use rgb402_payment::{rgb::RgbLightningClient, wallet::WalletService};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

type Calls = Arc<Mutex<Vec<(String, Value)>>>;

#[tokio::test]
async fn pinned_api_contract_and_invoice_only_submission() {
    let calls = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let app = Router::new()
        .route("/:endpoint", post(endpoint))
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let node =
        Arc::new(RgbLightningClient::new(&format!("http://{addr}"), Some("test-token")).unwrap());
    let path = std::env::temp_dir().join(format!("rgb-http-test-{}.jsonl", std::process::id()));
    let policy = WalletPolicy {
        allowed_assets: [AssetId::new("rgb:demo").unwrap()].into(),
        auto_approve_below: 10,
        max_single_payment: 50,
        max_daily_spend: 100,
        max_carrier_msat: 1000,
    };
    let mut wallet = WalletService::open(node, policy, &path).unwrap();
    assert_eq!(wallet.assets().await.unwrap()[0].ticker, "USDTRGBDEMO");
    let p = wallet
        .prepare_payment("lnbcrt-contract-fixture")
        .await
        .unwrap();
    let result = wallet.execute_payment(p.plan_id()).await.unwrap();
    assert_eq!(result.status, PaymentStatus::Pending);
    assert_eq!(
        wallet.payment_status(&result.payment_hash).await.unwrap(),
        PaymentStatus::Settled
    );
    let calls = calls.lock().unwrap();
    assert!(calls.contains(&("assetbalance".into(), json!({"asset_id":"rgb:demo"}))));
    assert!(calls.contains(&(
        "sendpayment".into(),
        json!({"invoice":"lnbcrt-contract-fixture"})
    )));
    assert!(calls.contains(&("getpayment".into(), json!({"payment_hash":"hash"}))));
    drop(calls);
    drop(wallet);
    std::fs::remove_file(path).unwrap();
    server.abort();
}
async fn endpoint(
    Path(path): Path<String>,
    State(calls): State<Calls>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    assert_eq!(headers.get("authorization").unwrap(), "Bearer test-token");
    calls.lock().unwrap().push((path.clone(), body));
    Ok(Json(match path.as_str() {
        "listassets" => {
            json!({"nia":[{"asset_id":"rgb:demo","name":"USDt RGB Demo","ticker":"USDTRGBDEMO","precision":2}],"uda":null,"cfa":null,"ifa":null})
        }
        "assetbalance" => {
            json!({"settled":500,"future":500,"spendable":500,"offchain_outbound":100,"offchain_inbound":0})
        }
        "decodelninvoice" => {
            json!({"asset_id":"rgb:demo","asset_amount":5,"amt_msat":1000,"expiry_sec":2402444800u64,"timestamp":1700000000u64,"payment_hash":"hash","payment_secret":"never-return-to-model","network":"Regtest"})
        }
        "sendpayment" => {
            json!({"payment_id":"node-id","payment_hash":"hash","payment_secret":"never-return-to-model","status":"Pending"})
        }
        "getpayment" => {
            json!({"payment":{"status":"Succeeded","payment_hash":"hash","preimage":"never-return-to-model"}})
        }
        _ => return Err(StatusCode::NOT_FOUND),
    }))
}
