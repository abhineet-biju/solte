use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    str::FromStr,
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use solana_hash::Hash;
use solana_message::{Message, VersionedMessage, v0, v1};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

use crate::wallet::Wallet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Auto,
    Legacy,
    V0,
    V1,
}

impl FromStr for Format {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "legacy" => Ok(Self::Legacy),
            "v0" => Ok(Self::V0),
            "v1" => Ok(Self::V1),
            _ => bail!("Choose Auto, Legacy, v0, or v1"),
        }
    }
}

pub fn label(transaction: &VersionedTransaction) -> &'static str {
    match transaction.message {
        VersionedMessage::Legacy(_) => "Legacy",
        VersionedMessage::V0(_) => "v0",
        VersionedMessage::V1(_) => "v1",
    }
}

pub fn transfer(
    format: Format,
    sender: &Pubkey,
    recipient: &Pubkey,
    amount: u64,
    blockhash: Hash,
) -> Result<VersionedTransaction> {
    let instructions = [solana_system_interface::instruction::transfer(
        sender, recipient, amount,
    )];
    build(format, sender, &instructions, blockhash)
}

pub fn build(
    format: Format,
    sender: &Pubkey,
    instructions: &[solana_instruction::Instruction],
    blockhash: Hash,
) -> Result<VersionedTransaction> {
    let message = match format {
        Format::Auto | Format::Legacy => VersionedMessage::Legacy(Message::new_with_blockhash(
            instructions,
            Some(sender),
            &blockhash,
        )),
        Format::V0 => VersionedMessage::V0(v0::Message::try_compile(
            sender,
            instructions,
            &[],
            blockhash,
        )?),
        Format::V1 => VersionedMessage::V1(v1::Message::try_compile_with_config(
            sender,
            instructions,
            blockhash,
            v1::TransactionConfig::default()
                .with_compute_unit_limit(1_400_000)
                .with_loaded_accounts_data_size_limit(64 * 1024 * 1024),
        )?),
    };
    Ok(VersionedTransaction {
        signatures: vec![Signature::default(); message.header().num_required_signatures as usize],
        message,
    })
}

pub fn validate(transaction: &VersionedTransaction) -> Result<()> {
    transaction
        .sanitize()
        .map_err(|e| anyhow::anyhow!("Invalid transaction structure: {e:?}"))?;
    let size = wincode::serialize(transaction)?.len();
    let max = if matches!(transaction.message, VersionedMessage::V1(_)) {
        4096
    } else {
        1232
    };
    ensure!(
        size <= max,
        "Transaction exceeds the {}-byte limit for {}",
        max,
        label(transaction)
    );
    for (signature, valid) in transaction
        .signatures
        .iter()
        .zip(transaction.verify_with_results())
    {
        ensure!(
            *signature == Signature::default() || valid,
            "An existing co-signature is invalid; import the original message again"
        );
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<VersionedTransaction> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .context("Cannot open transaction file")?
        .take(8193)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 8192, "Transaction file exceeds 8 KiB");
    let binary = if let Ok(text) = std::str::from_utf8(&bytes) {
        STANDARD.decode(text.trim()).unwrap_or(bytes)
    } else {
        bytes
    };
    let transaction: VersionedTransaction = wincode::deserialize(&binary)
        .context("Invalid or unsupported transaction encoding; use raw wire bytes or base64")?;
    ensure!(
        wincode::serialize(&transaction)? == binary,
        "Transaction encoding is noncanonical or contains trailing data"
    );
    validate(&transaction)?;
    Ok(transaction)
}

pub fn signer_index(transaction: &VersionedTransaction, wallet: &Wallet) -> Result<usize> {
    let address: Pubkey = wallet.address.parse()?;
    transaction
        .message
        .static_account_keys()
        .iter()
        .take(transaction.signatures.len())
        .position(|key| *key == address)
        .context("Selected wallet is not a required signer for this transaction")
}

pub fn sign(transaction: &mut VersionedTransaction, wallet: &Wallet) -> Result<()> {
    validate(transaction)?;
    let index = signer_index(transaction, wallet)?;
    let signer = wallet.signer()?;
    transaction.signatures[index] = signer.try_sign_message(&transaction.message.serialize())?;
    validate(transaction)
}

pub fn export(transaction: &VersionedTransaction, path: &Path) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("Cannot export transaction; destination must not already exist")?;
    file.write_all(STANDARD.encode(wincode::serialize(transaction)?).as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub async fn resolve_accounts(
    rpc: &RpcClient,
    transaction: &VersionedTransaction,
) -> Result<Vec<Pubkey>> {
    let mut keys = transaction.message.static_account_keys().to_vec();
    let mut writable = Vec::new();
    let mut readonly = Vec::new();
    if let Some(lookups) = transaction.message.address_table_lookups() {
        for lookup in lookups {
            let account = rpc
                .get_account(&lookup.account_key)
                .await
                .context("Cannot resolve address lookup table")?;
            ensure!(
                account.owner == solana_address_lookup_table_interface::program::id(),
                "Lookup table has an unexpected owner"
            );
            let table =
                solana_address_lookup_table_interface::state::AddressLookupTable::deserialize(
                    &account.data,
                )
                .map_err(|_| anyhow::anyhow!("Invalid address lookup table data"))?;
            for (indices, output) in [
                (&lookup.writable_indexes, &mut writable),
                (&lookup.readonly_indexes, &mut readonly),
            ] {
                for index in indices {
                    output.push(
                        *table
                            .addresses
                            .get(*index as usize)
                            .context("Lookup table index is unavailable")?,
                    );
                }
            }
        }
    }
    keys.extend(writable);
    keys.extend(readonly);
    Ok(keys)
}

pub fn review_lines(transaction: &VersionedTransaction, accounts: &[Pubkey]) -> Vec<String> {
    let mut lines = vec![
        format!("Format     {}", label(transaction)),
        format!("Blockhash  {}", transaction.message.recent_blockhash()),
        format!(
            "Signatures {}/{} present",
            transaction
                .signatures
                .iter()
                .filter(|s| **s != Signature::default())
                .count(),
            transaction.signatures.len()
        ),
    ];
    if let VersionedMessage::V1(message) = &transaction.message {
        lines.push(format!(
            "Compute limit   {} CU",
            message.config.compute_unit_limit.unwrap_or(0)
        ));
        lines.push(format!(
            "Account data    {} bytes",
            message.config.loaded_accounts_data_size_limit.unwrap_or(0)
        ));
        lines.push(format!(
            "Priority fee    {} lamports",
            message.config.priority_fee.unwrap_or(0)
        ));
    }
    lines.push("ACCOUNTS · resolved in message order".into());
    lines.extend(accounts.iter().enumerate().map(|(index, key)| {
        format!(
            "{index}: {key}{}",
            if transaction.message.is_signer(index) {
                " [signer]"
            } else {
                ""
            }
        )
    }));
    lines.push("INSTRUCTIONS · raw data is base64".into());
    for (index, instruction) in transaction.message.instructions().iter().enumerate() {
        lines.push(format!(
            "{index}: program {}",
            accounts
                .get(instruction.program_id_index as usize)
                .map(ToString::to_string)
                .unwrap_or_else(|| "Unresolved".into())
        ));
        lines.push(format!("Accounts: {:?}", instruction.accounts));
        lines.push(format!("Data: {}", STANDARD.encode(&instruction.data)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_formats_roundtrip_and_preserve_existing_signatures() {
        let root = tempfile::tempdir().unwrap();
        let first = crate::wallet::create(root.path(), "first").unwrap();
        let second = crate::wallet::create(root.path(), "second").unwrap();
        let payer: Pubkey = first.address.parse().unwrap();
        let other: Pubkey = second.address.parse().unwrap();
        let recipient = Pubkey::new_from_array([9; 32]);
        let instructions = [
            solana_system_interface::instruction::transfer(&payer, &recipient, 1),
            solana_system_interface::instruction::transfer(&other, &recipient, 2),
        ];
        for format in [Format::Legacy, Format::V0, Format::V1] {
            let blockhash = Hash::new_from_array([4; 32]);
            let message = match format {
                Format::Legacy => VersionedMessage::Legacy(Message::new_with_blockhash(
                    &instructions,
                    Some(&payer),
                    &blockhash,
                )),
                Format::V0 => VersionedMessage::V0(
                    v0::Message::try_compile(&payer, &instructions, &[], blockhash).unwrap(),
                ),
                _ => VersionedMessage::V1(
                    v1::Message::try_compile_with_config(
                        &payer,
                        &instructions,
                        blockhash,
                        v1::TransactionConfig::default()
                            .with_compute_unit_limit(1000)
                            .with_loaded_accounts_data_size_limit(32768),
                    )
                    .unwrap(),
                ),
            };
            let mut tx = VersionedTransaction {
                signatures: vec![Signature::default(); 2],
                message,
            };
            let original_message = tx.message.serialize();
            sign(&mut tx, &second).unwrap();
            let cosignature = tx.signatures[signer_index(&tx, &second).unwrap()];
            let path = root.path().join(format!("{format:?}.base64"));
            export(&tx, &path).unwrap();
            assert!(export(&tx, &path).is_err());
            let mut imported = read(&path).unwrap();
            assert_eq!(imported, tx);
            sign(&mut imported, &first).unwrap();
            assert_eq!(imported.message.serialize(), original_message);
            assert_eq!(
                imported.signatures[signer_index(&imported, &second).unwrap()],
                cosignature
            );
            imported.verify_and_hash_message().unwrap();
            let raw = root.path().join("raw.tx");
            std::fs::write(&raw, wincode::serialize(&imported).unwrap()).unwrap();
            assert_eq!(read(&raw).unwrap(), imported);
            imported
                .message
                .set_recent_blockhash(Hash::new_from_array([5; 32]));
            assert!(validate(&imported).is_err());
        }
    }

    #[test]
    fn rejects_trailing_data_and_unrelated_signers() {
        let root = tempfile::tempdir().unwrap();
        let wallet = crate::wallet::create(root.path(), "wallet").unwrap();
        let mut tx = transfer(
            Format::Auto,
            &Pubkey::new_from_array([1; 32]),
            &Pubkey::new_from_array([2; 32]),
            1,
            Hash::default(),
        )
        .unwrap();
        assert_eq!(label(&tx), "Legacy");
        assert!(sign(&mut tx, &wallet).is_err());
        let mut bytes = wincode::serialize(&tx).unwrap();
        bytes.push(1);
        let path = root.path().join("extra.tx");
        std::fs::write(&path, bytes).unwrap();
        assert!(read(&path).is_err());
        std::fs::write(&path, vec![0; 8193]).unwrap();
        assert!(read(&path).is_err());
    }
}
