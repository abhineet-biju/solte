use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

use crate::{
    config::RpcProfile, network, operations::PreparedTransfer, token_operations, tokens,
    transaction::Format, wallet::Wallet,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MintRecord {
    pub address: String,
    pub program: String,
    pub decimals: u8,
    pub creator: String,
    pub signature: String,
}

#[derive(Clone, Debug)]
pub struct ProjectMint {
    pub record: MintRecord,
    pub info: Option<Value>,
    pub error: Option<String>,
}

impl ProjectMint {
    pub fn can_mint(&self, wallets: &[Wallet]) -> bool {
        self.info
            .as_ref()
            .and_then(|info| info["mintAuthority"].as_str())
            .is_some_and(|authority| wallets.iter().any(|w| !w.program && w.address == authority))
    }

    pub fn lines(&self) -> Vec<String> {
        let program = if self.record.program == tokens::TOKEN_2022 {
            "Token-2022"
        } else {
            "SPL Token"
        };
        let mut lines = vec![format!("{program} mint"), String::new(), "SUPPLY".into()];
        if let Some(info) = &self.info {
            let supply = info["supply"].as_str().and_then(|v| v.parse::<u64>().ok());
            lines.extend([
                format!(
                    "Supply        {}",
                    supply
                        .map(|v| tokens::format_amount(v, self.record.decimals))
                        .unwrap_or_else(|| "Unavailable".into())
                ),
                format!(
                    "Raw supply    {}",
                    info["supply"].as_str().unwrap_or("Unavailable")
                ),
                format!("Decimals      {}", self.record.decimals),
                String::new(),
                "AUTHORITIES".into(),
                format!(
                    "Mint authority    {}",
                    info["mintAuthority"].as_str().unwrap_or("None")
                ),
                format!(
                    "Freeze authority  {}",
                    info["freezeAuthority"].as_str().unwrap_or("None")
                ),
            ]);
            if let Some(state) = crate::confidential::extension(info, "confidentialTransferMint") {
                lines.extend([
                    String::new(),
                    "CONFIDENTIAL TRANSFERS".into(),
                    "Confidential balances enabled".into(),
                    format!(
                        "Account approval  {}",
                        if state["autoApproveNewAccounts"] == true {
                            "Automatic"
                        } else {
                            "Manual"
                        }
                    ),
                    format!(
                        "Approval authority  {}",
                        state["authority"].as_str().unwrap_or("None")
                    ),
                    format!(
                        "Auditor             {}",
                        state["auditorElgamalPubkey"].as_str().unwrap_or("None")
                    ),
                ]);
            }
        } else {
            lines.push(format!(
                "Unavailable: {}",
                self.error
                    .as_deref()
                    .unwrap_or("Refresh to inspect on-chain state")
            ));
        }
        lines.extend([
            String::new(),
            "ADDRESSES".into(),
            "Mint".into(),
            self.record.address.clone(),
            String::new(),
            "Owning program".into(),
            self.record.program.clone(),
            String::new(),
            "CREATION".into(),
            format!("Created by  {}", self.record.creator),
        ]);
        if !self.record.signature.is_empty() {
            lines.extend(["Signature".into(), self.record.signature.clone()]);
        }
        lines
            .into_iter()
            .map(|line| crate::model::clean_text(&line))
            .collect()
    }
}

pub async fn fetch(
    profile: &RpcProfile,
    genesis: &str,
    records: Vec<MintRecord>,
) -> Result<Vec<ProjectMint>> {
    let rpc = network::client(profile);
    let mut mints = Vec::new();
    for record in records {
        let result = async {
            let (program, info) =
                token_operations::mint_info(&rpc, &record.address.parse()?).await?;
            ensure!(
                program.to_string() == record.program
                    && info["decimals"].as_u64() == Some(u64::from(record.decimals)),
                "Mint no longer matches its project record"
            );
            Ok::<_, anyhow::Error>(info)
        }
        .await;
        mints.push(match result {
            Ok(info) => ProjectMint {
                record,
                info: Some(info),
                error: None,
            },
            Err(error) => ProjectMint {
                record,
                info: None,
                error: Some(network::safe_error(error, profile)),
            },
        });
    }
    ensure!(
        rpc.get_genesis_hash().await?.to_string() == genesis,
        "Network changed during mint discovery"
    );
    Ok(mints)
}

pub struct ConfidentialMintConfig {
    pub auto_approve: bool,
    pub auditor: Option<solana_zk_sdk_pod::encryption::elgamal::PodElGamalPubkey>,
}

pub struct CreateMint {
    pub confidential: Option<ConfidentialMintConfig>,
    pub program: Pubkey,
    pub decimals: u8,
    pub authority: Pubkey,
    pub freeze: Option<Pubkey>,
    pub initial_supply: u64,
    pub format: Format,
}

pub async fn prepare_create(
    profile: &RpcProfile,
    payer: &Wallet,
    wallets: &[Wallet],
    options: CreateMint,
) -> Result<PreparedTransfer> {
    ensure!(!payer.program, "Program identities cannot sign");
    ensure!(
        [tokens::TOKEN_PROGRAM, tokens::TOKEN_2022].contains(&options.program.to_string().as_str()),
        "Choose SPL Token or Token-2022"
    );
    if let Some(config) = &options.confidential
        && let Some(auditor) = config.auditor
    {
        ensure!(
            auditor.0 != [0; 32]
                && solana_zk_sdk::encryption::elgamal::ElGamalPubkey::try_from(auditor).is_ok(),
            "Auditor must be a valid nonzero ElGamal public key"
        );
    }
    let authority = if options.initial_supply > 0 {
        Some(wallets.iter().find(|w| !w.program && w.address == options.authority.to_string()).context("Initial supply requires a mint authority keypair loaded in Solte; use zero supply for an external authority")?.clone())
    } else {
        None
    };
    let rpc = network::client(profile);
    let genesis = network::verify_development_network(&rpc).await?;
    let signer = Keypair::new();
    let mint = signer.pubkey();
    let payer_address = payer.address.parse()?;
    use spl_token_2022_interface::{extension::ExtensionType, state::Mint};
    let mint_size = if options.confidential.is_some() {
        ensure!(
            options.program.to_string() == tokens::TOKEN_2022,
            "Confidential mints require Token-2022"
        );
        ExtensionType::try_calculate_account_len::<Mint>(&[
            ExtensionType::ConfidentialTransferMint,
        ])?
    } else {
        82
    };
    let rent = rpc
        .get_minimum_balance_for_rent_exemption(mint_size)
        .await?;
    let mut instructions = vec![
        solana_system_interface::instruction::create_account(
            &payer_address,
            &mint,
            rent,
            mint_size as u64,
            &options.program,
        ),
        spl_token_2022_interface::instruction::initialize_mint2(
            &options.program,
            &mint,
            &options.authority,
            options.freeze.as_ref(),
            options.decimals,
        )?,
    ];
    if let Some(config) = &options.confidential {
        instructions.insert(1, spl_token_2022_interface::extension::confidential_transfer::instruction::initialize_mint(&options.program, &mint, Some(options.authority), config.auto_approve, config.auditor)?);
    }
    let destination = tokens::associated_address(&payer_address, &mint, &options.program);
    if options.initial_supply > 0 {
        instructions.push(token_operations::create_associated(
            &payer_address,
            &payer_address,
            &mint,
            &options.program,
        ));
        instructions.push(spl_token_2022_interface::instruction::mint_to_checked(
            &options.program,
            &mint,
            &destination,
            &options.authority,
            &[],
            options.initial_supply,
            options.decimals,
        )?);
    }
    let mut prepared = token_operations::prepare(
        profile,
        payer,
        &rpc,
        &genesis,
        &instructions,
        options.format,
    )
    .await?;
    prepared.mint_signer = Some(signer);
    if let Some(authority) = authority
        && authority.address != payer.address
    {
        prepared.extra_wallets.push(authority);
    }
    prepared.created_mint = Some(MintRecord {
        address: mint.to_string(),
        program: options.program.to_string(),
        decimals: options.decimals,
        creator: payer.address.clone(),
        signature: String::new(),
    });
    prepared.summary = vec![
        if options.confidential.is_some() {
            "Create confidential Token-2022 mint".into()
        } else {
            "Create token mint · no extensions".into()
        },
        format!("Mint       {mint}"),
        format!("Program    {}", options.program),
        format!("Decimals   {}", options.decimals),
        format!("Mint authority {}", options.authority),
        format!(
            "Freeze authority {}",
            options
                .freeze
                .map(|p| p.to_string())
                .unwrap_or_else(|| "None".into())
        ),
        format!(
            "Initial supply {} · raw {}",
            tokens::format_amount(options.initial_supply, options.decimals),
            options.initial_supply
        ),
        format!("Mint rent {} SOL", crate::amount::format_sol(rent)),
    ];
    if let Some(config) = &options.confidential {
        prepared.summary.extend([format!("Account approval {}", if config.auto_approve { "Automatic" } else { "Manual · mint authority approves accounts" }), format!("Auditor {}", config.auditor.map(|key| key.to_string()).unwrap_or_else(|| "None".into())), "Initial tokens are public. Configure the account, then deposit and apply to use confidential balances.".into()]);
    }
    if options.initial_supply > 0 {
        let account_size = if options.program.to_string() == tokens::TOKEN_2022 {
            170
        } else {
            165
        };
        let ata_rent = rpc
            .get_minimum_balance_for_rent_exemption(account_size)
            .await?;
        prepared.summary.extend([
            format!("Deposit account {destination}"),
            format!("Account rent {} SOL", crate::amount::format_sol(ata_rent)),
        ]);
    } else {
        prepared.summary.push(
            "Zero supply. Find this mint in Project mints even without a token account.".into(),
        );
    }
    Ok(prepared)
}

pub async fn prepare_mint_more(
    profile: &RpcProfile,
    payer: &Wallet,
    wallets: &[Wallet],
    mint: &str,
    owner: &str,
    amount: &str,
    format: Format,
) -> Result<PreparedTransfer> {
    ensure!(!payer.program, "Program identities cannot sign");
    let mint: Pubkey = mint.parse().context("Enter a valid mint address")?;
    let owner: Pubkey = owner
        .parse()
        .context("Enter a valid recipient wallet address")?;
    let payer_address = payer.address.parse()?;
    let rpc = network::client(profile);
    let genesis = network::verify_development_network(&rpc).await?;
    let (program, info) = token_operations::mint_info(&rpc, &mint).await?;
    token_operations::ensure_supported(&info)?;
    let decimals = u8::try_from(
        info["decimals"]
            .as_u64()
            .context("Mint decimals unavailable")?,
    )?;
    let authority = info["mintAuthority"]
        .as_str()
        .context("Mint authority revoked; no further tokens can be minted")?;
    let signer = wallets
        .iter()
        .find(|w| !w.program && w.address == authority)
        .context("Load the mint authority keypair in Solte first")?
        .clone();
    let amount = tokens::parse_amount(amount, decimals)?;
    let supply: u64 = info["supply"]
        .as_str()
        .context("Mint supply unavailable")?
        .parse()?;
    ensure!(
        supply.checked_add(amount).is_some(),
        "New supply would exceed u64"
    );
    let destination = tokens::associated_address(&owner, &mint, &program);
    let instructions = [
        token_operations::create_associated(&payer_address, &owner, &mint, &program),
        spl_token_2022_interface::instruction::mint_to_checked(
            &program,
            &mint,
            &destination,
            &authority.parse()?,
            &[],
            amount,
            decimals,
        )?,
    ];
    let mut prepared =
        token_operations::prepare(profile, payer, &rpc, &genesis, &instructions, format).await?;
    if signer.address != payer.address {
        prepared.extra_wallets.push(signer);
    }
    prepared.summary = vec![
        "Mint tokens · MintToChecked".into(),
        format!("Mint       {mint}"),
        format!("Program    {program}"),
        format!("Authority  {authority}"),
        format!("Recipient  {owner}"),
        format!("Account    {destination}"),
        format!(
            "Amount     {} · raw {amount} · {decimals} decimals",
            tokens::format_amount(amount, decimals)
        ),
        "Recipient's associated account is created if missing; payer covers rent.".into(),
    ];
    Ok(prepared)
}
