use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use solte::{
    config::RpcProfile,
    network::{MAINNET_GENESIS, client, verify_development_network},
    operations, wallet,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

struct MockRpc {
    profile: RpcProfile,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl Drop for MockRpc {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(genesis: &'static str) -> MockRpc {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let saved = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let saved = saved.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buffer = [0u8; 4096];
                let (header, length) = loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..count]);
                    if let Some(header) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let length = String::from_utf8_lossy(&data[..header])
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                            })
                            .unwrap();
                        break (header + 4, length);
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
                let value = match request["method"].as_str().unwrap() {
                    "getGenesisHash" => json!(genesis),
                    "getVersion" => json!({"solana-core":"3.1.0","feature-set":1}),
                    "getLatestBlockhash" => {
                        json!({"context":{"slot":1},"value":{"blockhash":"11111111111111111111111111111111","lastValidBlockHeight":200}})
                    }
                    "simulateTransaction" => {
                        json!({"context":{"slot":1},"value":{"err":null,"logs":["Program success"],"accounts":null,"unitsConsumed":150}})
                    }
                    "getFeeForMessage" => json!({"context":{"slot":1},"value":5000}),
                    method => panic!("Unexpected RPC method: {method}"),
                };
                let body = json!({"jsonrpc":"2.0","id":request["id"],"result":value}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    MockRpc {
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
async fn rejects_mainnet_by_genesis_even_when_profile_is_named_devnet() {
    let mut mock = server(MAINNET_GENESIS).await;
    mock.profile.name = "Devnet".into();
    let error = verify_development_network(&client(&mock.profile))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Mainnet signing is disabled"));
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn simulation_does_not_release_a_signed_transaction() {
    let mock = server(solte::network::DEVNET_GENESIS).await;
    let temp = tempfile::tempdir().unwrap();
    let wallet = wallet::create(temp.path(), "payer").unwrap();
    let recipient = wallet::create(temp.path(), "recipient").unwrap();
    let prepared = operations::prepare(&mock.profile, &wallet, &recipient.address, 1_000_000)
        .await
        .unwrap();
    assert_eq!(prepared.fee, 5000);
    assert!(prepared.simulation_error.is_none());
    assert!(
        prepared
            .transaction
            .signatures
            .iter()
            .all(|sig| *sig == solana_signature::Signature::default())
    );
    let requests = mock.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|r| r["method"] == "simulateTransaction" && r["params"][1]["sigVerify"] == false)
    );
    assert!(!requests.iter().any(|r| r["method"] == "sendTransaction"));
}
