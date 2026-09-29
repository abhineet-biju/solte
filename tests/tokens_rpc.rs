use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use solte::{
    app::App,
    config::{Config, RpcProfile},
    demo, network, tokens,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

struct Server {
    profile: RpcProfile,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(partial: bool) -> Server {
    server_on_network(partial, network::DEVNET_GENESIS).await
}

async fn server_on_network(partial: bool, genesis: &'static str) -> Server {
    let mut app = App::new("/fixture".into(), Config::default(), vec![]);
    demo::populate(&mut app);
    let accounts = Arc::new(app.tokens);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(vec![]));
    let saved = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let saved = saved.clone();
            let accounts = accounts.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buffer = [0; 4096];
                let (header, length) = loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..count]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let length = String::from_utf8_lossy(&data[..end])
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while data.len() < header + length {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..count]);
                }
                let request: Value =
                    serde_json::from_slice(&data[header..header + length]).unwrap();
                saved.lock().unwrap().push(request.clone());
                let result = match request["method"].as_str().unwrap() {
                    "getGenesisHash" => json!(genesis),
                    "getTokenAccountsByOwner" => {
                        json!({"context":{"slot":42},"value": accounts.iter()
                        .filter(|a| a.program == request["params"][1]["programId"])
                        .map(|a| json!({"pubkey":a.address,"account":a.account})).collect::<Vec<_>>()})
                    }
                    "getMultipleAccounts" => {
                        json!({"context":{"slot":42},"value":request["params"][0].as_array().unwrap().iter()
                        .map(|mint| { let a = accounts.iter().find(|a| a.mint == *mint).unwrap();
                            json!({"owner":a.program,"data":{"parsed":{"type":"mint","info":a.mint_info}}}) }).collect::<Vec<_>>()})
                    }
                    "getAccountInfo" => {
                        let address = request["params"][0].as_str().unwrap();
                        let value = if let Some(a) = accounts.iter().find(|a| a.address == address)
                        {
                            a.account.clone()
                        } else if let Some(a) = accounts.iter().find(|a| a.mint == address) {
                            json!({"owner":a.program,"data":{"parsed":{"type":"mint","info":a.mint_info}}})
                        } else {
                            Value::Null
                        };
                        json!({"context":{"slot":42},"value":value})
                    }
                    "getBlockHeight" => json!(43),
                    "sendTransaction" => {
                        use base64::Engine;
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(request["params"][0].as_str().unwrap())
                            .unwrap();
                        let tx: solana_transaction::versioned::VersionedTransaction =
                            wincode::deserialize(&bytes).unwrap();
                        tx.verify_and_hash_message().unwrap();
                        json!(tx.signatures[0].to_string())
                    }
                    "getSignatureStatuses" => {
                        json!({"context":{"slot":44},"value":[{"slot":44,"confirmations":null,"err":null,"status":{"Ok":null},"confirmationStatus":"finalized"}]})
                    }
                    "getMinimumBalanceForRentExemption" => json!(2039280),
                    "getLatestBlockhash" => {
                        json!({"context":{"slot":42},"value":{"blockhash":"11111111111111111111111111111111","lastValidBlockHeight":200}})
                    }
                    "simulateTransaction" => {
                        json!({"context":{"slot":42},"value":{"err":null,"logs":["Program success"],"accounts":null,"unitsConsumed":5000,"loadedAccountsDataSize":1000}})
                    }
                    "getFeeForMessage" => json!({"context":{"slot":42},"value":5000}),
                    method => panic!("Unexpected method: {method}"),
                };
                let body = if partial && request["method"] == "getTokenAccountsByOwner" && request["params"][1]["programId"] == tokens::TOKEN_2022 {
                    json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32603,"message":"Token-2022 unavailable"}})
                } else { json!({"jsonrpc":"2.0","id":request["id"],"result":result}) }.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    Server {
        profile: RpcProfile::custom(
            "mock",
            &format!("http://127.0.0.1:{port}"),
            &format!("ws://127.0.0.1:{port}"),
        )
        .unwrap(),
        requests,
        task,
    }
}

#[tokio::test]
async fn discovery_keeps_both_programs_empty_accounts_and_batched_mints() {
    let server = server(false).await;
    let owner = solana_pubkey::Pubkey::new_from_array([7; 32]);
    let snapshot = tokens::fetch(&network::client(&server.profile), &owner)
        .await
        .unwrap();
    assert_eq!(snapshot.accounts.len(), 4);
    assert!(snapshot.warnings.is_empty());
    assert!(snapshot.accounts.iter().all(|a| a.mint_info.is_some()));
    assert!(
        snapshot
            .accounts
            .iter()
            .any(|a| a.amount == 0 && !a.associated)
    );
    assert!(
        snapshot
            .accounts
            .iter()
            .any(|a| a.program == tokens::TOKEN_2022)
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r["method"] == "getMultipleAccounts")
            .count(),
        1
    );
    assert!(requests.iter().all(|r| r["method"] != "sendTransaction"));
}

#[tokio::test]
async fn failed_program_discovery_reports_partial_coverage() {
    let server = server(true).await;
    let snapshot = tokens::fetch(
        &network::client(&server.profile),
        &solana_pubkey::Pubkey::new_from_array([7; 32]),
    )
    .await
    .unwrap();
    assert_eq!(snapshot.accounts.len(), 3);
    assert!(!snapshot.warnings.is_empty());
    assert!(snapshot.warnings[0].contains("Token-2022"));
}

#[tokio::test]
async fn token_operations_simulate_exact_instructions_without_signing_or_submission() {
    let server = server(false).await;
    let mut app = App::new("/fixture".into(), Config::default(), vec![]);
    demo::populate(&mut app);
    let destination = solana_pubkey::Pubkey::new_from_array([99; 32]).to_string();
    for index in [0, 2] {
        for format in [
            solte::transaction::Format::Legacy,
            solte::transaction::Format::V0,
            solte::transaction::Format::V1,
        ] {
            let prepared = solte::token_operations::prepare_transfer(
                &server.profile,
                &app.wallets[0],
                &app.tokens[index],
                &destination,
                "0.000001",
                false,
                format,
            )
            .await
            .unwrap();
            assert!(prepared.simulation_error.is_none());
            assert!(
                prepared
                    .transaction
                    .signatures
                    .iter()
                    .all(|s| *s == solana_signature::Signature::default())
            );
            let instructions = prepared.transaction.message.instructions();
            assert_eq!(instructions.len(), 2);
            assert_eq!(instructions[0].data, [1]);
            assert_eq!(instructions[1].data, [12, 1, 0, 0, 0, 0, 0, 0, 0, 6]);
            assert!(prepared.summary.iter().any(|s| s.contains("raw 1")));
            assert_eq!(prepared.last_valid_block_height, 200);
        }
    }
    let prepared = solte::token_operations::prepare_create(
        &server.profile,
        &app.wallets[0],
        &app.tokens[2].mint,
        &destination,
        solte::transaction::Format::Auto,
    )
    .await
    .unwrap();
    assert_eq!(prepared.transaction.message.instructions().len(), 1);
    assert!(prepared.summary.iter().any(|s| s.contains("idempotent")));
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["method"] == "sendTransaction")
    );
}

#[tokio::test]
async fn token_transfers_reject_frozen_precision_balance_and_wrong_destination() {
    let server = server(false).await;
    let mut app = App::new("/fixture".into(), Config::default(), vec![]);
    demo::populate(&mut app);
    let destination = solana_pubkey::Pubkey::new_from_array([99; 32]).to_string();
    for (index, value, target, explicit) in [
        (1, "1", destination.as_str(), false),
        (0, "0.0000001", destination.as_str(), false),
        (0, "126", destination.as_str(), false),
        (0, "1", app.tokens[2].address.as_str(), true),
        (0, "1", destination.as_str(), true),
    ] {
        assert!(
            solte::token_operations::prepare_transfer(
                &server.profile,
                &app.wallets[0],
                &app.tokens[index],
                target,
                value,
                explicit,
                solte::transaction::Format::Auto
            )
            .await
            .is_err()
        );
    }
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["method"] == "simulateTransaction")
    );
}

#[tokio::test]
async fn mainnet_token_operations_are_rejected_before_reading_accounts() {
    let server = server_on_network(false, network::MAINNET_GENESIS).await;
    let mut app = App::new("/fixture".into(), Config::default(), vec![]);
    demo::populate(&mut app);
    assert!(
        solte::token_operations::prepare_create(
            &server.profile,
            &app.wallets[0],
            &app.tokens[0].mint,
            &app.wallets[0].address,
            solte::transaction::Format::Auto
        )
        .await
        .is_err()
    );
    assert!(
        solte::token_operations::prepare_transfer(
            &server.profile,
            &app.wallets[0],
            &app.tokens[0],
            &app.wallets[1].address,
            "1",
            false,
            solte::transaction::Format::Auto
        )
        .await
        .is_err()
    );
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r["method"] == "getGenesisHash")
    );
}

#[tokio::test]
async fn mint_creation_reviews_both_signers_and_tracks_public_data_before_submission() {
    use solte::{
        mints::{self, CreateMint},
        operations,
        storage::Store,
        transaction::Format,
        wallet,
    };
    let server = server(false).await;
    let root = tempfile::tempdir().unwrap();
    let payer = wallet::create(root.path(), "payer").unwrap();
    let authority = wallet::create(root.path(), "authority").unwrap();
    let store = Store::open(root.path()).unwrap();
    for program in [tokens::TOKEN_PROGRAM, tokens::TOKEN_2022] {
        for format in [Format::Legacy, Format::V0, Format::V1] {
            let prepared = mints::prepare_create(
                &server.profile,
                &payer,
                std::slice::from_ref(&authority),
                CreateMint {
                    program: program.parse().unwrap(),
                    decimals: 6,
                    authority: authority.address.parse().unwrap(),
                    freeze: None,
                    initial_supply: 1_250_000,
                    format,
                },
            )
            .await
            .unwrap();
            assert_eq!(prepared.transaction.message.instructions().len(), 4);
            assert_eq!(prepared.transaction.signatures.len(), 3);
            assert!(
                prepared
                    .transaction
                    .signatures
                    .iter()
                    .all(|s| *s == solana_signature::Signature::default())
            );
            let record = prepared.created_mint.as_ref().unwrap().clone();
            let init = &prepared.transaction.message.instructions()[1];
            assert!(matches!(
                spl_token_2022_interface::instruction::TokenInstruction::unpack(&init.data)
                    .unwrap(),
                spl_token_2022_interface::instruction::TokenInstruction::InitializeMint2 {
                    decimals: 6,
                    ..
                }
            ));
            let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
            operations::submit(prepared, 1, store.clone(), sender).await;
            let mut finished = false;
            while let Ok(event) = receiver.try_recv() {
                match event.update {
                    operations::OperationUpdate::Finished(_) => finished = true,
                    operations::OperationUpdate::Failed(message) => panic!("{message}"),
                    _ => {}
                }
            }
            assert!(finished);
            let saved = store
                .mints(&server.profile.http, network::DEVNET_GENESIS)
                .await
                .unwrap();
            assert!(
                saved
                    .iter()
                    .any(|m| m.address == record.address && !m.signature.is_empty())
            );
            assert!(
                store
                    .mints(&server.profile.http, "other-chain")
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }
    let reopened = Store::open(root.path()).unwrap();
    assert_eq!(
        reopened
            .mints(&server.profile.http, network::DEVNET_GENESIS)
            .await
            .unwrap()
            .len(),
        6
    );
    let zero = mints::prepare_create(
        &server.profile,
        &payer,
        &[],
        CreateMint {
            program: tokens::TOKEN_PROGRAM.parse().unwrap(),
            decimals: 0,
            authority: authority.address.parse().unwrap(),
            freeze: None,
            initial_supply: 0,
            format: Format::Auto,
        },
    )
    .await
    .unwrap();
    assert_eq!(zero.transaction.message.instructions().len(), 2);
    assert_eq!(zero.transaction.signatures.len(), 2);
    assert!(
        mints::prepare_create(
            &server.profile,
            &payer,
            &[],
            CreateMint {
                program: tokens::TOKEN_PROGRAM.parse().unwrap(),
                decimals: 6,
                authority: authority.address.parse().unwrap(),
                freeze: None,
                initial_supply: 1,
                format: Format::Auto
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn mint_more_checks_authority_and_exact_amount_without_signing_during_review() {
    use solte::{mints, transaction::Format};
    let server = server(false).await;
    let mut app = App::new("/fixture".into(), Config::default(), vec![]);
    demo::populate(&mut app);
    for account in [&app.tokens[0], &app.tokens[2]] {
        let prepared = mints::prepare_mint_more(
            &server.profile,
            &app.wallets[0],
            &app.wallets,
            &account.mint,
            &app.wallets[1].address,
            "1.25",
            Format::V0,
        )
        .await
        .unwrap();
        assert_eq!(prepared.transaction.message.instructions().len(), 2);
        assert!(
            prepared
                .transaction
                .signatures
                .iter()
                .all(|s| *s == solana_signature::Signature::default())
        );
        assert!(
            mints::prepare_mint_more(
                &server.profile,
                &app.wallets[0],
                &[],
                &account.mint,
                &app.wallets[0].address,
                "1",
                Format::Auto
            )
            .await
            .is_err()
        );
        assert!(
            mints::prepare_mint_more(
                &server.profile,
                &app.wallets[0],
                &app.wallets,
                &account.mint,
                &app.wallets[0].address,
                "0",
                Format::Auto
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn mint_mainnet_and_failed_reviews_never_create_project_records_or_send() {
    use solte::{
        mints::{self, CreateMint},
        operations,
        storage::Store,
        transaction::Format,
        wallet,
    };
    let root = tempfile::tempdir().unwrap();
    let payer = wallet::create(root.path(), "payer").unwrap();
    let store = Store::open(root.path()).unwrap();
    let mainnet = server_on_network(false, network::MAINNET_GENESIS).await;
    let options = || CreateMint {
        program: tokens::TOKEN_PROGRAM.parse().unwrap(),
        decimals: 6,
        authority: payer.address.parse().unwrap(),
        freeze: None,
        initial_supply: 0,
        format: Format::Auto,
    };
    assert!(
        mints::prepare_create(&mainnet.profile, &payer, &[], options())
            .await
            .is_err()
    );
    assert!(
        mainnet
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r["method"] == "getGenesisHash")
    );
    let server = server(false).await;
    let mut prepared = mints::prepare_create(&server.profile, &payer, &[], options())
        .await
        .unwrap();
    prepared.simulation_error = Some("Insufficient funds".into());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    operations::submit(prepared, 1, store.clone(), sender).await;
    assert!(matches!(
        receiver.recv().await.unwrap().update,
        operations::OperationUpdate::Failed(_)
    ));
    assert!(
        store
            .mints(&server.profile.http, network::DEVNET_GENESIS)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["method"] == "sendTransaction")
    );
}
