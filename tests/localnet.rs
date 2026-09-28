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
