use std::{
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-startup-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent"));
        command.env_clear().current_dir(&self.0);
        command
    }
    fn defaults(&self) {
        std::fs::write(self.0.join(".env.example"), "OPENAI_API_KEY=\nRGB_NODE_URL=http://127.0.0.1:3101\nALLOWED_ASSET_IDS=rgb:demo\nAUTO_APPROVE_BELOW=1\nMAX_SINGLE_PAYMENT=100\nMAX_DAILY_SPEND=500\nMAX_CARRIER_MSAT=3000000\n").unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn real_model_startup_requires_key_before_wallet_config() {
    let fixture = Fixture::new();
    fixture.defaults();
    let output = fixture.command().output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("OPENAI_API_KEY is required"));
    assert!(!stderr.contains("RGB_NODE_URL"));
}
#[test]
fn help_is_offline_and_does_not_require_credentials() {
    let fixture = Fixture::new();
    let output = fixture.command().arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("[y/N]"));
}
#[test]
fn example_supplies_missing_wallet_settings_and_empty_key_does_not_override_export() {
    let fixture = Fixture::new();
    fixture.defaults();
    // EOF exits the interactive loop before any model/node API request.
    let output = fixture
        .command()
        .env("OPENAI_API_KEY", "not-a-credential")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("RGB402 Agent"));
    assert!(!fixture.0.join(".wallet-state.lock").exists());
}
#[test]
fn exported_setting_wins_over_example() {
    let fixture = Fixture::new();
    fixture.defaults();
    let output = fixture
        .command()
        .env("OPENAI_API_KEY", "not-a-credential")
        .env("MAX_SINGLE_PAYMENT", "0")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("require 0 < single <= daily"));
}
