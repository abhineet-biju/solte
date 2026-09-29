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
                    "getGenesisHash" => json!(network::DEVNET_GENESIS),
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
