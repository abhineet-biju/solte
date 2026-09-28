use std::time::Instant;

use serde_json::json;
use solana_pubkey::Pubkey;

use crate::{
    app::App,
    model::{LogEntry, NetworkState, TransactionRecord, now},
    wallet::Wallet,
};

pub fn populate(app: &mut App) {
    app.demo = true;
    app.connected = true;
    app.subscribed = true;
    app.status = "Demo session · fixture data · network and signing disabled".into();
    app.wallets = ["dev.wallet", "deployer", "treasury", "test-user"]
        .iter()
        .enumerate()
        .map(|(i, name)| Wallet {
            name: (*name).into(),
            address: Pubkey::new_from_array([i as u8 + 7; 32]).to_string(),
            path: format!("/demo/{name}.json").into(),
            program: false,
        })
        .collect();
    app.selected_wallet = 0;
    app.wallet_cursor = 0;
    app.balance = Some(12_482_100_000);
    app.network = Some(NetworkState {
        genesis: crate::network::DEVNET_GENESIS.into(),
        cluster: "Devnet".into(),
        slot: 415_239_881,
        block_height: 403_172_042,
        epoch: 962,
        latency_ms: 42,
        healthy: true,
        version: "3.1.12".into(),
        token_accounts: 3,
    });
    app.last_update = Some(Instant::now());
    let owner = app.wallets[0].address.clone();
    app.records = (0..9).map(|i| {
        let failed = i == 3 || i == 7;
        let signature = solana_signature::Signature::from([i as u8 + 3; 64]).to_string();
        let error = failed.then(|| "InstructionError: insufficient funds (0x1)".into());
        TransactionRecord { signature, slot: 415_239_880 - i * 320, timestamp: Some(now() as i64 - 30 - i as i64 * 180), error, details: Some(json!({"version": if i % 3 == 0 { json!("legacy") } else { json!((i - 1) % 3) }, "transaction":{"message":{"accountKeys":[{"pubkey":owner}],"instructions":[{"program":"system","parsed":{"type":if i % 2 == 0 {"transfer"}else{"createAccount"},"info":{"lamports":500000000}}}]}},"meta":{"fee":5000,"computeUnitsConsumed":450,"err":if failed{json!({"InstructionError":[0,"InsufficientFunds"]})}else{json!(null)},"preBalances":[12000000000u64],"postBalances":[if failed {11999995000u64} else if i%2==0 {11499995000u64}else{14000000000u64}],"logMessages":["Program 11111111111111111111111111111111 invoke [1]",if failed {"Transfer: insufficient lamports"}else{"Program log: transfer"},if failed {"Program failed: custom program error: 0x1"}else{"Program 11111111111111111111111111111111 success"}],"innerInstructions":[],"preTokenBalances":[],"postTokenBalances":[]}})) }
    }).collect();
    app.push_log(LogEntry::new(
        "INFO",
        format!("Confirmed {}", app.records[0].signature),
    ));
    for (level, message) in [
        ("INFO", "Project loaded · 4 development identities"),
        ("INFO", "Devnet RPC connected · wallet logs subscribed"),
        ("INFO", "Airdrop confirmed · 1.0 SOL received"),
        (
            "ERROR",
            "Transfer failed: insufficient funds · open transaction for program logs",
        ),
        (
            "INFO",
            "History retained locally · 3 token accounts watched",
        ),
    ] {
        app.push_log(LogEntry::new(level, message));
    }
}
