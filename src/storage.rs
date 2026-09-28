use std::path::Path;

use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

use crate::{
    config::private_dir,
    model::{LogEntry, TransactionRecord},
};

type Reply<T> = oneshot::Sender<Result<T>>;

enum Request {
    Config(std::path::PathBuf, crate::config::Config, Reply<()>),
    Load(String, Reply<Vec<TransactionRecord>>),
    Save(String, String, Vec<TransactionRecord>, Reply<()>),
    Log(String, LogEntry, Reply<()>),
    Logs(String, Reply<Vec<LogEntry>>),
}

#[derive(Clone)]
pub struct Store {
    sender: mpsc::Sender<Request>,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let dir = root.join(".solte");
        private_dir(&dir)?;
        let path = dir.join("history.sqlite");
        let conn = Connection::open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS networks(scope TEXT PRIMARY KEY, chain TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS transactions(scope TEXT NOT NULL, chain TEXT NOT NULL, signature TEXT NOT NULL, slot INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(scope, chain, signature));
            CREATE INDEX IF NOT EXISTS transaction_order ON transactions(scope, chain, slot DESC);
            CREATE TABLE IF NOT EXISTS logs(id INTEGER PRIMARY KEY, scope TEXT NOT NULL, payload TEXT NOT NULL);")?;
        let (sender, mut receiver) = mpsc::channel(128);
        std::thread::Builder::new()
            .name("solte-storage".into())
            .spawn(move || {
                let mut conn = conn;
                while let Some(request) = receiver.blocking_recv() {
                    match request {
                        Request::Config(root, config, reply) => {
                            let _ = reply.send(config.save(&root));
                        }
                        Request::Load(scope, reply) => {
                            let _ = reply.send(load(&conn, &scope));
                        }
                        Request::Save(scope, chain, records, reply) => {
                            let _ = reply.send(save(&mut conn, &scope, &chain, &records));
                        }
                        Request::Log(scope, entry, reply) => {
                            let _ = reply.send(save_log(&mut conn, &scope, &entry));
                        }
                        Request::Logs(scope, reply) => {
                            let _ = reply.send(load_logs(&conn, &scope));
                        }
                    }
                }
            })?;
        Ok(Self { sender })
    }

    pub async fn config(&self, root: &Path, config: crate::config::Config) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::Config(root.into(), config, send))
            .await?;
        receive.await?
    }

    pub async fn load(&self, scope: &str) -> Result<Vec<TransactionRecord>> {
        let (send, receive) = oneshot::channel();
        self.sender.send(Request::Load(scope.into(), send)).await?;
        receive.await?
    }

    pub async fn save(
        &self,
        scope: &str,
        chain: &str,
        records: Vec<TransactionRecord>,
    ) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::Save(scope.into(), chain.into(), records, send))
            .await?;
        receive.await?
    }

    pub async fn log(&self, scope: &str, entry: LogEntry) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::Log(scope.into(), entry, send))
            .await?;
        receive.await?
    }

    pub async fn logs(&self, scope: &str) -> Result<Vec<LogEntry>> {
        let (send, receive) = oneshot::channel();
        self.sender.send(Request::Logs(scope.into(), send)).await?;
        receive.await?
    }
}

pub fn scope(endpoint: &str, address: &str) -> String {
    format!("{:x}:{address}", Sha256::digest(endpoint.as_bytes()))
}

fn load(conn: &Connection, scope: &str) -> Result<Vec<TransactionRecord>> {
    let mut stmt = conn.prepare("SELECT payload FROM transactions WHERE scope=?1 AND chain=(SELECT chain FROM networks WHERE scope=?1) ORDER BY slot DESC LIMIT 1000")?;
    let rows = stmt.query_map([scope], |r| r.get::<_, String>(0))?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

fn save(
    conn: &mut Connection,
    scope: &str,
    chain: &str,
    records: &[TransactionRecord],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("INSERT INTO networks(scope,chain) VALUES(?1,?2) ON CONFLICT(scope) DO UPDATE SET chain=excluded.chain", params![scope, chain])?;
    for record in records {
        let slot = i64::try_from(record.slot).map_err(|_| anyhow!("Invalid slot"))?;
        tx.execute("INSERT INTO transactions(scope,chain,signature,slot,payload) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(scope,chain,signature) DO UPDATE SET slot=excluded.slot,payload=excluded.payload", params![scope, chain, record.signature, slot, serde_json::to_string(record)?])?;
    }
    tx.commit()?;
    Ok(())
}

fn save_log(conn: &mut Connection, scope: &str, entry: &LogEntry) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO logs(scope,payload) VALUES(?1,?2)",
        params![scope, serde_json::to_string(entry)?],
    )?;
    tx.execute("DELETE FROM logs WHERE scope=?1 AND id NOT IN (SELECT id FROM logs WHERE scope=?1 ORDER BY id DESC LIMIT 1000)", [scope])?;
    tx.commit()?;
    Ok(())
}

fn load_logs(conn: &Connection, scope: &str) -> Result<Vec<LogEntry>> {
    let mut stmt =
        conn.prepare("SELECT payload FROM logs WHERE scope=?1 ORDER BY id DESC LIMIT 200")?;
    let mut rows: Vec<LogEntry> = stmt
        .query_map([scope], |r| r.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str(&row?)?))
        .collect::<Result<_>>()?;
    rows.reverse();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deduplicates_transactions_and_separates_cluster_resets() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let record = TransactionRecord {
            signature: "a".into(),
            slot: 1,
            timestamp: None,
            error: None,
            details: None,
        };
        store
            .save("wallet", "chain-a", vec![record.clone(), record.clone()])
            .await
            .unwrap();
        assert_eq!(store.load("wallet").await.unwrap().len(), 1);
        store.save("wallet", "chain-b", vec![]).await.unwrap();
        assert!(store.load("wallet").await.unwrap().is_empty());
        store.save("wallet", "chain-a", vec![]).await.unwrap();
        assert_eq!(store.load("wallet").await.unwrap().len(), 1);
    }
}
