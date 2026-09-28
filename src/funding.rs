use anyhow::Result;
use solana_pubkey::Pubkey;
use std::str::FromStr;

use crate::{config::RpcProfile, network::DEVNET_GENESIS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Faucet {
    Solana,
    Quicknode,
}

pub fn is_devnet(profile: &RpcProfile, genesis: Option<&str>) -> bool {
    match genesis {
        Some(genesis) => genesis == DEVNET_GENESIS,
        None => profile.http.trim_end_matches('/') == RpcProfile::devnet().http,
    }
}

pub fn faucet_url(faucet: Faucet, address: &str) -> Result<String> {
    let address = Pubkey::from_str(address)?;
    let mut url = url::Url::parse(match faucet {
        Faucet::Solana => "https://faucet.solana.com/",
        Faucet::Quicknode => "https://faucet.quicknode.com/solana/devnet",
    })?;
    if faucet == Faucet::Solana {
        url.query_pairs_mut()
            .append_pair("walletAddress", &address.to_string())
            .append_pair("cluster", "devnet");
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_faucet_prefills_only_the_public_address_and_devnet() {
        let address = Pubkey::new_from_array([7; 32]).to_string();
        let url = url::Url::parse(&faucet_url(Faucet::Solana, &address).unwrap()).unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(params.get("walletAddress").unwrap(), &address);
        assert_eq!(params.get("cluster").unwrap(), "devnet");
        assert_eq!(params.len(), 2);
        assert!(faucet_url(Faucet::Solana, "invalid&secret=anything").is_err());
    }

    #[test]
    fn verified_network_takes_precedence_over_profile_labels() {
        assert!(is_devnet(&RpcProfile::devnet(), None));
        assert!(!is_devnet(
            &RpcProfile::devnet(),
            Some(crate::network::MAINNET_GENESIS)
        ));
        assert!(!is_devnet(&RpcProfile::localnet(), None));
    }
}
