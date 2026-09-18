//! Minimal recipient front door: no wallet execution, approval, or status routes.
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
struct Config {
    domain: String,
    origin: String,
    asset: String,
    client: reqwest::Client,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resource {
    resource: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    subject: String,
    asset_id: String,
    asset_amount: u64,
}
fn node_port(account: &str) -> Option<u16> {
    match account {
        "alice" => Some(3101),
        "bob" => Some(3102),
        _ => None,
    }
}
fn subject(c: &Config, account: &str) -> String {
    format!("acct:{account}@{}", c.domain)
}
async fn webfinger(State(c): State<Arc<Config>>, Query(q): Query<Resource>) -> impl IntoResponse {
    let account = ["alice", "bob"]
        .into_iter()
        .find(|account| q.resource == subject(&c, account));
    let Some(account) = account else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ([("content-type", "application/jrd+json")], Json(json!({"subject":subject(&c,account),"links":[{"rel":"https://rgb402.example/relations/rgb-invoice","href":format!("{}/rgb/invoice/{account}", c.origin)},{"rel":"https://rgb402.example/relations/btc-invoice","href":format!("{}/btc/invoice/{account}", c.origin)}]}))).into_response()
}
fn valid(c: &Config, account: &str, r: &Request) -> bool {
    node_port(account).is_some()
        && r.subject == subject(c, account)
        && r.asset_id == c.asset
        && r.asset_amount > 0
}
async fn invoice(
    State(c): State<Arc<Config>>,
    Path(account): Path<String>,
    Json(r): Json<Request>,
) -> Result<Json<Value>, StatusCode> {
    if !valid(&c, &account, &r) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let port = node_port(&account).ok_or(StatusCode::NOT_FOUND)?;
    let response = c.client.post(format!("http://127.0.0.1:{port}/lninvoice"))
        .json(&json!({"asset_id":r.asset_id,"asset_amount":r.asset_amount,"amt_msat":3_000_000,"expiry_sec":3600}))
        .send().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !response.status().is_success() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let value: Value = response.json().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
    let invoice = value
        .get("invoice")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 12 * 1024)
        .ok_or(StatusCode::BAD_GATEWAY)?;
    Ok(Json(json!({"invoice":invoice})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BtcRequest {
    subject: String,
    amount_sats: u64,
}
fn btc_payload(c: &Config, account: &str, r: &BtcRequest) -> Result<Value, StatusCode> {
    if node_port(account).is_none() || r.subject != subject(c, account) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let msat = r
        .amount_sats
        .checked_mul(1000)
        .filter(|n| *n > 0)
        .ok_or(StatusCode::BAD_REQUEST)?;
    Ok(json!({"amt_msat":msat,"expiry_sec":3600,"description":"RGB402 BTC recipient transfer"}))
}
async fn btc_invoice(
    State(c): State<Arc<Config>>,
    Path(account): Path<String>,
    Json(r): Json<BtcRequest>,
) -> Result<Json<Value>, StatusCode> {
    let payload = btc_payload(&c, &account, &r)?;
    let port = node_port(&account).ok_or(StatusCode::NOT_FOUND)?;
    let response = c
        .client
        .post(format!("http://127.0.0.1:{port}/lninvoice"))
        .json(&payload)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !response.status().is_success() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let value: Value = response.json().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
    let invoice = value
        .get("invoice")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .ok_or(StatusCode::BAD_GATEWAY)?;
    Ok(Json(json!({"invoice":invoice})))
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let domain = std::env::var("RECIPIENT_DOMAIN").map_err(|_| "missing RECIPIENT_DOMAIN")?;
    if domain.len() > 253
        || !domain.contains('.')
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
    {
        return Err("invalid RECIPIENT_DOMAIN hostname".into());
    }
    let asset = std::env::var("RECIPIENT_ASSET_ID").map_err(|_| "missing RECIPIENT_ASSET_ID")?;
    let config = Arc::new(Config {
        domain: domain.clone(),
        origin: format!("https://{domain}"),
        asset,
        client: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .build()?,
    });
    let app = Router::new()
        .route("/.well-known/webfinger", get(webfinger))
        .route("/rgb/invoice/:account", post(invoice))
        .route("/btc/invoice/:account", post(btc_invoice))
        .layer(DefaultBodyLimit::max(2048))
        .with_state(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3050").await?;
    eprintln!("Recipient-only service listening on 127.0.0.1:3050");
    axum::serve(listener, app).await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_issuance_rejects_changed_intent_and_noninteger_amounts() {
        let c = Config {
            domain: "example.com".into(),
            origin: "https://example.com".into(),
            asset: "rgb:test".into(),
            client: reqwest::Client::new(),
        };
        let mut r = Request {
            subject: subject(&c, "bob"),
            asset_id: c.asset.clone(),
            asset_amount: 5,
        };
        assert!(valid(&c, "bob", &r));
        for amount in [1, 5, 6, 50, u64::MAX] {
            r.asset_amount = amount;
            assert!(valid(&c, "bob", &r));
        }
        r.asset_amount = 0;
        assert!(!valid(&c, "bob", &r));
        r.asset_amount = 5;
        r.subject = "acct:alice@example.com".into();
        assert!(!valid(&c, "bob", &r));
        r.subject = subject(&c, "bob");
        r.asset_id = "rgb:other".into();
        assert!(!valid(&c, "bob", &r));
        for amount in [json!(-1), json!(5.5), json!("5")] {
            assert!(serde_json::from_value::<Request>(
                json!({"subject":subject(&c,"bob"),"asset_id":c.asset,"asset_amount":amount})
            )
            .is_err());
        }
        assert!(serde_json::from_value::<Request>(
            json!({"subject":subject(&c,"bob"),"asset_id":c.asset,"asset_amount":5,"approve":true})
        )
        .is_err());
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::*;
    #[tokio::test]
    async fn each_alias_advertises_only_its_own_invoice_route() {
        use axum::body::to_bytes;
        let c = Arc::new(Config {
            domain: "example.com".into(),
            origin: "https://example.com".into(),
            asset: "rgb:test".into(),
            client: reqwest::Client::new(),
        });
        for (account, port) in [("alice", 3101), ("bob", 3102)] {
            assert_eq!(node_port(account), Some(port));
            let response = webfinger(
                State(c.clone()),
                Query(Resource {
                    resource: subject(&c, account),
                }),
            )
            .await
            .into_response();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), 16384).await.unwrap();
            let j: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(j["subject"], subject(&c, account));
            assert_eq!(
                j["links"][0]["href"],
                format!("https://example.com/rgb/invoice/{account}")
            );
            let r = Request {
                subject: subject(&c, account),
                asset_id: c.asset.clone(),
                asset_amount: 5,
            };
            assert!(valid(&c, account, &r));
            assert!(!valid(
                &c,
                if account == "alice" { "bob" } else { "alice" },
                &r
            ));
        }
        assert_eq!(node_port("mallory"), None);
        let response = webfinger(
            State(c),
            Query(Resource {
                resource: "acct:mallory@example.com".into(),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[cfg(test)]
mod btc_tests {
    use super::*;
    #[test]
    fn btc_request_preserves_sats_and_never_adds_rgb_carrier() {
        let c = Config {
            domain: "example.com".into(),
            origin: "https://example.com".into(),
            asset: "rgb:demo".into(),
            client: reqwest::Client::new(),
        };
        for account in ["alice", "bob"] {
            let r = BtcRequest {
                subject: subject(&c, account),
                amount_sats: 5,
            };
            let payload = btc_payload(&c, account, &r).unwrap();
            assert_eq!(payload["amt_msat"], 5000);
            assert!(payload.get("asset_id").is_none());
            assert!(payload.get("asset_amount").is_none());
            assert!(btc_payload(&c, if account == "alice" { "bob" } else { "alice" }, &r).is_err());
        }
        for amount in [0, u64::MAX] {
            assert!(btc_payload(
                &c,
                "alice",
                &BtcRequest {
                    subject: subject(&c, "alice"),
                    amount_sats: amount
                }
            )
            .is_err())
        }
        for amount in [json!(-1), json!(5.1), json!("5")] {
            assert!(serde_json::from_value::<BtcRequest>(
                json!({"subject":subject(&c,"alice"),"amount_sats":amount})
            )
            .is_err())
        }
        assert!(serde_json::from_value::<BtcRequest>(
            json!({"subject":subject(&c,"alice"),"amount_sats":5,"asset_id":"rgb:demo"})
        )
        .is_err());
    }
}
