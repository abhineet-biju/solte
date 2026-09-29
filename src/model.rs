use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::amount::format_sol;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub error: Option<String>,
    pub details: Option<Value>,
}

impl TransactionRecord {
    pub fn version_label(&self) -> String {
        match self.details.as_ref().and_then(|d| d.get("version")) {
            Some(Value::String(value)) if value == "legacy" => "Legacy".into(),
            Some(Value::Number(value)) => format!("v{value}"),
            _ => "Unknown".into(),
        }
    }

    pub fn detail_failure(&mut self, message: &str) {
        let details = self.details.get_or_insert_with(|| serde_json::json!({}));
        details["_solteDetailError"] = message.into();
    }

    pub fn fee(&self) -> Option<u64> {
        self.details.as_ref()?.pointer("/meta/fee")?.as_u64()
    }

    pub fn balance_change(&self, address: &str) -> Option<i128> {
        let value = self.details.as_ref()?;
        let keys = value
            .pointer("/transaction/message/accountKeys")?
            .as_array()?;
        let index = keys.iter().position(|key| {
            key.as_str() == Some(address)
                || key.get("pubkey").and_then(Value::as_str) == Some(address)
        })?;
        let pre = value.pointer("/meta/preBalances")?.get(index)?.as_u64()?;
        let post = value.pointer("/meta/postBalances")?.get(index)?.as_u64()?;
        Some(i128::from(post) - i128::from(pre))
    }

    pub fn activity(&self, address: &str) -> String {
        if self.error.is_some() {
            return "Failed".into();
        }
        match self.balance_change(address) {
            Some(delta) if delta > 0 => "Received".into(),
            Some(delta) if delta < -i128::from(self.fee().unwrap_or(0)) => "Sent".into(),
            _ => self.kind(),
        }
    }

    pub fn kind(&self) -> String {
        if self.error.is_some() {
            return "Failed".into();
        }
        self.details
            .as_ref()
            .and_then(|d| d.pointer("/transaction/message/instructions"))
            .and_then(Value::as_array)
            .and_then(|v| v.first())
            .and_then(|v| v.pointer("/parsed/type"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| "Transaction".into())
    }

    pub fn log_lines(&self) -> Vec<String> {
        match self
            .details
            .as_ref()
            .and_then(|d| d.pointer("/meta/logMessages"))
        {
            Some(Value::Array(lines)) => {
                if lines.is_empty() {
                    vec!["No program logs were emitted.".into()]
                } else {
                    lines
                        .iter()
                        .filter_map(Value::as_str)
                        .map(clean_text)
                        .collect()
                }
            }
            _ => vec!["Program logs are not available from the RPC yet.".into()],
        }
    }

    pub fn inspection_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Signature  {}", self.signature),
            format!("Slot       {}", self.slot),
            format!("Format     {}", self.version_label()),
        ];
        if let Some(error) = &self.error {
            lines.push(format!("Error      {}", clean_text(error)));
        }
        if let Some(fee) = self.fee() {
            lines.push(format!("Fee        {} SOL", format_sol(fee)));
        }
        if let Some(value) = &self.details {
            if let Some(error) = value.get("_solteDetailError").and_then(Value::as_str) {
                lines.push(format!("Details unavailable: {}", clean_text(error)));
            }
            if let Some(units) = value
                .pointer("/meta/computeUnitsConsumed")
                .and_then(Value::as_u64)
            {
                lines.push(format!("Compute    {units} CU"));
            }
            lines.push(String::new());
            lines.push("PROGRAM LOGS".into());
            lines.extend(self.log_lines());
            for (title, pointer) in [
                (
                    "V1 RESOURCE CONFIG",
                    "/transaction/message/transactionConfig",
                ),
                ("ACCOUNTS", "/transaction/message/accountKeys"),
                (
                    "ADDRESS LOOKUP TABLES",
                    "/transaction/message/addressTableLookups",
                ),
                ("INSTRUCTIONS", "/transaction/message/instructions"),
                ("INNER INSTRUCTIONS", "/meta/innerInstructions"),
                ("TOKEN BALANCES BEFORE", "/meta/preTokenBalances"),
                ("TOKEN BALANCES AFTER", "/meta/postTokenBalances"),
            ] {
                if let Some(data) = value.pointer(pointer) {
                    lines.push(String::new());
                    lines.push(title.into());
                    lines.extend(
                        serde_json::to_string_pretty(data)
                            .unwrap_or_default()
                            .lines()
                            .map(clean_text),
                    );
                }
            }
        } else {
            lines.push("Details have not been fetched or are unavailable from this RPC.".into());
        }
        lines
    }
}

#[derive(Clone, Debug, Default)]
pub struct NetworkState {
    pub genesis: String,
    pub cluster: String,
    pub slot: u64,
    pub block_height: u64,
    pub epoch: u64,
    pub latency_ms: u128,
    pub healthy: bool,
    pub version: String,
    pub token_accounts: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: u64,
    pub level: String,
    pub message: String,
}

impl LogEntry {
    pub fn preview(&self) -> String {
        let mut text = clean_text(&self.message).replace('\n', " · ");
        for value in self.message.split(|c: char| !c.is_ascii_alphanumeric()) {
            if value.parse::<solana_signature::Signature>().is_ok() {
                text = text.replace(value, &short(value));
            }
        }
        text
    }

    pub fn signature(&self) -> Option<String> {
        self.message
            .split(|c: char| !c.is_ascii_alphanumeric())
            .find(|part| part.parse::<solana_signature::Signature>().is_ok())
            .map(str::to_owned)
    }

    pub fn new(level: &str, message: impl Into<String>) -> Self {
        Self {
            timestamp: now(),
            level: level.into(),
            message: clean_text(&message.into()),
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn clean_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}

pub fn short(value: &str) -> String {
    if value.chars().count() <= 16 {
        value.into()
    } else {
        format!(
            "{}…{}",
            value.chars().take(7).collect::<String>(),
            value
                .chars()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_signatures_require_a_full_valid_signature() {
        let signature = solana_signature::Signature::from([7; 64]).to_string();
        assert_eq!(
            LogEntry::new("INFO", format!("Confirmed ({signature}).")).signature(),
            Some(signature)
        );
        let address = solana_pubkey::Pubkey::new_from_array([7; 32]).to_string();
        assert!(
            LogEntry::new("INFO", format!("Wallet {address}; 5abc…def"))
                .signature()
                .is_none()
        );
    }

    #[test]
    fn activity_distinguishes_inflow_outflow_fees_and_failures() {
        let mut record = TransactionRecord {
            signature: String::new(),
            slot: 0,
            timestamp: None,
            error: None,
            details: Some(
                serde_json::json!({"transaction":{"message":{"accountKeys":["wallet"]}},"meta":{"fee":5,"preBalances":[100],"postBalances":[150]}}),
            ),
        };
        assert_eq!(record.activity("wallet"), "Received");
        record.details.as_mut().unwrap()["meta"]["postBalances"][0] = 50.into();
        assert_eq!(record.activity("wallet"), "Sent");
        record.details.as_mut().unwrap()["meta"]["postBalances"][0] = 95.into();
        assert_eq!(record.activity("wallet"), "Transaction");
        record.error = Some("Failed".into());
        assert_eq!(record.activity("wallet"), "Failed");
        assert_eq!(record.activity("other"), "Failed");
    }

    #[test]
    fn missing_logs_are_not_reported_as_empty() {
        let record = TransactionRecord {
            signature: "test".into(),
            slot: 1,
            timestamp: None,
            error: None,
            details: None,
        };
        assert!(record.log_lines()[0].contains("not available"));
        let record = TransactionRecord {
            details: Some(serde_json::json!({"meta":{"logMessages":[]}})),
            ..record
        };
        assert!(record.log_lines()[0].contains("No program logs"));
    }

    #[test]
    fn balance_changes_support_large_values_without_overflow() {
        let record = TransactionRecord {
            signature: "test".into(),
            slot: 1,
            timestamp: None,
            error: None,
            details: Some(
                serde_json::json!({"transaction":{"message":{"accountKeys":[{"pubkey":"wallet"}]}},"meta":{"preBalances":[18446744073709551615u64],"postBalances":[0]}}),
            ),
        };
        assert_eq!(record.balance_change("wallet"), Some(-i128::from(u64::MAX)));
    }
}
