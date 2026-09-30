use serde_json::json;
use solte::{
    app::{App, Modal, View},
    config::Config,
    demo, ui,
};
fn main() -> anyhow::Result<()> {
    let output = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("solte-confidential-previews"));
    std::fs::create_dir_all(&output)?;
    let mut app = App::new("/test".into(), Config::default(), vec![]);
    demo::populate(&mut app);
    app.tokens[2].account["data"]["parsed"]["info"]["extensions"] = json!([
        {"extension":"confidentialTransferAccount","state":{"elgamalPubkey":"CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=","approved":true,"allowConfidentialCredits":true,"allowNonConfidentialCredits":true,"pendingBalanceCreditCounter":2,"maximumPendingBalanceCreditCounter":65536}}
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
            &output.join(format!("solte-ct-list-{width}.svg")),
            width,
            height,
        )?;
        app.modal = Some(Modal::Token {
            account: Box::new(app.tokens[2].clone()),
            scroll: 0,
        });
        ui::snapshot(
            &app,
            &output.join(format!("solte-ct-inspector-{width}.svg")),
            width,
            height,
        )?;
        for selected in [0, 3, 5] {
            app.modal = Some(Modal::Confidential {
                account: Box::new(app.tokens[2].clone()),
                selected,
            });
            ui::snapshot(
                &app,
                &output.join(format!("solte-ct-menu-{width}-{selected}.svg")),
                width,
                height,
            )?;
        }
        for operation in [
            solte::confidential_operations::Operation::Configure,
            solte::confidential_operations::Operation::Transfer,
            solte::confidential_operations::Operation::Apply,
        ] {
            app.open_form(solte::app::FormKind::Confidential(operation));
            ui::snapshot(
                &app,
                &output.join(format!("solte-ct-form-{width}-{operation:?}.svg")),
                width,
                height,
            )?;
        }
        app.open_form(solte::app::FormKind::ConfidentialMint);
        ui::snapshot(
            &app,
            &output.join(format!("solte-ct-mint-{width}.svg")),
            width,
            height,
        )?;
    }
    Ok(())
}
