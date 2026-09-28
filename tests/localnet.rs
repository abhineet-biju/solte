use std::time::Duration;

use solte::{
    config::RpcProfile,
    network::{self, Update},
    operations::{self, OperationUpdate},
    storage::Store,
    wallet,
};
use tokio::sync::mpsc;

#[tokio::test]
#[ignore = "requires a local validator; set SOLTE_TEST_RPC and SOLTE_TEST_WS"]
async fn funds_simulates_transfers_and_retains_inspectable_history() {
    let http = std::env::var("SOLTE_TEST_RPC").expect("Set SOLTE_TEST_RPC to a loopback validator");
    let ws = std::env::var("SOLTE_TEST_WS").expect("Set SOLTE_TEST_WS to its WebSocket endpoint");
    let host = url::Url::parse(&http).unwrap();
    assert!(matches!(
        host.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    let profile = RpcProfile::custom("Local test", &http, &ws).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let payer = wallet::create(temp.path(), "payer").unwrap();
    let recipient = wallet::create(temp.path(), "recipient").unwrap();
    let store = Store::open(temp.path()).unwrap();
    let (sender, mut receiver) = mpsc::channel(32);
    operations::fund(
        profile.clone(),
        payer.clone(),
        2_000_000_000,
        1,
        store.clone(),
        sender.clone(),
    )
    .await;
    let mut funded = false;
    while let Ok(event) = receiver.try_recv() {
        match event.update {
            OperationUpdate::Finished(_) => funded = true,
            OperationUpdate::Failed(message) => panic!("Funding failed: {message}"),
            _ => {}
        }
    }
    assert!(funded);
    let prepared = operations::prepare(&profile, &payer, &recipient.address, 100_000_000)
        .await
        .unwrap();
    assert!(
        prepared.simulation_error.is_none(),
        "{:?}",
        prepared.simulation_error
    );
    assert_eq!(prepared.fee, 5000);
    operations::submit(prepared, 1, store.clone(), sender).await;
    let mut signature = None;
    while let Ok(event) = receiver.try_recv() {
        match event.update {
            OperationUpdate::Finished(sig) => signature = Some(sig),
            OperationUpdate::Failed(message) => panic!("Transfer failed: {message}"),
            _ => {}
        }
    }
    let signature = signature.expect("Transfer should confirm");
    let rpc = network::client(&profile);
    assert_eq!(
        rpc.get_balance(&recipient.address.parse().unwrap())
            .await
            .unwrap(),
        100_000_000
    );
    let detail = network::fetch_detail(&rpc, &signature)
        .await
        .unwrap()
        .unwrap();
    assert!(detail.error.is_none());
    assert_eq!(detail.balance_change(&payer.address), Some(-100_005_000));
    assert!(
        detail
            .log_lines()
            .iter()
            .any(|line| line.contains("success"))
    );
    let insufficient = operations::prepare(&profile, &payer, &recipient.address, 100_000_000_000)
        .await
        .unwrap();
    assert!(insufficient.simulation_error.is_some());
    let (events, mut updates) = mpsc::channel(128);
    let monitor = network::start(
        9,
        Some(payer.address.clone()),
        profile.clone(),
        store.clone(),
        events,
        false,
    );
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let event = updates.recv().await.unwrap();
            if let Update::Records(records) = event.update
                && records
                    .iter()
                    .any(|r| r.signature == signature && r.fee().is_some())
            {
                break;
            }
        }
    })
    .await
    .expect("Monitor should recover the transfer history");
    drop(monitor);
    let scope = solte::storage::scope(&profile.http, &payer.address);
    assert!(
        store
            .load(&scope)
            .await
            .unwrap()
            .iter()
            .any(|r| r.signature == signature)
    );
    assert!(
        store
            .logs(&scope)
            .await
            .unwrap()
            .iter()
            .any(|r| r.message.contains("Confirmed"))
    );
}

#[tokio::test]
#[ignore = "requires an Agave 4.3+ local validator with v1 enabled"]
async fn all_transaction_formats_and_import_export_work_on_chain() {
    use solte::transaction::{self, Format};
    let http = std::env::var("SOLTE_TEST_RPC").unwrap();
    let ws = std::env::var("SOLTE_TEST_WS").unwrap();
    assert!(matches!(
        url::Url::parse(&http).unwrap().host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    let profile = RpcProfile::custom("Version tests", &http, &ws).unwrap();
    let root = tempfile::tempdir().unwrap();
    let payer = wallet::create(root.path(), "payer").unwrap();
    let recipient = wallet::create(root.path(), "recipient").unwrap();
    let store = Store::open(root.path()).unwrap();
    let (sender, mut receiver) = mpsc::channel(32);
    operations::fund(
        profile.clone(),
        payer.clone(),
        2_000_000_000,
        1,
        store.clone(),
        sender.clone(),
    )
    .await;
    while let Ok(event) = receiver.try_recv() {
        if let OperationUpdate::Failed(error) = event.update {
            panic!("{error}");
        }
    }
    let rpc = network::client(&profile);
    for (index, format) in [Format::Legacy, Format::V0, Format::V1]
        .into_iter()
        .enumerate()
    {
        let prepared =
            operations::prepare_format(&profile, &payer, &recipient.address, 100_000_000, format)
                .await
                .unwrap();
        assert!(
            prepared.simulation_error.is_none(),
            "{format:?}: {:?}",
            prepared.simulation_error
        );
        let label = transaction::label(&prepared.transaction);
        operations::submit(prepared, 1, store.clone(), sender.clone()).await;
        let mut signature = None;
        while let Ok(event) = receiver.try_recv() {
            match event.update {
                OperationUpdate::Finished(sig) => signature = Some(sig),
                OperationUpdate::Failed(error) => panic!("{format:?}: {error}"),
                _ => {}
            }
        }
        let detail = network::fetch_detail(&rpc, &signature.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.version_label(), label);
        assert_eq!(
            rpc.get_balance(&recipient.address.parse().unwrap())
                .await
                .unwrap(),
            (index as u64 + 1) * 100_000_000
        );
    }
    let prepared =
        operations::prepare_format(&profile, &payer, &recipient.address, 1_000_000, Format::V1)
            .await
            .unwrap();
    let source = root.path().join("unsigned.base64");
    let destination = root.path().join("signed.base64");
    let original = prepared.transaction.message.serialize();
    transaction::export(&prepared.transaction, &source).unwrap();
    let imported = operations::prepare_import(&profile, &payer, &source, Some(destination.clone()))
        .await
        .unwrap();
    operations::sign_export(imported, 1, sender.clone()).await;
    let event = receiver.recv().await.unwrap();
    assert!(
        matches!(event.update, OperationUpdate::Exported(_)),
        "Expected export"
    );
    let signed = transaction::read(&destination).unwrap();
    assert_eq!(signed.message.serialize(), original);
    signed.verify_and_hash_message().unwrap();
    assert_eq!(
        rpc.get_balance(&recipient.address.parse().unwrap())
            .await
            .unwrap(),
        300_000_000,
        "Export must not submit"
    );
    let imported = operations::prepare_import(&profile, &payer, &destination, None)
        .await
        .unwrap();
    operations::submit(imported, 1, store.clone(), sender.clone()).await;
    let mut submitted = false;
    while let Ok(event) = receiver.try_recv() {
        match event.update {
            OperationUpdate::Finished(_) => submitted = true,
            OperationUpdate::Failed(error) => panic!("Import submit: {error}"),
            _ => {}
        }
    }
    assert!(submitted);
    assert_eq!(
        rpc.get_balance(&recipient.address.parse().unwrap())
            .await
            .unwrap(),
        301_000_000
    );
}

#[tokio::test]
#[ignore = "requires a local validator with address lookup table support"]
async fn imported_v0_resolves_lookup_tables_before_signing() {
    use solana_message::{AddressLookupTableAccount, VersionedMessage, v0};
    use solana_signature::Signature;
    use solana_transaction::{Transaction, versioned::VersionedTransaction};
    use solte::transaction;
    let http = std::env::var("SOLTE_TEST_RPC").unwrap();
    assert!(matches!(
        url::Url::parse(&http).unwrap().host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    let profile = RpcProfile::custom(
        "Lookup test",
        &http,
        &std::env::var("SOLTE_TEST_WS").unwrap(),
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let payer = wallet::create(root.path(), "payer").unwrap();
    let recipient = wallet::create(root.path(), "recipient").unwrap();
    let payer_key = payer.address.parse().unwrap();
    let recipient_key = recipient.address.parse().unwrap();
    let store = Store::open(root.path()).unwrap();
    let (sender, mut receiver) = mpsc::channel(32);
    operations::fund(
        profile.clone(),
        payer.clone(),
        2_000_000_000,
        1,
        store.clone(),
        sender.clone(),
    )
    .await;
    while let Ok(event) = receiver.try_recv() {
        if let OperationUpdate::Failed(error) = event.update {
            panic!("{error}");
        }
    }
    let rpc = network::client(&profile);
    let slot = rpc
        .get_slot_with_commitment(solana_commitment_config::CommitmentConfig::finalized())
        .await
        .unwrap();
    let (create, table_key) =
        solana_address_lookup_table_interface::instruction::create_lookup_table(
            payer_key, payer_key, slot,
        );
    let extend = solana_address_lookup_table_interface::instruction::extend_lookup_table(
        table_key,
        payer_key,
        Some(payer_key),
        vec![recipient_key],
    );
    let hash = rpc.get_latest_blockhash().await.unwrap();
    let setup = Transaction::new_signed_with_payer(
        &[create, extend],
        Some(&payer_key),
        &[&payer.signer().unwrap()],
        hash,
    );
    rpc.send_and_confirm_transaction(&setup).await.unwrap();
    let extended_slot = rpc.get_slot().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while rpc.get_slot().await.unwrap() <= extended_slot {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap();
    let message = v0::Message::try_compile(
        &payer_key,
        &[solana_system_interface::instruction::transfer(
            &payer_key,
            &recipient_key,
            100_000_000,
        )],
        &[AddressLookupTableAccount {
            key: table_key,
            addresses: vec![recipient_key],
        }],
        rpc.get_latest_blockhash().await.unwrap(),
    )
    .unwrap();
    assert_eq!(message.address_table_lookups.len(), 1);
    let tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::V0(message),
    };
    let source = root.path().join("lookup.base64");
    transaction::export(&tx, &source).unwrap();
    let prepared = operations::prepare_import(&profile, &payer, &source, None)
        .await
        .unwrap();
    assert!(prepared.accounts.contains(&recipient_key));
    assert!(prepared.simulation_error.is_none());
    operations::submit(prepared, 1, store, sender).await;
    let mut signature = None;
    while let Ok(event) = receiver.try_recv() {
        match event.update {
            OperationUpdate::Finished(sig) => signature = Some(sig),
            OperationUpdate::Failed(error) => panic!("{error}"),
            _ => {}
        }
    }
    let record = network::fetch_detail(&rpc, &signature.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.version_label(), "v0");
    assert_eq!(record.balance_change(&recipient.address), Some(100_000_000));
}
