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
        token_accounts: 4,
    });
    app.last_update = Some(Instant::now());
    let owner = app.wallets[0].address.clone();
    app.tokens = (0..4).map(|i| {
        let mint = Pubkey::new_from_array([30+i; 32]);
        let program = if i == 2 { crate::tokens::TOKEN_2022 } else { crate::tokens::TOKEN_PROGRAM };
        let address = if i == 3 { Pubkey::new_from_array([50; 32]) }
            else { crate::tokens::associated_address(&owner.parse().unwrap(), &mint, &program.parse().unwrap()) };
        let data = json!({"owner":program,"lamports":2039280,"space":165,"data":{"parsed":{"type":"account","info":{
            "owner":owner,"mint":mint.to_string(),"state":if i==1 {"frozen"} else {"initialized"},"isNative":false,
            "tokenAmount":{"amount":if i==3 {"0"} else {"125000000"},"decimals":6},
            "extensions":if i==2 {json!([{"extension":"immutableOwner"}])} else {json!([])}
        }}}});
        let mut account = crate::tokens::decode(&address.to_string(), data, &owner.parse().unwrap(), 415239881).unwrap();
        account.mint_info = Some(json!({"isInitialized":true,"supply":"1000000000","decimals":6,"mintAuthority":owner,"freezeAuthority":null,
            "extensions":[{"extension":"tokenMetadata","state":{"symbol":(["TEST","FROZEN","T22","EMPTY"][i as usize]),"name":"Demo token"}}]}));
        account
    }).collect();
    app.token_genesis = Some(crate::network::DEVNET_GENESIS.into());
    app.token_updated = Some(Instant::now());
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
            "History retained locally · 4 token accounts watched",
        ),
    ] {
        app.push_log(LogEntry::new(level, message));
    }
    app.project_mints = app
        .tokens
        .iter()
        .map(|account| crate::mints::ProjectMint {
            record: crate::mints::MintRecord {
                address: account.mint.clone(),
                program: account.program.clone(),
                decimals: account.decimals,
                creator: account.authority.clone(),
                signature: String::new(),
            },
            info: account.mint_info.clone(),
            error: None,
            label: None,
        })
        .collect();
    app.project_mints
        .sort_by(|a, b| a.record.address.cmp(&b.record.address));
    app.project_mints
        .dedup_by(|a, b| a.record.address == b.record.address);
}
