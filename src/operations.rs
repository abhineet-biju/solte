use std::{str::FromStr, time::Duration};

use anyhow::{Result, bail};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSimulateTransactionConfig;
use solana_signature::Signature;
use solana_system_interface::instruction;
use solana_transaction::Transaction;
use tokio::sync::mpsc;

use crate::{
    config::RpcProfile,
    model::LogEntry,
    network::{client, safe_error, verify_development_network},
    storage::{self, Store},
    wallet::Wallet,
};

pub struct PreparedTransfer {
    pub transaction: Transaction,
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
    Failed(String),
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
    if wallet.program {
        bail!("Select a development wallet; program identities are read-only");
    }
    if lamports == 0 {
        bail!("Amount must be greater than zero");
    }
    let recipient = Pubkey::from_str(recipient.trim())
        .map_err(|_| anyhow::anyhow!("Recipient must be a valid Solana address"))?;
    let sender = Pubkey::from_str(&wallet.address)?;
    let rpc = client(profile);
    let genesis = verify_development_network(&rpc).await?;
    let (blockhash, last_valid_block_height) = rpc
        .get_latest_blockhash_with_commitment(rpc.commitment())
        .await?;
    let mut transaction = Transaction::new_with_payer(
        &[instruction::transfer(&sender, &recipient, lamports)],
        Some(&sender),
    );
    transaction.message.recent_blockhash = blockhash;
    let simulation = rpc
        .simulate_transaction_with_config(
            &transaction,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                commitment: Some(rpc.commitment()),
                ..Default::default()
            },
        )
        .await?
        .value;
    let fee = rpc.get_fee_for_message(&transaction.message).await?;
    Ok(PreparedTransfer {
        transaction,
        wallet: wallet.clone(),
        profile: profile.clone(),
        genesis,
        recipient: recipient.to_string(),
        lamports,
        fee,
        logs: simulation.logs.unwrap_or_default(),
        units: simulation.units_consumed,
        simulation_error: simulation.err.map(|e| format!("{e:?}")),
        last_valid_block_height,
    })
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
        if prepared.simulation_error.is_some() {
            bail!("Simulation failed; this transfer cannot be submitted");
        }
        let rpc = client(&profile);
        if verify_development_network(&rpc).await? != prepared.genesis {
            bail!("RPC network changed. Review the transfer again.");
        }
        if rpc.get_block_height().await? > prepared.last_valid_block_height {
            bail!("Review expired. Prepare the transfer again for a fresh blockhash.");
        }
        let signer = prepared.wallet.signer()?;
        prepared
            .transaction
            .try_sign(&[&signer], prepared.transaction.message.recent_blockhash)?;
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
    let result = async {
        if lamports == 0 {
            bail!("Amount must be greater than zero");
        }
        let rpc = client(&profile);
        verify_development_network(&rpc).await?;
        let address = Pubkey::from_str(&wallet.address)?;
        let signature = rpc.request_airdrop(&address, lamports).await?;
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
