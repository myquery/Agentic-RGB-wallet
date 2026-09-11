use std::process::Command;
#[test]
fn real_model_startup_requires_environment_key_before_wallet_config() {
    let output = Command::new(env!("CARGO_BIN_EXE_agent"))
        .env_remove("OPENAI_API_KEY")
        .env_remove("RGB_NODE_URL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("OPENAI_API_KEY is required in the process environment"));
    assert!(!stderr.contains("RGB_NODE_URL"));
}
#[test]
fn help_is_offline_and_does_not_require_credentials() {
    let output = Command::new(env!("CARGO_BIN_EXE_agent"))
        .arg("--help")
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("[y/N]"));
}
