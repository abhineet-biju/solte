use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, EventStream, KeyEventKind, MouseButton, MouseEventKind,
    },
    execute,
};
use futures_util::StreamExt;
use tokio::{sync::mpsc, task::JoinSet};

use crate::{
    amount::parse_sol,
    app::{Action, App, Appearance, FormKind, Modal, Pane, RefreshState},
    config::{RpcProfile, WalletRef, expand_path},
    funding::{self, Faucet},
    network::{self, Command, Monitor, Update},
    operations::{self, OperationEvent, OperationUpdate},
    storage::Store,
    ui::Ui,
    wallet::{self, Wallet},
};

enum LocalEvent {
    Tokens {
        session: u64,
        result: Result<crate::tokens::Snapshot>,
    },
    Wallet(Result<Wallet>),
    Notice(Result<String>),
}

struct Services {
    store: Store,
    network_sender: mpsc::Sender<network::Event>,
    operation_sender: mpsc::Sender<OperationEvent>,
    local_sender: mpsc::Sender<LocalEvent>,
    monitor: Option<Monitor>,
    jobs: JoinSet<()>,
    offline: bool,
}

impl Services {
    fn restart(&mut self, app: &mut App) {
        self.monitor = None;
        if app.session > 0 {
            app.logs.clear();
        }
        app.resume_logs();
        app.session += 1;
        app.records.clear();
        app.tokens.clear();
        app.token_cursor = 0;
        app.token_loading = false;
        app.token_started = None;
        app.token_updated = None;
        app.token_genesis = None;
        app.token_error = None;
        app.token_export = None;
        app.token_operation = None;
        app.token_warnings.clear();
        app.history_loading = false;
        app.refresh = RefreshState::Idle;
        app.transaction_cursor = 0;
        app.balance = None;
        app.network = None;
        app.connected = false;
        app.subscribed = false;
        app.last_update = None;
        app.last_signature = None;
        app.status = format!("Connecting to {}…", app.profile().name);
        app.log(
            "INFO",
            format!(
                "Monitoring {} on {}",
                app.wallet().map(|w| w.name.as_str()).unwrap_or("network"),
                app.profile().name
            ),
        );
        self.monitor = Some(network::start(
            app.session,
            app.wallet().map(|w| w.address.clone()),
            app.profile().clone(),
            self.store.clone(),
            self.network_sender.clone(),
            self.offline,
        ));
    }
    fn load_tokens(&mut self, app: &mut App) {
        if app.demo || app.token_loading {
            return;
        }
        if self.offline {
            app.token_error = Some("Token accounts are unavailable offline".into());
            return;
        }
        let Some(wallet) = app.wallet() else {
            return;
        };
        let owner = wallet.address.clone();
        let profile = app.profile().clone();
        let session = app.session;
        let sender = self.local_sender.clone();
        app.token_loading = true;
        app.token_started = Some(Instant::now());
        app.token_error = None;
        self.jobs.spawn(async move {
            let result =
                async { crate::tokens::fetch(&network::client(&profile), &owner.parse()?).await }
                    .await
                    .map_err(|error| anyhow::anyhow!(network::safe_error(error, &profile)));
            let _ = sender.send(LocalEvent::Tokens { session, result }).await;
        });
    }
    fn command(&self, command: Command) {
        if let Some(monitor) = &self.monitor {
            let _ = monitor.commands.try_send(command);
        }
    }
    async fn save(&self, app: &mut App) {
        if !app.demo
            && let Err(error) = self.store.config(&app.root, app.config.clone()).await
        {
            app.log("ERROR", format!("Cannot save preferences: {error}"));
        }
    }
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            crossterm::event::DisableFocusChange
        );
        ratatui::restore();
    }
}

pub async fn run(mut app: App, offline: bool) -> Result<()> {
    app.offline = offline;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(
            "Solte needs an interactive terminal. Use --check for an RPC diagnostic or --snapshot for a render."
        );
    }
    let temporary = if app.demo {
        Some(tempfile::tempdir()?)
    } else {
        None
    };
    let store = Store::open(temporary.as_ref().map(|d| d.path()).unwrap_or(&app.root))?;
    let diagnostics_dir = temporary
        .as_ref()
        .map(|d| d.path())
        .unwrap_or(&app.root)
        .join(".solte");
    let (writer, _logging_guard) = tracing_appender::non_blocking(
        tracing_appender::rolling::never(diagnostics_dir, "diagnostics.log"),
    );
    let _ = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter("solte=info")
        .try_init();
    let (network_sender, mut network_receiver) = mpsc::channel(128);
    let (operation_sender, mut operation_receiver) = mpsc::channel(32);
    let (local_sender, mut local_receiver) = mpsc::channel(32);
    let mut services = Services {
        store,
        network_sender,
        operation_sender,
        local_sender,
        monitor: None,
        jobs: JoinSet::new(),
        offline,
    };
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    execute!(
        io::stdout(),
        EnableMouseCapture,
        EnableBracketedPaste,
        crossterm::event::EnableFocusChange
    )?;
    let mut events = EventStream::new();
    let mut ui = Ui::default();
    if !app.demo {
        services.restart(&mut app);
    }
    ui.animate(&app);
    let mut animation_tick = tokio::time::interval(Duration::from_millis(33));
    animation_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_tick = tokio::time::interval(Duration::from_secs(1));
    terminal.draw(|frame| ui.draw(frame, &app))?;
    ui.sync_scroll(&mut app);
    loop {
        let mut redraw = true;
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else { break; };
                let action = match event? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                    Event::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => ui.click(&mut app, mouse.column, mouse.row),
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                            if app.modal.is_none() && let Some(action) = ui.hits.iter().find(|h| h.area.contains((mouse.column, mouse.row).into()) && matches!(h.action, Action::Focus(_))).map(|h| h.action.clone()) { app.navigate(&action); }
                            Some(Action::Scroll(if mouse.kind == MouseEventKind::ScrollUp { -3 } else { 3 }))
                        },
                        _ => { redraw = false; None },
                    },
                    Event::Paste(text) => { app.paste(&text); None },
                    Event::Resize(width, height) => { terminal.resize(ratatui::layout::Rect::new(0, 0, width, height))?; None },
                    Event::FocusGained => { services.command(Command::Refresh); None },
                    _ => { redraw = false; None },
                };
                let action = match action {
                    Some(Action::CycleFocus(forward)) => { ui.cycle_region(&mut app, forward); None },
                    Some(Action::Navigate(direction)) => { ui.navigate_control(&mut app, direction); None },
                    Some(Action::Activate) if app.modal.is_none() => ui.focused_action(&app).or(Some(Action::Activate)),
                    other => other,
                };
                if let Some(action) = action {
                    let animate = matches!(action, Action::Focus(_) | Action::CycleFocus(_) | Action::Selector(_) | Action::Theme | Action::New | Action::Import | Action::Inspect | Action::SelectTransaction(_));
                    match handle(&mut app, &mut services, action).await {
                        Ok(true) => break,
                        Ok(false) => {},
                        Err(error) => report_error(&mut app, error.to_string()),
                    }
                    if animate { ui.animate(&app); }
                }
            },
            Some(event) = network_receiver.recv() => {
                if event.session != app.session { continue; }
                match event.update {
                    Update::RefreshFinished(result) => app.finish_refresh(result),
                    Update::Records(records) => app.replace_records(records),
                    Update::HistoryFinished(result) => {
                        app.history_loading = false;
                        let (level, message) = match result {
                            Ok(0) => ("INFO", "No additional history available from this source".into()),
                            Ok(count) => ("INFO", format!("Loaded {count} additional transactions · scroll down to view")),
                            Err(message) => ("WARN", format!("Older history failed: {message}")),
                        };
                        app.status = message.clone();
                        app.log(level, message);
                    },
                    Update::Network(network, balance) => {
                        if app.token_genesis.as_ref().is_some_and(|genesis| *genesis != network.genesis) {
                            app.tokens.clear(); app.token_cursor = 0; app.token_updated = None;
                            app.token_genesis = None; app.token_error = None; app.token_warnings.clear();
                        }
                        if !matches!(app.refresh, RefreshState::Pending(_)) {
                            app.status = format!("{} · confirmed activity · captured history stored locally", network.cluster);
                        }
                        app.network = Some(network); app.balance = balance; app.connected = true; app.last_update = Some(Instant::now());
                    },
                    Update::Offline(message) => { app.connected = false; app.status = message; },
                    Update::Subscription(connected) => app.subscribed = connected,
                    Update::Log(entry) => app.push_log(entry),
                }
            },
            Some(event) = operation_receiver.recv() => {
                if event.session != app.session { continue; }
                match event.update {
                    OperationUpdate::Prepared(prepared) => { app.busy = None; app.modal = Some(Modal::Review { prepared, scroll: 0 }); ui.animate(&app); },
                    OperationUpdate::Submitted(signature) => { app.last_signature = Some(signature.clone()); app.busy = Some("Waiting for transaction confirmation…".into()); app.log("INFO", format!("Submitted {signature}")); services.command(Command::Refresh); },
                    OperationUpdate::Finished(signature) => { if app.view == crate::app::View::Tokens { services.load_tokens(&mut app); } app.busy = None; app.status = "Transaction confirmed · press [o] to open explorer".into(); app.last_signature = Some(signature.clone()); app.log("INFO", format!("Confirmed {signature}")); services.command(Command::Refresh); },
                    OperationUpdate::Exported(path) => { app.busy = None; app.status = format!("Signed transaction exported to {path} · not submitted"); app.log("INFO", app.status.clone()); },
                    OperationUpdate::Failed(message) => { app.busy = None; report_error(&mut app, message); services.command(Command::Refresh); },
                    OperationUpdate::FundingFailed(message) => {
                        app.busy = None;
                        report_error(&mut app, message.clone());
                        app.modal = Some(Modal::Funding { selected: 0, reason: Some(message) });
                        services.command(Command::Refresh);
                    },
                }
            },
            Some(event) = local_receiver.recv() => match event {
                LocalEvent::Tokens { session, result } => apply_token_update(&mut app, session, result),
                LocalEvent::Wallet(result) => {
                    app.busy = None;
                    match result {
                        Ok(wallet) => {
                            let name = wallet.name.clone();
                            if let Some(index) = app.wallets.iter().position(|w| w.address == wallet.address) { app.selected_wallet = index; }
                            else { app.config.wallets.push(WalletRef { name: wallet.name.clone(), path: wallet.path.clone() }); app.wallets.push(wallet); app.selected_wallet = app.wallets.len() - 1; }
                            app.wallet_cursor = app.selected_wallet;
                            app.config.selected_wallet = app.wallet().map(|w| w.path.clone());
                            app.modal = None;
                            services.save(&mut app).await;
                            services.restart(&mut app);
                            app.log("INFO", format!("Selected {name}"));
                        },
                        Err(error) => report_error(&mut app, error.to_string()),
                    }
                },
                LocalEvent::Notice(result) => match result { Ok(message) => app.status = message, Err(error) => report_error(&mut app, error.to_string()) },
            },
            Some(result) = services.jobs.join_next(), if !services.jobs.is_empty() => {
                if let Err(error) = result { app.busy = None; report_error(&mut app, format!("Background task stopped: {error}")); }
            },
            _ = animation_tick.tick(), if ui.animating() || (!app.config.reduced_motion && (matches!(app.refresh, RefreshState::Pending(_)) || (app.view == crate::app::View::Tokens && app.token_loading))) => {},
            _ = status_tick.tick() => {},
        }
        if app.view == crate::app::View::Tokens
            && !app.demo
            && !app.token_loading
            && app.token_error.is_none()
            && app.wallet().is_some()
            && app
                .token_updated
                .is_none_or(|at| at.elapsed().as_secs() >= 15)
        {
            services.load_tokens(&mut app);
        }
        if redraw {
            terminal.draw(|frame| ui.draw(frame, &app))?;
            ui.sync_scroll(&mut app);
        }
    }
    services.monitor = None;
    services.jobs.abort_all();
    Ok(())
}

fn apply_token_update(app: &mut App, session: u64, result: Result<crate::tokens::Snapshot>) {
    if session != app.session {
        return;
    }
    app.token_loading = false;
    match result {
        Ok(snapshot) => {
            if app
                .network
                .as_ref()
                .is_some_and(|network| network.genesis != snapshot.genesis)
            {
                app.token_error = Some("Network changed; refresh token accounts again".into());
                return;
            }
            let selected = app.selected_token().map(|a| a.address.clone());
            app.tokens = snapshot.accounts;
            app.token_genesis = Some(snapshot.genesis);
            app.token_warnings = snapshot.warnings;
            app.token_updated = Some(Instant::now());
            app.token_error = None;
            app.token_cursor = app
                .visible_tokens()
                .iter()
                .position(|a| Some(&a.address) == selected.as_ref())
                .unwrap_or(0);
            if matches!(app.focused_control, Some(Action::SelectToken(_))) {
                app.focused_control = Some(Action::SelectToken(app.token_cursor));
            }
            app.status = format!(
                "Token accounts refreshed · {} accounts{}",
                app.tokens.len(),
                if app.token_warnings.is_empty() {
                    ""
                } else {
                    " · partial coverage"
                }
            );
        }
        Err(error) => {
            app.token_error = Some(error.to_string());
            app.status = format!("Token refresh failed: {error}");
        }
    }
}

fn report_error(app: &mut App, message: String) {
    app.status = message.clone();
    app.log("ERROR", &message);
    if let Some(Modal::Form(form)) = &mut app.modal {
        form.error = Some(message);
    }
}

async fn handle(app: &mut App, services: &mut Services, mut action: Action) -> Result<bool> {
    if action == Action::ForceQuit {
        return Ok(true);
    }
    if action == Action::Quit {
        if app.busy.is_some() {
            app.status = "An operation is pending. [Ctrl-C] exits immediately; an already submitted transaction can still land.".into();
            return Ok(false);
        }
        return Ok(true);
    }
    if let Action::SelectTransaction(index) = action {
        app.transaction_cursor = index;
        app.pane = Pane::Wallet;
        action = Action::Inspect;
    }
    if action == Action::Activate {
        action = match app.pane {
            Pane::Tokens => Action::SelectToken(app.token_cursor),
            Pane::Wallets => Action::SelectWallet(app.wallet_cursor),
            Pane::Wallet => Action::Inspect,
            Pane::Network => Action::Profiles,
            Pane::Logs => Action::Follow,
        };
    }
    if action == Action::Submit
        && let Some(Modal::Funding { selected, .. }) = &app.modal
    {
        action = match selected {
            0 => Action::BrowserFaucet(Faucet::Solana),
            1 => Action::BrowserFaucet(Faucet::Quicknode),
            _ => Action::RpcAirdrop,
        };
    }
    if action == Action::Submit
        && let Some(Modal::Appearance { kind, selected }) = &app.modal
    {
        action = Action::SelectAppearance(*kind, *selected);
    }
    if app.navigate(&action) {
        return Ok(false);
    }
    if app.busy.is_some()
        && matches!(
            action,
            Action::SelectWallet(_)
                | Action::ActivateWallet(_)
                | Action::SelectProfile(_)
                | Action::NextWallet
                | Action::New
                | Action::Import
                | Action::Fund
                | Action::RpcAirdrop
                | Action::BrowserFaucet(_)
                | Action::Send
                | Action::CreateTokenAccount
                | Action::ImportTransaction
                | Action::Submit
        )
    {
        bail!("Wait for the current operation to finish");
    }
    match action {
        Action::Send
            if app.view == crate::app::View::Tokens
                || matches!(app.modal, Some(Modal::Token { .. })) =>
        {
            let account = if let Some(Modal::Token { account, .. }) = &app.modal {
                account.as_ref()
            } else {
                app.selected_token()
                    .context("Select a token account first")?
            }
            .clone();
            anyhow::ensure!(
                account.state == "initialized",
                "Source account is frozen or uninitialized"
            );
            crate::token_operations::ensure_supported(account.info())?;
            crate::token_operations::ensure_supported(
                account
                    .mint_info
                    .as_ref()
                    .context("Refresh mint details before transferring")?,
            )?;
            anyhow::ensure!(
                !app.wallet()
                    .context("Select a signing wallet first")?
                    .program,
                "Program identities are read-only"
            );
            app.token_operation = Some(account);
            app.open_form(FormKind::TokenTransfer);
        }
        Action::CreateTokenAccount => {
            anyhow::ensure!(
                !app.wallet()
                    .context("Select a signing wallet first")?
                    .program,
                "Program identities are read-only"
            );
            let owner = app
                .wallet()
                .context("Select a signing wallet first")?
                .address
                .clone();
            let mint = if let Some(Modal::Token { account, .. }) = &app.modal {
                Some(account.mint.clone())
            } else {
                app.selected_token().map(|a| a.mint.clone())
            };
            app.open_form(FormKind::TokenCreate);
            if let Some(Modal::Form(form)) = &mut app.modal {
                form.fields[1].insert(&owner);
                if let Some(mint) = mint {
                    form.fields[0].insert(&mint);
                }
            }
        }
        Action::SelectToken(index) => {
            app.token_cursor = index;
            if let Some(account) = app.selected_token() {
                app.modal = Some(Modal::Token {
                    account: Box::new(account.clone()),
                    scroll: 0,
                });
            }
        }
        Action::CopyToken(mint) | Action::ExplorerToken(mint) => {
            let account = if let Some(Modal::Token { account, .. }) = &app.modal {
                Some(account.as_ref())
            } else {
                app.selected_token()
            }
            .context("Select a token account first")?;
            let address = if mint {
                &account.mint
            } else {
                &account.address
            }
            .clone();
            if matches!(action, Action::CopyToken(_)) {
                copy(address.clone())?;
                app.status = if mint {
                    "Mint address copied"
                } else {
                    "Token account address copied"
                }
                .into();
            } else {
                let url = network::explorer_url(
                    app.profile(),
                    app.token_genesis.as_deref(),
                    "address",
                    &address,
                )?;
                let sender = services.local_sender.clone();
                app.status = "Opening Solana Explorer…".into();
                services.jobs.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || open::that(url))
                        .await
                        .map_err(anyhow::Error::from)
                        .and_then(|result| result.map_err(anyhow::Error::from))
                        .map(|_| "Opened Solana Explorer".into());
                    let _ = sender.send(LocalEvent::Notice(result)).await;
                });
            }
        }
        Action::ExportToken => {
            app.token_export = Some(
                if let Some(Modal::Token { account, .. }) = &app.modal {
                    account.as_ref()
                } else {
                    app.selected_token()
                        .context("Select a token account first")?
                }
                .clone(),
            );
            app.open_form(FormKind::TokenExport);
        }
        Action::New => app.open_form(FormKind::New),
        Action::Import => app.open_form(FormKind::Import),
        Action::ImportTransaction => {
            if services.offline {
                bail!("Online simulation is required before signing imported transactions");
            }
            app.wallet()
                .context("Create or import a signing wallet first")?;
            app.open_form(FormKind::TransactionImport);
        }
        Action::Fund | Action::Send => {
            if services.offline {
                bail!("Offline: restart without --offline.");
            }
            let wallet = app.wallet().context("Create or import a wallet first")?;
            if wallet.program && action == Action::Send {
                bail!("Program identities are read-only");
            }
            if action == Action::Fund
                && funding::is_devnet(
                    app.profile(),
                    app.network.as_ref().map(|n| n.genesis.as_str()),
                )
            {
                app.modal = Some(Modal::Funding {
                    selected: 0,
                    reason: None,
                });
            } else {
                app.open_form(if action == Action::Fund {
                    FormKind::Fund
                } else {
                    FormKind::Transfer
                });
            }
        }
        Action::RpcAirdrop => {
            app.wallet().context("No wallet selected")?;
            app.open_form(FormKind::Fund);
        }
        Action::BrowserFaucet(faucet) => open_faucet(app, services, faucet)?,
        Action::Profiles => {
            app.modal = Some(Modal::Profiles {
                selected: app.config.selected_profile,
            })
        }
        Action::AddProfile => app.open_form(FormKind::Profile),
        Action::Search => {
            let tokens = app.view == crate::app::View::Tokens;
            let logs = app.view == crate::app::View::Logs;
            if !logs && !tokens {
                app.switch_view(crate::app::View::Activity);
            }
            let value = if tokens {
                app.token_filter.clone()
            } else if logs {
                app.log_filter.clone()
            } else {
                app.filter.clone()
            };
            app.open_form(if tokens {
                FormKind::TokenSearch
            } else if logs {
                FormKind::LogSearch
            } else {
                FormKind::Search
            });
            if let Some(Modal::Form(form)) = &mut app.modal {
                form.fields[0].insert(&value);
            }
        }
        Action::SelectWallet(index) if app.view == crate::app::View::Wallets => {
            if index < app.wallets.len() {
                app.wallet_cursor = index;
                app.modal = Some(Modal::Wallet { index, scroll: 0 });
            }
        }
        Action::SelectLog(index) => {
            if let Some(entry) = app.visible_logs().get(index).map(|entry| (*entry).clone()) {
                app.focus_log(index);
                app.modal = Some(Modal::Log { entry, scroll: 0 });
            }
        }
        Action::CopyLog => {
            if let Some(Modal::Log { entry, .. }) = &app.modal {
                copy(format!(
                    "{} {}\n{}",
                    entry.timestamp, entry.level, entry.message
                ))?;
                app.status = "Log copy requested · terminal clipboard support required".into();
            }
        }
        Action::CopyWallet(index) => {
            copy(
                app.wallets
                    .get(index)
                    .context("Wallet unavailable")?
                    .address
                    .clone(),
            )?;
            app.status = "Address copy requested · active wallet unchanged".into();
        }
        Action::SelectWallet(index) | Action::ActivateWallet(index) => {
            app.modal = None;
            if index < app.wallets.len() && index != app.selected_wallet {
                app.selected_wallet = index;
                app.wallet_cursor = index;
                app.config.selected_wallet = app.wallet().map(|w| w.path.clone());
                services.save(app).await;
                if !app.demo {
                    services.restart(app);
                }
            }
        }
        Action::NextWallet => {
            if !app.wallets.is_empty() {
                app.selected_wallet = (app.selected_wallet + 1) % app.wallets.len();
                app.wallet_cursor = app.selected_wallet;
                app.config.selected_wallet = app.wallet().map(|w| w.path.clone());
                services.save(app).await;
                if !app.demo {
                    services.restart(app);
                }
            }
        }
        Action::SelectProfile(index) => {
            if index < app.config.profiles.len() {
                app.config.selected_profile = index;
                app.modal = None;
                services.save(app).await;
                if !app.demo {
                    services.restart(app);
                }
            }
        }
        Action::Theme | Action::Motion => {
            let kind = if action == Action::Theme {
                Appearance::Theme
            } else {
                Appearance::Motion
            };
            app.modal = Some(Modal::Appearance {
                kind,
                selected: kind.current(&app.config),
            });
        }
        Action::SelectAppearance(kind, index) => {
            if let Some((id, _)) = kind.choices().get(index) {
                match kind {
                    Appearance::Theme => app.config.theme = (*id).into(),
                    Appearance::Motion => app.config.reduced_motion = index == 1,
                }
                app.modal = None;
                services.save(app).await;
            }
        }
        Action::Refresh => {
            if app.view == crate::app::View::Tokens {
                services.load_tokens(app);
                return Ok(false);
            }
            if matches!(app.refresh, RefreshState::Pending(_)) {
                return Ok(false);
            }
            if app.demo {
                app.status = "Demo data is fixed; no RPC request was sent".into();
            } else if services
                .monitor
                .as_ref()
                .is_some_and(|monitor| monitor.commands.try_send(Command::RefreshRequested).is_ok())
            {
                app.refresh = RefreshState::Pending(Instant::now());
                app.status = if services.offline {
                    "Reloading cached history…"
                } else {
                    "Refreshing RPC and wallet state…"
                }
                .into();
            } else {
                app.finish_refresh(Err("Worker unavailable or busy; try again".into()));
            }
        }
        Action::Older => {
            app.switch_view(crate::app::View::Activity);
            app.pane = Pane::Wallet;
            if app.history_loading {
                return Ok(false);
            }
            if app.demo {
                app.status = "Demo history is fixed; no older transactions to load".into();
            } else if app.wallet().is_none() {
                app.status = "Select a wallet before loading older history".into();
            } else if services
                .monitor
                .as_ref()
                .is_some_and(|monitor| monitor.commands.try_send(Command::Older).is_ok())
            {
                app.history_loading = true;
                app.status = "Loading older history…".into();
            } else {
                app.status = "History worker unavailable or busy; try again shortly".into();
                app.log("WARN", app.status.clone());
            }
        }
        Action::Inspect => {
            if let Some(record) = app.selected_transaction() {
                let signature = record.signature.clone();
                services.command(Command::Detail(signature.clone()));
                app.modal = Some(Modal::Inspect {
                    signature,
                    scroll: 0,
                });
            }
        }
        Action::CopyAddress => {
            copy(app.wallet().context("No wallet selected")?.address.clone())?;
            app.status = "Copy requested · terminal clipboard support required".into();
        }
        Action::CopySignature => {
            copy(selected_signature(app).context("No transaction selected")?)?;
            app.status = "Copy requested · terminal clipboard support required".into();
        }
        Action::ExplorerWallet | Action::ExplorerTransaction => {
            let (kind, value) = if action == Action::ExplorerWallet {
                (
                    "address",
                    app.wallet().context("No wallet selected")?.address.clone(),
                )
            } else {
                (
                    "tx",
                    selected_signature(app).context("No transaction selected")?,
                )
            };
            let url = network::explorer_url(
                app.profile(),
                app.network.as_ref().map(|n| n.genesis.as_str()),
                kind,
                &value,
            )?;
            let sender = services.local_sender.clone();
            services.jobs.spawn(async move {
                let result = tokio::task::spawn_blocking(move || open::that(url))
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|r| r.map_err(anyhow::Error::from))
                    .map(|_| "Opened Solana Explorer".into());
                let _ = sender.send(LocalEvent::Notice(result)).await;
            });
        }
        Action::Submit => submit_form(app, services).await?,
        _ => {}
    }
    Ok(false)
}

fn selected_signature(app: &App) -> Option<String> {
    if let Some(Modal::Log { entry, .. }) = &app.modal {
        return entry.signature();
    }
    if let Some(Modal::Inspect { signature, .. }) = &app.modal {
        return Some(signature.clone());
    }
    if app.pane == Pane::Wallet {
        app.selected_transaction()
            .map(|r| r.signature.clone())
            .or_else(|| app.last_signature.clone())
    } else {
        app.last_signature
            .clone()
            .or_else(|| app.selected_transaction().map(|r| r.signature.clone()))
    }
}

fn copy(value: String) -> Result<()> {
    execute!(
        io::stdout(),
        crossterm::clipboard::CopyToClipboard::to_clipboard_from(value)
    )?;
    Ok(())
}

fn open_faucet(app: &mut App, services: &mut Services, faucet: Faucet) -> Result<()> {
    if app.demo || services.offline {
        bail!("Browser funding is disabled in demo/offline mode");
    }
    if app
        .network
        .as_ref()
        .is_some_and(|n| n.genesis != network::DEVNET_GENESIS)
        || (!funding::is_devnet(
            app.profile(),
            app.network.as_ref().map(|n| n.genesis.as_str()),
        ) && !matches!(app.modal, Some(Modal::Funding { .. })))
    {
        bail!("These web faucets fund Devnet only; select a Devnet profile first");
    }
    let address = app.wallet().context("No wallet selected")?.address.clone();
    let url = funding::faucet_url(faucet, &address)?;
    if faucet == Faucet::Quicknode {
        copy(address)?;
    }
    app.modal = None;
    app.status =
        "Opening faucet. Complete the request in your browser; Solte will refresh the balance."
            .into();
    let sender = services.local_sender.clone();
    services.jobs.spawn(async move {
        let result = tokio::task::spawn_blocking(move || open::that(url)).await.map_err(anyhow::Error::from).and_then(|result| result.map_err(anyhow::Error::from)).map(|_| match faucet {
            Faucet::Solana => "Opened the Solana faucet with your address prefilled. Complete verification in the browser.".into(),
            Faucet::Quicknode => "Opened Quicknode. Paste your address; clipboard support depends on your terminal.".into(),
        });
        let _ = sender.send(LocalEvent::Notice(result)).await;
    });
    Ok(())
}

async fn submit_form(app: &mut App, services: &mut Services) -> Result<()> {
    if let Some(Modal::Profiles { selected }) = &app.modal {
        app.config.selected_profile = *selected;
        app.modal = None;
        services.save(app).await;
        if !app.demo {
            services.restart(app);
        }
        return Ok(());
    }
    if let Some(Modal::Form(form)) = &app.modal
        && matches!(
            form.kind,
            FormKind::Search | FormKind::LogSearch | FormKind::TokenSearch
        )
    {
        if form.kind == FormKind::TokenSearch {
            app.token_filter = form.fields[0].value.trim().to_owned();
            app.token_cursor = 0;
            app.focused_control = None;
        } else if form.kind == FormKind::LogSearch {
            app.log_filter = form.fields[0].value.trim().to_owned();
            app.log_scroll = 0;
        } else {
            app.filter = form.fields[0].value.trim().to_owned();
        }
        app.transaction_cursor = 0;
        app.modal = None;
        return Ok(());
    }
    if let Some(Modal::Form(form)) = &app.modal
        && form.kind == FormKind::TokenExport
    {
        let account = app
            .token_export
            .clone()
            .context("No token account selected for export")?;
        ensure_export_path(&form.fields[0].value)?;
        let path = expand_path(&app.root, &PathBuf::from(form.fields[0].value.trim()));
        let sender = services.local_sender.clone();
        services.jobs.spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || crate::tokens::export(&account, &path))
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|v| v)
                    .map(|_| "Token account exported as JSON".into());
            let _ = sender.send(LocalEvent::Notice(result)).await;
        });
        app.modal = None;
        return Ok(());
    }
    if app.demo {
        bail!(
            "Demo mode uses fixture data. Relaunch without --demo to manage real development wallets."
        );
    }
    if matches!(app.modal, Some(Modal::Review { .. })) {
        if let Some(Modal::Review { prepared, .. }) = app.modal.take() {
            if prepared.simulation_error.is_some() {
                app.modal = Some(Modal::Review {
                    prepared,
                    scroll: 0,
                });
                bail!("Simulation failed; this transfer cannot be submitted");
            }
            let session = app.session;
            let store = services.store.clone();
            let sender = services.operation_sender.clone();
            if prepared.export_path.is_some() {
                app.busy = Some("Signing and exporting transaction…".into());
                services
                    .jobs
                    .spawn(operations::sign_export(*prepared, session, sender));
            } else {
                app.busy = Some("Signing and submitting transaction…".into());
                services
                    .jobs
                    .spawn(operations::submit(*prepared, session, store, sender));
            }
        }
        return Ok(());
    }
    let Some(Modal::Form(form)) = &app.modal else {
        return Ok(());
    };
    let kind = form.kind;
    let values: Vec<_> = form
        .fields
        .iter()
        .map(|f| f.value.trim().to_owned())
        .collect();
    match kind {
        FormKind::New | FormKind::Import => {
            let root = app.root.clone();
            let sender = services.local_sender.clone();
            app.busy = Some("Loading wallet…".into());
            services.jobs.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    if kind == FormKind::New {
                        wallet::create(&root, &values[0])
                    } else {
                        Wallet::load(
                            &expand_path(&root, &PathBuf::from(&values[0])),
                            Some(&values[1]),
                        )
                    }
                })
                .await
                .map_err(anyhow::Error::from)
                .and_then(|r| r);
                let _ = sender.send(LocalEvent::Wallet(result)).await;
            });
        }
        FormKind::Fund | FormKind::Transfer => {
            if services.offline {
                bail!("Network operations are disabled in offline mode");
            }
            let wallet = app.wallet().context("No wallet selected")?.clone();
            let profile = app.profile().clone();
            let session = app.session;
            let sender = services.operation_sender.clone();
            let store = services.store.clone();
            let lamports = parse_sol(&values[if kind == FormKind::Fund { 0 } else { 1 }])?;
            if kind == FormKind::Fund {
                app.modal = None;
                app.busy = Some("Requesting Devnet SOL…".into());
                services.jobs.spawn(operations::fund(
                    profile, wallet, lamports, session, store, sender,
                ));
            } else {
                let recipient = values[0].clone();
                let format = values[2].parse::<crate::transaction::Format>()?;
                app.busy = Some("Simulating transfer…".into());
                services.jobs.spawn(async move {
                    let update = match operations::prepare_format(
                        &profile, &wallet, &recipient, lamports, format,
                    )
                    .await
                    {
                        Ok(prepared) => {
                            if let Some(error) = &prepared.simulation_error {
                                let scope = crate::storage::scope(&profile.http, &wallet.address);
                                let message = format!(
                                    "Simulation failed; no transaction submitted: {error}\n{}",
                                    prepared.logs.join("\n")
                                );
                                if let Err(error) = store
                                    .log(&scope, crate::model::LogEntry::new("ERROR", message))
                                    .await
                                {
                                    tracing::error!(%error, "Cannot persist simulation failure");
                                }
                            }
                            OperationUpdate::Prepared(Box::new(prepared))
                        }
                        Err(error) => OperationUpdate::Failed(network::safe_error(error, &profile)),
                    };
                    let _ = sender.send(OperationEvent { session, update }).await;
                });
            }
        }
        FormKind::TokenTransfer | FormKind::TokenCreate => {
            if services.offline {
                bail!("Online simulation is required for token operations");
            }
            let wallet = app
                .wallet()
                .context("Select a signing wallet first")?
                .clone();
            let profile = app.profile().clone();
            let selected = app.token_operation.clone();
            if kind == FormKind::TokenTransfer {
                anyhow::ensure!(
                    app.token_genesis.as_deref()
                        == app.network.as_ref().map(|n| n.genesis.as_str()),
                    "Network identity changed or unavailable; refresh token accounts first"
                );
            }
            let expected_genesis = app.token_genesis.clone();
            let session = app.session;
            let sender = services.operation_sender.clone();
            app.busy = Some("Simulating token operation…".into());
            services.jobs.spawn(async move {
                let result = async {
                    let prepared = if kind == FormKind::TokenCreate {
                        crate::token_operations::prepare_create(
                            &profile,
                            &wallet,
                            &values[0],
                            &values[1],
                            values[2].parse()?,
                        )
                        .await?
                    } else {
                        crate::token_operations::prepare_transfer(
                            &profile,
                            &wallet,
                            selected.as_ref().context("No source account")?,
                            &values[0],
                            &values[1],
                            values[2] == "Token account",
                            values[3].parse()?,
                        )
                        .await?
                    };
                    if kind == FormKind::TokenTransfer {
                        anyhow::ensure!(
                            Some(&prepared.genesis) == expected_genesis.as_ref(),
                            "Token accounts belong to a different network; refresh first"
                        );
                    }
                    Ok::<_, anyhow::Error>(prepared)
                }
                .await;
                let update = match result {
                    Ok(prepared) => OperationUpdate::Prepared(Box::new(prepared)),
                    Err(error) => OperationUpdate::Failed(network::safe_error(error, &profile)),
                };
                let _ = sender.send(OperationEvent { session, update }).await;
            });
        }
        FormKind::TransactionImport => {
            if services.offline {
                bail!("Online simulation is required before signing imported transactions");
            }
            let wallet = app.wallet().context("No wallet selected")?.clone();
            let profile = app.profile().clone();
            let path = expand_path(&app.root, &PathBuf::from(&values[0]));
            let export = if values[1] == "Sign & export" {
                if values[2].is_empty() {
                    bail!("Choose an export path");
                }
                Some(expand_path(&app.root, &PathBuf::from(&values[2])))
            } else {
                None
            };
            let session = app.session;
            let sender = services.operation_sender.clone();
            app.busy = Some("Decoding and simulating imported transaction…".into());
            services.jobs.spawn(async move {
                let update =
                    match operations::prepare_import(&profile, &wallet, &path, export).await {
                        Ok(prepared) => OperationUpdate::Prepared(Box::new(prepared)),
                        Err(error) => OperationUpdate::Failed(network::safe_error(error, &profile)),
                    };
                let _ = sender.send(OperationEvent { session, update }).await;
            });
        }
        FormKind::Profile => {
            let profile = RpcProfile::custom(&values[0], &values[1], &values[2])?;
            if app
                .config
                .profiles
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case(&profile.name))
            {
                bail!("A profile with that name already exists");
            }
            app.config.profiles.push(profile);
            app.config.selected_profile = app.config.profiles.len() - 1;
            app.modal = None;
            services.save(app).await;
            services.restart(app);
        }
        FormKind::TokenExport => unreachable!(),
        FormKind::Search | FormKind::LogSearch | FormKind::TokenSearch => {
            app.filter = values[0].clone();
            app.transaction_cursor = 0;
            app.modal = None;
        }
    }
    Ok(())
}

fn ensure_export_path(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("Choose an export path");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_updates_retain_selection_and_ignore_old_sessions_or_networks() {
        let mut app = App::new("/fixture".into(), crate::config::Config::default(), vec![]);
        crate::demo::populate(&mut app);
        app.session = 3;
        app.token_cursor = 2;
        let selected = app.selected_token().unwrap().address.clone();
        let snapshot = crate::tokens::Snapshot {
            genesis: network::DEVNET_GENESIS.into(),
            accounts: app.tokens.iter().rev().cloned().collect(),
            warnings: vec![],
        };
        app.token_loading = true;
        apply_token_update(&mut app, 2, Ok(snapshot.clone()));
        assert!(app.token_loading);
        assert_eq!(app.token_cursor, 2);
        apply_token_update(&mut app, 3, Ok(snapshot.clone()));
        assert!(!app.token_loading);
        assert_eq!(app.selected_token().unwrap().address, selected);
        let mut wrong = snapshot;
        wrong.genesis = "different-network".into();
        apply_token_update(&mut app, 3, Ok(wrong));
        assert_eq!(app.selected_token().unwrap().address, selected);
        assert!(
            app.token_error
                .as_ref()
                .unwrap()
                .contains("Network changed")
        );
    }

    #[tokio::test]
    async fn token_keyboard_and_mouse_actions_open_the_same_forms() {
        let mut app = App::new("/fixture".into(), crate::config::Config::default(), vec![]);
        crate::demo::populate(&mut app);
        app.switch_view(crate::app::View::Tokens);
        let directory = tempfile::tempdir().unwrap();
        let (network_sender, _) = mpsc::channel(1);
        let (operation_sender, _) = mpsc::channel(1);
        let (local_sender, _) = mpsc::channel(1);
        let mut services = Services {
            store: Store::open(directory.path()).unwrap(),
            network_sender,
            operation_sender,
            local_sender,
            monitor: None,
            jobs: JoinSet::new(),
            offline: false,
        };
        for action in [
            app.key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('s'),
                crossterm::event::KeyModifiers::NONE,
            ))
            .unwrap(),
            Action::Send,
        ] {
            handle(&mut app, &mut services, action).await.unwrap();
            assert!(
                matches!(&app.modal,Some(Modal::Form(form)) if form.kind == FormKind::TokenTransfer)
            );
            app.modal = None;
        }
        handle(&mut app, &mut services, Action::CreateTokenAccount)
            .await
            .unwrap();
        assert!(
            matches!(&app.modal,Some(Modal::Form(form)) if form.kind == FormKind::TokenCreate && form.fields[1].value == app.wallet().unwrap().address)
        );
    }

    #[tokio::test]
    async fn log_inspection_keeps_the_selected_entry_when_new_logs_arrive() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), crate::config::Config::default(), vec![]);
        app.switch_view(crate::app::View::Logs);
        let signature = solana_signature::Signature::from([8; 64]).to_string();
        app.log("INFO", format!("Confirmed {signature}"));
        let (network_sender, _) = mpsc::channel(8);
        let (operation_sender, _) = mpsc::channel(8);
        let (local_sender, _) = mpsc::channel(8);
        let mut services = Services {
            store: Store::open(root.path()).unwrap(),
            network_sender,
            operation_sender,
            local_sender,
            monitor: None,
            jobs: JoinSet::new(),
            offline: true,
        };
        handle(&mut app, &mut services, Action::SelectLog(0))
            .await
            .unwrap();
        app.log("WARN", "New unrelated entry");
        assert_eq!(selected_signature(&app), Some(signature));
        assert!(!app.follow);
        assert_eq!(app.visible_logs().len(), 1);
        handle(&mut app, &mut services, Action::Close)
            .await
            .unwrap();
        app.navigate(&Action::Follow);
        assert_eq!(app.visible_logs().len(), 2);
        assert_eq!(app.log_scroll, 0);
    }

    #[tokio::test]
    async fn wallet_preview_does_not_activate_until_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), crate::config::Config::default(), vec![]);
        crate::demo::populate(&mut app);
        app.switch_view(crate::app::View::Wallets);
        let (network_sender, _) = mpsc::channel(8);
        let (operation_sender, _) = mpsc::channel(8);
        let (local_sender, _) = mpsc::channel(8);
        let mut services = Services {
            store: Store::open(root.path()).unwrap(),
            network_sender,
            operation_sender,
            local_sender,
            monitor: None,
            jobs: JoinSet::new(),
            offline: true,
        };
        handle(&mut app, &mut services, Action::SelectWallet(1))
            .await
            .unwrap();
        assert_eq!(app.selected_wallet, 0);
        assert!(matches!(app.modal, Some(Modal::Wallet { index: 1, .. })));
        assert_eq!(
            app.key(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Char('y')
            )),
            Some(Action::CopyWallet(1))
        );
        handle(&mut app, &mut services, Action::Close)
            .await
            .unwrap();
        assert_eq!(app.selected_wallet, 0);
        handle(&mut app, &mut services, Action::SelectWallet(1))
            .await
            .unwrap();
        handle(&mut app, &mut services, Action::ActivateWallet(1))
            .await
            .unwrap();
        assert_eq!(app.selected_wallet, 1);
        assert!(app.modal.is_none());
    }
}
