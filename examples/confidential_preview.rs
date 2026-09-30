use serde_json::json;
use solte::{
    app::{App, Modal, View},
    config::Config,
    demo, ui,
};
fn main() -> anyhow::Result<()> {
    let mut app = App::new("/test".into(), Config::default(), vec![]);
    demo::populate(&mut app);
    app.tokens[2].account["data"]["parsed"]["info"]["extensions"] = json!([
        {"extension":"confidentialTransferAccount","state":{"approved":true,"allowConfidentialCredits":true,"allowNonConfidentialCredits":true,"pendingBalanceCreditCounter":2,"maximumPendingBalanceCreditCounter":65536}}
    ]);
    app.tokens[2].mint_info.as_mut().unwrap()["extensions"] = json!([
        {"extension":"confidentialTransferMint","state":{"authority":app.tokens[2].authority,"autoApproveNewAccounts":true,"auditorElgamalPubkey":null}}
    ]);
    app.switch_view(View::Tokens);
    app.token_cursor = 2;
    for (width, height) in [(60, 10), (80, 20), (100, 28), (160, 48)] {
        app.modal = None;
        ui::snapshot(
            &app,
            &std::path::PathBuf::from(format!("/private/tmp/solte-ct-list-{width}.svg")),
            width,
            height,
        )?;
        app.modal = Some(Modal::Token {
            account: Box::new(app.tokens[2].clone()),
            scroll: 0,
        });
        ui::snapshot(
            &app,
            &std::path::PathBuf::from(format!("/private/tmp/solte-ct-inspector-{width}.svg")),
            width,
            height,
        )?;
    }
    Ok(())
}
