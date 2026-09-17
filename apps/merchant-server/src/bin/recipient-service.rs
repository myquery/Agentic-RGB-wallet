//! Minimal recipient front door: no wallet execution, approval, or status routes.
use axum::{
    extract::{DefaultBodyLimit, Query, State},
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
    subject: String,
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
async fn webfinger(State(c): State<Arc<Config>>, Query(q): Query<Resource>) -> impl IntoResponse {
    if q.resource != c.subject {
        return StatusCode::NOT_FOUND.into_response();
    }
    ([("content-type", "application/jrd+json")], Json(json!({"subject":c.subject,"links":[{"rel":"https://rgb402.example/relations/rgb-invoice","href":format!("{}/rgb/invoice/alice", c.origin)}]}))).into_response()
}
fn valid(c: &Config, r: &Request) -> bool {
    r.subject == c.subject && r.asset_id == c.asset && r.asset_amount == 5
}
async fn invoice(
    State(c): State<Arc<Config>>,
    Json(r): Json<Request>,
) -> Result<Json<Value>, StatusCode> {
    // This public demo issues only the explicitly configured five-unit asset request.
    if !valid(&c, &r) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let response = c.client.post("http://127.0.0.1:3102/lninvoice")
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
        subject: format!("acct:alice@{domain}"),
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
        .route("/rgb/invoice/alice", post(invoice))
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
            subject: "acct:alice@example.com".into(),
            origin: "https://example.com".into(),
            asset: "rgb:test".into(),
            client: reqwest::Client::new(),
        };
        let mut r = Request {
            subject: c.subject.clone(),
            asset_id: c.asset.clone(),
            asset_amount: 5,
        };
        assert!(valid(&c, &r));
        r.asset_amount = 6;
        assert!(!valid(&c, &r));
        r.asset_amount = 5;
        r.subject = "acct:bob@example.com".into();
        assert!(!valid(&c, &r));
        r.subject = c.subject.clone();
        r.asset_id = "rgb:other".into();
        assert!(!valid(&c, &r));
        for amount in [json!(-1), json!(5.5), json!("5")] {
            assert!(serde_json::from_value::<Request>(
                json!({"subject":c.subject,"asset_id":c.asset,"asset_amount":amount})
            )
            .is_err());
        }
        assert!(serde_json::from_value::<Request>(
            json!({"subject":c.subject,"asset_id":c.asset,"asset_amount":5,"approve":true})
        )
        .is_err());
    }
}
