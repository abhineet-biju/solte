use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::request::RpcRequest;

use crate::{
    config::RpcProfile,
    network,
    operations::{self, PreparedTransfer},
    tokens::{self, TokenAccount},
    transaction::{self, Format},
    wallet::Wallet,
};

async fn parsed_account(rpc: &RpcClient, address: &Pubkey) -> Result<Value> {
    let response: Value = rpc
        .send(
            RpcRequest::GetAccountInfo,
            json!([address.to_string(), {"encoding":"jsonParsed","commitment":"confirmed"}]),
        )
        .await?;
    ensure!(
        !response["value"].is_null(),
        "Account {address} does not exist"
    );
    Ok(response["value"].clone())
}

async fn mint_info(rpc: &RpcClient, address: &Pubkey) -> Result<(Pubkey, Value)> {
    let account = parsed_account(rpc, address).await?;
    let program: Pubkey = account["owner"]
        .as_str()
        .context("Mint owning program unavailable")?
        .parse()?;
    ensure!(
        [tokens::TOKEN_PROGRAM, tokens::TOKEN_2022].contains(&program.to_string().as_str()),
        "Mint must belong to SPL Token or Token-2022"
    );
    ensure!(
        account["data"]["parsed"]["type"] == "mint",
        "Address must be a mint, not a token account"
    );
    let info = account["data"]["parsed"]["info"].clone();
    ensure!(
        info["isInitialized"].as_bool().unwrap_or(false),
        "Mint is not initialized"
    );
    Ok((program, info))
}

pub fn ensure_supported(info: &Value) -> Result<()> {
    if let Some(extensions) = info.get("extensions") {
        let items = extensions
            .as_array()
            .context("Cannot decode extensions; operation unavailable")?;
        for item in items {
            let name = item["extension"]
                .as_str()
                .context("Cannot identify extension")?;
            ensure!(
                [
                    "immutableOwner",
                    "mintCloseAuthority",
                    "metadataPointer",
                    "tokenMetadata",
                    "groupPointer",
                    "tokenGroup",
                    "groupMemberPointer",
                    "tokenGroupMember"
                ]
                .contains(&name),
                "{name} needs additional handling. This account is inspectable, but Solte cannot transfer it yet"
            );
        }
    }
    Ok(())
}

pub fn create_associated(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    program: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: tokens::ASSOCIATED_PROGRAM
            .parse()
            .expect("associated program address"),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(tokens::associated_address(owner, mint, program), false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(*program, false),
        ],
        data: vec![1], // CreateIdempotent preserves an already initialized ATA.
    }
}

pub async fn prepare_create(
    profile: &RpcProfile,
    wallet: &Wallet,
    mint: &str,
    owner: &str,
    format: Format,
) -> Result<PreparedTransfer> {
    ensure!(!wallet.program, "Program identities cannot sign");
    let mint: Pubkey = mint.parse().context("Enter a valid mint address")?;
    let owner: Pubkey = owner
        .parse()
        .context("Enter a valid recipient wallet address")?;
    let payer: Pubkey = wallet.address.parse()?;
    let rpc = network::client(profile);
    let genesis = network::verify_development_network(&rpc).await?;
    let (program, _) = mint_info(&rpc, &mint).await?;
    let account = tokens::associated_address(&owner, &mint, &program);
    let rent = rpc.get_minimum_balance_for_rent_exemption(165).await?;
    let instructions = [create_associated(&payer, &owner, &mint, &program)];
    let mut prepared = prepare(profile, wallet, &rpc, &genesis, &instructions, format).await?;
    prepared.summary = vec!["Create associated token account · idempotent".into(), format!("Mint       {mint}"),
        format!("Owner      {owner}"), format!("Account    {account}"), format!("Program    {program}"),
        format!("Base rent estimate {} SOL · Token-2022 extensions may require more", crate::amount::format_sol(rent)),
        "Existing associated accounts are preserved. New account rent is paid by the selected wallet.".into()];
    Ok(prepared)
}

pub async fn prepare_transfer(
    profile: &RpcProfile,
    wallet: &Wallet,
    selected: &TokenAccount,
    recipient: &str,
    value: &str,
    destination_is_account: bool,
    format: Format,
) -> Result<PreparedTransfer> {
    ensure!(!wallet.program, "Program identities cannot sign");
    let payer: Pubkey = wallet.address.parse()?;
    let rpc = network::client(profile);
    let genesis = network::verify_development_network(&rpc).await?;
    let source: Pubkey = selected.address.parse()?;
    let current = parsed_account(&rpc, &source).await?;
    let account = tokens::decode(&selected.address, current, &payer, 0)?;
    ensure!(
        account.mint == selected.mint
            && account.program == selected.program
            && account.decimals == selected.decimals,
        "Source account changed; refresh and review again"
    );
    ensure!(
        account.state == "initialized",
        "Source account is frozen or uninitialized"
    );
    ensure_supported(account.info())?;
    let mint: Pubkey = account.mint.parse()?;
    let (program, mint_data) = mint_info(&rpc, &mint).await?;
    ensure!(
        program.to_string() == account.program,
        "Mint program does not match the source account"
    );
    ensure!(
        mint_data["decimals"].as_u64() == Some(u64::from(account.decimals)),
        "Mint decimals changed; refresh again"
    );
    ensure_supported(&mint_data)?;
    let amount = tokens::parse_amount(value, account.decimals)?;
    ensure!(amount <= account.amount, "Insufficient token balance");
    let recipient: Pubkey = recipient
        .parse()
        .context("Enter a valid destination address")?;
    let mut instructions = Vec::new();
    let destination = if destination_is_account {
        recipient
    } else {
        instructions.push(create_associated(&payer, &recipient, &mint, &program));
        tokens::associated_address(&recipient, &mint, &program)
    };
    // Inspect existing destinations too, including extension requirements.
    let response: Value = rpc
        .send(
            RpcRequest::GetAccountInfo,
            json!([destination.to_string(), {"encoding":"jsonParsed","commitment":"confirmed"}]),
        )
        .await?;
    if response["value"].is_null() {
        ensure!(
            !destination_is_account,
            "Destination token account does not exist; use Wallet / create ATA instead"
        );
    } else {
        let data = &response["value"];
        let info = &data["data"]["parsed"]["info"];
        ensure!(
            data["owner"] == account.program
                && data["data"]["parsed"]["type"] == "account"
                && info["mint"] == account.mint,
            "Destination must be a token account for the same mint and program"
        );
        ensure!(
            info["state"] == "initialized",
            "Destination account is frozen or uninitialized"
        );
        ensure_supported(info)?;
    }
    ensure!(
        destination != source,
        "Choose a different destination account"
    );
    instructions.push(spl_token_2022_interface::instruction::transfer_checked(
        &program,
        &source,
        &mint,
        &destination,
        &payer,
        &[],
        amount,
        account.decimals,
    )?);
    let mut prepared = prepare(profile, wallet, &rpc, &genesis, &instructions, format).await?;
    prepared.summary = vec![
        "Transfer tokens · TransferChecked".into(),
        format!("Mint       {mint}"),
        format!("Source     {source}"),
        format!("Destination {destination}"),
        format!("Program    {program}"),
        format!(
            "Amount     {} · raw {amount} · {} decimals",
            tokens::format_amount(amount, account.decimals),
            account.decimals
        ),
        if destination_is_account {
            "Recipient is an explicit token account.".into()
        } else {
            format!(
                "Recipient wallet {recipient} · associated account created if missing; payer covers rent."
            )
        },
    ];
    Ok(prepared)
}

async fn prepare(
    profile: &RpcProfile,
    wallet: &Wallet,
    rpc: &RpcClient,
    genesis: &str,
    instructions: &[Instruction],
    format: Format,
) -> Result<PreparedTransfer> {
    let (blockhash, height) = rpc
        .get_latest_blockhash_with_commitment(rpc.commitment())
        .await?;
    let transaction =
        transaction::build(format, &wallet.address.parse()?, instructions, blockhash)?;
    let mut prepared =
        operations::inspect(profile, wallet, transaction, genesis.into(), false).await?;
    prepared.last_valid_block_height = height;
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_extensions_fail_closed_and_ata_derivation_includes_program() {
        for extension in [
            "transferFeeConfig",
            "transferHook",
            "nonTransferable",
            "confidentialTransferMint",
            "pausable",
            "unknownFutureExtension",
        ] {
            assert!(
                ensure_supported(&json!({"extensions":[{"extension":extension}]})).is_err(),
                "{extension}"
            );
        }
        assert!(ensure_supported(&json!({"extensions":[{"extension":"immutableOwner"}]})).is_ok());
        let owner = Pubkey::new_from_array([7; 32]);
        let mint = Pubkey::new_from_array([30; 32]);
        let classic: Pubkey = tokens::TOKEN_PROGRAM.parse().unwrap();
        let extended: Pubkey = tokens::TOKEN_2022.parse().unwrap();
        assert_ne!(
            tokens::associated_address(&owner, &mint, &classic),
            tokens::associated_address(&owner, &mint, &extended)
        );
        for program in [classic, extended] {
            let instruction = create_associated(&owner, &owner, &mint, &program);
            assert!(instruction.accounts[0].is_signer && instruction.accounts[0].is_writable);
            assert_eq!(
                instruction.accounts[1].pubkey,
                tokens::associated_address(&owner, &mint, &program)
            );
            assert_eq!(instruction.accounts[5].pubkey, program);
        }
    }
}
