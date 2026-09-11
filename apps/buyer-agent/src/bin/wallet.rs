use rgb402_core::{wallet::PolicyDecision, AssetId, PaymentId};
use rgb402_payment::{config::WalletConfig, rgb::RgbLightningClient, wallet::WalletService};
use std::{
    io::{self, Write},
    sync::Arc,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter("rgb402_payment=info")
        .init();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!("Demo/test RGB wallet (regtest only)\nwallet assets\nwallet balance [ASSET_ID]\nwallet decode INVOICE\nwallet prepare INVOICE\nwallet pay INVOICE\nwallet status PAYMENT_HASH\nAmounts and policy limits use integer asset base units.");
        return Ok(());
    }
    let config = WalletConfig::from_env()?;
    let node = Arc::new(RgbLightningClient::new(
        &config.node_url,
        config.node_token.as_deref(),
    )?);
    let mut wallet = WalletService::open(node, config.policy, &config.state_path)?;
    let argument = || {
        args.get(1)
            .map(String::as_str)
            .ok_or("missing command argument")
    };
    eprintln!("DEMO/TEST RGB ASSETS — regtest only; not official Tether USD₮");
    match args[0].as_str() {
        "assets" => println!("{}", serde_json::to_string_pretty(&wallet.assets().await?)?),
        "balance" => {
            if let Some(id) = args.get(1) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&wallet.balance(&AssetId::new(id)?).await?)?
                );
            } else {
                for asset in wallet.assets().await? {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&wallet.balance(&asset.asset_id).await?)?
                    );
                }
            }
        }
        "decode" => println!(
            "{}",
            serde_json::to_string_pretty(&wallet.decode(argument()?).await?)?
        ),
        "prepare" | "pay" => {
            let plan = wallet.prepare_payment(argument()?).await?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
            if args[0] == "prepare" {
                return Ok(());
            }
            if let PolicyDecision::Deny { reason } = plan.policy() {
                return Err(reason.clone().into());
            }
            if matches!(plan.policy(), PolicyDecision::RequireApproval { .. }) {
                print!("Approve this exact demo payment? [y/N] ");
                io::stdout().flush()?;
                let mut answer = String::new();
                io::stdin().read_line(&mut answer)?;
                if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    println!("Payment cancelled.");
                    return Ok(());
                }
                wallet.approve_from_human(plan.plan_id())?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&wallet.execute_payment(plan.plan_id()).await?)?
            );
        }
        "status" => println!(
            "{}",
            serde_json::to_string_pretty(
                &wallet.payment_status(&PaymentId::new(argument()?)?).await?
            )?
        ),
        _ => return Err("unknown command; use --help".into()),
    }
    Ok(())
}
