use std::{collections::HashMap, path::Path};

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
    Labels(String, String, Reply<crate::labels::Labels>),
    SaveLabels(String, String, Vec<crate::labels::Change>, Reply<()>),
    SaveMint(String, String, crate::mints::MintRecord, Reply<()>),
    Mints(String, String, Reply<Vec<crate::mints::MintRecord>>),
    Config(std::path::PathBuf, crate::config::Config, Reply<()>),
    Load(String, usize, Reply<Vec<TransactionRecord>>),
    Cursors(String, Reply<HashMap<String, String>>),
    SaveCursors(String, String, HashMap<String, String>, Reply<()>),
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
            CREATE TABLE IF NOT EXISTS cursors(scope TEXT NOT NULL, chain TEXT NOT NULL, address TEXT NOT NULL, signature TEXT NOT NULL, PRIMARY KEY(scope, chain, address));
            CREATE TABLE IF NOT EXISTS mints(endpoint TEXT NOT NULL, chain TEXT NOT NULL, address TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(endpoint, chain, address));
            CREATE TABLE IF NOT EXISTS asset_labels(endpoint TEXT NOT NULL, chain TEXT NOT NULL, kind TEXT NOT NULL, address TEXT NOT NULL, label TEXT NOT NULL, PRIMARY KEY(endpoint, chain, kind, address));
            CREATE TABLE IF NOT EXISTS logs(id INTEGER PRIMARY KEY, scope TEXT NOT NULL, payload TEXT NOT NULL);")?;
        let (sender, mut receiver) = mpsc::channel(128);
        std::thread::Builder::new()
            .name("solte-storage".into())
            .spawn(move || {
                let mut conn = conn;
                while let Some(request) = receiver.blocking_recv() {
                    match request {
                        Request::Labels(endpoint, chain, reply) => {
                            let _ = reply.send(load_labels(&conn, &endpoint, &chain));
                        }
                        Request::SaveLabels(endpoint, chain, changes, reply) => {
                            let _ = reply.send(save_labels(&mut conn, &endpoint, &chain, &changes));
                        }
                        Request::SaveMint(endpoint, chain, record, reply) => {
                            let result = (|| -> Result<()> {
                                conn.execute("INSERT INTO mints(endpoint,chain,address,payload) VALUES(?1,?2,?3,?4) ON CONFLICT(endpoint,chain,address) DO UPDATE SET payload=excluded.payload", params![endpoint, chain, record.address, serde_json::to_string(&record)?])?;
                                Ok(())
                            })();
                            let _ = reply.send(result);
                        }
                        Request::Mints(endpoint, chain, reply) => {
                            let result = (|| -> Result<Vec<crate::mints::MintRecord>> {
                                let mut stmt = conn.prepare("SELECT payload FROM mints WHERE endpoint=?1 AND chain=?2 ORDER BY rowid DESC")?;
                                stmt.query_map(params![endpoint, chain], |row| row.get::<_, String>(0))?
                                    .map(|row| Ok(serde_json::from_str(&row?)?)).collect()
                            })();
                            let _ = reply.send(result);
                        }
                        Request::Config(root, config, reply) => {
                            let _ = reply.send(config.save(&root));
                        }
                        Request::Load(scope, limit, reply) => {
                            let _ = reply.send(load(&conn, &scope, limit));
                        }
                        Request::Cursors(scope, reply) => {
                            let _ = reply.send(load_cursors(&conn, &scope));
                        }
                        Request::SaveCursors(scope, chain, cursors, reply) => {
                            let _ = reply.send(save_cursors(&mut conn, &scope, &chain, &cursors));
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

    pub async fn labels(&self, endpoint: &str, chain: &str) -> Result<crate::labels::Labels> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::Labels(
                scope(endpoint, "labels"),
                chain.into(),
                send,
            ))
            .await?;
        receive.await?
    }

    pub async fn save_labels(
        &self,
        endpoint: &str,
        chain: &str,
        changes: Vec<crate::labels::Change>,
    ) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::SaveLabels(
                scope(endpoint, "labels"),
                chain.into(),
                changes,
                send,
            ))
            .await?;
        receive.await?
    }

    pub async fn save_mint(
        &self,
        endpoint: &str,
        chain: &str,
        record: crate::mints::MintRecord,
    ) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::SaveMint(
                scope(endpoint, "mints"),
                chain.into(),
                record,
                send,
            ))
            .await?;
        receive.await?
    }

    pub async fn mints(
        &self,
        endpoint: &str,
        chain: &str,
    ) -> Result<Vec<crate::mints::MintRecord>> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::Mints(scope(endpoint, "mints"), chain.into(), send))
            .await?;
        receive.await?
    }

    pub async fn config(&self, root: &Path, config: crate::config::Config) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::Config(root.into(), config, send))
            .await?;
        receive.await?
    }

    pub async fn load(&self, scope: &str) -> Result<Vec<TransactionRecord>> {
        self.load_limit(scope, 1000).await
    }

    pub async fn load_limit(&self, scope: &str, limit: usize) -> Result<Vec<TransactionRecord>> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::Load(scope.into(), limit.min(10_000), send))
            .await?;
        receive.await?
    }

    pub async fn cursors(&self, scope: &str) -> Result<HashMap<String, String>> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::Cursors(scope.into(), send))
            .await?;
        receive.await?
    }

    pub async fn save_cursors(
        &self,
        scope: &str,
        chain: &str,
        cursors: HashMap<String, String>,
    ) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(Request::SaveCursors(
                scope.into(),
                chain.into(),
                cursors,
                send,
            ))
            .await?;
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

fn load_labels(conn: &Connection, endpoint: &str, chain: &str) -> Result<crate::labels::Labels> {
    let mut labels = crate::labels::Labels::default();
    let mut stmt =
        conn.prepare("SELECT kind,address,label FROM asset_labels WHERE endpoint=?1 AND chain=?2")?;
    let rows = stmt.query_map(params![endpoint, chain], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (kind, address, raw) = row?;
        let Some(label) = crate::labels::normalize(&raw).ok().flatten() else {
            continue;
        };
        match kind.as_str() {
            "mint" => {
                labels.mints.insert(address, label);
            }
            "account" => {
                labels.accounts.insert(address, label);
            }
            _ => {}
        }
    }
    Ok(labels)
}

fn save_labels(
    conn: &mut Connection,
    endpoint: &str,
    chain: &str,
    changes: &[crate::labels::Change],
) -> Result<()> {
    let tx = conn.transaction()?;
    for change in changes {
        if let Some(label) = &change.label {
            let label = crate::labels::normalize(label)?
                .ok_or_else(|| anyhow!("Empty name must be cleared"))?;
            tx.execute("INSERT INTO asset_labels(endpoint,chain,kind,address,label) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(endpoint,chain,kind,address) DO UPDATE SET label=excluded.label", params![endpoint, chain, change.kind.key(), change.address, label])?;
        } else {
            tx.execute("DELETE FROM asset_labels WHERE endpoint=?1 AND chain=?2 AND kind=?3 AND address=?4", params![endpoint, chain, change.kind.key(), change.address])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn load(conn: &Connection, scope: &str, limit: usize) -> Result<Vec<TransactionRecord>> {
    let mut stmt = conn.prepare("SELECT payload FROM transactions WHERE scope=?1 AND chain=(SELECT chain FROM networks WHERE scope=?1) ORDER BY slot DESC LIMIT ?2")?;
    let rows = stmt.query_map(params![scope, limit as i64], |r| r.get::<_, String>(0))?;
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

fn load_cursors(conn: &Connection, scope: &str) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT address,signature FROM cursors WHERE scope=?1 AND chain=(SELECT chain FROM networks WHERE scope=?1)")?;
    Ok(stmt
        .query_map([scope], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?)
}

fn save_cursors(
    conn: &mut Connection,
    scope: &str,
    chain: &str,
    cursors: &HashMap<String, String>,
) -> Result<()> {
    let tx = conn.transaction()?;
    for (address, signature) in cursors {
        tx.execute("INSERT INTO cursors(scope,chain,address,signature) VALUES(?1,?2,?3,?4) ON CONFLICT(scope,chain,address) DO UPDATE SET signature=excluded.signature", params![scope, chain, address, signature])?;
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
    async fn asset_names_persist_with_separate_project_rpc_network_and_kind_scopes() {
        use crate::labels::{Change, Kind};
        let root = tempfile::tempdir().unwrap();
        let other_root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let changes = vec![
            Change {
                kind: Kind::Mint,
                address: "same-address".into(),
                label: Some("Dev USD".into()),
            },
            Change {
                kind: Kind::Account,
                address: "same-address".into(),
                label: Some("Alice balance".into()),
            },
        ];
        store
            .save_labels("rpc-a", "chain-a", changes)
            .await
            .unwrap();
        let reopened = Store::open(root.path()).unwrap();
        let names = reopened.labels("rpc-a", "chain-a").await.unwrap();
        assert_eq!(names.mints["same-address"], "Dev USD");
        assert_eq!(names.accounts["same-address"], "Alice balance");
        assert!(
            reopened
                .labels("rpc-b", "chain-a")
                .await
                .unwrap()
                .mints
                .is_empty()
        );
        assert!(
            reopened
                .labels("rpc-a", "chain-b")
                .await
                .unwrap()
                .mints
                .is_empty()
        );
        assert!(
            Store::open(other_root.path())
                .unwrap()
                .labels("rpc-a", "chain-a")
                .await
                .unwrap()
                .mints
                .is_empty()
        );
        reopened
            .save_labels(
                "rpc-a",
                "chain-a",
                vec![Change {
                    kind: Kind::Mint,
                    address: "same-address".into(),
                    label: None,
                }],
            )
            .await
            .unwrap();
        let names = reopened.labels("rpc-a", "chain-a").await.unwrap();
        assert!(names.mints.is_empty());
        assert_eq!(names.accounts["same-address"], "Alice balance");
        let bad_batch = vec![
            Change {
                kind: Kind::Account,
                address: "same-address".into(),
                label: Some("Changed".into()),
            },
            Change {
                kind: Kind::Mint,
                address: "other".into(),
                label: Some("Invalid\nname".into()),
            },
        ];
        assert!(
            reopened
                .save_labels("rpc-a", "chain-a", bad_batch)
                .await
                .is_err()
        );
        assert_eq!(
            reopened.labels("rpc-a", "chain-a").await.unwrap().accounts["same-address"],
            "Alice balance"
        );
    }

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

    #[tokio::test]
    async fn archived_pages_and_per_address_cursors_survive_reopening() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let records = (0..1100)
            .map(|i| TransactionRecord {
                signature: format!("signature-{i}"),
                slot: i,
                timestamp: None,
                error: None,
                details: None,
            })
            .collect();
        store.save("scope", "chain-a", records).await.unwrap();
        store
            .save_cursors(
                "scope",
                "chain-a",
                HashMap::from([
                    ("wallet".into(), "older-wallet-signature".into()),
                    ("token".into(), "older-token-signature".into()),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(store.load("scope").await.unwrap().len(), 1000);
        assert_eq!(store.load_limit("scope", 1250).await.unwrap().len(), 1100);
        let reopened = Store::open(temp.path()).unwrap();
        assert_eq!(reopened.cursors("scope").await.unwrap().len(), 2);
        reopened.save("scope", "chain-b", vec![]).await.unwrap();
        assert!(reopened.cursors("scope").await.unwrap().is_empty());
    }
}
