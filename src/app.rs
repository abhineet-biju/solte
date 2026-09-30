use std::{collections::VecDeque, path::PathBuf, time::Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    config::{Config, RpcProfile},
    model::{LogEntry, NetworkState, TransactionRecord},
    operations::PreparedTransfer,
    wallet::Wallet,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Tokens,
    Wallets,
    Wallet,
    Network,
    Logs,
}

impl Pane {
    pub const ALL: [Self; 5] = [
        Self::Wallets,
        Self::Tokens,
        Self::Wallet,
        Self::Network,
        Self::Logs,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Tokens => "Tokens",
            Self::Wallets => "Wallets",
            Self::Wallet => "Activity",
            Self::Network => "Network",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum View {
    Overview,
    Wallets,
    Tokens,
    #[value(name = "transactions", alias = "activity")]
    Activity,
    Network,
    Logs,
}

impl View {
    pub const ALL: [Self; 6] = [
        Self::Overview,
        Self::Wallets,
        Self::Tokens,
        Self::Activity,
        Self::Network,
        Self::Logs,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Tokens => "Tokens",
            Self::Wallets => "Wallets",
            Self::Activity => "Transactions",
            Self::Network => "Network",
            Self::Logs => "Logs",
        }
    }
    pub fn nav_label(self, width: u16) -> &'static str {
        if self == Self::Activity && width < 110 {
            return "Txns";
        }
        if width < 80 {
            match self {
                Self::Overview => "Home",
                Self::Wallets => "Keys",
                Self::Activity => "Txns",
                Self::Network => "RPC",
                _ => self.name(),
            }
        } else {
            self.name()
        }
    }

    pub fn pane(self) -> Pane {
        match self {
            Self::Overview | Self::Activity => Pane::Wallet,
            Self::Tokens => Pane::Tokens,
            Self::Wallets => Pane::Wallets,
            Self::Network => Pane::Network,
            Self::Logs => Pane::Logs,
        }
    }
    pub fn for_pane(pane: Pane) -> Self {
        match pane {
            Pane::Wallet => Self::Activity,
            Pane::Tokens => Self::Tokens,
            Pane::Wallets => Self::Wallets,
            Pane::Network => Self::Network,
            Pane::Logs => Self::Logs,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    New,
    Import,
    Fund,
    Transfer,
    TransactionImport,
    Profile,
    Search,
    LogSearch,
    TokenSearch,
    TokenExport,
    TokenCreate,
    TokenTransfer,
    MintCreate,
    ConfidentialMint,
    Confidential(crate::confidential_operations::Operation),
    MintMore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    BrowserFaucet(crate::funding::Faucet),
    RpcAirdrop,
    ForceQuit,
    Quit,
    Focus(Pane),
    Selector(View),
    CycleFocus(bool),
    Navigate(Direction),
    Move(i32),
    Activate,
    SelectWallet(usize),
    ActivateWallet(usize),
    CopyWallet(usize),
    SelectLog(usize),
    SelectToken(usize),
    CopyToken(bool),
    ExplorerToken(bool),
    ExportToken,
    CreateTokenAccount,
    TokenCreation,
    CreateMint,
    CreateConfidentialMint,
    Confidential,
    ConfidentialOperation(crate::confidential_operations::Operation),
    ProjectMints,
    SelectMint(usize),
    MintMore,
    ViewMintAccount,
    CopyLog,
    SelectTransaction(usize),
    New,
    Import,
    Fund,
    Send,
    ImportTransaction,
    Profiles,
    AddProfile,
    SelectProfile(usize),
    Theme,
    Motion,
    SelectAppearance(Appearance, usize),
    Refresh,
    Older,
    Inspect,
    Close,
    Submit,
    CopyAddress,
    CopySignature,
    ExplorerWallet,
    ExplorerTransaction,
    Follow,
    ClearLogs,
    Search,
    ClearFilter,
    Failures,
    Help,
    OpenSummary,
    Expand(Pane),
    Field(usize),
    Choice(usize, i32),
    Scroll(i32),
    NextWallet,
}

pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub cursor: usize,
    pub choices: &'static [&'static str],
}

impl Field {
    pub fn new(label: &'static str, value: &str) -> Self {
        Self {
            label,
            value: value.into(),
            cursor: value.len(),
            choices: &[],
        }
    }
    pub fn choice(label: &'static str, choices: &'static [&'static str]) -> Self {
        let mut field = Self::new(label, choices[0]);
        field.choices = choices;
        field
    }
    pub fn cycle(&mut self, delta: i32) {
        if self.choices.is_empty() {
            return;
        }
        let index = self
            .choices
            .iter()
            .position(|v| *v == self.value)
            .unwrap_or(0) as i32;
        self.value =
            self.choices[(index + delta).rem_euclid(self.choices.len() as i32) as usize].into();
        self.cursor = self.value.len();
    }
    pub fn insert(&mut self, value: &str) {
        if !self.choices.is_empty() {
            return;
        }
        let clean: String = value
            .chars()
            .filter(|c| !c.is_control())
            .take(2048usize.saturating_sub(self.value.chars().count()))
            .collect();
        self.value.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }
    fn left(&mut self) {
        self.cursor = self.value[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
    }
    fn right(&mut self) {
        self.cursor += self.value[self.cursor..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
    }
}

pub struct Form {
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub active: usize,
    pub error: Option<String>,
}

impl Form {
    pub fn new(kind: FormKind, next_wallet: usize) -> Self {
        let fields = match kind {
            FormKind::New => vec![Field::new(
                "Wallet name",
                &format!("test-user-{next_wallet}"),
            )],
            FormKind::Import => vec![
                Field::new("Keypair JSON path", ""),
                Field::new("Display name", "imported-wallet"),
            ],
            FormKind::Fund => vec![Field::new("Amount · SOL", "1")],
            FormKind::Transfer => vec![
                Field::new("Recipient address", ""),
                Field::new("Amount · SOL", "0.1"),
                Field::choice("Format · [←]/[→] choose", &["Auto", "Legacy", "v0", "v1"]),
            ],
            FormKind::TransactionImport => vec![
                Field::new("Transaction file · raw bytes or base64", ""),
                Field::choice(
                    "After review · [←]/[→] choose",
                    &["Sign & export", "Sign & submit"],
                ),
                Field::new(
                    "Export path · required for export",
                    "signed-transaction.base64",
                ),
            ],
            FormKind::Profile => vec![
                Field::new("Profile name", ""),
                Field::new("HTTP RPC endpoint", "http://127.0.0.1:8899"),
                Field::new("WebSocket endpoint", "ws://127.0.0.1:8900"),
            ],
            FormKind::MintCreate | FormKind::ConfidentialMint => vec![
                Field::choice(
                    "Token program · [←]/[→] choose",
                    &["SPL Token", "Token-2022"],
                ),
                Field::new("Decimals", "6"),
                Field::new("Mint authority address", ""),
                Field::new("Freeze authority · blank disables", ""),
                Field::new("Initial supply · tokens", "0"),
                Field::choice("Format · [←]/[→] choose", &["Auto", "Legacy", "v0", "v1"]),
            ],
            FormKind::Confidential(operation) => match operation {
                crate::confidential_operations::Operation::Configure => {
                    vec![Field::new("Maximum pending credits", "65536")]
                }
                crate::confidential_operations::Operation::Approve => {
                    vec![Field::new("Token account to approve", "")]
                }
                crate::confidential_operations::Operation::Transfer => vec![
                    Field::new("Recipient address", ""),
                    Field::new("Amount · tokens", ""),
                    Field::choice("Destination type", &["Wallet ATA", "Token account"]),
                ],
                crate::confidential_operations::Operation::Apply => {
                    vec![Field::choice("Action", &["Apply pending balance"])]
                }
                _ => vec![Field::new("Amount · tokens", "")],
            },
            FormKind::MintMore => vec![
                Field::new("Mint address", ""),
                Field::new("Recipient wallet address", ""),
                Field::new("Amount · tokens", ""),
                Field::choice("Format · [←]/[→] choose", &["Auto", "Legacy", "v0", "v1"]),
            ],
            FormKind::TokenCreate => vec![
                Field::new("Mint address", ""),
                Field::new("Recipient wallet address", ""),
                Field::choice("Format · [←]/[→] choose", &["Auto", "Legacy", "v0", "v1"]),
            ],
            FormKind::TokenTransfer => vec![
                Field::new("Destination address", ""),
                Field::new("Amount · tokens", ""),
                Field::choice(
                    "Destination type · [←]/[→] choose",
                    &["Wallet / create ATA", "Token account"],
                ),
                Field::choice("Format · [←]/[→] choose", &["Auto", "Legacy", "v0", "v1"]),
            ],
            FormKind::TokenSearch => {
                vec![Field::new("Mint, account, program, state or delegate", "")]
            }
            FormKind::TokenExport => vec![Field::new("JSON export path", "token-account.json")],
            FormKind::LogSearch => vec![Field::new("Message or level", "")],
            FormKind::Search => vec![Field::new("Signature, instruction, or error", "")],
        };
        let mut fields = fields;
        if kind == FormKind::ConfidentialMint {
            fields[0] = Field::choice("Token program", &["Token-2022"]);
            fields.push(Field::choice(
                "Confidential account approval",
                &["Automatic", "Manual"],
            ));
            fields.push(Field::new("Auditor ElGamal public key · optional", ""));
        }
        Self {
            kind,
            fields,
            active: 0,
            error: None,
        }
    }
    pub fn title(&self) -> &'static str {
        match self.kind {
            FormKind::New => "Create development wallet",
            FormKind::Import => "Import existing keypair",
            FormKind::Fund => "Fund wallet",
            FormKind::Transfer => "Send SOL",
            FormKind::TransactionImport => "Import transaction",
            FormKind::Profile => "Add RPC profile",
            FormKind::Search => "Filter transactions",
            FormKind::LogSearch => "Filter logs",
            FormKind::TokenSearch => "Filter token accounts",
            FormKind::TokenExport => "Export token account",
            FormKind::TokenCreate => "Create associated token account",
            FormKind::TokenTransfer => "Send tokens",
            FormKind::MintCreate => "Create token mint",
            FormKind::ConfidentialMint => "Create confidential token mint",
            FormKind::Confidential(operation) => operation.label(),
            FormKind::MintMore => "Mint tokens",
        }
    }
    pub fn submit_label(&self) -> &'static str {
        match self.kind {
            FormKind::New => "Create wallet",
            FormKind::Import => "Import wallet",
            FormKind::Fund => "Request airdrop",
            FormKind::Transfer | FormKind::TransactionImport => "Simulate & review",
            FormKind::Profile => "Save profile",
            FormKind::TokenExport => "Export JSON",
            FormKind::TokenCreate
            | FormKind::TokenTransfer
            | FormKind::MintCreate
            | FormKind::MintMore
            | FormKind::ConfidentialMint
            | FormKind::Confidential(_) => "Simulate & review",
            FormKind::Search | FormKind::LogSearch | FormKind::TokenSearch => "Apply filter",
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Option<Action> {
        if !self.fields[self.active].choices.is_empty() {
            match key.code {
                KeyCode::Left | KeyCode::Char('h') => {
                    self.fields[self.active].cycle(-1);
                    return None;
                }
                KeyCode::Right | KeyCode::Char('l' | ' ') => {
                    self.fields[self.active].cycle(1);
                    return None;
                }
                KeyCode::Char(_)
                | KeyCode::Delete
                | KeyCode::Backspace
                | KeyCode::Home
                | KeyCode::End => return None,
                _ => {}
            }
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            self.fields[self.active].value.clear();
            self.fields[self.active].cursor = 0;
            return None;
        }
        match key.code {
            KeyCode::Esc => return Some(Action::Close),
            KeyCode::Enter => return Some(Action::Submit),
            KeyCode::Tab => return Some(Action::Field((self.active + 1) % self.fields.len())),
            KeyCode::BackTab => {
                return Some(Action::Field(
                    (self.active + self.fields.len() - 1) % self.fields.len(),
                ));
            }
            KeyCode::Down => {
                return Some(Action::Field((self.active + 1).min(self.fields.len() - 1)));
            }
            KeyCode::Up => return Some(Action::Field(self.active.saturating_sub(1))),
            KeyCode::Left => self.fields[self.active].left(),
            KeyCode::Right => self.fields[self.active].right(),
            KeyCode::Home => self.fields[self.active].cursor = 0,
            KeyCode::End => self.fields[self.active].cursor = self.fields[self.active].value.len(),
            KeyCode::Backspace => {
                let field = &mut self.fields[self.active];
                let end = field.cursor;
                field.left();
                field.value.replace_range(field.cursor..end, "");
            }
            KeyCode::Delete => {
                let field = &mut self.fields[self.active];
                if field.cursor < field.value.len() {
                    field.value.remove(field.cursor);
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.fields[self.active].insert(&c.to_string())
            }
            _ => {}
        }
        self.error = None;
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Theme,
    Motion,
}

impl Appearance {
    pub fn choices(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Theme => &[
                ("ember", "Ember · warm gold"),
                ("glacier", "Glacier · cool blue"),
                ("orchid", "Orchid · soft violet"),
                ("neon", "Neon · cyan & magenta"),
            ],
            Self::Motion => &[
                ("animated", "Subtle animations"),
                ("reduced", "Reduced motion"),
            ],
        }
    }

    pub fn current(self, config: &Config) -> usize {
        match self {
            Self::Theme => self
                .choices()
                .iter()
                .position(|(id, _)| *id == config.theme)
                .unwrap_or(0),
            Self::Motion => usize::from(config.reduced_motion),
        }
    }
}

pub enum Modal {
    Confidential {
        account: Box<crate::tokens::TokenAccount>,
        selected: usize,
    },
    TokenCreation {
        selected: usize,
    },
    ProjectMints {
        selected: usize,
    },
    Mint {
        mint: Box<crate::mints::ProjectMint>,
        scroll: u16,
    },
    Token {
        account: Box<crate::tokens::TokenAccount>,
        scroll: u16,
    },
    Log {
        entry: LogEntry,
        scroll: u16,
    },
    Wallet {
        index: usize,
        scroll: u16,
    },
    Appearance {
        kind: Appearance,
        selected: usize,
    },
    Funding {
        selected: usize,
        reason: Option<String>,
    },
    Form(Form),
    Profiles {
        selected: usize,
    },
    Inspect {
        signature: String,
        scroll: u16,
    },
    Review {
        prepared: Box<PreparedTransfer>,
        scroll: u16,
    },
    Help {
        scroll: u16,
    },
}

#[derive(Default)]
pub enum RefreshState {
    #[default]
    Idle,
    Pending(Instant),
    Finished {
        at: Instant,
        success: bool,
    },
}

pub struct App {
    pub root: PathBuf,
    pub config: Config,
    pub wallets: Vec<Wallet>,
    pub selected_wallet: usize,
    pub wallet_cursor: usize,
    pub pane: Pane,
    pub view: View,
    view_focus: [Option<(Pane, Option<Action>)>; 6],
    pub focused_control: Option<Action>,
    pub selector_focus: bool,
    pub tokens: Vec<crate::tokens::TokenAccount>,
    pub project_mints: Vec<crate::mints::ProjectMint>,
    pub mint_receipt: Option<crate::mints::MintRecord>,
    pub token_cursor: usize,
    pub token_export: Option<crate::tokens::TokenAccount>,
    pub token_operation: Option<crate::tokens::TokenAccount>,
    pub confidential_request: u64,
    pub confidential_balances: Option<(
        String,
        zeroize::Zeroizing<crate::confidential_operations::Balances>,
    )>,
    pub token_filter: String,
    pub token_loading: bool,
    pub token_refresh_after: bool,
    pub token_started: Option<Instant>,
    pub token_updated: Option<Instant>,
    pub token_error: Option<String>,
    pub token_genesis: Option<String>,
    pub token_warnings: Vec<String>,
    pub records: Vec<TransactionRecord>,
    pub transaction_cursor: usize,
    pub filter: String,
    pub failures_only: bool,
    pub history_loading: bool,
    pub refresh: RefreshState,
    pub offline: bool,
    pub modal_scroll_limit: u16,
    pub logs: VecDeque<LogEntry>,
    pub log_scroll: usize,
    pub log_filter: String,
    pub follow: bool,
    paused_logs: Option<VecDeque<LogEntry>>,
    pub network: Option<NetworkState>,
    pub network_scroll: u16,
    pub balance: Option<u64>,
    pub connected: bool,
    pub subscribed: bool,
    pub last_update: Option<Instant>,
    pub status: String,
    pub modal: Option<Modal>,
    pub busy: Option<String>,
    pub last_signature: Option<String>,
    pub session: u64,
    pub demo: bool,
}

impl App {
    pub fn new(root: PathBuf, config: Config, wallets: Vec<Wallet>) -> Self {
        let selected_wallet = config
            .selected_wallet
            .as_ref()
            .and_then(|path| wallets.iter().position(|w| &w.path == path))
            .unwrap_or(0);
        Self {
            root,
            config,
            wallets,
            selected_wallet,
            wallet_cursor: selected_wallet,
            pane: Pane::Wallet,
            view: View::Overview,
            view_focus: std::array::from_fn(|_| None),
            focused_control: None,
            selector_focus: false,
            tokens: vec![],
            project_mints: vec![],
            mint_receipt: None,
            token_export: None,
            token_operation: None,
            token_cursor: 0,
            confidential_request: 0,
            confidential_balances: None,
            token_filter: String::new(),
            token_loading: false,
            token_refresh_after: false,
            token_started: None,
            token_updated: None,
            token_error: None,
            token_genesis: None,
            token_warnings: vec![],
            records: Vec::new(),
            transaction_cursor: 0,
            filter: String::new(),
            failures_only: false,
            history_loading: false,
            refresh: RefreshState::Idle,
            offline: false,
            modal_scroll_limit: 0,
            logs: VecDeque::new(),
            log_scroll: 0,
            log_filter: String::new(),
            follow: true,
            paused_logs: None,
            network: None,
            network_scroll: 0,
            balance: None,
            connected: false,
            subscribed: false,
            last_update: None,
            status: "Connecting to development RPC…".into(),
            modal: None,
            busy: None,
            last_signature: None,
            session: 0,
            demo: false,
        }
    }

    pub fn refresh_label(&self) -> String {
        match self.refresh {
            RefreshState::Pending(started) => {
                if self.config.reduced_motion {
                    "Refreshing…".into()
                } else {
                    format!(
                        "Refreshing {}",
                        ['|', '/', '-', '\\'][(started.elapsed().as_millis() / 120 % 4) as usize]
                    )
                }
            }
            RefreshState::Finished { at, success } if at.elapsed().as_secs() < 4 => {
                if success {
                    "Refreshed ✓".into()
                } else {
                    "Failed [r]".into()
                }
            }
            _ => "Refresh [r]".into(),
        }
    }

    pub fn finish_refresh(&mut self, result: Result<(), String>) {
        self.refresh = RefreshState::Finished {
            at: Instant::now(),
            success: result.is_ok(),
        };
        let (level, message) = match result {
            Ok(()) => (
                "INFO",
                if self.offline {
                    "Cached history reloaded".into()
                } else {
                    "RPC and wallet state refreshed".into()
                },
            ),
            Err(message) => ("WARN", format!("Refresh failed: {message}")),
        };
        self.status = message.clone();
        self.log(level, message);
    }

    pub fn switch_view(&mut self, view: View) {
        if view != self.view {
            self.view_focus[self.view as usize] = Some((self.pane, self.focused_control.clone()));
            self.view = view;
            let (pane, focus) = self.view_focus[view as usize]
                .clone()
                .unwrap_or((view.pane(), None));
            self.pane = pane;
            self.focused_control = focus;
        }
    }

    pub fn log_transport(&self) -> &'static str {
        if self.offline {
            "Offline"
        } else if !self.connected {
            "Waiting"
        } else if self.subscribed {
            "Live"
        } else {
            "Polling"
        }
    }

    pub fn wallet(&self) -> Option<&Wallet> {
        self.wallets.get(self.selected_wallet)
    }
    pub fn profile(&self) -> &RpcProfile {
        &self.config.profiles[self.config.selected_profile]
    }
    pub fn visible_tokens(&self) -> Vec<&crate::tokens::TokenAccount> {
        let query = self.token_filter.to_lowercase();
        self.tokens
            .iter()
            .filter(|a| {
                query.is_empty()
                    || format!(
                        "{} {} {} {} {} {} {}",
                        a.address,
                        a.mint,
                        a.label(),
                        a.program_label(),
                        a.state,
                        a.info()["delegate"],
                        if crate::confidential::enabled(a) {
                            "confidential"
                        } else {
                            ""
                        }
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }
    pub fn selected_token(&self) -> Option<&crate::tokens::TokenAccount> {
        self.visible_tokens().get(self.token_cursor).copied()
    }
    pub fn visible_records(&self) -> Vec<&TransactionRecord> {
        let filter = self.filter.to_lowercase();
        self.records
            .iter()
            .filter(|r| {
                (!self.failures_only || r.error.is_some())
                    && (filter.is_empty()
                        || r.signature.to_lowercase().contains(&filter)
                        || r.kind().to_lowercase().contains(&filter)
                        || r.activity(
                            self.wallet()
                                .map(|w| w.address.as_str())
                                .unwrap_or_default(),
                        )
                        .to_lowercase()
                        .contains(&filter)
                        || r.error
                            .as_ref()
                            .is_some_and(|e| e.to_lowercase().contains(&filter)))
            })
            .collect()
    }
    pub fn selected_transaction(&self) -> Option<&TransactionRecord> {
        self.visible_records().get(self.transaction_cursor).copied()
    }
    pub fn log(&mut self, level: &str, message: impl Into<String>) {
        self.push_log(LogEntry::new(level, message));
    }
    pub fn log_entries(&self) -> &VecDeque<LogEntry> {
        self.paused_logs.as_ref().unwrap_or(&self.logs)
    }

    pub fn visible_logs(&self) -> Vec<&LogEntry> {
        let query = if self.view == View::Logs {
            self.log_filter.to_lowercase()
        } else {
            String::new()
        };
        self.log_entries()
            .iter()
            .rev()
            .filter(|entry| {
                query.is_empty()
                    || entry.message.to_lowercase().contains(&query)
                    || entry.level.to_lowercase().contains(&query)
            })
            .collect()
    }

    pub fn focus_log(&mut self, index: usize) {
        self.pause_logs();
        self.log_scroll = index.min(self.visible_logs().len().saturating_sub(1));
        self.focused_control = Some(Action::SelectLog(self.log_scroll));
    }

    pub fn resume_logs(&mut self) {
        self.follow = true;
        self.paused_logs = None;
        self.log_scroll = 0;
        if matches!(self.focused_control, Some(Action::SelectLog(_))) {
            self.focused_control = Some(Action::SelectLog(0));
        }
    }

    fn pause_logs(&mut self) {
        if self.follow {
            self.paused_logs = Some(self.logs.clone());
        }
        self.follow = false;
    }

    pub fn push_log(&mut self, entry: LogEntry) {
        if self.logs.iter().any(|existing| {
            existing.timestamp == entry.timestamp
                && existing.message == entry.message
                && existing.level == entry.level
        }) {
            return;
        }
        let index = self
            .logs
            .iter()
            .position(|existing| existing.timestamp > entry.timestamp)
            .unwrap_or(self.logs.len());
        self.logs.insert(index, entry);
        while self.logs.len() > 1000 {
            self.logs.pop_front();
        }
        if self.follow {
            self.log_scroll = 0;
        }
    }
    pub fn replace_records(&mut self, records: Vec<TransactionRecord>) {
        let selected = self.selected_transaction().map(|r| r.signature.clone());
        self.records = records;
        self.transaction_cursor = selected
            .and_then(|s| self.visible_records().iter().position(|r| r.signature == s))
            .unwrap_or(0);
        if matches!(self.focused_control, Some(Action::SelectTransaction(_))) {
            self.focused_control = Some(Action::SelectTransaction(self.transaction_cursor));
        }
    }
    pub fn open_form(&mut self, kind: FormKind) {
        self.modal = Some(Modal::Form(Form::new(kind, self.wallets.len() + 1)));
    }
    pub fn paste(&mut self, text: &str) {
        if let Some(Modal::Form(form)) = &mut self.modal {
            form.fields[form.active].insert(text);
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Action::ForceQuit);
        }
        if let Some(modal) = &mut self.modal {
            if let Modal::Form(form) = modal {
                return form.key(key);
            }
            return match key.code {
                KeyCode::Char('y') if matches!(modal, Modal::Log { .. }) => Some(Action::CopyLog),
                KeyCode::Char('s') if matches!(modal, Modal::Log { entry, .. } if entry.signature().is_some()) => {
                    Some(Action::CopySignature)
                }
                KeyCode::Char('o') if matches!(modal, Modal::Log { entry, .. } if entry.signature().is_some()) => {
                    Some(Action::ExplorerTransaction)
                }
                KeyCode::Char('y') if matches!(modal, Modal::Wallet { .. }) => {
                    let Modal::Wallet { index, .. } = modal else {
                        unreachable!()
                    };
                    Some(Action::CopyWallet(*index))
                }
                KeyCode::Enter if matches!(modal, Modal::Wallet { .. }) => {
                    let Modal::Wallet { index, .. } = modal else {
                        unreachable!()
                    };
                    Some(Action::ActivateWallet(*index))
                }
                KeyCode::Char('y') if matches!(modal, Modal::Funding { .. }) => {
                    Some(Action::CopyAddress)
                }
                KeyCode::Tab
                    if matches!(
                        modal,
                        Modal::Funding { .. }
                            | Modal::Appearance { .. }
                            | Modal::Profiles { .. }
                            | Modal::Confidential { .. }
                            | Modal::TokenCreation { .. }
                            | Modal::ProjectMints { .. }
                    ) =>
                {
                    Some(Action::Scroll(1))
                }
                KeyCode::BackTab
                    if matches!(
                        modal,
                        Modal::Funding { .. }
                            | Modal::Appearance { .. }
                            | Modal::Profiles { .. }
                            | Modal::Confidential { .. }
                            | Modal::TokenCreation { .. }
                            | Modal::ProjectMints { .. }
                    ) =>
                {
                    Some(Action::Scroll(-1))
                }
                KeyCode::Char('1') if matches!(modal, Modal::Funding { .. }) => {
                    Some(Action::BrowserFaucet(crate::funding::Faucet::Solana))
                }
                KeyCode::Char('2') if matches!(modal, Modal::Funding { .. }) => {
                    Some(Action::BrowserFaucet(crate::funding::Faucet::Quicknode))
                }
                KeyCode::Char('3') if matches!(modal, Modal::Funding { .. }) => {
                    Some(Action::RpcAirdrop)
                }
                KeyCode::Char('t') if matches!(modal, Modal::Mint { .. }) => {
                    Some(Action::ViewMintAccount)
                }
                KeyCode::Char('r')
                    if matches!(modal, Modal::Mint { .. } | Modal::ProjectMints { .. }) =>
                {
                    Some(Action::Refresh)
                }
                KeyCode::Char('m') if matches!(modal, Modal::Token { .. } | Modal::Mint { .. }) => {
                    Some(Action::MintMore)
                }
                KeyCode::Char('y' | 'M') if matches!(modal, Modal::Mint { .. }) => {
                    Some(Action::CopyToken(true))
                }
                KeyCode::Char('o' | 'O') if matches!(modal, Modal::Mint { .. }) => {
                    Some(Action::ExplorerToken(true))
                }
                KeyCode::Char('a') if matches!(modal, Modal::Mint { .. }) => {
                    Some(Action::CreateTokenAccount)
                }
                KeyCode::Enter if matches!(modal, Modal::Mint { .. }) => Some(Action::Close),
                KeyCode::Enter if matches!(modal, Modal::Confidential { .. }) => {
                    let Modal::Confidential { account, selected } = modal else {
                        unreachable!()
                    };
                    crate::confidential::choices(account, self.confidential_balances.is_some())
                        .get(*selected)
                        .copied()
                        .map(Action::ConfidentialOperation)
                }
                KeyCode::Char('c') if matches!(modal, Modal::Token { .. }) => {
                    Some(Action::Confidential)
                }
                KeyCode::Char('s') if matches!(modal, Modal::Token { .. }) => Some(Action::Send),
                KeyCode::Char('a') if matches!(modal, Modal::Token { .. }) => {
                    Some(Action::CreateTokenAccount)
                }
                KeyCode::Char('y') if matches!(modal, Modal::Token { .. }) => {
                    Some(Action::CopyToken(false))
                }
                KeyCode::Char('M') if matches!(modal, Modal::Token { .. }) => {
                    Some(Action::CopyToken(true))
                }
                KeyCode::Char('o') if matches!(modal, Modal::Token { .. }) => {
                    Some(Action::ExplorerToken(false))
                }
                KeyCode::Char('O') if matches!(modal, Modal::Token { .. }) => {
                    Some(Action::ExplorerToken(true))
                }
                KeyCode::Char('E') if matches!(modal, Modal::Token { .. }) => {
                    Some(Action::ExportToken)
                }
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::Close),
                KeyCode::Enter if matches!(modal, Modal::Token { .. }) => Some(Action::Close),
                KeyCode::Enter => Some(Action::Submit),
                KeyCode::Char('j' | 'J') | KeyCode::Down => Some(Action::Scroll(1)),
                KeyCode::Char('k' | 'K') | KeyCode::Up => Some(Action::Scroll(-1)),
                KeyCode::Home => Some(Action::Scroll(-65535)),
                KeyCode::End => Some(Action::Scroll(65535)),
                KeyCode::PageDown => Some(Action::Scroll(10)),
                KeyCode::PageUp => Some(Action::Scroll(-10)),
                KeyCode::Char('a') if matches!(modal, Modal::Profiles { .. }) => {
                    Some(Action::AddProfile)
                }
                KeyCode::Char('o') if matches!(modal, Modal::Inspect { .. }) => {
                    Some(Action::ExplorerTransaction)
                }
                KeyCode::Char('y') if matches!(modal, Modal::Inspect { .. }) => {
                    Some(Action::CopySignature)
                }
                _ => None,
            };
        }
        match key.code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Tab => Some(Action::CycleFocus(true)),
            KeyCode::BackTab => Some(Action::CycleFocus(false)),
            KeyCode::Char('1') => Some(Action::Selector(View::Overview)),
            KeyCode::Char('2') => Some(Action::Selector(View::Wallets)),
            KeyCode::Char('3') => Some(Action::Selector(View::Tokens)),
            KeyCode::Char('4') => Some(Action::Selector(View::Activity)),
            KeyCode::Char('5') => Some(Action::Selector(View::Network)),
            KeyCode::Char('6') => Some(Action::Selector(View::Logs)),
            KeyCode::Down | KeyCode::Char('j' | 'J') => Some(Action::Navigate(Direction::Down)),
            KeyCode::Up | KeyCode::Char('k' | 'K') => Some(Action::Navigate(Direction::Up)),
            KeyCode::PageDown => Some(Action::Move(10)),
            KeyCode::PageUp => Some(Action::Move(-10)),
            KeyCode::Left => Some(Action::Selector(
                View::ALL[(self.view as usize).saturating_sub(1)],
            )),
            KeyCode::Char('h' | 'H') => Some(Action::Navigate(Direction::Left)),
            KeyCode::Right => Some(Action::Selector(
                View::ALL[(self.view as usize + 1).min(View::ALL.len() - 1)],
            )),
            KeyCode::Char('l' | 'L') => Some(Action::Navigate(Direction::Right)),
            KeyCode::Enter => Some(Action::Activate),
            KeyCode::Char('c') if self.view == View::Tokens => Some(Action::TokenCreation),
            KeyCode::Char('v') if self.view == View::Tokens => Some(Action::ProjectMints),
            KeyCode::Char('m') if self.view == View::Tokens => Some(Action::MintMore),
            KeyCode::Char('a') if self.view == View::Tokens => Some(Action::CreateTokenAccount),
            KeyCode::Char('n') => Some(Action::New),
            KeyCode::Char('i') => Some(Action::Import),
            KeyCode::Char('f') => Some(Action::Fund),
            KeyCode::Char('s') => Some(Action::Send),
            KeyCode::Char('I') => Some(Action::ImportTransaction),
            KeyCode::Char('p') => Some(Action::Profiles),
            KeyCode::Char('r' | 'R') => Some(Action::Refresh),
            KeyCode::Char('t') => Some(Action::Theme),
            KeyCode::Char('m') => Some(Action::Motion),
            KeyCode::Char('o') if self.view == View::Tokens => Some(Action::ExplorerToken(false)),
            KeyCode::Char('o') => Some(Action::ExplorerTransaction),
            KeyCode::Char('y') if self.view == View::Tokens => Some(Action::CopyToken(false)),
            KeyCode::Char('M') if self.view == View::Tokens => Some(Action::CopyToken(true)),
            KeyCode::Char('O') if self.view == View::Tokens => Some(Action::ExplorerToken(true)),
            KeyCode::Char('E') if self.view == View::Tokens => Some(Action::ExportToken),
            KeyCode::Char('y') if self.view == View::Wallets => {
                Some(Action::CopyWallet(self.wallet_cursor))
            }
            KeyCode::Char('y') => Some(Action::CopyAddress),
            KeyCode::Char('F') => Some(Action::Follow),
            KeyCode::Char('C') => Some(Action::ClearLogs),
            KeyCode::Char('/') => Some(Action::Search),
            KeyCode::Char('x') => Some(Action::ClearFilter),
            KeyCode::Char('e') => Some(Action::Failures),
            KeyCode::Char('?') => Some(Action::Help),
            KeyCode::Char('z') => Some(Action::OpenSummary),
            KeyCode::Char(']') => Some(Action::NextWallet),
            KeyCode::Char('b') => Some(Action::Older),
            KeyCode::Esc => Some(Action::Close),
            _ => None,
        }
    }

    pub fn navigate(&mut self, action: &Action) -> bool {
        match *action {
            Action::Focus(pane) => {
                if pane == Pane::Tokens || self.view != View::Overview {
                    self.switch_view(View::for_pane(pane));
                }
                self.selector_focus = false;
                if self.pane != pane {
                    self.focused_control = None;
                }
                self.pane = pane;
            }
            Action::Selector(view) => {
                self.switch_view(view);
                self.selector_focus = true;
            }
            Action::Move(delta) => match self.pane {
                Pane::Tokens => {
                    self.token_cursor =
                        move_index(self.token_cursor, delta, self.visible_tokens().len());
                    self.focused_control = Some(Action::SelectToken(self.token_cursor));
                }
                Pane::Wallets => {
                    self.wallet_cursor = move_index(self.wallet_cursor, delta, self.wallets.len());
                    self.focused_control = Some(Action::SelectWallet(self.wallet_cursor));
                }
                Pane::Wallet => {
                    self.transaction_cursor =
                        move_index(self.transaction_cursor, delta, self.visible_records().len());
                    self.focused_control = Some(Action::SelectTransaction(self.transaction_cursor));
                }
                Pane::Logs => {
                    let index = move_index(self.log_scroll, delta, self.visible_logs().len());
                    self.focus_log(index);
                }
                Pane::Network => {
                    self.network_scroll =
                        (i32::from(self.network_scroll) + delta).clamp(0, 18) as u16
                }
            },
            Action::Scroll(delta) => match &mut self.modal {
                Some(Modal::Confidential { account, selected }) => {
                    *selected = move_index(
                        *selected,
                        delta,
                        crate::confidential::choices(account, self.confidential_balances.is_some())
                            .len(),
                    )
                }
                Some(Modal::TokenCreation { selected }) => {
                    *selected = move_index(*selected, delta, 3)
                }
                Some(Modal::ProjectMints { selected }) => {
                    *selected = move_index(*selected, delta, self.project_mints.len())
                }
                Some(Modal::Appearance { kind, selected }) => {
                    *selected = move_index(*selected, delta, kind.choices().len());
                }
                Some(Modal::Funding { selected, .. }) => {
                    *selected = move_index(*selected, delta, 3)
                }
                Some(Modal::Form(form)) => {
                    form.active = move_index(form.active, delta, form.fields.len());
                }
                Some(Modal::Profiles { selected }) => {
                    *selected = move_index(*selected, delta, self.config.profiles.len())
                }
                Some(
                    Modal::Mint { scroll, .. }
                    | Modal::Token { scroll, .. }
                    | Modal::Log { scroll, .. }
                    | Modal::Wallet { scroll, .. }
                    | Modal::Inspect { scroll, .. }
                    | Modal::Review { scroll, .. }
                    | Modal::Help { scroll },
                ) => {
                    *scroll =
                        (*scroll as i32 + delta).clamp(0, i32::from(self.modal_scroll_limit)) as u16
                }
                _ => return self.navigate(&Action::Move(delta)),
            },
            Action::Choice(index, delta) => {
                if let Some(Modal::Form(form)) = &mut self.modal
                    && let Some(field) = form.fields.get_mut(index)
                {
                    field.cycle(delta);
                    form.active = index;
                    form.error = None;
                }
            }
            Action::Field(index) => {
                if let Some(Modal::Form(form)) = &mut self.modal {
                    form.active = index.min(form.fields.len() - 1);
                    form.error = None;
                }
            }
            Action::SelectTransaction(index) => {
                self.transaction_cursor = index;
                self.pane = Pane::Wallet;
            }
            Action::Follow => {
                if self.follow {
                    self.pause_logs();
                } else {
                    self.resume_logs();
                }
                self.log_scroll = 0;
                self.status = if self.follow {
                    "Following live logs"
                } else {
                    "Log follow paused"
                }
                .into();
            }
            Action::ClearLogs => {
                self.logs.clear();
                if let Some(entries) = &mut self.paused_logs {
                    entries.clear();
                }
                self.log_scroll = 0;
                self.status = "Visible session log cleared".into();
            }
            Action::ClearFilter => {
                if self.view == View::Tokens {
                    self.token_filter.clear();
                    self.token_cursor = 0;
                } else if self.view == View::Logs {
                    self.log_filter.clear();
                    self.log_scroll = 0;
                } else {
                    self.filter.clear();
                }
                self.transaction_cursor = 0;
                self.status = "Search cleared".into();
            }
            Action::Failures => {
                self.failures_only = !self.failures_only;
                self.switch_view(View::Activity);
                self.pane = Pane::Wallet;
                self.status = if self.failures_only {
                    format!(
                        "Errors filter on · {} matching failed transactions · [e] shows all",
                        self.visible_records().len()
                    )
                } else {
                    format!(
                        "Errors filter off · {} matching transactions",
                        self.visible_records().len()
                    )
                };
                self.transaction_cursor = 0;
            }
            Action::Help => self.modal = Some(Modal::Help { scroll: 0 }),
            Action::OpenSummary => {
                self.switch_view(if self.view == View::Overview {
                    View::for_pane(self.pane)
                } else {
                    View::Overview
                });
                self.selector_focus = false;
            }
            Action::Expand(pane) => {
                self.switch_view(View::for_pane(pane));
                self.selector_focus = false;
            }
            Action::Close => {
                if matches!(
                    self.modal,
                    Some(Modal::Confidential { .. })
                        | Some(Modal::Form(Form {
                            kind: FormKind::Confidential(_),
                            ..
                        }))
                ) {
                    self.confidential_request = self.confidential_request.wrapping_add(1);
                    self.busy = None;
                }
                self.confidential_balances = None;
                self.modal = None;
            }
            _ => return false,
        }
        true
    }
}

fn move_index(index: usize, delta: i32, count: usize) -> usize {
    (index as i64 + i64::from(delta)).clamp(0, count.saturating_sub(1) as i64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_logs_are_chronological_and_pause_freezes_visible_entries() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        for timestamp in [20, 5, 10] {
            app.push_log(LogEntry {
                timestamp,
                level: "INFO".into(),
                message: timestamp.to_string(),
            });
        }
        assert_eq!(
            app.log_entries()
                .iter()
                .map(|entry| entry.timestamp)
                .collect::<Vec<_>>(),
            vec![5, 10, 20]
        );
        app.navigate(&Action::Follow);
        app.push_log(LogEntry {
            timestamp: 30,
            level: "INFO".into(),
            message: "new".into(),
        });
        assert_eq!(app.log_entries().back().unwrap().timestamp, 20);
        assert_eq!(app.logs.back().unwrap().timestamp, 30);
        app.navigate(&Action::Follow);
        assert_eq!(app.log_entries().back().unwrap().timestamp, 30);
    }

    #[test]
    fn refresh_feedback_covers_pending_completion_error_and_reduced_motion() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        app.refresh = RefreshState::Pending(Instant::now());
        assert!(app.refresh_label().starts_with("Refreshing"));
        app.config.reduced_motion = true;
        assert_eq!(app.refresh_label(), "Refreshing…");
        app.finish_refresh(Ok(()));
        assert_eq!(app.refresh_label(), "Refreshed ✓");
        assert!(app.status.contains("RPC"));
        app.offline = true;
        app.finish_refresh(Ok(()));
        assert_eq!(app.status, "Cached history reloaded");
        app.finish_refresh(Err("RPC unreachable".into()));
        assert_eq!(app.refresh_label(), "Failed [r]");
        assert!(app.status.contains("RPC unreachable"));
        app.refresh = RefreshState::Finished {
            at: Instant::now() - std::time::Duration::from_secs(5),
            success: true,
        };
        assert_eq!(app.refresh_label(), "Refresh [r]");
    }

    #[test]
    fn horizontal_arrows_switch_views_but_remain_local_in_dialogs() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(app.key(right), Some(Action::Selector(View::Wallets)));
        app.switch_view(View::Network);
        assert_eq!(app.key(right), Some(Action::Selector(View::Logs)));
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
            Some(Action::Navigate(Direction::Right))
        );
        app.open_form(FormKind::Import);
        app.paste("test");
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            None
        );
        assert!(matches!(&app.modal, Some(Modal::Form(form)) if form.fields[0].cursor == 3));
        assert_eq!(app.view, View::Network);
        app.modal = Some(Modal::Profiles { selected: 0 });
        assert_eq!(app.key(right), None);
    }

    #[test]
    fn views_restore_focus_without_losing_filters_or_scroll() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        app.switch_view(View::Activity);
        app.focused_control = Some(Action::Failures);
        app.filter = "transfer".into();
        app.transaction_cursor = 4;
        app.switch_view(View::Network);
        app.network_scroll = 3;
        app.switch_view(View::Activity);
        assert_eq!(app.focused_control, Some(Action::Failures));
        assert_eq!(app.transaction_cursor, 4);
        assert_eq!(app.filter, "transfer");
        app.switch_view(View::Network);
        assert_eq!(app.network_scroll, 3);
    }

    #[test]
    fn dialog_dismissal_preserves_layout_and_search_can_be_cleared() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        app.switch_view(View::Network);
        app.modal = Some(Modal::Help { scroll: 0 });
        app.navigate(&Action::Close);
        assert!(app.view == View::Network && app.modal.is_none());
        app.navigate(&Action::Close);
        assert_eq!(app.view, View::Network);
        app.filter = "missing".into();
        app.failures_only = true;
        app.navigate(&Action::ClearFilter);
        assert!(app.filter.is_empty());
        assert!(app.failures_only);
    }

    #[test]
    fn errors_filter_opens_activity_and_restores_all_records() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        crate::demo::populate(&mut app);
        let total = app.records.len();
        app.navigate(&Action::Failures);
        assert_eq!(app.view, View::Activity);
        assert_eq!(app.pane, Pane::Wallet);
        assert!(
            app.visible_records()
                .iter()
                .all(|record| record.error.is_some())
        );
        assert!(app.visible_records().len() < total);
        app.navigate(&Action::Failures);
        assert_eq!(app.visible_records().len(), total);
        app.records.clear();
        app.navigate(&Action::Failures);
        assert!(app.status.contains("0 matching failed"));
    }

    #[test]
    fn closing_confidential_work_cancels_pending_results_and_clears_balances() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        app.open_form(FormKind::Confidential(
            crate::confidential_operations::Operation::Apply,
        ));
        app.busy = Some("Preparing proofs".into());
        let request = app.confidential_request;
        app.navigate(&Action::Close);
        assert!(app.busy.is_none() && app.modal.is_none());
        assert_ne!(app.confidential_request, request);
    }

    #[test]
    fn text_input_handles_unicode_and_does_not_trigger_shortcuts() {
        let mut form = Form::new(FormKind::Search, 0);
        form.fields[0].insert("猫a");
        form.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        form.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(form.fields[0].value, "a");
        assert_eq!(
            form.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(form.fields[0].value, "qa");
    }

    #[test]
    fn pane_focus_and_empty_lists_are_safe() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        app.pane = Pane::Wallets;
        for expected in [Pane::Wallet, Pane::Network, Pane::Logs, Pane::Wallets] {
            app.navigate(&Action::Focus(expected));
            assert_eq!(app.pane, expected);
            app.navigate(&Action::Move(10));
        }
        assert_eq!(app.wallet_cursor, 0);
    }

    #[test]
    fn vim_and_arrow_keys_navigate_locally_while_tab_changes_panes() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        for (code, direction) in [
            (KeyCode::Char('h'), Direction::Left),
            (KeyCode::Char('H'), Direction::Left),
            (KeyCode::Char('l'), Direction::Right),
            (KeyCode::Char('L'), Direction::Right),
            (KeyCode::Char('j'), Direction::Down),
            (KeyCode::Char('k'), Direction::Up),
        ] {
            assert_eq!(
                app.key(KeyEvent::new(code, KeyModifiers::NONE)),
                Some(Action::Navigate(direction))
            );
        }
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Action::CycleFocus(true))
        );
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::CycleFocus(false))
        );
    }
}
