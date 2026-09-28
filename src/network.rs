use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use futures_util::{StreamExt, stream};
use serde_json::json;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_pubsub_client::nonblocking::pubsub_client::PubsubClient;
use solana_rpc_client::{
    nonblocking::rpc_client::RpcClient, rpc_client::GetConfirmedSignaturesForAddress2Config,
};
use solana_rpc_client_api::{
    config::{RpcTransactionConfig, RpcTransactionLogsConfig, RpcTransactionLogsFilter},
    request::RpcRequest,
};
use solana_signature::Signature;
use solana_transaction_status_client_types::UiTransactionEncoding;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    config::RpcProfile,
    model::{LogEntry, NetworkState, TransactionRecord},
    storage::{self, Store},
};

pub const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
pub const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";

#[derive(Debug)]
pub enum Update {
    Records(Vec<TransactionRecord>),
    Network(NetworkState, Option<u64>),
    Offline(String),
    Subscription(bool),
    Log(LogEntry),
}

#[derive(Debug)]
pub struct Event {
    pub session: u64,
    pub update: Update,
}

pub enum Command {
    Refresh,
    Older,
    Detail(String),
}

pub struct Monitor {
    pub commands: mpsc::Sender<Command>,
    task: JoinHandle<()>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct AbortTask(JoinHandle<()>);
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn client(profile: &RpcProfile) -> RpcClient {
    RpcClient::new_with_timeout_and_commitment(
        profile.http.clone(),
        Duration::from_secs(10),
        CommitmentConfig::confirmed(),
    )
}

pub fn cluster_name(genesis: &str, profile: &RpcProfile) -> String {
    match genesis {
        MAINNET_GENESIS => "Mainnet · read-only".into(),
        DEVNET_GENESIS => "Devnet".into(),
        "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY" => "Testnet".into(),
        _ if profile.name == "Localnet" => "Localnet".into(),
        _ => "Custom development".into(),
    }
}

pub async fn verify_development_network(rpc: &RpcClient) -> Result<String> {
    let genesis = rpc.get_genesis_hash().await?.to_string();
    if genesis == MAINNET_GENESIS {
        bail!("Mainnet signing is disabled in Solte");
    }
    Ok(genesis)
}

pub fn safe_error(error: impl std::fmt::Display, profile: &RpcProfile) -> String {
    let message = error
        .to_string()
        .replace(&profile.http, "[RPC]")
        .replace(&profile.websocket, "[subscriptions]");
    message
        .split_whitespace()
        .map(|part| {
            if part.contains("http://")
                || part.contains("https://")
                || part.contains("wss://")
                || part.contains("ws://")
            {
                "[endpoint]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn start(
    session: u64,
    address: Option<String>,
    profile: RpcProfile,
    store: Store,
    sender: mpsc::Sender<Event>,
    offline: bool,
) -> Monitor {
    let (commands, receiver) = mpsc::channel(16);
    let task = tokio::spawn(run(
        session, address, profile, store, sender, receiver, offline,
    ));
    Monitor { commands, task }
}

async fn emit(sender: &mpsc::Sender<Event>, session: u64, update: Update) {
    let _ = sender.send(Event { session, update }).await;
}

async fn run(
    session: u64,
    address: Option<String>,
    profile: RpcProfile,
    store: Store,
    sender: mpsc::Sender<Event>,
    mut commands: mpsc::Receiver<Command>,
    offline: bool,
) {
    let scope = storage::scope(&profile.http, address.as_deref().unwrap_or("network"));
    let mut records = HashMap::<String, TransactionRecord>::new();
    match store.load(&scope).await {
        Ok(cached) => {
            for record in cached {
                records.insert(record.signature.clone(), record);
            }
            emit(&sender, session, Update::Records(sorted(&records))).await;
        }
        Err(error) => {
            emit(
                &sender,
                session,
                Update::Log(LogEntry::new(
                    "ERROR",
                    format!("Cannot load history: {error}"),
                )),
            )
            .await
        }
    }
    if let Ok(logs) = store.logs(&scope).await {
        for entry in logs {
            emit(&sender, session, Update::Log(entry)).await;
        }
    }
    if offline {
        emit(
            &sender,
            session,
            Update::Offline("Offline mode · showing captured history".into()),
        )
        .await;
        return;
    }
    let rpc = Arc::new(client(&profile));
    let (live_sender, mut live_receiver) = mpsc::channel(128);
    let _subscription = address.as_ref().map(|address| {
        AbortTask(tokio::spawn(watch(
            session,
            address.clone(),
            profile.clone(),
            sender.clone(),
            live_sender,
        )))
    });
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut chain = String::new();
    let mut cursors = HashMap::new();
    let mut last_error = String::new();
    loop {
        let command = tokio::select! {
            _ = interval.tick() => Command::Refresh,
            command = commands.recv() => match command { Some(c) => c, None => return },
            Some(record) = live_receiver.recv() => {
                if !chain.is_empty() {
                    let record: TransactionRecord = record;
                    let signature = record.signature.clone();
                    records.entry(signature.clone()).or_insert(record);
                    let entry = records[&signature].clone();
                    if let Err(error) = store.save(&scope, &chain, vec![entry]).await {
                        emit(&sender, session, Update::Log(LogEntry::new("ERROR", format!("History write failed: {error}")))).await;
                    }
                    emit(&sender, session, Update::Records(sorted(&records))).await;
                }
                continue;
            },
        };
        if let Command::Detail(signature) = command {
            match fetch_detail(&rpc, &signature).await {
                Ok(Some(record)) => {
                    records.insert(signature, record.clone());
                    if let Err(error) = store.save(&scope, &chain, vec![record]).await {
                        emit(
                            &sender,
                            session,
                            Update::Log(LogEntry::new(
                                "ERROR",
                                format!("History write failed: {error}"),
                            )),
                        )
                        .await;
                    }
                    emit(&sender, session, Update::Records(sorted(&records))).await;
                }
                Ok(None) => {
                    emit(
                        &sender,
                        session,
                        Update::Log(LogEntry::new(
                            "WARN",
                            "Transaction details are unavailable from this RPC",
                        )),
                    )
                    .await
                }
                Err(error) => {
                    emit(
                        &sender,
                        session,
                        Update::Log(LogEntry::new("WARN", safe_error(error, &profile))),
                    )
                    .await
                }
            }
            continue;
        }
        let result = tokio::time::timeout(
            Duration::from_secs(45),
            refresh(
                &rpc,
                &profile,
                address.as_deref(),
                &mut records,
                &mut cursors,
                &store,
                &scope,
                &mut chain,
                matches!(command, Command::Older),
            ),
        )
        .await;
        match result {
            Ok(Ok((network, balance, warnings))) => {
                last_error.clear();
                emit(&sender, session, Update::Network(network, balance)).await;
                emit(&sender, session, Update::Records(sorted(&records))).await;
                for warning in warnings {
                    emit(
                        &sender,
                        session,
                        Update::Log(LogEntry::new("WARN", warning)),
                    )
                    .await;
                }
            }
            other => {
                let message = match other {
                    Ok(Err(error)) => safe_error(error, &profile),
                    _ => "RPC refresh timed out; retaining captured records".into(),
                };
                emit(&sender, session, Update::Offline(message.clone())).await;
                if message != last_error {
                    emit(
                        &sender,
                        session,
                        Update::Log(LogEntry::new("WARN", &message)),
                    )
                    .await;
                    last_error = message;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn refresh(
    rpc: &RpcClient,
    profile: &RpcProfile,
    address: Option<&str>,
    records: &mut HashMap<String, TransactionRecord>,
    cursors: &mut HashMap<String, Signature>,
    store: &Store,
    scope: &str,
    chain: &mut String,
    older: bool,
) -> Result<(NetworkState, Option<u64>, Vec<String>)> {
    let started = Instant::now();
    let (genesis, epoch, health, version) = tokio::join!(
        rpc.get_genesis_hash(),
        rpc.get_epoch_info(),
        rpc.get_health(),
        rpc.get_version()
    );
    let genesis = genesis?.to_string();
    let epoch = epoch?;
    let latency_ms = started.elapsed().as_millis();
    if *chain != genesis {
        store.save(scope, &genesis, vec![]).await?;
        records.clear();
        cursors.clear();
        for record in store.load(scope).await? {
            records.insert(record.signature.clone(), record);
        }
        *chain = genesis.clone();
    }
    let mut network = NetworkState {
        cluster: cluster_name(&genesis, profile),
        genesis,
        slot: epoch.absolute_slot,
        block_height: epoch.block_height,
        epoch: epoch.epoch,
        healthy: health.is_ok(),
        latency_ms,
        version: version
            .map(|v| v.solana_core)
            .unwrap_or_else(|_| "Unavailable".into()),
        token_accounts: 0,
    };
    let mut warnings = Vec::new();
    let Some(address) = address else {
        return Ok((network, None, warnings));
    };
    let owner = Pubkey::from_str(address)?;
    let balance = rpc.get_balance(&owner).await?;
    let mut watched = vec![owner];
    for program in [
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
    ] {
        let response: Result<serde_json::Value, _> = rpc.send(RpcRequest::GetTokenAccountsByOwner, json!([address, {"programId":program}, {"encoding":"jsonParsed","commitment":"confirmed"}])).await;
        match response {
            Ok(value) => {
                if let Some(accounts) = value["value"].as_array() {
                    for account in accounts {
                        if let Some(address) = account["pubkey"]
                            .as_str()
                            .and_then(|s| Pubkey::from_str(s).ok())
                        {
                            watched.push(address);
                        }
                    }
                }
            }
            Err(_) => warnings.push(
                "Token-account discovery unavailable; token history coverage may be incomplete"
                    .into(),
            ),
        }
    }
    network.token_accounts = watched.len().saturating_sub(1);
    if watched.len() > 33 {
        warnings.push(
            "Monitoring the first 32 token accounts; additional accounts are outside this view"
                .into(),
        );
        watched.truncate(33);
    }
    let mut seen = HashSet::new();
    watched.retain(|address| seen.insert(*address));
    for account in watched {
        let key = account.to_string();
        let advance_cursor = older || !cursors.contains_key(&key);
        let mut before = if older {
            cursors.get(&key).copied()
        } else {
            None
        };
        let mut reached_known = false;
        for page in 0..3 {
            let entries = rpc
                .get_signatures_for_address_with_config(
                    &account,
                    GetConfirmedSignaturesForAddress2Config {
                        before,
                        until: None,
                        limit: Some(50),
                        commitment: Some(CommitmentConfig::confirmed()),
                    },
                )
                .await?;
            let count = entries.len();
            if let Some(last) = entries
                .last()
                .and_then(|entry| Signature::from_str(&entry.signature).ok())
            {
                before = Some(last);
                if advance_cursor {
                    cursors.insert(key.clone(), last);
                }
            }
            for entry in entries {
                if records.contains_key(&entry.signature) {
                    reached_known = true;
                }
                let record = records
                    .entry(entry.signature.clone())
                    .or_insert(TransactionRecord {
                        signature: entry.signature,
                        slot: entry.slot,
                        timestamp: entry.block_time,
                        error: entry.err.as_ref().map(|e| format!("{e:?}")),
                        details: None,
                    });
                record.timestamp = entry.block_time;
                record.error = entry.err.map(|e| format!("{e:?}"));
            }
            if older || reached_known || count < 50 {
                break;
            }
            if page == 2 {
                if let Some(before) = before {
                    cursors.insert(key.clone(), before);
                }
                warnings.push("History backfill reached its page limit. Use Older to fetch earlier records; RPC retention may leave gaps.".into());
            }
        }
    }
    let missing: Vec<_> = sorted(records)
        .into_iter()
        .filter(|r| {
            r.details
                .as_ref()
                .is_none_or(|d| d.get("transaction").is_none())
        })
        .take(12)
        .map(|r| r.signature)
        .collect();
    let mut results = stream::iter(missing)
        .map(|signature| async move { fetch_detail(rpc, &signature).await })
        .buffer_unordered(3);
    while let Some(result) = results.next().await {
        if let Ok(Some(record)) = result {
            records.insert(record.signature.clone(), record);
        }
    }
    let saved = sorted(records);
    store.save(scope, chain, saved.clone()).await?;
    if records.len() > 1000 {
        records.clear();
        for record in saved.into_iter().take(1000) {
            records.insert(record.signature.clone(), record);
        }
    }
    Ok((network, Some(balance), warnings))
}

pub async fn fetch_detail(rpc: &RpcClient, signature: &str) -> Result<Option<TransactionRecord>> {
    let signature = Signature::from_str(signature)?;
    let result = rpc
        .get_transaction_with_config(
            &signature,
            RpcTransactionConfig {
                encoding: Some(UiTransactionEncoding::JsonParsed),
                commitment: Some(CommitmentConfig::confirmed()),
                max_supported_transaction_version: Some(0),
            },
        )
        .await?;
    let value = serde_json::to_value(&result)?;
    let error = value
        .pointer("/meta/err")
        .filter(|v| !v.is_null())
        .map(ToString::to_string);
    Ok(Some(TransactionRecord {
        signature: signature.to_string(),
        slot: result.slot,
        timestamp: result.block_time,
        error,
        details: Some(value),
    }))
}

fn sorted(records: &HashMap<String, TransactionRecord>) -> Vec<TransactionRecord> {
    let mut records: Vec<_> = records.values().cloned().collect();
    records.sort_by(|a, b| {
        b.slot
            .cmp(&a.slot)
            .then_with(|| a.signature.cmp(&b.signature))
    });
    records
}

async fn watch(
    session: u64,
    address: String,
    profile: RpcProfile,
    sender: mpsc::Sender<Event>,
    live: mpsc::Sender<TransactionRecord>,
) {
    let mut delay = 2;
    loop {
        if let Ok(Ok(client)) = tokio::time::timeout(
            Duration::from_secs(10),
            PubsubClient::new(&profile.websocket),
        )
        .await
            && let Ok(Ok((mut notifications, _unsubscribe))) = tokio::time::timeout(
                Duration::from_secs(10),
                client.logs_subscribe(
                    RpcTransactionLogsFilter::Mentions(vec![address.clone()]),
                    RpcTransactionLogsConfig {
                        commitment: Some(CommitmentConfig::confirmed()),
                    },
                ),
            )
            .await
        {
            emit(&sender, session, Update::Subscription(true)).await;
            delay = 2;
            while let Some(notification) = notifications.next().await {
                let value = notification.value;
                let error = value.err.as_ref().map(|e| format!("{e:?}"));
                let record = TransactionRecord {
                    signature: value.signature,
                    slot: notification.context.slot,
                    timestamp: None,
                    error,
                    details: Some(json!({"meta":{"logMessages":value.logs}})),
                };
                if live.send(record).await.is_err() {
                    return;
                }
            }
        }
        emit(&sender, session, Update::Subscription(false)).await;
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(30);
    }
}

pub fn explorer_url(
    profile: &RpcProfile,
    genesis: Option<&str>,
    kind: &str,
    value: &str,
) -> Result<String> {
    let mut url = url::Url::parse("https://explorer.solana.com")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid explorer base"))?
        .extend([kind, value]);
    if genesis == Some(DEVNET_GENESIS) || (genesis.is_none() && profile == &RpcProfile::devnet()) {
        url.query_pairs_mut().append_pair("cluster", "devnet");
    } else if genesis == Some("4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY") {
        url.query_pairs_mut().append_pair("cluster", "testnet");
    } else if genesis != Some(MAINNET_GENESIS) {
        let endpoint = url::Url::parse(&profile.http)?;
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || !matches!(endpoint.path(), "" | "/")
        {
            bail!(
                "This custom RPC may contain credentials. Copy the signature and open your explorer manually."
            );
        }
        url.query_pairs_mut()
            .append_pair("cluster", "custom")
            .append_pair("customUrl", &profile.http);
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_links_keep_cluster_and_do_not_export_credentials() {
        let link = explorer_url(&RpcProfile::devnet(), None, "tx", "signature").unwrap();
        assert_eq!(
            link,
            "https://explorer.solana.com/tx/signature?cluster=devnet"
        );
        let custom =
            RpcProfile::custom("work", "https://rpc.example/key", "wss://rpc.example/key").unwrap();
        assert!(explorer_url(&custom, None, "tx", "signature").is_err());
    }

    #[test]
    fn rpc_errors_redact_endpoint_credentials() {
        let profile = RpcProfile::custom(
            "work",
            "https://rpc.example/?key=secret",
            "wss://rpc.example/?key=secret",
        )
        .unwrap();
        assert!(
            !safe_error(format!("request failed: {}", profile.http), &profile).contains("secret")
        );
    }
}
