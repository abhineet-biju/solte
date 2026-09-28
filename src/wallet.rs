use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use solana_keypair::{Keypair, read_keypair_file};
use solana_signer::Signer;

use crate::config::{Config, expand_path, private_dir};

#[derive(Clone, Debug)]
pub struct Wallet {
    pub name: String,
    pub address: String,
    pub path: PathBuf,
    pub program: bool,
}

impl Wallet {
    pub fn load(path: &Path, name: Option<&str>) -> Result<Self> {
        let path = path.canonicalize().context("Keypair file does not exist")?;
        if fs::metadata(&path)?.len() > 4096 {
            bail!("Keypair file is unexpectedly large");
        }
        let keypair =
            read_keypair_file(&path).map_err(|_| anyhow::anyhow!("Invalid Solana keypair JSON"))?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("wallet");
        let program =
            path.components().any(|p| p.as_os_str() == "deploy") && stem.ends_with("-keypair");
        Ok(Self {
            name: name.unwrap_or(stem).into(),
            address: keypair.pubkey().to_string(),
            path,
            program,
        })
    }

    pub fn signer(&self) -> Result<Keypair> {
        if self.program {
            bail!("Program identity is read-only; select a development wallet to sign");
        }
        let keypair = read_keypair_file(&self.path)
            .map_err(|_| anyhow::anyhow!("Cannot read wallet keypair"))?;
        if keypair.pubkey().to_string() != self.address {
            bail!("Keypair file changed; reload the wallet before signing");
        }
        Ok(keypair)
    }
}

pub fn discover(root: &Path, config: &Config) -> (Vec<Wallet>, Vec<String>) {
    let mut candidates: Vec<(PathBuf, Option<String>)> = config
        .wallets
        .iter()
        .map(|w| (expand_path(root, &w.path), Some(w.name.clone())))
        .collect();
    if let Ok(anchor) = fs::read_to_string(root.join("Anchor.toml"))
        && let Ok(anchor) = toml::from_str::<toml::Value>(&anchor)
        && let Some(path) = anchor
            .get("provider")
            .and_then(|v| v.get("wallet"))
            .and_then(|v| v.as_str())
    {
        candidates.push((
            expand_path(root, Path::new(path)),
            Some("anchor-wallet".into()),
        ));
    }
    for dir in [
        root.to_path_buf(),
        root.join("keys"),
        root.join("wallets"),
        root.join(".solte/keys"),
        root.join("target/deploy"),
    ] {
        if let Ok(entries) = fs::read_dir(dir) {
            let mut paths: Vec<_> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|v| v == "json"))
                .collect();
            paths.sort();
            candidates.extend(paths.into_iter().map(|p| (p, None)));
        }
    }
    let mut seen = HashSet::new();
    let mut wallets = Vec::new();
    let mut warnings = Vec::new();
    for (path, name) in candidates {
        match Wallet::load(&path, name.as_deref()) {
            Ok(wallet) if seen.insert(wallet.address.clone()) => wallets.push(wallet),
            Ok(_) => {}
            Err(error) if name.is_some() => {
                warnings.push(format!("{}: {error}", name.unwrap_or_default()))
            }
            Err(_) => {}
        }
    }
    wallets.sort_by_key(|w| w.program);
    (wallets, warnings)
}

pub fn create(root: &Path, name: &str) -> Result<Wallet> {
    if name.is_empty()
        || name.len() > 40
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        bail!("Use 1–40 letters, numbers, dashes, or underscores for the name");
    }
    private_dir(&root.join(".solte"))?;
    let dir = root.join(".solte/keys");
    private_dir(&dir)?;
    let path = dir.join(format!("{name}.json"));
    let keypair = Keypair::new();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .context("Cannot create keypair; that name may already exist")?;
    file.write_all(serde_json::to_string(&keypair.to_bytes().to_vec())?.as_bytes())?;
    file.sync_all()?;
    Wallet::load(&path, Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_never_overwrites_and_preserves_cli_format() {
        let temp = tempfile::tempdir().unwrap();
        let wallet = create(temp.path(), "buyer").unwrap();
        assert_eq!(
            wallet.signer().unwrap().pubkey().to_string(),
            wallet.address
        );
        assert!(create(temp.path(), "buyer").is_err());
        assert!(create(temp.path(), "../escape").is_err());
        assert_eq!(discover(temp.path(), &Config::default()).0.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(wallet.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn discovers_anchor_wallet_without_treating_other_json_as_keys() {
        let temp = tempfile::tempdir().unwrap();
        let original = create(temp.path(), "payer").unwrap();
        fs::write(
            temp.path().join("Anchor.toml"),
            "[provider]\nwallet = '.solte/keys/payer.json'\n",
        )
        .unwrap();
        fs::write(temp.path().join("package.json"), "{\"name\":\"sample\"}").unwrap();
        let (wallets, warnings) = discover(temp.path(), &Config::default());
        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].address, original.address);
        assert_eq!(wallets[0].name, "anchor-wallet");
        assert!(warnings.is_empty());
    }

    #[test]
    fn refuses_to_sign_if_file_changes() {
        let temp = tempfile::tempdir().unwrap();
        let wallet = create(temp.path(), "buyer").unwrap();
        let replacement = create(temp.path(), "seller").unwrap();
        fs::copy(replacement.path, &wallet.path).unwrap();
        assert!(wallet.signer().is_err());
    }
}
