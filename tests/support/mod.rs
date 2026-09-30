use std::sync::{Arc, Mutex};

use serde_json::Value;
use solte::config::RpcProfile;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

pub struct MockRpc {
    pub profile: RpcProfile,
    pub requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl MockRpc {
    pub async fn start(handler: impl Fn(&Value) -> Value + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let saved = requests.clone();
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let saved = saved.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buffer = [0u8; 4096];
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
                                        .and_then(|s| s.trim().parse::<usize>().ok())
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
                    let body = handler(&request).to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        Self {
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
}

impl Drop for MockRpc {
    fn drop(&mut self) {
        self.task.abort();
    }
}
