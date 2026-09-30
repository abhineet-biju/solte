use std::collections::HashMap;

use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mint,
    Account,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Mint => "mint",
            Self::Account => "account",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub mint: String,
    pub account: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Change {
    pub kind: Kind,
    pub address: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Labels {
    pub mints: HashMap<String, String>,
    pub accounts: HashMap<String, String>,
}

impl Labels {
    pub fn update(&mut self, changes: &[Change]) {
        for change in changes {
            let labels = match change.kind {
                Kind::Mint => &mut self.mints,
                Kind::Account => &mut self.accounts,
            };
            if let Some(label) = &change.label {
                labels.insert(change.address.clone(), label.clone());
            } else {
                labels.remove(&change.address);
            }
        }
    }

    pub fn apply(
        &self,
        accounts: &mut [crate::tokens::TokenAccount],
        mints: &mut [crate::mints::ProjectMint],
    ) {
        for account in accounts {
            account.mint_label = self.mints.get(&account.mint).cloned();
            account.account_label = self.accounts.get(&account.address).cloned();
        }
        for mint in mints {
            mint.label = self.mints.get(&mint.record.address).cloned();
        }
    }
}

pub fn normalize(value: &str) -> Result<Option<String>> {
    ensure!(
        !value.chars().any(char::is_control),
        "Names must be a single line without control characters"
    );
    let value = value.trim();
    ensure!(
        value.chars().count() <= 40,
        "Use a name of at most 40 characters"
    );
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

pub fn changes(target: &Target, values: &[String]) -> Result<Vec<Change>> {
    ensure!(
        values.len() == if target.account.is_some() { 2 } else { 1 },
        "The naming target changed; reopen its inspector"
    );
    let mut changes = vec![Change {
        kind: Kind::Mint,
        address: target.mint.clone(),
        label: normalize(&values[0])?,
    }];
    if let Some(address) = &target.account {
        changes.push(Change {
            kind: Kind::Account,
            address: address.clone(),
            label: normalize(&values[1])?,
        });
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_preserve_unicode_trim_spaces_and_allow_clearing() {
        assert_eq!(
            normalize("  開発用 USD  ").unwrap().as_deref(),
            Some("開発用 USD")
        );
        assert_eq!(normalize("   ").unwrap(), None);
        assert!(normalize(&"é".repeat(40)).is_ok());
        assert!(normalize(&"é".repeat(41)).is_err());
        for value in ["two\nlines", "escape\u{1b}[0m", "tab\tname"] {
            assert!(normalize(value).is_err());
        }
    }
}
