use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcProfile {
    pub name: String,
    pub http: String,
    pub websocket: String,
}

impl RpcProfile {
    pub fn devnet() -> Self {
        Self {
            name: "Devnet".into(),
            http: "https://api.devnet.solana.com".into(),
            websocket: "wss://api.devnet.solana.com".into(),
        }
    }

    pub fn localnet() -> Self {
        Self {
            name: "Localnet".into(),
            http: "http://127.0.0.1:8899".into(),
            websocket: "ws://127.0.0.1:8900".into(),
        }
    }

    pub fn custom(name: &str, http: &str, websocket: &str) -> Result<Self> {
        if name.trim().is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control)
        {
            bail!("Choose a profile name between 1 and 40 characters");
        }
        let http_url = Url::parse(http.trim()).context("Invalid HTTP endpoint")?;
        let ws_url = Url::parse(websocket.trim()).context("Invalid WebSocket endpoint")?;
        if !matches!(http_url.scheme(), "http" | "https") || http_url.host_str().is_none() {
            bail!("RPC endpoint must use http:// or https://");
        }
        if !matches!(ws_url.scheme(), "ws" | "wss") || ws_url.host_str().is_none() {
            bail!("Subscription endpoint must use ws:// or wss://");
        }
        Ok(Self {
            name: name.trim().into(),
            http: http_url.to_string(),
            websocket: ws_url.to_string(),
        })
    }

    pub fn display_endpoint(&self) -> String {
        Url::parse(&self.http)
            .map(|url| {
                let port = url.port().map(|v| format!(":{v}")).unwrap_or_default();
                format!(
                    "{}://{}{port}",
                    url.scheme(),
                    url.host_str().unwrap_or("unknown")
                )
            })
            .unwrap_or_else(|_| "Invalid endpoint".into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletRef {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub profiles: Vec<RpcProfile>,
    pub selected_profile: usize,
    pub selected_wallet: Option<PathBuf>,
    pub wallets: Vec<WalletRef>,
    pub theme: String,
    pub reduced_motion: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profiles: vec![RpcProfile::devnet(), RpcProfile::localnet()],
            selected_profile: 0,
            selected_wallet: None,
            wallets: Vec::new(),
            theme: "neon".into(),
            reduced_motion: false,
        }
    }
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".solte/config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut config: Self = toml::from_str(&fs::read_to_string(&path)?)
            .context("Cannot read .solte/config.toml")?;
        if config.profiles.is_empty() {
            config.profiles = Self::default().profiles;
        }
        for profile in &config.profiles {
            RpcProfile::custom(&profile.name, &profile.http, &profile.websocket)?;
        }
        config.selected_profile = config.selected_profile.min(config.profiles.len() - 1);
        Ok(config)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let dir = root.join(".solte");
        private_dir(&dir)?;
        let mut file = tempfile::NamedTempFile::new_in(&dir)?;
        use std::io::Write;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(dir.join("config.toml"))?;
        Ok(())
    }
}

pub fn project_root(start: &Path) -> Result<PathBuf> {
    let home = directories::BaseDirs::new().and_then(|dirs| dirs.home_dir().canonicalize().ok());
    project_root_with_home(start, home.as_deref())
}

fn project_root_with_home(start: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let start = start
        .canonicalize()
        .context("Project directory does not exist")?;
    if !start.is_dir() {
        bail!("Project path must be a directory");
    }
    if Some(start.as_path()) == home {
        return Ok(start);
    }
    Ok(start
        .ancestors()
        .take_while(|path| Some(*path) != home)
        .find(|p| {
            p.join(".solte/config.toml").is_file()
                || p.join("Anchor.toml").is_file()
                || p.join(".git").exists()
        })
        .unwrap_or(&start)
        .to_path_buf())
}

pub fn expand_path(root: &Path, path: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = directories::BaseDirs::new()
    {
        return home.home_dir().join(rest);
    }
    if path.is_absolute() {
        path.into()
    } else {
        root.join(path)
    }
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let ignore = path.join(".gitignore");
    if !ignore.exists() {
        fs::write(ignore, "*\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_wallets_do_not_capture_unrelated_subdirectories() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        Config::default().save(&home).unwrap();
        let original = crate::wallet::create(&home, "home-wallet").unwrap();
        let original_bytes = fs::read(&original.path).unwrap();
        for name in ["projects/first", "projects/second"] {
            let project = home.join(name);
            fs::create_dir_all(&project).unwrap();
            let root = project_root_with_home(&project, Some(&home)).unwrap();
            assert_eq!(root, project);
            let config = Config::load(&root).unwrap();
            assert!(crate::wallet::discover(&root, &config).0.is_empty());
            let wallet = crate::wallet::create(&root, "local-wallet").unwrap();
            assert_eq!(wallet.path, project.join(".solte/keys/local-wallet.json"));
        }
        assert_eq!(fs::read(&original.path).unwrap(), original_bytes);
        assert!(!home.join(".solte/keys/local-wallet.json").exists());
        assert_eq!(project_root_with_home(&home, Some(&home)).unwrap(), home);
    }

    #[test]
    fn project_detection_stops_at_home_but_preserves_nested_projects() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().canonicalize().unwrap();
        fs::create_dir(parent.join(".git")).unwrap();
        let home = parent.join("home");
        let project = home.join("project");
        let nested = project.join("tests");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(project_root_with_home(&home, Some(&home)).unwrap(), home);
        assert_eq!(
            project_root_with_home(&nested, Some(&home)).unwrap(),
            nested
        );
        fs::create_dir(home.join(".git")).unwrap();
        assert_eq!(
            project_root_with_home(&nested, Some(&home)).unwrap(),
            nested
        );
        for marker in [".git", "Anchor.toml", ".solte/config.toml"] {
            let path = project.join(marker);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "").unwrap();
            assert_eq!(
                project_root_with_home(&nested, Some(&home)).unwrap(),
                project
            );
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn detects_anchor_project_from_nested_directory() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("Anchor.toml"), "").unwrap();
        fs::create_dir(temp.path().join("tests")).unwrap();
        assert_eq!(
            project_root(&temp.path().join("tests")).unwrap(),
            temp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn custom_rpc_requires_both_valid_transports() {
        assert!(RpcProfile::custom("work", "file:///tmp/rpc", "ws://localhost").is_err());
        assert!(RpcProfile::custom("work", "https://rpc.example", "https://rpc.example").is_err());
        let profile = RpcProfile::custom(
            "work",
            "https://user:secret@rpc.example/private?token=secret",
            "wss://rpc.example",
        )
        .unwrap();
        assert_eq!(profile.display_endpoint(), "https://rpc.example");
    }
}
