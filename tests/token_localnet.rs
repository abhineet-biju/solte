use solana_instruction::Instruction;
use solana_signer::Signer;
use solte::{
    config::RpcProfile,
    network,
    operations::{self, OperationUpdate, PreparedTransfer},
    storage::Store,
    token_operations, tokens,
    transaction::Format,
    wallet::{self, Wallet},
};
use tokio::sync::mpsc;

async fn submit(prepared: PreparedTransfer, store: &Store) {
    assert!(
        prepared.simulation_error.is_none(),
        "{:?}\n{}",
        prepared.simulation_error,
        prepared.logs.join("\n")
    );
    let (sender, mut receiver) = mpsc::channel(32);
    operations::submit(prepared, 1, store.clone(), sender).await;
    let mut confirmed = false;
    while let Ok(event) = receiver.try_recv() {
        match event.update {
            OperationUpdate::Finished(_) => confirmed = true,
            OperationUpdate::Failed(message) => panic!("{message}"),
            _ => {}
        }
    }
    assert!(confirmed);
}

async fn send(
    rpc: &solana_rpc_client::nonblocking::rpc_client::RpcClient,
    wallet: &Wallet,
    instructions: &[Instruction],
    mint: Option<&solana_keypair::Keypair>,
) {
    let signer = wallet.signer().unwrap();
    let blockhash = rpc.get_latest_blockhash().await.unwrap();
    let message =
        solana_message::VersionedMessage::Legacy(solana_message::Message::new_with_blockhash(
            instructions,
            Some(&signer.pubkey()),
            &blockhash,
        ));
    let mut signers: Vec<&dyn Signer> = vec![&signer];
    if let Some(mint) = mint {
        signers.push(mint);
    }
    let transaction =
        solana_transaction::versioned::VersionedTransaction::try_new(message, &signers).unwrap();
    rpc.send_and_confirm_transaction(&transaction)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires an isolated loopback validator; set SOLTE_TEST_RPC and SOLTE_TEST_WS"]
async fn token_accounts_and_transfers_work_on_both_programs() {
    let http = std::env::var("SOLTE_TEST_RPC").expect("Set a loopback RPC");
    let ws = std::env::var("SOLTE_TEST_WS").expect("Set its WebSocket endpoint");
    assert!(matches!(
        url::Url::parse(&http).unwrap().host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    let profile = RpcProfile::custom("Local token test", &http, &ws).unwrap();
    let rpc = network::client(&profile);
    let root = tempfile::tempdir().unwrap();
    let payer = wallet::create(root.path(), "token-payer").unwrap();
    let recipient = wallet::create(root.path(), "token-recipient").unwrap();
    let store = Store::open(root.path()).unwrap();
    let (sender, mut receiver) = mpsc::channel(32);
    operations::fund(
        profile.clone(),
        payer.clone(),
        10_000_000_000,
        1,
        store.clone(),
        sender,
    )
    .await;
    let mut funded = false;
    while let Ok(event) = receiver.try_recv() {
        match event.update {
            OperationUpdate::Finished(_) => funded = true,
            OperationUpdate::Failed(message) => panic!("{message}"),
            _ => {}
        }
    }
    assert!(funded);
    let owner = payer.address.parse().unwrap();
    for program in [tokens::TOKEN_PROGRAM, tokens::TOKEN_2022] {
        let program = program.parse().unwrap();
        let mint = solana_keypair::Keypair::new();
        let address = mint.pubkey();
        let rent = rpc
            .get_minimum_balance_for_rent_exemption(82)
            .await
            .unwrap();
        send(
            &rpc,
            &payer,
            &[
                solana_system_interface::instruction::create_account(
                    &owner, &address, rent, 82, &program,
                ),
                spl_token_2022_interface::instruction::initialize_mint2(
                    &program,
                    &address,
                    &owner,
                    Some(&owner),
                    6,
                )
                .unwrap(),
            ],
            Some(&mint),
        )
        .await;
        for _ in 0..2 {
            submit(
                token_operations::prepare_create(
                    &profile,
                    &payer,
                    &address.to_string(),
                    &payer.address,
                    Format::Auto,
                )
                .await
                .unwrap(),
                &store,
            )
            .await;
        }
        let ata = tokens::associated_address(&owner, &address, &program);
        send(
            &rpc,
            &payer,
            &[spl_token_2022_interface::instruction::mint_to_checked(
                &program,
                &address,
                &ata,
                &owner,
                &[],
                125_000_000,
                6,
            )
            .unwrap()],
            None,
        )
        .await;
        let snapshot = tokens::fetch(&rpc, &owner).await.unwrap();
        let account = snapshot
            .accounts
            .iter()
            .find(|a| a.mint == address.to_string())
            .unwrap();
        assert_eq!(account.amount, 125_000_000);
        assert!(account.associated && account.mint_info.is_some());
        submit(
            token_operations::prepare_transfer(
                &profile,
                &payer,
                account,
                &recipient.address,
                "1.25",
                false,
                Format::V0,
            )
            .await
            .unwrap(),
            &store,
        )
        .await;
        let received = tokens::fetch(&rpc, &recipient.address.parse().unwrap())
            .await
            .unwrap();
        let target = received
            .accounts
            .iter()
            .find(|a| a.mint == address.to_string())
            .unwrap();
        assert_eq!(target.amount, 1_250_000);
        submit(
            token_operations::prepare_transfer(
                &profile,
                &payer,
                account,
                &target.address,
                "0.000001",
                true,
                Format::Legacy,
            )
            .await
            .unwrap(),
            &store,
        )
        .await;
        let received = tokens::fetch(&rpc, &recipient.address.parse().unwrap())
            .await
            .unwrap();
        assert_eq!(
            received
                .accounts
                .iter()
                .find(|a| a.mint == address.to_string())
                .unwrap()
                .amount,
            1_250_001
        );
        send(
            &rpc,
            &payer,
            &[spl_token_2022_interface::instruction::freeze_account(
                &program,
                &ata,
                &address,
                &owner,
                &[],
            )
            .unwrap()],
            None,
        )
        .await;
        assert!(
            token_operations::prepare_transfer(
                &profile,
                &payer,
                account,
                &recipient.address,
                "1",
                false,
                Format::Auto
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
#[ignore = "requires an isolated loopback validator; set SOLTE_TEST_RPC and SOLTE_TEST_WS"]
async fn project_mint_creation_and_issuance_work_on_both_programs() {
    use solte::mints::{self, CreateMint};
    let http = std::env::var("SOLTE_TEST_RPC").expect("Set a loopback RPC");
    let ws = std::env::var("SOLTE_TEST_WS").expect("Set its WebSocket endpoint");
    assert!(matches!(
        url::Url::parse(&http).unwrap().host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    let profile = RpcProfile::custom("Mint test", &http, &ws).unwrap();
    let rpc = network::client(&profile);
    let root = tempfile::tempdir().unwrap();
    let payer = wallet::create(root.path(), "mint-payer").unwrap();
    let authority = wallet::create(root.path(), "mint-authority").unwrap();
    let recipient = wallet::create(root.path(), "mint-recipient").unwrap();
    let wallets = [payer.clone(), authority.clone(), recipient.clone()];
    let store = Store::open(root.path()).unwrap();
    let signature = rpc
        .request_airdrop(&payer.address.parse().unwrap(), 5_000_000_000)
        .await
        .unwrap();
    while !rpc.confirm_transaction(&signature).await.unwrap() {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    send(
        &rpc,
        &payer,
        &[solana_system_interface::instruction::transfer(
            &payer.address.parse().unwrap(),
            &authority.address.parse().unwrap(),
            100_000_000,
        )],
        None,
    )
    .await;
    let genesis = rpc.get_genesis_hash().await.unwrap().to_string();
    for program in [tokens::TOKEN_PROGRAM, tokens::TOKEN_2022] {
        let program_address = program.parse().unwrap();
        for (initial_supply, format) in [(0, Format::Legacy), (1_250_000, Format::V0)] {
            let prepared = mints::prepare_create(
                &profile,
                &payer,
                &wallets,
                CreateMint {
                    confidential: None,
                    program: program_address,
                    decimals: 6,
                    authority: authority.address.parse().unwrap(),
                    freeze: (initial_supply > 0).then(|| authority.address.parse().unwrap()),
                    initial_supply,
                    format,
                },
            )
            .await
            .unwrap();
            let address = prepared.created_mint.as_ref().unwrap().address.clone();
            submit(prepared, &store).await;
            let registered = store.mints(&profile.http, &genesis).await.unwrap();
            let snapshot = mints::fetch(&profile, &genesis, registered).await.unwrap();
            let mint = snapshot
                .iter()
                .find(|mint| mint.record.address == address)
                .unwrap();
            assert_eq!(
                mint.info.as_ref().unwrap()["supply"],
                initial_supply.to_string()
            );
            if initial_supply == 0 {
                assert!(mint.info.as_ref().unwrap()["freezeAuthority"].is_null());
            } else {
                assert_eq!(
                    mint.info.as_ref().unwrap()["freezeAuthority"],
                    authority.address
                );
            }
            assert!(mint.can_mint(&wallets));
            let payer_accounts = tokens::fetch(&rpc, &payer.address.parse().unwrap())
                .await
                .unwrap();
            assert_eq!(
                payer_accounts
                    .accounts
                    .iter()
                    .find(|a| a.mint == address)
                    .map(|a| a.amount),
                if initial_supply == 0 {
                    None
                } else {
                    Some(initial_supply)
                }
            );
            submit(
                mints::prepare_mint_more(
                    &profile,
                    &payer,
                    &wallets,
                    &address,
                    &recipient.address,
                    "2.5",
                    Format::V0,
                )
                .await
                .unwrap(),
                &store,
            )
            .await;
            let accounts = tokens::fetch(&rpc, &recipient.address.parse().unwrap())
                .await
                .unwrap();
            assert_eq!(
                accounts
                    .accounts
                    .iter()
                    .find(|a| a.mint == address)
                    .unwrap()
                    .amount,
                2_500_000
            );
            // Revoked authorities must stop issuance even if the old wallet is loaded.
            send(
                &rpc,
                &authority,
                &[spl_token_2022_interface::instruction::set_authority(
                    &program_address,
                    &address.parse().unwrap(),
                    None,
                    spl_token_2022_interface::instruction::AuthorityType::MintTokens,
                    &authority.address.parse().unwrap(),
                    &[],
                )
                .unwrap()],
                None,
            )
            .await;
            assert!(
                mints::prepare_mint_more(
                    &profile,
                    &payer,
                    &wallets,
                    &address,
                    &recipient.address,
                    "1",
                    Format::Auto
                )
                .await
                .is_err()
            );
        }
    }
    assert_eq!(
        Store::open(root.path())
            .unwrap()
            .mints(&profile.http, &genesis)
            .await
            .unwrap()
            .len(),
        4
    );
}
