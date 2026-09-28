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
    Wallets,
    Wallet,
    Network,
    Logs,
}

impl Pane {
    pub const ALL: [Self; 4] = [Self::Wallets, Self::Wallet, Self::Network, Self::Logs];
    pub fn name(self) -> &'static str {
        match self {
            Self::Wallets => "Wallets",
            Self::Wallet => "Activity",
            Self::Network => "Network",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Transactions,
    Settings,
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
    Profile,
    Search,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    BrowserFaucet(crate::funding::Faucet),
    RpcAirdrop,
    ForceQuit,
    Quit,
    Focus(Pane),
    Selector(Pane),
    CycleFocus(bool),
    Navigate(Direction),
    Move(i32),
    Activate,
    SelectWallet(usize),
    SelectTransaction(usize),
    SetTab(Tab),
    New,
    Import,
    Fund,
    Send,
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
    Zoom,
    Expand(Pane),
    Field(usize),
    Scroll(i32),
    NextWallet,
}

pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub cursor: usize,
}

impl Field {
    pub fn new(label: &'static str, value: &str) -> Self {
        Self {
            label,
            value: value.into(),
            cursor: value.len(),
        }
    }
    pub fn insert(&mut self, value: &str) {
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
            ],
            FormKind::Profile => vec![
                Field::new("Profile name", ""),
                Field::new("HTTP RPC endpoint", "http://127.0.0.1:8899"),
                Field::new("WebSocket endpoint", "ws://127.0.0.1:8900"),
            ],
            FormKind::Search => vec![Field::new("Signature, instruction, or error", "")],
        };
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
            FormKind::Profile => "Add RPC profile",
            FormKind::Search => "Filter transactions",
        }
    }
    pub fn submit_label(&self) -> &'static str {
        match self.kind {
            FormKind::New => "Create wallet",
            FormKind::Import => "Import wallet",
            FormKind::Fund => "Request airdrop",
            FormKind::Transfer => "Simulate & review",
            FormKind::Profile => "Save profile",
            FormKind::Search => "Apply filter",
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            self.fields[self.active].value.clear();
            self.fields[self.active].cursor = 0;
            return None;
        }
        match key.code {
            KeyCode::Esc => return Some(Action::Close),
            KeyCode::Enter => return Some(Action::Submit),
            KeyCode::Tab | KeyCode::Down => self.active = (self.active + 1) % self.fields.len(),
            KeyCode::BackTab | KeyCode::Up => {
                self.active = (self.active + self.fields.len() - 1) % self.fields.len()
            }
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

pub struct App {
    pub root: PathBuf,
    pub config: Config,
    pub wallets: Vec<Wallet>,
    pub selected_wallet: usize,
    pub wallet_cursor: usize,
    pub pane: Pane,
    pub focused_control: Option<Action>,
    pub selector_focus: bool,
    pub tab: Tab,
    pub records: Vec<TransactionRecord>,
    pub transaction_cursor: usize,
    pub filter: String,
    pub failures_only: bool,
    pub history_loading: bool,
    pub offline: bool,
    pub modal_scroll_limit: u16,
    pub logs: VecDeque<LogEntry>,
    pub log_scroll: usize,
    pub follow: bool,
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
    pub zoomed: bool,
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
            focused_control: None,
            selector_focus: false,
            tab: Tab::Overview,
            records: Vec::new(),
            transaction_cursor: 0,
            filter: String::new(),
            failures_only: false,
            history_loading: false,
            offline: false,
            modal_scroll_limit: 0,
            logs: VecDeque::new(),
            log_scroll: 0,
            follow: true,
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
            zoomed: false,
            demo: false,
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
    pub fn visible_records(&self) -> Vec<&TransactionRecord> {
        let filter = self.filter.to_lowercase();
        self.records
            .iter()
            .filter(|r| {
                (!self.failures_only || r.error.is_some())
                    && (filter.is_empty()
                        || r.signature.to_lowercase().contains(&filter)
                        || r.kind().to_lowercase().contains(&filter)
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
    pub fn push_log(&mut self, entry: LogEntry) {
        if self
            .logs
            .back()
            .is_some_and(|last| last.message == entry.message && last.level == entry.level)
        {
            return;
        }
        self.logs.push_back(entry);
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
                KeyCode::Char('y') if matches!(modal, Modal::Funding { .. }) => {
                    Some(Action::CopyAddress)
                }
                KeyCode::Tab
                    if matches!(modal, Modal::Funding { .. } | Modal::Appearance { .. }) =>
                {
                    Some(Action::Scroll(1))
                }
                KeyCode::BackTab
                    if matches!(modal, Modal::Funding { .. } | Modal::Appearance { .. }) =>
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
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::Close),
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
            KeyCode::Char('1') => Some(Action::Focus(Pane::Wallets)),
            KeyCode::Char('2') => Some(Action::Focus(Pane::Wallet)),
            KeyCode::Char('3') => Some(Action::Focus(Pane::Network)),
            KeyCode::Char('4') => Some(Action::Focus(Pane::Logs)),
            KeyCode::Down | KeyCode::Char('j' | 'J') => Some(Action::Navigate(Direction::Down)),
            KeyCode::Up | KeyCode::Char('k' | 'K') => Some(Action::Navigate(Direction::Up)),
            KeyCode::PageDown => Some(Action::Move(10)),
            KeyCode::PageUp => Some(Action::Move(-10)),
            KeyCode::Left | KeyCode::Char('h' | 'H') => Some(Action::Navigate(Direction::Left)),
            KeyCode::Right | KeyCode::Char('l' | 'L') => Some(Action::Navigate(Direction::Right)),
            KeyCode::Enter => Some(Action::Activate),
            KeyCode::Char('n') => Some(Action::New),
            KeyCode::Char('i') => Some(Action::Import),
            KeyCode::Char('f') => Some(Action::Fund),
            KeyCode::Char('s') => Some(Action::Send),
            KeyCode::Char('p') => Some(Action::Profiles),
            KeyCode::Char('r') => Some(Action::Refresh),
            KeyCode::Char('t') => Some(Action::Theme),
            KeyCode::Char('m') => Some(Action::Motion),
            KeyCode::Char('o') => Some(Action::ExplorerTransaction),
            KeyCode::Char('y') => Some(Action::CopyAddress),
            KeyCode::Char('F') => Some(Action::Follow),
            KeyCode::Char('C') => Some(Action::ClearLogs),
            KeyCode::Char('/') => Some(Action::Search),
            KeyCode::Char('x') => Some(Action::ClearFilter),
            KeyCode::Char('e') => Some(Action::Failures),
            KeyCode::Char('?') => Some(Action::Help),
            KeyCode::Char('z') => Some(Action::Zoom),
            KeyCode::Char(']') => Some(Action::NextWallet),
            KeyCode::Char('b') => Some(Action::Older),
            KeyCode::Char('v') => Some(Action::SetTab(match self.tab {
                Tab::Overview => Tab::Transactions,
                Tab::Transactions => Tab::Settings,
                Tab::Settings => Tab::Overview,
            })),
            KeyCode::Esc => Some(Action::Close),
            _ => None,
        }
    }

    pub fn navigate(&mut self, action: &Action) -> bool {
        match *action {
            Action::Focus(pane) => {
                self.selector_focus = false;
                if self.pane != pane {
                    self.focused_control = None;
                }
                self.pane = pane;
            }
            Action::CycleFocus(forward) => {
                self.selector_focus = false;
                let index = Pane::ALL.iter().position(|p| *p == self.pane).unwrap_or(0);
                self.pane = Pane::ALL[(index + if forward { 1 } else { 3 }) % 4];
                self.focused_control = None;
            }
            Action::Selector(pane) => {
                if self.pane != pane {
                    self.focused_control = None;
                }
                self.pane = pane;
                self.selector_focus = true;
            }
            Action::SetTab(tab) => {
                self.selector_focus = false;
                self.pane = Pane::Wallet;
                self.tab = tab;
                self.focused_control = Some(Action::SetTab(tab));
            }
            Action::Move(delta) => match self.pane {
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
                    self.follow = false;
                    self.log_scroll = move_index(self.log_scroll, -delta, self.logs.len());
                }
                Pane::Network => {
                    self.network_scroll =
                        (i32::from(self.network_scroll) + delta).clamp(0, 18) as u16
                }
            },
            Action::Scroll(delta) => match &mut self.modal {
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
                    Modal::Inspect { scroll, .. }
                    | Modal::Review { scroll, .. }
                    | Modal::Help { scroll },
                ) => {
                    *scroll =
                        (*scroll as i32 + delta).clamp(0, i32::from(self.modal_scroll_limit)) as u16
                }
                _ => return self.navigate(&Action::Move(delta)),
            },
            Action::Field(index) => {
                if let Some(Modal::Form(form)) = &mut self.modal {
                    form.active = index.min(form.fields.len() - 1);
                }
            }
            Action::SelectTransaction(index) => {
                self.transaction_cursor = index;
                self.pane = Pane::Wallet;
            }
            Action::Follow => {
                self.follow = !self.follow;
                self.log_scroll = 0;
            }
            Action::ClearLogs => {
                self.logs.clear();
                self.log_scroll = 0;
            }
            Action::ClearFilter => {
                self.filter.clear();
                self.transaction_cursor = 0;
                self.status = "Search cleared".into();
            }
            Action::Failures => {
                self.failures_only = !self.failures_only;
                self.pane = Pane::Wallet;
                self.tab = Tab::Transactions;
                self.status = if self.failures_only {
                    format!(
                        "Errors filter on · {} matching failed transactions · e shows all",
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
            Action::Zoom => self.zoomed = !self.zoomed,
            Action::Expand(pane) => {
                self.zoomed = !(self.zoomed && self.pane == pane);
                self.pane = pane;
            }
            Action::Close => {
                if self.modal.take().is_none() {
                    self.zoomed = false;
                }
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
    fn dialog_dismissal_preserves_layout_and_search_can_be_cleared() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        app.zoomed = true;
        app.modal = Some(Modal::Help { scroll: 0 });
        app.navigate(&Action::Close);
        assert!(app.zoomed && app.modal.is_none());
        app.navigate(&Action::Close);
        assert!(!app.zoomed);
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
        assert_eq!(app.tab, Tab::Transactions);
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
    fn focus_cycles_to_every_pane_and_empty_lists_are_safe() {
        let mut app = App::new(PathBuf::new(), Config::default(), vec![]);
        app.pane = Pane::Wallets;
        for expected in [Pane::Wallet, Pane::Network, Pane::Logs, Pane::Wallets] {
            app.navigate(&Action::CycleFocus(true));
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
            (KeyCode::Left, Direction::Left),
            (KeyCode::Char('l'), Direction::Right),
            (KeyCode::Char('L'), Direction::Right),
            (KeyCode::Right, Direction::Right),
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
