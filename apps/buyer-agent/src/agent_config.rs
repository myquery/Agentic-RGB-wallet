//! CLI-only fallback defaults. Run before creating the async runtime.
use std::{collections::HashMap, env, fs, io, path::Path};

const NAMES: &[&str] = &[
    "OPENAI_API_KEY",
    "AGENT_MODEL",
    "RGB_NODE_URL",
    "RGB_NODE_TOKEN",
    "ALLOWED_ASSET_IDS",
    "AUTO_APPROVE_BELOW",
    "MAX_SINGLE_PAYMENT",
    "MAX_DAILY_SPEND",
    "MAX_CARRIER_MSAT",
    "WALLET_STATE_PATH",
    "AUTO_APPROVE_MACHINE_BELOW_SATS",
    "MAX_MACHINE_PAYMENT_SATS",
    "MAX_MACHINE_DAILY_SPEND_SATS",
    "ALLOWED_L402_ORIGINS",
    "MACHINE_STATE_PATH",
];

pub fn load_defaults(path: &Path) -> io::Result<()> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(io::Error::other("could not read .env.example")),
    };
    let defaults = parse(&contents)?;
    for (name, value) in defaults {
        // An explicitly exported value (including empty) always wins.
        if env::var_os(&name).is_none() {
            env::set_var(name, value);
        }
    }
    Ok(())
}
fn parse(contents: &str) -> io::Result<HashMap<String, String>> {
    let mut defaults = HashMap::new();
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid .env.example assignment on line {}", index + 1),
            )
        };
        let (name, value) = line.split_once('=').ok_or_else(invalid)?;
        let name = name.trim();
        if !NAMES.contains(&name) {
            continue;
        }
        let value = value.trim();
        let value = if value.starts_with(['\'', '"']) {
            let quote = value.chars().next().ok_or_else(invalid)?;
            let end = value[1..].find(quote).map(|n| n + 1).ok_or_else(invalid)?;
            let rest = value[end + 1..].trim();
            if !rest.is_empty() && !rest.starts_with('#') {
                return Err(invalid());
            }
            &value[1..end]
        } else {
            value
                .split_once(" #")
                .map_or(value, |(value, _)| value)
                .trim()
        };
        if value.contains('\0') {
            return Err(invalid());
        }
        // Literal values only: no shell execution, interpolation, or secret logging.
        defaults.insert(name.into(), value.into());
    }
    Ok(defaults)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_defaults_without_shell_evaluation_or_unrelated_variables() {
        let values = parse("# defaults\nexport MAX_SINGLE_PAYMENT=100 # base units\nAGENT_MODEL='demo-model'\nOPENAI_API_KEY=\nRGB_NODE_TOKEN=\"$(not-executed)\"\nPATH=ignored\n").unwrap();
        assert_eq!(values["MAX_SINGLE_PAYMENT"], "100");
        assert_eq!(values["AGENT_MODEL"], "demo-model");
        assert_eq!(values["OPENAI_API_KEY"], "");
        assert_eq!(values["RGB_NODE_TOKEN"], "$(not-executed)");
        assert!(!values.contains_key("PATH"));
    }
    #[test]
    fn errors_include_line_number_without_value() {
        let error = parse("AGENT_MODEL='private-value").unwrap_err().to_string();
        assert!(error.contains("line 1"));
        assert!(!error.contains("private-value"));
    }
}
