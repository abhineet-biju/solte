use serde_json::Value;

use crate::tokens::TokenAccount;

pub fn extension<'a>(info: &'a Value, name: &str) -> Option<&'a Value> {
    info["extensions"]
        .as_array()?
        .iter()
        .find(|item| item["extension"] == name)
        .map(|item| &item["state"])
}

pub fn enabled(account: &TokenAccount) -> bool {
    account.program == crate::tokens::TOKEN_2022
        && (extension(account.info(), "confidentialTransferAccount").is_some()
            || account
                .mint_info
                .as_ref()
                .is_some_and(|mint| extension(mint, "confidentialTransferMint").is_some()))
}

pub fn lines(account: &TokenAccount) -> Vec<String> {
    if !enabled(account) {
        return vec![];
    }
    let mut lines = vec![String::new(), "CONFIDENTIAL BALANCES".into()];
    if let Some(state) = extension(account.info(), "confidentialTransferAccount") {
        lines.extend([
            "Available    Locked · reveal with the owner wallet".into(),
            "Pending      Encrypted · apply before spending".into(),
            format!("Approved     {}", flag(state, "approved")),
            format!(
                "Receive private {}",
                flag(state, "allowConfidentialCredits")
            ),
            format!(
                "Receive public  {}",
                flag(state, "allowNonConfidentialCredits")
            ),
            format!(
                "Pending credits {} / {}",
                number(state, "pendingBalanceCreditCounter"),
                number(state, "maximumPendingBalanceCreditCounter")
            ),
        ]);
    } else {
        lines.push("Account not configured for confidential balances".into());
    }
    if let Some(mint) = account
        .mint_info
        .as_ref()
        .and_then(|mint| extension(mint, "confidentialTransferMint"))
    {
        lines.extend([
            format!("Auto approval {}", flag(mint, "autoApproveNewAccounts")),
            format!(
                "Approval authority {}",
                mint["authority"].as_str().unwrap_or("None")
            ),
            format!(
                "Auditor {}",
                mint["auditorElgamalPubkey"].as_str().unwrap_or("None")
            ),
        ]);
    }
    lines.push("Addresses stay public. Deposits and withdrawals reveal amounts.".into());
    lines
}

fn flag(state: &Value, key: &str) -> &'static str {
    match state[key].as_bool() {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unavailable",
    }
}

fn number(state: &Value, key: &str) -> String {
    match &state[key] {
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        _ => "Unavailable".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovery_distinguishes_locked_balances_from_zero_and_missing_state() {
        let mut app =
            crate::app::App::new("/test".into(), crate::config::Config::default(), vec![]);
        crate::demo::populate(&mut app);
        let account = &mut app.tokens[2];
        account.amount = 0;
        account.account["data"]["parsed"]["info"]["extensions"] = json!([
            {"extension":"confidentialTransferAccount","state":{"approved":false,"pendingBalanceCreditCounter":2,"maximumPendingBalanceCreditCounter":10}}
        ]);
        let text = lines(account).join("\n");
        assert!(text.contains("Locked") && text.contains("2 / 10"));
        assert!(text.contains("Approved     No") && text.contains("Unavailable"));
        app.token_filter = "confidential".into();
        assert_eq!(app.visible_tokens().len(), 1);
    }
}
