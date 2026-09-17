#[path = "../agent_config.rs"]
mod agent_config;

use rgb402_agent::wallet_agent::{openai::OpenAiModel, AgentModel, TurnOutcome, WalletAgent};
use rgb402_payment::{config::WalletConfig, rgb::RgbLightningClient, wallet::WalletService};
use std::{
    io::{self, BufRead, Write},
    sync::Arc,
};

const HELP: &str = "RGB402 Agent — demo regtest RGB wallet\nUsage: cargo run -p buyer-agent --bin agent\nRequired: OPENAI_API_KEY and existing wallet environment configuration.\nOptional: AGENT_MODEL (default gpt-4.1-mini). .env.example in the current directory supplies missing settings; exported variables win.\nAsk about assets, balances, an invoice, or a payment. Paste the invoice on the same line.\nEvery payment pauses at an application-owned [y/N] prompt.\nType /quit to exit. Maximum eight tool/model steps per turn.";

fn main() {
    if let Err(error) = launch() {
        eprintln!("Agent startup failed: {error}");
        std::process::exit(1);
    }
}
fn launch() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("{HELP}");
        return Ok(());
    }
    if !args.is_empty() {
        return Err("unknown argument; use --help".into());
    }
    // Set fallback environment before any runtime worker threads exist.
    agent_config::load_defaults(std::path::Path::new(".env.example"))?;
    tokio::runtime::Runtime::new()?.block_on(start())
}
async fn start() -> Result<(), Box<dyn std::error::Error>> {
    // Validate provider configuration before opening wallet state or contacting any node.
    let model = OpenAiModel::from_env()?;
    let config = WalletConfig::from_env()?;
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter("rgb402_agent=info,rgb402_payment=info")
        .init();
    let node = Arc::new(RgbLightningClient::new(
        &config.node_url,
        config.node_token.as_deref(),
    )?);
    let wallet = WalletService::open(node, config.policy, &config.state_path)?;
    let mut agent = WalletAgent::new(model, wallet);
    run(&mut agent, &mut io::stdin().lock(), &mut io::stdout()).await?;
    Ok(())
}
// Neutralize terminal controls in untrusted model text and metadata.
fn terminal_text(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if (c.is_control() && c != '\n' && c != '\t')
                || matches!(c,'\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                c.escape_unicode().to_string().chars().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
async fn run<M: AgentModel>(
    agent: &mut WalletAgent<M>,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<()> {
    writeln!(
        output,
        "RGB402 Agent — DEMO/TEST assets, regtest only. Type /quit to exit."
    )?;
    loop {
        write!(output, "> ")?;
        output.flush()?;
        let mut user = String::new();
        if input.read_line(&mut user)? == 0 {
            return Ok(());
        }
        let user = user.trim();
        if user == "/quit" || user == "/exit" {
            return Ok(());
        }
        if user.is_empty() {
            continue;
        }
        let mut message = user;
        loop {
            let mut display_error = None;
            let outcome = agent
                .turn_with_observer(message, |result| {
                    let rendered =
                        serde_json::to_string_pretty(result).expect("serializable wallet output");
                    if let Err(error) =
                        writeln!(output, "Wallet result:\n{}", terminal_text(&rendered))
                            .and_then(|_| output.flush())
                    {
                        display_error = Some(error);
                    }
                })
                .await;
            if let Some(error) = display_error {
                return Err(error);
            }
            match outcome {
                Ok(TurnOutcome::Reply(text)) => {
                    writeln!(output, "Agent: {}", terminal_text(&text))?;
                    break;
                }
                Ok(TurnOutcome::LimitReached) => {
                    writeln!(output,"Eight-step limit reached. Review wallet results; ask for payment status before retrying.")?;
                    break;
                }
                Err(error) => {
                    writeln!(output,"Model error: {error}. Wallet results above remain authoritative; a model error does not cancel a submitted payment.")?;
                    break;
                }
                Ok(TurnOutcome::MachinePrepared(_)) => {
                    eprintln!("Machine approval requires the PWA application.");
                }
                Ok(TurnOutcome::Prepared(plan)) => {
                    if let Some(recipient) = &plan.recipient {
                        writeln!(
                            output,
                            "Recipient: {}\nAuthoritative domain: {}",
                            terminal_text(&recipient.identifier),
                            terminal_text(&recipient.authoritative_domain)
                        )?;
                    }
                    writeln!(output,"Application payment confirmation\nAsset ID: {}\nAmount: {} base units\nAvailable outbound: {} base units\nInvoice/destination: {}\nPayment hash: {}\nCarrier: {} msat, plus node-managed routing fees\nPolicy: {}\nPlan ID: {}",
                        terminal_text(plan.request.asset_id.as_str()),plan.request.amount,plan.available_balance,
                        terminal_text(plan.recipient.as_ref().map(|r|r.identifier.as_str()).unwrap_or(&plan.request.invoice)),terminal_text(plan.request.payment_hash.as_str()),plan.request.carrier_msat,
                        terminal_text(&serde_json::to_string(&plan.policy).expect("serializable policy")),terminal_text(&plan.plan_id))?;
                    write!(output, "Approve this exact demo payment? [y/N] ")?;
                    output.flush()?;
                    let mut answer = String::new();
                    let read = input.read_line(&mut answer)?;
                    let affirmative =
                        matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
                    agent
                        .confirm_from_human(&plan.plan_id, affirmative)
                        .map_err(io::Error::other)?;
                    if !affirmative {
                        writeln!(output, "Payment cancelled.")?;
                        if read == 0 {
                            return Ok(());
                        }
                        break;
                    }
                    writeln!(output, "Application confirmed this plan. Continuing…")?;
                    message = "";
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "../agent_cli_tests.rs"]
mod tests;
