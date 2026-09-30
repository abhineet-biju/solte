use solte::{
    confidential_operations::{self as ct, Operation, Request},
    config::RpcProfile,
    mints::{self, ConfidentialMintConfig, CreateMint},
    network,
    operations::{self, OperationUpdate, PreparedTransfer},
    storage::Store,
    tokens,
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
            OperationUpdate::Failed(error) => panic!("{error}"),
            _ => {}
        }
    }
    assert!(confirmed);
}
async fn selected(profile: &RpcProfile, wallet: &Wallet) -> tokens::TokenAccount {
    tokens::fetch(&network::client(profile), &wallet.address.parse().unwrap())
        .await
        .unwrap()
        .accounts
        .remove(0)
}
fn request(operation: Operation, value: &str, recipient: &str) -> Request {
    Request {
        operation,
        value: value.into(),
        recipient: recipient.into(),
        recipient_is_account: false,
    }
}

#[tokio::test]
#[ignore = "requires an isolated Agave 4.3+ loopback validator with v1 and ZK proofs"]
async fn confidential_lifecycle_preserves_public_transfers_and_rejects_stale_reviews() {
    let http = std::env::var("SOLTE_TEST_RPC").expect("Set loopback RPC");
    let ws = std::env::var("SOLTE_TEST_WS").expect("Set loopback WebSocket");
    assert!(matches!(
        url::Url::parse(&http).unwrap().host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    let profile = RpcProfile::custom("Confidential test", &http, &ws).unwrap();
    let rpc = network::client(&profile);
    let root = tempfile::tempdir().unwrap();
    let alice = wallet::create(root.path(), "alice").unwrap();
    let bob = wallet::create(root.path(), "bob").unwrap();
    let wallets = vec![alice.clone(), bob.clone()];
    let store = Store::open(root.path()).unwrap();
    for wallet in &wallets {
        let signature = rpc
            .request_airdrop(&wallet.address.parse().unwrap(), 5_000_000_000)
            .await
            .unwrap();
        for _ in 0..50 {
            if rpc.confirm_transaction(&signature).await.unwrap_or(false) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    let mint = mints::prepare_create(
        &profile,
        &alice,
        &wallets,
        CreateMint {
            confidential: Some(ConfidentialMintConfig {
                auto_approve: false,
                auditor: Some(
                    solana_zk_sdk::encryption::elgamal::ElGamalKeypair::new_rand()
                        .pubkey()
                        .to_owned()
                        .into(),
                ),
            }),
            program: tokens::TOKEN_2022.parse().unwrap(),
            decimals: 6,
            authority: alice.address.parse().unwrap(),
            freeze: None,
            initial_supply: 100_000_000,
            format: Format::Legacy,
        },
    )
    .await
    .unwrap();
    let mint_address = mint.created_mint.as_ref().unwrap().address.clone();
    submit(mint, &store).await;
    submit(
        solte::token_operations::prepare_create(
            &profile,
            &alice,
            &mint_address,
            &bob.address,
            Format::Legacy,
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    for wallet in &wallets {
        let account = selected(&profile, wallet).await;
        assert!(solte::confidential::enabled(&account));
        submit(
            ct::prepare(
                &profile,
                wallet,
                &wallets,
                &account,
                request(Operation::Configure, "65536", ""),
            )
            .await
            .unwrap(),
            &store,
        )
        .await;
        let account = selected(&profile, wallet).await;
        assert!(
            ct::prepare(
                &profile,
                wallet,
                &wallets,
                &account,
                request(Operation::Configure, "65536", "")
            )
            .await
            .is_err()
        );
        let mut approve = request(Operation::Approve, "", &account.address);
        approve.recipient_is_account = true;
        submit(
            ct::prepare(&profile, &alice, &wallets, &account, approve)
                .await
                .unwrap(),
            &store,
        )
        .await;
    }
    let account = selected(&profile, &alice).await;
    let public = solte::token_operations::prepare_transfer(
        &profile,
        &alice,
        &account,
        &bob.address,
        "1",
        false,
        Format::Legacy,
    )
    .await
    .unwrap();
    submit(public, &store).await;
    submit(
        ct::prepare(
            &profile,
            &alice,
            &wallets,
            &selected(&profile, &alice).await,
            request(Operation::Deposit, "10", ""),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    let account = selected(&profile, &alice).await;
    let balances = ct::reveal(&profile, &alice, &account).await.unwrap();
    assert_eq!((balances.available, balances.pending), (0, 10_000_000));
    assert!(
        ct::prepare(
            &profile,
            &alice,
            &wallets,
            &account,
            request(Operation::Transfer, "1", &bob.address)
        )
        .await
        .is_err()
    );
    submit(
        ct::prepare(
            &profile,
            &alice,
            &wallets,
            &account,
            request(Operation::Apply, "", ""),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    let account = selected(&profile, &alice).await;
    let stale = ct::prepare(
        &profile,
        &alice,
        &wallets,
        &account,
        request(Operation::Transfer, "2", &bob.address),
    )
    .await
    .unwrap();
    assert!(
        stale
            .transaction
            .signatures
            .iter()
            .all(|s| *s == solana_signature::Signature::default())
    );
    submit(
        ct::prepare(
            &profile,
            &alice,
            &wallets,
            &account,
            request(Operation::Deposit, "1", ""),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    let (sender, mut receiver) = mpsc::channel(32);
    operations::submit(stale, 1, store.clone(), sender).await;
    let mut rejected = false;
    while let Ok(event) = receiver.try_recv() {
        match event.update {
            OperationUpdate::Failed(message) => {
                assert!(message.contains("changed"), "{message}");
                rejected = true;
            }
            OperationUpdate::Submitted(_) => panic!("stale transaction submitted"),
            _ => {}
        }
    }
    assert!(rejected);
    submit(
        ct::prepare(
            &profile,
            &alice,
            &wallets,
            &selected(&profile, &alice).await,
            request(Operation::Transfer, "2", &bob.address),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    let account = selected(&profile, &bob).await;
    let balances = ct::reveal(&profile, &bob, &account).await.unwrap();
    assert_eq!((balances.available, balances.pending), (0, 2_000_000));
    submit(
        ct::prepare(
            &profile,
            &bob,
            &wallets,
            &account,
            request(Operation::Apply, "", ""),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    submit(
        ct::prepare(
            &profile,
            &bob,
            &wallets,
            &selected(&profile, &bob).await,
            request(Operation::Withdraw, "1", ""),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    let account = selected(&profile, &bob).await;
    assert_eq!(account.amount, 2_000_000);
    let balances = ct::reveal(&profile, &bob, &account).await.unwrap();
    assert_eq!((balances.available, balances.pending), (1_000_000, 0));
    assert!(ct::reveal(&profile, &alice, &account).await.is_err());
    assert!(
        !std::fs::read_to_string(root.path().join(".solte/diagnostics.log"))
            .unwrap_or_default()
            .contains("solana-conf-bal/v1")
    );
}

#[tokio::test]
#[ignore = "requires an isolated Agave 4.3+ loopback validator with v1 and ZK proofs"]
async fn automatic_approval_and_transfers_without_an_auditor_work() {
    let http = std::env::var("SOLTE_TEST_RPC").expect("Set loopback RPC");
    let ws = std::env::var("SOLTE_TEST_WS").expect("Set loopback WebSocket");
    assert!(matches!(
        url::Url::parse(&http).unwrap().host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    let profile = RpcProfile::custom("Automatic confidential test", &http, &ws).unwrap();
    let rpc = network::client(&profile);
    let root = tempfile::tempdir().unwrap();
    let alice = wallet::create(root.path(), "alice").unwrap();
    let bob = wallet::create(root.path(), "bob").unwrap();
    let wallets = vec![alice.clone(), bob.clone()];
    let store = Store::open(root.path()).unwrap();
    for wallet in &wallets {
        let signature = rpc
            .request_airdrop(&wallet.address.parse().unwrap(), 5_000_000_000)
            .await
            .unwrap();
        for _ in 0..50 {
            if rpc.confirm_transaction(&signature).await.unwrap_or(false) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    let mint = mints::prepare_create(
        &profile,
        &alice,
        &wallets,
        CreateMint {
            confidential: Some(ConfidentialMintConfig {
                auto_approve: true,
                auditor: None,
            }),
            program: tokens::TOKEN_2022.parse().unwrap(),
            decimals: 6,
            authority: alice.address.parse().unwrap(),
            freeze: None,
            initial_supply: 10_000_000,
            format: Format::Legacy,
        },
    )
    .await
    .unwrap();
    let address = mint.created_mint.as_ref().unwrap().address.clone();
    submit(mint, &store).await;
    submit(
        solte::token_operations::prepare_create(
            &profile,
            &alice,
            &address,
            &bob.address,
            Format::Legacy,
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    for wallet in &wallets {
        submit(
            ct::prepare(
                &profile,
                wallet,
                &wallets,
                &selected(&profile, wallet).await,
                request(Operation::Configure, "65536", ""),
            )
            .await
            .unwrap(),
            &store,
        )
        .await;
    }
    submit(
        ct::prepare(
            &profile,
            &alice,
            &wallets,
            &selected(&profile, &alice).await,
            request(Operation::Deposit, "3", ""),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    submit(
        ct::prepare(
            &profile,
            &alice,
            &wallets,
            &selected(&profile, &alice).await,
            request(Operation::Apply, "", ""),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    submit(
        ct::prepare(
            &profile,
            &alice,
            &wallets,
            &selected(&profile, &alice).await,
            request(Operation::Transfer, "1", &bob.address),
        )
        .await
        .unwrap(),
        &store,
    )
    .await;
    let balances = ct::reveal(&profile, &bob, &selected(&profile, &bob).await)
        .await
        .unwrap();
    assert_eq!((balances.available, balances.pending), (0, 1_000_000));
}
