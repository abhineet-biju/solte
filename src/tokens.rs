use std::{collections::BTreeSet, path::Path, str::FromStr};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::request::RpcRequest;

pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const ASSOCIATED_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenAccount {
    pub address: String,
    pub mint: String,
    pub authority: String,
    pub program: String,
    pub amount: u64,
    pub decimals: u8,
    pub lamports: u64,
    pub associated: bool,
    pub state: String,
    pub slot: u64,
    pub account: Value,
    pub mint_info: Option<Value>,
    #[serde(skip)]
    pub mint_label: Option<String>,
    #[serde(skip)]
    pub account_label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub genesis: String,
    pub accounts: Vec<TokenAccount>,
    pub mints: Vec<crate::mints::ProjectMint>,
    pub warnings: Vec<String>,
    pub labels: crate::labels::Labels,
    pub labels_revision: u64,
}

pub fn associated_address(owner: &Pubkey, mint: &Pubkey, program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), program.as_ref(), mint.as_ref()],
        &Pubkey::from_str(ASSOCIATED_PROGRAM).expect("associated program address"),
    )
    .0
}

pub fn format_amount(amount: u64, decimals: u8) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let digits = format!("{:0>width$}", amount, width = usize::from(decimals) + 1);
    let split = digits.len() - usize::from(decimals);
    let fraction = digits[split..].trim_end_matches('0');
    if fraction.is_empty() {
        digits[..split].into()
    } else {
        format!("{}.{}", &digits[..split], fraction)
    }
}

pub fn parse_amount(value: &str, decimals: u8) -> Result<u64> {
    let amount = parse_supply(value, decimals)?;
    ensure!(amount > 0, "Amount must be greater than zero");
    Ok(amount)
}

pub fn parse_supply(value: &str, decimals: u8) -> Result<u64> {
    let (whole, fraction) = value.trim().split_once('.').unwrap_or((value.trim(), ""));
    ensure!(
        !whole.is_empty()
            && whole.bytes().all(|c| c.is_ascii_digit())
            && fraction.bytes().all(|c| c.is_ascii_digit())
            && fraction.len() <= usize::from(decimals),
        "Enter an exact amount with at most {decimals} decimal places"
    );
    let digits = format!("{whole}{fraction:0<width$}", width = usize::from(decimals));
    let significant = digits.trim_start_matches('0');
    let amount: u64 = if significant.is_empty() {
        "0"
    } else {
        significant
    }
    .parse()
    .context("Amount must be positive and fit the token's raw u64 balance")?;
    Ok(amount)
}

impl TokenAccount {
    pub fn program_label(&self) -> &'static str {
        if self.program == TOKEN_2022 {
            "Token-2022"
        } else {
            "SPL Token"
        }
    }
    pub fn info(&self) -> &Value {
        &self.account["data"]["parsed"]["info"]
    }
    pub fn label(&self) -> String {
        if let Some(label) = &self.mint_label {
            return crate::model::clean_text(label);
        }
        self.mint_info
            .as_ref()
            .and_then(|info| info["extensions"].as_array())
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["extension"] == "tokenMetadata")
            })
            .and_then(|item| item["state"]["symbol"].as_str())
            .filter(|s| !s.is_empty())
            .map(crate::model::clean_text)
            .unwrap_or_else(|| crate::model::short(&self.mint))
    }
    pub fn lines(&self) -> Vec<String> {
        let info = self.info();
        let mut lines = vec![
            format!("{} · {}", self.label(), self.program_label()),
            String::new(),
            "BALANCE".into(),
            format!(
                "Public balance  {}",
                format_amount(self.amount, self.decimals)
            ),
            format!("Raw amount      {}", self.amount),
            format!("Decimals        {}", self.decimals),
            String::new(),
            "ACCOUNT".into(),
            format!(
                "Type       {}",
                if self.associated {
                    "Associated (ATA)"
                } else {
                    "Custom"
                }
            ),
            format!("State      {}", self.state),
            format!(
                "Lamports   {} · {} SOL",
                self.lamports,
                crate::amount::format_sol(self.lamports)
            ),
            format!(
                "Size       {}",
                self.account["space"]
                    .as_u64()
                    .map(|size| format!("{size} bytes"))
                    .unwrap_or_else(|| "Unavailable".into())
            ),
            format!("Read slot  {}", self.slot),
            String::new(),
            "ADDRESSES".into(),
            "Token account".into(),
            self.address.clone(),
            String::new(),
            "Mint".into(),
            self.mint.clone(),
            String::new(),
            "Wallet authority".into(),
            self.authority.clone(),
            String::new(),
            "Owning program".into(),
            self.program.clone(),
            String::new(),
            "PERMISSIONS".into(),
            format!(
                "Delegate         {}",
                info["delegate"].as_str().unwrap_or("None")
            ),
            format!(
                "Delegated raw    {}",
                info["delegatedAmount"]["amount"].as_str().unwrap_or("0")
            ),
            format!(
                "Close authority  {}",
                info["closeAuthority"]
                    .as_str()
                    .unwrap_or("Wallet authority")
            ),
            format!(
                "Wrapped SOL      {}",
                info["isNative"].as_bool().unwrap_or(false)
            ),
            String::new(),
            "MINT DETAILS".into(),
        ];
        if let Some(label) = &self.account_label {
            lines.insert(
                8,
                format!("Local name  {}", crate::model::clean_text(label)),
            );
        }
        if let Some(mint) = &self.mint_info {
            for (name, key) in [
                ("Supply · raw", "supply"),
                ("Mint authority", "mintAuthority"),
                ("Freeze authority", "freezeAuthority"),
            ] {
                lines.push(format!("{name}: {}", mint[key].as_str().unwrap_or("None")));
            }
            lines.extend(extensions("MINT EXTENSIONS", mint));
        } else {
            lines.push("Mint details unavailable from this RPC".into());
        }
        lines.extend(crate::confidential::lines(self));
        lines.extend(extensions("ACCOUNT EXTENSIONS", info));
        lines
            .into_iter()
            .map(|line| crate::model::clean_text(&line))
            .collect()
    }
}

fn extensions(title: &str, info: &Value) -> Vec<String> {
    let mut lines = vec![String::new(), title.into()];
    if let Some(items) = info["extensions"].as_array().filter(|v| !v.is_empty()) {
        for item in items {
            lines.push(
                item["extension"]
                    .as_str()
                    .unwrap_or("Unknown extension")
                    .into(),
            );
            if let Some(state) = item.get("state") {
                lines.extend(
                    serde_json::to_string_pretty(state)
                        .unwrap_or_default()
                        .lines()
                        .map(str::to_owned),
                );
            }
        }
    } else {
        lines.push("None reported".into());
    }
    lines
}

pub fn decode(address: &str, account: Value, owner: &Pubkey, slot: u64) -> Result<TokenAccount> {
    let address: Pubkey = address.parse()?;
    let program: Pubkey = account["owner"]
        .as_str()
        .context("Missing token program")?
        .parse()?;
    ensure!(
        [TOKEN_PROGRAM, TOKEN_2022].contains(&program.to_string().as_str()),
        "Unexpected token program"
    );
    ensure!(
        account["data"]["parsed"]["type"] == "account",
        "RPC did not decode a token account"
    );
    let info = &account["data"]["parsed"]["info"];
    let authority: Pubkey = info["owner"]
        .as_str()
        .context("Missing authority")?
        .parse()?;
    ensure!(
        authority == *owner,
        "Token authority differs from selected wallet"
    );
    let mint: Pubkey = info["mint"].as_str().context("Missing mint")?.parse()?;
    let amount = info["tokenAmount"]["amount"]
        .as_str()
        .context("Missing exact token amount")?
        .parse()?;
    let decimals = u8::try_from(
        info["tokenAmount"]["decimals"]
            .as_u64()
            .context("Missing decimals")?,
    )?;
    let state = info["state"]
        .as_str()
        .context("Missing token state")?
        .to_owned();
    let lamports = account["lamports"].as_u64().context("Missing lamports")?;
    Ok(TokenAccount {
        address: address.to_string(),
        mint: mint.to_string(),
        authority: authority.to_string(),
        program: program.to_string(),
        amount,
        decimals,
        lamports,
        associated: address == associated_address(owner, &mint, &program),
        state,
        slot,
        account,
        mint_info: None,
        mint_label: None,
        account_label: None,
    })
}

pub async fn fetch(rpc: &RpcClient, owner: &Pubkey) -> Result<Snapshot> {
    let genesis = rpc.get_genesis_hash().await?.to_string();
    let mut accounts = Vec::new();
    let mut warnings = Vec::new();
    let mut successful = 0;
    for program in [TOKEN_PROGRAM, TOKEN_2022] {
        let response: Result<Value, _> = rpc.send(RpcRequest::GetTokenAccountsByOwner,
            json!([owner.to_string(), {"programId":program}, {"encoding":"jsonParsed","commitment":"confirmed"}])).await;
        match response {
            Ok(value) => {
                let Some(items) = value["value"].as_array() else {
                    warnings.push(format!("Invalid account response for {program}"));
                    continue;
                };
                successful += 1;
                let slot = value["context"]["slot"].as_u64().unwrap_or(0);
                for item in items {
                    match decode(
                        item["pubkey"].as_str().unwrap_or_default(),
                        item["account"].clone(),
                        owner,
                        slot,
                    ) {
                        Ok(account) => accounts.push(account),
                        Err(_) => warnings.push(
                            "An account could not be decoded; account coverage is incomplete"
                                .into(),
                        ),
                    }
                }
            }
            Err(_) => warnings.push(format!(
                "Discovery unavailable for {}",
                if program == TOKEN_2022 {
                    "Token-2022"
                } else {
                    "SPL Token"
                }
            )),
        }
    }
    if successful == 0 {
        bail!("Token discovery failed for both programs; retry with [r]");
    }
    let mints: Vec<_> = accounts
        .iter()
        .map(|a| a.mint.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    for chunk in mints.chunks(100) {
        let response: Result<Value, _> = rpc
            .send(
                RpcRequest::GetMultipleAccounts,
                json!([chunk, {"encoding":"jsonParsed","commitment":"confirmed"}]),
            )
            .await;
        if let Ok(value) = response
            && let Some(items) = value["value"].as_array()
        {
            for (mint, value) in chunk.iter().zip(items) {
                for account in accounts.iter_mut().filter(|a| &a.mint == mint) {
                    if value["owner"] == account.program
                        && value["data"]["parsed"]["type"] == "mint"
                        && value["data"]["parsed"]["info"]["decimals"].as_u64()
                            == Some(u64::from(account.decimals))
                    {
                        account.mint_info = Some(value["data"]["parsed"]["info"].clone());
                    }
                }
            }
        } else {
            warnings.push("Some mint details are unavailable".into());
        }
    }
    if accounts.iter().any(|account| account.mint_info.is_none()) {
        warnings.push("Some mint details are unavailable".into());
    }
    warnings.sort();
    warnings.dedup();
    ensure!(
        rpc.get_genesis_hash().await?.to_string() == genesis,
        "Network changed during token discovery; refresh again"
    );
    accounts.sort_by(|a, b| (&a.mint, &a.address).cmp(&(&b.mint, &b.address)));
    accounts.dedup_by(|a, b| a.address == b.address);
    Ok(Snapshot {
        genesis,
        accounts,
        mints: vec![],
        warnings,
        labels: crate::labels::Labels::default(),
        labels_revision: 0,
    })
}

pub fn export(account: &TokenAccount, path: &Path) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("Cannot create export; choose a new file path")?;
    file.write_all(serde_json::to_string_pretty(account)?.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_preserve_all_raw_digits_without_rounding() {
        for decimals in [0, 6, 9, 19, 255] {
            for raw in [1, 10, 1_000_001, u64::MAX] {
                assert_eq!(
                    parse_amount(&format_amount(raw, decimals), decimals).unwrap(),
                    raw
                );
            }
        }
        for value in [
            "0",
            "-1",
            "1e6",
            "NaN",
            "0.0000001",
            "18446744073709551616",
            "1.2.3",
        ] {
            assert!(parse_amount(value, 6).is_err(), "{value}");
        }
    }

    #[test]
    fn accounts_validate_authority_and_program_and_exports_do_not_overwrite() {
        let mut app = crate::app::App::new("/tmp".into(), crate::config::Config::default(), vec![]);
        crate::demo::populate(&mut app);
        let account = &app.tokens[0];
        assert!(account.associated);
        assert!(!app.tokens[3].associated);
        assert!(
            decode(
                &account.address,
                account.account.clone(),
                &Pubkey::new_from_array([1; 32]),
                0
            )
            .is_err()
        );
        let mut invalid = account.account.clone();
        invalid["owner"] = json!(ASSOCIATED_PROGRAM);
        assert!(
            decode(
                &account.address,
                invalid,
                &account.authority.parse().unwrap(),
                0
            )
            .is_err()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account.json");
        export(account, &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(export(&app.tokens[1], &path).is_err());
        assert_eq!(before, std::fs::read(path).unwrap());
        let copy: TokenAccount = serde_json::from_slice(&before).unwrap();
        assert_eq!(copy.address, account.address);
        assert_eq!(copy.amount, account.amount);
    }
    #[test]
    fn initial_supply_allows_exact_zero_without_allowing_zero_transfers() {
        for value in ["0", "0.0", "00.000000"] {
            assert_eq!(parse_supply(value, 6).unwrap(), 0);
            assert!(parse_amount(value, 6).is_err());
        }
        for value in ["-1", "1e6", ".", "0.0000001", "18446744073709551616"] {
            assert!(parse_supply(value, 6).is_err());
        }
        assert_eq!(parse_supply("1.25", 6).unwrap(), 1_250_000);
    }
}
