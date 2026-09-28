use std::{
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use anyhow::{Result, bail};
use solana_message::VersionedMessage;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSimulateTransactionConfig;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use tokio::sync::mpsc;

use crate::{
    config::RpcProfile,
    model::LogEntry,
    network::{client, safe_error, verify_development_network},
    storage::{self, Store},
    wallet::Wallet,
};

pub struct PreparedTransfer {
    pub transaction: VersionedTransaction,
    pub accounts: Vec<Pubkey>,
    pub imported: bool,
    pub export_path: Option<PathBuf>,
    pub wallet: Wallet,
    pub profile: RpcProfile,
    pub genesis: String,
    pub recipient: String,
    pub lamports: u64,
    pub fee: u64,
    pub logs: Vec<String>,
    pub units: Option<u64>,
    pub simulation_error: Option<String>,
    pub last_valid_block_height: u64,
}

pub enum OperationUpdate {
    Prepared(Box<PreparedTransfer>),
    Submitted(String),
    Finished(String),
    Exported(String),
    Failed(String),
    FundingFailed(String),
}

pub struct OperationEvent {
    pub session: u64,
    pub update: OperationUpdate,
}

pub async fn prepare(
    profile: &RpcProfile,
    wallet: &Wallet,
    recipient: &str,
    lamports: u64,
) -> Result<PreparedTransfer> {
    prepare_format(
        profile,
        wallet,
        recipient,
        lamports,
        crate::transaction::Format::Auto,
    )
    .await
}

pub async fn prepare_format(
    profile: &RpcProfile,
    wallet: &Wallet,
    recipient: &str,
    lamports: u64,
    format: crate::transaction::Format,
) -> Result<PreparedTransfer> {
    if wallet.program {
        bail!("Select a development wallet; program identities are read-only");
    }
    if lamports == 0 {
        bail!("Amount must be greater than zero");
    }
    let recipient = Pubkey::from_str(recipient.trim())
        .map_err(|_| anyhow::anyhow!("Recipient must be a valid Solana address"))?;
    let rpc = client(profile);
    let genesis = verify_development_network(&rpc).await?;
    let (blockhash, last_valid_block_height) = rpc
        .get_latest_blockhash_with_commitment(rpc.commitment())
        .await?;
    let transaction = crate::transaction::transfer(
        format,
        &wallet.address.parse()?,
        &recipient,
        lamports,
        blockhash,
    )?;
    let mut prepared = inspect(profile, wallet, transaction, genesis, false).await?;
    prepared.recipient = recipient.to_string();
    prepared.lamports = lamports;
    prepared.last_valid_block_height = last_valid_block_height;
    Ok(prepared)
}

pub async fn prepare_import(
    profile: &RpcProfile,
    wallet: &Wallet,
    path: &Path,
    export_path: Option<PathBuf>,
) -> Result<PreparedTransfer> {
    if wallet.program {
        bail!("Program identities are read-only");
    }
    let path = path.to_owned();
    let transaction =
        tokio::task::spawn_blocking(move || crate::transaction::read(&path)).await??;
    crate::transaction::signer_index(&transaction, wallet)?;
    if transaction.uses_durable_nonce() {
        bail!("Durable-nonce imports are not supported yet; the transaction was not modified");
    }
    let rpc = client(profile);
    let genesis = verify_development_network(&rpc).await?;
    if !rpc
        .is_blockhash_valid(transaction.message.recent_blockhash(), rpc.commitment())
        .await?
    {
        bail!(
            "Imported blockhash has expired; rebuild externally and collect signatures again. Solte will not modify co-signed messages."
        );
    }
    let mut prepared = inspect(profile, wallet, transaction, genesis, true).await?;
    prepared.export_path = export_path;
    Ok(prepared)
}

async fn inspect(
    profile: &RpcProfile,
    wallet: &Wallet,
    mut transaction: VersionedTransaction,
    genesis: String,
    imported: bool,
) -> Result<PreparedTransfer> {
    crate::transaction::validate(&transaction)?;
    let rpc = client(profile);
    let accounts = crate::transaction::resolve_accounts(&rpc, &transaction).await?;
    let config = RpcSimulateTransactionConfig {
        sig_verify: false,
        commitment: Some(rpc.commitment()),
        ..Default::default()
    };
    let mut simulation = rpc
        .simulate_transaction_with_config(&transaction, config.clone())
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "{} simulation unavailable: {}. The selected format was not changed.",
                crate::transaction::label(&transaction),
                crate::network::safe_error(e, profile)
            )
        })?
        .value;
    if !imported
        && simulation.err.is_none()
        && let VersionedMessage::V1(message) = &mut transaction.message
    {
        let units = simulation.units_consumed.unwrap_or(1_400_000);
        message.config.compute_unit_limit =
            Some((units.saturating_mul(120) / 100).clamp(1_000, 1_400_000) as u32);
        message.config.loaded_accounts_data_size_limit = Some(
            simulation
                .loaded_accounts_data_size
                .map(|bytes| bytes.saturating_add(32767) / 32768 * 32768)
                .unwrap_or(64 * 1024 * 1024)
                .clamp(32768, 64 * 1024 * 1024),
        );
        simulation = rpc
            .simulate_transaction_with_config(&transaction, config)
            .await?
            .value;
    }
    use base64::Engine;
    let response: serde_json::Value = rpc.send(solana_rpc_client_api::request::RpcRequest::GetFeeForMessage,
        serde_json::json!([base64::engine::general_purpose::STANDARD.encode(transaction.message.serialize()), {"commitment":"confirmed"}])).await?;
    let fee = response
        .get("value")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("Fee unavailable; blockhash may have expired"))?;
    Ok(PreparedTransfer {
        transaction,
        accounts,
        imported,
        export_path: None,
        wallet: wallet.clone(),
        profile: profile.clone(),
        genesis,
        recipient: String::new(),
        lamports: 0,
        fee,
        logs: simulation.logs.unwrap_or_default(),
        units: simulation.units_consumed,
        simulation_error: simulation.err.map(|e| format!("{e:?}")),
        last_valid_block_height: u64::MAX,
    })
}

async fn validate_review(prepared: &PreparedTransfer) -> Result<()> {
    if prepared.simulation_error.is_some() {
        bail!("Simulation failed; this transaction cannot be signed");
    }
    let rpc = client(&prepared.profile);
    if verify_development_network(&rpc).await? != prepared.genesis {
        bail!("RPC network changed. Review the transaction again.");
    }
    if prepared.imported {
        if !rpc
            .is_blockhash_valid(
                prepared.transaction.message.recent_blockhash(),
                rpc.commitment(),
            )
            .await?
        {
            bail!("Imported review expired; rebuild and review the transaction again");
        }
    } else if rpc.get_block_height().await? > prepared.last_valid_block_height {
        bail!("Review expired. Prepare the transfer again for a fresh blockhash.");
    }
    if crate::transaction::resolve_accounts(&rpc, &prepared.transaction).await? != prepared.accounts
    {
        bail!("Lookup-table resolution changed. Review the transaction again.");
    }
    Ok(())
}

pub async fn sign_export(
    mut prepared: PreparedTransfer,
    session: u64,
    sender: mpsc::Sender<OperationEvent>,
) {
    let profile = prepared.profile.clone();
    let result = async {
        validate_review(&prepared).await?;
        crate::transaction::sign(&mut prepared.transaction, &prepared.wallet)?;
        let path = prepared
            .export_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No export path"))?;
        let display = path.display().to_string();
        tokio::task::spawn_blocking(move || {
            crate::transaction::export(&prepared.transaction, &path)
        })
        .await??;
        Ok::<_, anyhow::Error>(display)
    }
    .await;
    let update = match result {
        Ok(path) => OperationUpdate::Exported(path),
        Err(error) => OperationUpdate::Failed(crate::network::safe_error(error, &profile)),
    };
    let _ = sender.send(OperationEvent { session, update }).await;
}

pub async fn submit(
    mut prepared: PreparedTransfer,
    session: u64,
    store: Store,
    sender: mpsc::Sender<OperationEvent>,
) {
    let profile = prepared.profile.clone();
    let scope = storage::scope(&profile.http, &prepared.wallet.address);
    let result = async {
        validate_review(&prepared).await?;
        crate::transaction::sign(&mut prepared.transaction, &prepared.wallet)?;
        if prepared.transaction.signatures.iter().any(|s| *s == Signature::default()) {
            bail!("Additional signatures are required. Import again with an export path to save a partially signed transaction.");
        }
        prepared.transaction.verify_and_hash_message().map_err(|_| anyhow::anyhow!("Signature verification failed"))?;
        let rpc = client(&profile);
        let signature = prepared.transaction.signatures[0];
        persist(
            &store,
            &scope,
            "INFO",
            format!("Submitting transfer {signature}"),
        )
        .await;
        let _ = sender
            .send(OperationEvent {
                session,
                update: OperationUpdate::Submitted(signature.to_string()),
            })
            .await;
        // The signature is known before submission, so an uncertain response can be checked safely.
        if let Err(error) = rpc.send_transaction(&prepared.transaction).await {
            if error.get_transaction_error().is_some() {
                bail!(
                    "Preflight rejected {signature}: {}",
                    safe_error(error, &profile)
                );
            }
            persist(
                &store,
                &scope,
                "WARN",
                format!(
                    "Submission response uncertain for {signature}: {}",
                    safe_error(error, &profile)
                ),
            )
            .await;
        }
        confirm(&rpc, &signature).await?;
        Ok::<_, anyhow::Error>(signature)
    }
    .await;
    finish(result, session, &store, &scope, &profile, &sender).await;
}

pub async fn fund(
    profile: RpcProfile,
    wallet: Wallet,
    lamports: u64,
    session: u64,
    store: Store,
    sender: mpsc::Sender<OperationEvent>,
) {
    let scope = storage::scope(&profile.http, &wallet.address);
    let mut devnet = false;
    let mut submitted = false;
    let result = async {
        if lamports == 0 {
            bail!("Amount must be greater than zero");
        }
        let rpc = client(&profile);
        devnet = verify_development_network(&rpc).await? == crate::network::DEVNET_GENESIS;
        let address = Pubkey::from_str(&wallet.address)?;
        let signature = tokio::time::timeout(Duration::from_secs(15), rpc.request_airdrop(&address, lamports)).await
            .map_err(|_| anyhow::anyhow!("Airdrop request timed out. It may still land; check the balance before requesting again."))??;
        submitted = true;
        persist(
            &store,
            &scope,
            "INFO",
            format!("Airdrop submitted: {signature}"),
        )
        .await;
        let _ = sender
            .send(OperationEvent {
                session,
                update: OperationUpdate::Submitted(signature.to_string()),
            })
            .await;
        confirm(&rpc, &signature).await?;
        Ok::<_, anyhow::Error>(signature)
    }
    .await;
    if let Err(error) = &result
        && devnet
        && !submitted
    {
        let message = safe_error(error, &profile);
        persist(&store, &scope, "ERROR", message.clone()).await;
        let _ = sender
            .send(OperationEvent {
                session,
                update: OperationUpdate::FundingFailed(message),
            })
            .await;
        return;
    }
    finish(result, session, &store, &scope, &profile, &sender).await;
}

async fn confirm(rpc: &RpcClient, signature: &Signature) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        if let Ok(response) = rpc.get_signature_statuses_with_history(&[*signature]).await
            && let Some(status) = response.value.first().and_then(Option::as_ref)
        {
            if let Some(error) = &status.err {
                bail!("Transaction {signature} failed: {error:?}");
            }
            if status.satisfies_commitment(rpc.commitment()) {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    bail!(
        "Confirmation is uncertain for {signature}. Check its status before retrying; it may still land."
    )
}

async fn persist(store: &Store, scope: &str, level: &str, message: String) {
    if let Err(error) = store.log(scope, LogEntry::new(level, message)).await {
        tracing::error!(%error, "Cannot persist operation log");
    }
}

async fn finish(
    result: Result<Signature>,
    session: u64,
    store: &Store,
    scope: &str,
    profile: &RpcProfile,
    sender: &mpsc::Sender<OperationEvent>,
) {
    let update = match result {
        Ok(signature) => {
            persist(store, scope, "INFO", format!("Confirmed: {signature}")).await;
            OperationUpdate::Finished(signature.to_string())
        }
        Err(error) => {
            let message = safe_error(error, profile);
            persist(store, scope, "ERROR", message.clone()).await;
            OperationUpdate::Failed(message)
        }
    };
    let _ = sender.send(OperationEvent { session, update }).await;
}
