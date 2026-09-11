use rgb402_agent::{AgentOutcome, BuyerAgent};
use rgb402_core::{AssetId, PaymentAmount, PolicyDecision, SpendingPolicy};
use rgb402_payment::{PaymentProvider, SimulatedRgbPaymentProvider};
use std::env;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "buyer_agent=info".to_owned()))
        .init();

    let merchant_url =
        env::var("RGB402_MERCHANT_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_owned());
    let ledger_path =
        env::var("RGB402_SIM_LEDGER_PATH").unwrap_or_else(|_| ".rgb402-sim-ledger.json".to_owned());

    let boss = AssetId::boss();
    let max_payment = PaymentAmount::from_major_minor(5, 0, 2)?;
    let session_budget = PaymentAmount::from_major_minor(20, 0, 2)?;
    let policy = SpendingPolicy::new(vec![boss.clone()], max_payment, session_budget);
    let provider: Arc<dyn PaymentProvider> =
        Arc::new(SimulatedRgbPaymentProvider::with_json_file(&ledger_path));
    let mut agent = BuyerAgent::new(merchant_url, policy, provider)?;

    println!("Requesting premium-analysis...");

    match agent.fetch_paid_resource("/premium-analysis").await? {
        AgentOutcome::Purchased {
            challenge,
            decision,
            receipt,
            body,
            session_spend,
        } => {
            println!();
            println!("Merchant response: 402 Payment Required");
            println!();
            println!("Payment challenge:");
            println!("asset: {}", challenge.asset_id);
            println!("amount: {}", challenge.amount);
            println!("invoice: {}", challenge.invoice);
            println!();
            println!("Policy:");
            println!("allowed_asset: yes");
            println!("within_max_payment: yes");
            println!("within_budget: yes");
            println!();
            print_decision(&decision);
            println!();
            println!("Paying simulated RGB invoice...");
            println!();
            println!("Payment settled:");
            println!("payment_id: {}", receipt.payment_id);
            println!();
            println!("Retrying premium-analysis...");
            println!();
            println!("200 OK");
            println!();
            println!("Premium analysis received:");
            println!("{body}");
            println!();
            println!("Session spend:");
            println!("{session_spend} / {session_budget} {}", boss);
        }
        AgentOutcome::Rejected {
            challenge,
            decision,
        } => {
            println!();
            println!("Merchant response: 402 Payment Required");
            println!("asset: {}", challenge.asset_id);
            println!("amount: {}", challenge.amount);
            println!();
            print_decision(&decision);
            println!("No payment performed.");
        }
        AgentOutcome::AlreadyAccessible { body } => {
            println!("200 OK");
            println!("{body}");
        }
    }

    Ok(())
}

fn print_decision(decision: &PolicyDecision) {
    match decision {
        PolicyDecision::Approved { .. } => println!("Decision: APPROVED"),
        PolicyDecision::Rejected { reason } => println!("Decision: REJECTED ({reason:?})"),
    }
}
