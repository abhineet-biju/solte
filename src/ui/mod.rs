mod layout;
mod navigation;
mod short;
pub mod theme;

use std::{fmt::Write, path::Path, time::Instant};

use anyhow::Result;
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table,
        TableState, Wrap,
    },
};
use tachyonfx::{Effect, fx};

use crate::{
    amount::format_sol,
    app::{Action, App, Appearance, Modal, Pane, View},
    model::{clean_text, now, short},
};
use theme::Theme;

pub struct Hit {
    pub area: Rect,
    pub action: Action,
}

pub struct Ui {
    pub hits: Vec<Hit>,
    pub modal_scroll_limit: u16,
    wallets: ListState,
    transactions: TableState,
    effect: Option<Effect>,
    effect_area: Rect,
    last_frame: Instant,
    last_area: Rect,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            hits: vec![],
            modal_scroll_limit: 0,
            wallets: ListState::default(),
            transactions: TableState::default(),
            effect: None,
            effect_area: Rect::default(),
            last_frame: Instant::now(),
            last_area: Rect::default(),
        }
    }
}

impl Ui {
    pub fn hit(&self, x: u16, y: u16) -> Option<Action> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.area.contains((x, y).into()))
            .map(|hit| hit.action.clone())
    }
    pub fn animate(&mut self, app: &App) {
        if !app.config.reduced_motion {
            self.effect = Some(fx::fade_from_fg(Theme::named(&app.config.theme).muted, 160));
            self.last_frame = Instant::now();
        }
    }
    pub fn sync_scroll(&self, app: &mut App) {
        if self.last_area.width < 60 || self.last_area.height < 10 {
            return;
        }
        app.modal_scroll_limit = self.modal_scroll_limit;
        if let Some(
            Modal::Log { scroll, .. }
            | Modal::Wallet { scroll, .. }
            | Modal::Inspect { scroll, .. }
            | Modal::Review { scroll, .. }
            | Modal::Help { scroll },
        ) = &mut app.modal
        {
            *scroll = (*scroll).min(self.modal_scroll_limit);
        }
    }

    pub fn animating(&self) -> bool {
        self.effect.is_some()
    }
    fn target(&mut self, area: Rect, action: Action) {
        if area.width > 0 && area.height > 0 {
            self.hits.push(Hit { area, action });
        }
    }
    fn button(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        text: &str,
        action: Action,
        theme: Theme,
        active: bool,
    ) {
        let style = if active {
            theme.selected_control()
        } else {
            theme.control()
        };
        frame.render_widget(
            Paragraph::new(text)
                .alignment(ratatui::layout::Alignment::Center)
                .style(style),
            area,
        );
        self.target(area, action);
    }
    fn shortcut_button(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        label: &str,
        key: &str,
        action: Action,
        theme: Theme,
    ) {
        self.button(
            frame,
            area,
            &format!("{label} [{key}]"),
            action,
            theme,
            false,
        );
    }

    fn border_button(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        text: &str,
        action: Action,
        theme: Theme,
    ) {
        self.button(frame, area, text, action, theme, false);
    }

    fn welcome(&mut self, frame: &mut Frame, area: Rect, theme: Theme) {
        let inset = u16::from(area.width >= 44);
        let area = area.inner(ratatui::layout::Margin::new(inset, 0));
        let roomy = area.height >= 8;
        let top = area.y + u16::from(roomy);
        frame.render_widget(
            Paragraph::new("Your next project starts here.")
                .style(Style::default().fg(theme.text).bold()),
            Rect::new(area.x, top, area.width, 1),
        );
        frame.render_widget(
            Paragraph::new("Create a wallet or import a Solana keypair.")
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.muted)),
            Rect::new(area.x, top + 1, area.width, 2),
        );
        let y = top + 3 + u16::from(roomy);
        let side_by_side = area.width >= 39;
        self.button(
            frame,
            Rect::new(area.x, y, 18.min(area.width), 1),
            "Create wallet [n]",
            Action::New,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(
                if side_by_side { area.x + 20 } else { area.x },
                y + u16::from(!side_by_side),
                19.min(area.width),
                1,
            ),
            "Import keypair [i]",
            Action::Import,
            theme,
            false,
        );
        if area.height >= 11 {
            frame.render_widget(
                Paragraph::new("Standard key files. Ready for Anchor and the CLI.")
                    .wrap(Wrap { trim: true })
                    .style(Style::default().fg(theme.muted)),
                Rect::new(area.x, y + 3, area.width, 2),
            );
        }
    }

    fn panel(
        &mut self,
        frame: &mut Frame,
        app: &App,
        area: Rect,
        pane: Pane,
        title: &str,
        theme: Theme,
    ) -> Rect {
        let focused = app.pane == pane;
        if focused {
            self.effect_area = area;
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(
                Style::default().fg(if focused && app.view != View::Overview {
                    theme.accent
                } else {
                    theme.border
                }),
            )
            .style(Style::default().bg(theme.panel))
            .title(Line::from(vec![Span::styled(
                format!(" {title} "),
                Style::default().fg(theme.heading).bold(),
            )]));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.target(area, Action::Focus(pane));
        inner
    }

    pub fn draw(&mut self, frame: &mut Frame, app: &App) {
        self.hits.clear();
        self.modal_scroll_limit = 0;
        if app.config.reduced_motion {
            self.effect = None;
        }
        let theme = Theme::named(&app.config.theme);
        let area = frame.area();
        if self.last_area != area {
            self.effect = None;
            self.wallets = ListState::default();
            self.transactions = TableState::default();
            self.last_area = area;
        }
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.bg).fg(theme.text)),
            area,
        );
        if area.width < 60 || area.height < 10 {
            frame.render_widget(
                Paragraph::new(format!("SOLTE\n\nTerminal: {} columns × {} rows\nMinimum: 60 × 10. Resize or reduce the font size.\nPress [q] to quit.", area.width, area.height))
                .style(Style::default().fg(theme.accent)),
                area,
            );
            return;
        }
        if area.height < 24 || area.width < 80 {
            self.short_layout(frame, app, area, theme);
        } else {
            let outer = Layout::vertical([
                Constraint::Length(4),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(area);
            self.header(frame, app, outer[0], theme);
            self.workspace(frame, app, outer[1], theme);
            self.footer(frame, app, outer[2], theme);
        }
        if app.modal.is_some() {
            self.hits.clear();
            self.modal(frame, app, theme);
        }
        if let Some(effect) = &mut self.effect {
            let elapsed = self.last_frame.elapsed();
            effect.process(elapsed.into(), frame.buffer_mut(), self.effect_area);
            if effect.done() {
                self.effect = None;
            }
        }
        self.last_frame = Instant::now();
        self.paint_control_focus(frame, app, theme);
    }

    fn header(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let mark = vec![
            Line::from(Span::styled(
                "█▀▀ █▀█ █   ▀█▀ █▀▀",
                Style::default().fg(theme.heading).bold(),
            )),
            Line::from(Span::styled(
                "▀▀█ █ █ █    █  █▀▀",
                Style::default().fg(theme.accent).bold(),
            )),
            Line::from(Span::styled(
                "▀▀▀ ▀▀▀ ▀▀▀  ▀  ▀▀▀",
                Style::default().fg(theme.heading).bold(),
            )),
        ];
        frame.render_widget(Paragraph::new(mark), Rect::new(area.x + 1, area.y, 20, 3));
        frame.render_widget(
            Paragraph::new(if app.demo {
                "SOLTE · DEMO"
            } else {
                "SOLTE · DEVELOPMENT WALLET"
            })
            .style(Style::default().fg(theme.text).bold()),
            Rect::new(area.x + 24, area.y, area.width.saturating_sub(51), 1),
        );
        if app.view != View::Overview {
            let project = app
                .root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("project");
            frame.render_widget(
                Paragraph::new(format!(
                    "{} · {project}",
                    app.wallet()
                        .map(|wallet| wallet.name.as_str())
                        .unwrap_or("No wallet selected"),
                ))
                .style(Style::default().fg(theme.muted)),
                Rect::new(area.x + 24, area.y + 1, area.width.saturating_sub(46), 1),
            );
        }
        self.button(
            frame,
            Rect::new(area.right() - 21, area.y + 1, 10, 1),
            "Theme [t]",
            Action::Theme,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(area.right() - 10, area.y + 1, 10, 1),
            "Motion [m]",
            Action::Motion,
            theme,
            false,
        );
        let mut x = area.x + 1;
        for (i, view) in View::ALL.iter().enumerate() {
            let label = format!("[{}] {}", i + 1, view.nav_label(area.width));
            let width = label.len() as u16;
            self.button(
                frame,
                Rect::new(x, area.y + 3, width.min(area.right().saturating_sub(x)), 1),
                &label,
                Action::Selector(*view),
                theme,
                app.view == *view,
            );
            x += width + 1;
        }
        if app.view != View::Overview && area.width > 84 {
            let mut name = clean_text(&app.profile().name);
            if Line::from(name.as_str()).width() > 15 {
                while Line::from(name.as_str()).width() > 14 {
                    name.pop();
                }
                name.push('…');
            }
            let label = format!("{name} [p] ▾");
            let width = (Line::from(label.as_str()).width() as u16 + 2).min(25);
            self.button(
                frame,
                Rect::new(area.right() - width - 1, area.y, width, 1),
                &label,
                Action::Profiles,
                theme,
                false,
            );
        }
    }

    fn draw_pane(&mut self, frame: &mut Frame, app: &App, area: Rect, pane: Pane, theme: Theme) {
        match pane {
            Pane::Wallets => self.wallets(frame, app, area, theme),
            Pane::Wallet => self.wallet(frame, app, area, theme),
            Pane::Network => self.network(frame, app, area, theme),
            Pane::Logs => self.logs(frame, app, area, theme),
        }
    }

    fn wallets(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self.panel(frame, app, area, Pane::Wallets, "Wallets", theme);
        let regions = Layout::vertical([Constraint::Min(2), Constraint::Length(5)])
            .margin(1)
            .split(inner);
        if app.wallets.is_empty() {
            frame.render_widget(Paragraph::new("No wallets yet.\n\nCreate a development identity or import an existing keypair.").wrap(Wrap { trim: false }).style(Style::default().fg(theme.muted)), regions[0]);
        } else {
            let items: Vec<_> = app
                .wallets
                .iter()
                .enumerate()
                .map(|(index, wallet)| {
                    let active = index == app.selected_wallet;
                    ListItem::new(vec![
                        Line::from(vec![
                            Span::styled(
                                if active { "● " } else { "○ " },
                                Style::default().fg(if active { theme.green } else { theme.muted }),
                            ),
                            Span::styled(
                                clean_text(&wallet.name),
                                Style::default().fg(if active { theme.green } else { theme.text }),
                            ),
                        ]),
                        Line::from(Span::styled(
                            if wallet.program {
                                "  program · read-only".into()
                            } else {
                                format!("  {}", short(&wallet.address))
                            },
                            Style::default().fg(theme.muted),
                        )),
                        Line::from(""),
                    ])
                })
                .collect();
            self.wallets.select(Some(app.wallet_cursor));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(Style::default().bg(theme.selected))
                    .highlight_symbol("▎"),
                regions[0],
                &mut self.wallets,
            );
            for (row, index) in (self.wallets.offset()..app.wallets.len()).enumerate() {
                let y = regions[0].y + row as u16 * 3;
                if y >= regions[0].bottom() {
                    break;
                }
                self.target(
                    Rect::new(
                        regions[0].x,
                        y,
                        regions[0].width,
                        2.min(regions[0].bottom() - y),
                    ),
                    Action::SelectWallet(index),
                );
            }
        }
        self.button(
            frame,
            Rect::new(regions[1].x, regions[1].y, regions[1].width, 1),
            "New wallet [n]",
            Action::New,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(regions[1].x, regions[1].y + 2, regions[1].width, 1),
            "Import keypair [i]",
            Action::Import,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(regions[1].x, regions[1].y + 4, regions[1].width, 1),
            "Cycle identity []]",
            Action::NextWallet,
            theme,
            false,
        );
    }

    fn wallet(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let title = app
            .wallet()
            .map(|w| format!("Transactions · {}", clean_text(&w.name)))
            .unwrap_or_else(|| "Wallet overview".into());
        let inner = self.panel(frame, app, area, Pane::Wallet, &title, theme);
        let inner = inner.inner(ratatui::layout::Margin::new(1, 0));
        if inner.height < 5 {
            return;
        }
        let body = inner;
        let Some(wallet) = app.wallet() else {
            self.welcome(frame, body, theme);
            return;
        };
        if app.view == View::Activity {
            frame.render_widget(
                Paragraph::new(format!(
                    "{} · {} SOL · {}",
                    clean_text(&wallet.name),
                    app.balance.map(format_sol).unwrap_or_else(|| "—".into()),
                    short(&wallet.address)
                ))
                .style(Style::default().fg(theme.green)),
                Rect::new(body.x, body.y, body.width, 1),
            );
            let mut x = body.x;
            for (label, action) in [
                ("Fund [f]", Action::Fund),
                ("Send [s]", Action::Send),
                ("Copy [y]", Action::CopyAddress),
                ("Explorer", Action::ExplorerWallet),
            ] {
                let width = label.len() as u16 + 2;
                self.button(
                    frame,
                    Rect::new(x, body.y + 1, width, 1),
                    label,
                    action,
                    theme,
                    false,
                );
                x += width + 1;
            }
            self.transactions(
                frame,
                app,
                Rect::new(
                    body.x,
                    body.y + 3,
                    body.width,
                    body.height.saturating_sub(3),
                ),
                theme,
            );
            return;
        }
        if app.view == View::Overview && body.height >= 18 && body.width >= 40 {
            frame.render_widget(
                Paragraph::new(short(&wallet.address)).style(Style::default().fg(theme.muted)),
                Rect::new(body.x, body.y, 16, 1),
            );
            self.button(
                frame,
                Rect::new(body.x + 18, body.y, 10, 1),
                "Copy [y]",
                Action::CopyAddress,
                theme,
                false,
            );
            self.button(
                frame,
                Rect::new(body.x + 30, body.y, 10, 1),
                "Explorer",
                Action::ExplorerWallet,
                theme,
                false,
            );
            frame.render_widget(
                Paragraph::new("AVAILABLE BALANCE").style(Style::default().fg(theme.muted)),
                Rect::new(body.x, body.y + 2, body.width, 1),
            );
            let balance = app.balance.map(format_sol).unwrap_or_else(|| "—".into());
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(balance, Style::default().fg(theme.green).bold()),
                    Span::styled("  SOL", Style::default().fg(theme.green)),
                    Span::styled(
                        if !app.connected && app.balance.is_some() {
                            "  · stale"
                        } else {
                            ""
                        },
                        Style::default().fg(theme.muted),
                    ),
                ])),
                Rect::new(body.x, body.y + 3, body.width, 1),
            );
            frame.render_widget(
                Paragraph::new(format!(
                    "{}  ·  {}",
                    app.profile().name,
                    if wallet.program {
                        "Program identity · read-only"
                    } else {
                        "Development keypair"
                    }
                ))
                .style(Style::default().fg(theme.muted)),
                Rect::new(body.x, body.y + 4, body.width, 1),
            );
            self.button(
                frame,
                Rect::new(body.x, body.y + 6, 16, 1),
                "Fund wallet [f]",
                Action::Fund,
                theme,
                false,
            );
            self.button(
                frame,
                Rect::new(
                    body.x + 18,
                    body.y + 6,
                    14.min(body.width.saturating_sub(18)),
                    1,
                ),
                "Send SOL [s]",
                Action::Send,
                theme,
                false,
            );
            self.transactions(
                frame,
                app,
                Rect::new(body.x, body.y + 9, body.width, body.height - 9),
                theme,
            );
        } else if app.view == View::Overview && body.height >= 10 {
            let balance = app.balance.map(format_sol).unwrap_or_else(|| "—".into());
            frame.render_widget(
                Paragraph::new(format!(
                    "{balance} SOL   ·   {}   ·   {}",
                    app.profile().name,
                    short(&wallet.address)
                ))
                .style(Style::default().fg(theme.green)),
                Rect::new(body.x, body.y, body.width, 1),
            );
            self.target(
                Rect::new(body.x, body.y, body.width, 1),
                Action::CopyAddress,
            );
            self.button(
                frame,
                Rect::new(body.x, body.y + 2, 16, 1),
                "Fund wallet [f]",
                Action::Fund,
                theme,
                false,
            );
            self.button(
                frame,
                Rect::new(body.x + 18, body.y + 2, 14, 1),
                "Send SOL [s]",
                Action::Send,
                theme,
                false,
            );
            self.transactions(
                frame,
                app,
                Rect::new(body.x, body.y + 4, body.width, body.height - 4),
                theme,
            );
        } else {
            self.transactions(frame, app, body, theme);
        }
    }

    fn transactions(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        if area.height < 3 {
            return;
        }
        let visible = app.visible_records();
        let heading = if app.filter.is_empty() {
            format!(
                "{}  ·  {} captured",
                if app.failures_only {
                    "ERRORS"
                } else {
                    "ACTIVITY"
                },
                visible.len()
            )
        } else {
            format!("FILTER  {}", clean_text(&app.filter))
        };
        frame.render_widget(
            Paragraph::new(heading).style(Style::default().fg(theme.accent).bold()),
            Rect::new(area.x, area.y, area.width.saturating_sub(20), 1),
        );
        if area.width > 36 {
            self.button(
                frame,
                Rect::new(area.right() - 20, area.y, 8, 1),
                if app.filter.is_empty() {
                    "Find [/]"
                } else {
                    "Clear [x]"
                },
                if app.filter.is_empty() {
                    Action::Search
                } else {
                    Action::ClearFilter
                },
                theme,
                false,
            );
            self.button(
                frame,
                Rect::new(area.right() - 10, area.y, 10, 1),
                if app.failures_only {
                    "All [e]"
                } else {
                    "Errors [e]"
                },
                Action::Failures,
                theme,
                app.failures_only,
            );
        }
        let table_area = Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(4),
        );
        let address = app.wallet().map(|w| w.address.as_str()).unwrap_or_default();
        let rows: Vec<_> = visible
            .iter()
            .map(|record| {
                let color = theme.transaction_color(record, address);
                let delta = record
                    .balance_change(address)
                    .map(|d| {
                        format!(
                            "{}{}",
                            if d < 0 { "−" } else { "+" },
                            format_sol(d.unsigned_abs().min(u128::from(u64::MAX)) as u64)
                        )
                    })
                    .unwrap_or_else(|| "—".into());
                let age = record
                    .timestamp
                    .map(|t| age(now().saturating_sub(t.max(0) as u64)))
                    .unwrap_or_else(|| "live".into());
                Row::new(vec![
                    Cell::from(if record.error.is_some() { "×" } else { "✓" })
                        .style(Style::default().fg(color)),
                    Cell::from(short(&record.signature)),
                    Cell::from(record.activity(address)).style(Style::default().fg(color)),
                    Cell::from(delta).style(Style::default().fg(color)),
                    Cell::from(age).style(Style::default().fg(theme.muted)),
                ])
            })
            .collect();
        self.transactions.select(if visible.is_empty() {
            None
        } else {
            Some(app.transaction_cursor.min(visible.len() - 1))
        });
        let table = Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Min(10),
                Constraint::Percentage(22),
                Constraint::Length(12),
                Constraint::Length(6),
            ],
        )
        .header(
            Row::new(["", "Signature", "Action", "Δ SOL", "Age"])
                .style(Style::default().fg(theme.muted))
                .bottom_margin(1),
        )
        .row_highlight_style(
            Style::default()
                .bg(theme.selected)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(table, table_area, &mut self.transactions);
        for (row, index) in (self.transactions.offset()..visible.len()).enumerate() {
            let y = table_area.y + 2 + row as u16;
            if y >= table_area.bottom() {
                break;
            }
            self.target(
                Rect::new(table_area.x, y, table_area.width, 1),
                Action::SelectTransaction(index),
            );
        }
        if visible.is_empty() && table_area.height > 3 {
            frame.render_widget(
                Paragraph::new(if app.failures_only {
                    "No matching failed transactions. Press [e] to show all."
                } else if app.connected {
                    "No matching transactions in captured history."
                } else {
                    "Waiting for history. Cached records appear here."
                })
                .style(Style::default().fg(theme.muted))
                .wrap(Wrap { trim: false }),
                Rect::new(
                    table_area.x,
                    table_area.y + 2,
                    table_area.width,
                    table_area.height - 2,
                ),
            );
        }
        if area.height > 4 {
            let y = area.bottom() - 1;
            self.button(
                frame,
                Rect::new(area.x, y, 15.min(area.width), 1),
                "Inspect [Enter]",
                Action::Inspect,
                theme,
                false,
            );
            if area.width > 34 {
                self.button(
                    frame,
                    Rect::new(area.right() - 16, y, 16, 1),
                    if app.history_loading {
                        "Loading…"
                    } else {
                        "Older [b]"
                    },
                    Action::Older,
                    theme,
                    false,
                );
            }
        }
    }

    fn network(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self
            .panel(frame, app, area, Pane::Network, "Network", theme)
            .inner(ratatui::layout::Margin::new(1, 0));
        let n = app.network.as_ref().filter(|_| app.connected);
        let text = |label: &str, value: String, color| {
            Line::from(vec![
                Span::styled(format!("{label:<13}"), Style::default().fg(theme.muted)),
                Span::styled(value, Style::default().fg(color)),
            ])
        };
        let number =
            |value: Option<u64>| value.map(|v| v.to_string()).unwrap_or_else(|| "—".into());
        let lines = vec![
            Line::from(Span::styled(
                n.map(|n| n.cluster.as_str()).unwrap_or(&app.profile().name),
                Style::default().fg(theme.green).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "RPC ENDPOINT",
                Style::default().fg(theme.muted),
            )),
            Line::from(app.profile().display_endpoint()),
            Line::from(""),
            text(
                "Connection",
                if app.connected { "Online" } else { "Offline" }.into(),
                if app.connected {
                    theme.green
                } else {
                    theme.accent
                },
            ),
            text(
                "Node health",
                n.map(|n| if n.healthy { "Healthy" } else { "Unavailable" })
                    .unwrap_or("—")
                    .into(),
                theme.text,
            ),
            text(
                "Latency",
                n.map(|n| format!("{} ms", n.latency_ms))
                    .unwrap_or_else(|| "—".into()),
                theme.text,
            ),
            text("Slot", number(n.map(|n| n.slot)), theme.text),
            text(
                "Block height",
                number(n.map(|n| n.block_height)),
                theme.text,
            ),
            text("Epoch", number(n.map(|n| n.epoch)), theme.text),
            text(
                "Node version",
                n.map(|n| n.version.clone()).unwrap_or_else(|| "—".into()),
                theme.text,
            ),
            Line::from(""),
            Line::from(Span::styled(
                "MONITOR",
                Style::default().fg(theme.accent).bold(),
            )),
            text(
                "Wallet logs",
                app.log_transport().into(),
                if app.subscribed {
                    theme.green
                } else {
                    theme.muted
                },
            ),
            text(
                "Token accts",
                number(n.map(|n| n.token_accounts as u64)),
                theme.text,
            ),
            text("Commitment", "Confirmed".into(), theme.text),
            text(
                "Refreshed",
                app.last_update
                    .map(|t| age(t.elapsed().as_secs()))
                    .unwrap_or_else(|| "—".into()),
                theme.text,
            ),
        ];
        let lines = if inner.height < 14 {
            vec![
                text(
                    "Cluster",
                    n.map(|n| n.cluster.clone())
                        .unwrap_or_else(|| app.profile().name.clone()),
                    theme.accent,
                ),
                text(
                    "RPC",
                    if app.connected {
                        "Connected"
                    } else {
                        "Offline"
                    }
                    .into(),
                    theme.text,
                ),
                text(
                    "Latency",
                    n.map(|n| format!("{} ms", n.latency_ms))
                        .unwrap_or_else(|| "—".into()),
                    theme.text,
                ),
                text("Slot", number(n.map(|n| n.slot)), theme.text),
                text("Block", number(n.map(|n| n.block_height)), theme.text),
                text("Epoch", number(n.map(|n| n.epoch)), theme.text),
                text("Wallet logs", app.log_transport().into(), theme.text),
                Line::from(app.profile().display_endpoint()),
            ]
        } else {
            lines
        };
        let content = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(1),
        );
        let paragraph = Paragraph::new(lines)
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: false });
        let limit = paragraph
            .line_count(content.width)
            .saturating_sub(content.height as usize) as u16;
        frame.render_widget(
            paragraph.scroll((app.network_scroll.min(limit), 0)),
            content,
        );
        self.button(
            frame,
            Rect::new(inner.x, inner.bottom() - 1, 18, 1),
            "RPC profiles [p]",
            Action::Profiles,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(inner.x + 20, inner.bottom() - 1, 13, 1),
            &app.refresh_label(),
            Action::Refresh,
            theme,
            false,
        );
    }

    fn logs(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self
            .panel(frame, app, area, Pane::Logs, "Logs", theme)
            .inner(ratatui::layout::Margin::new(1, 0));
        if area.width > 44 {
            self.border_button(
                frame,
                Rect::new(area.right() - 30, area.y, 17, 1),
                if app.follow {
                    "Following [F]"
                } else {
                    "Paused [F]"
                },
                Action::Follow,
                theme,
            );
            self.border_button(
                frame,
                Rect::new(area.right() - 12, area.y, 10, 1),
                "Clear [C]",
                Action::ClearLogs,
                theme,
            );
        }
        let query = if app.view == View::Logs {
            app.log_filter.to_lowercase()
        } else {
            String::new()
        };
        let entries = app.visible_logs();
        let inner = if app.view == View::Logs {
            self.button(
                frame,
                Rect::new(inner.x, inner.bottom() - 1, 10, 1),
                if query.is_empty() {
                    "Find [/]"
                } else {
                    "Clear [x]"
                },
                if query.is_empty() {
                    Action::Search
                } else {
                    Action::ClearFilter
                },
                theme,
                false,
            );
            frame.render_widget(
                Paragraph::new(if query.is_empty() {
                    format!("{} entries · newest first", entries.len())
                } else {
                    format!(
                        "Filter: {} · {} matches",
                        clean_text(&app.log_filter),
                        entries.len()
                    )
                })
                .style(Style::default().fg(theme.muted)),
                Rect::new(
                    inner.x + 12,
                    inner.bottom() - 1,
                    inner.width.saturating_sub(12),
                    1,
                ),
            );
            Rect::new(
                inner.x,
                inner.y,
                inner.width,
                inner.height.saturating_sub(1),
            )
        } else {
            inner
        };
        if entries.is_empty() {
            frame.render_widget(
                Paragraph::new(if query.is_empty() {
                    "Ready. Wallet actions and connection events appear here."
                } else {
                    "No matching logs. Press [x] to clear the filter."
                })
                .style(Style::default().fg(theme.muted)),
                inner,
            );
            return;
        }
        let detailed = app.view == View::Logs && inner.height >= 4;
        let row_height = if detailed { 3 } else { 1 };
        let count = (inner.height / row_height).max(1) as usize;
        let cursor = app.log_scroll.min(entries.len() - 1);
        let start = cursor.saturating_sub(count - 1);
        for (row, (index, entry)) in entries
            .iter()
            .enumerate()
            .skip(start)
            .take(count)
            .enumerate()
        {
            let y = inner.y + row as u16 * row_height;
            let seconds = entry.timestamp % 86_400;
            let time = format!(
                "{:02}:{:02}:{:02} UTC",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            );
            let color = theme.log_color(&entry.level);
            let rect = Rect::new(inner.x, y, inner.width, if detailed { 2 } else { 1 });
            let header = Line::from(vec![
                Span::styled(format!("{time}  "), Style::default().fg(theme.muted)),
                Span::styled(
                    format!("{:5}  ", entry.level),
                    Style::default().fg(color).bold(),
                ),
                Span::styled(
                    if detailed {
                        if entry.signature().is_some() {
                            "Signature available · [Enter] details".into()
                        } else {
                            "[Enter] details".into()
                        }
                    } else {
                        clean_text(&entry.message).replace('\n', " · ")
                    },
                    Style::default().fg(if detailed { theme.muted } else { color }),
                ),
            ]);
            let mut lines = vec![header];
            if detailed {
                lines.push(Line::from(Span::styled(
                    clean_text(&entry.message).replace('\n', " · "),
                    Style::default().fg(color),
                )));
            }
            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(
                    if index == cursor && app.pane == Pane::Logs {
                        theme.selected
                    } else {
                        theme.panel
                    },
                )),
                rect,
            );
            self.target(rect, Action::SelectLog(index));
            if detailed && y + 2 < inner.bottom() {
                frame.render_widget(
                    Paragraph::new("─".repeat(inner.width as usize))
                        .style(Style::default().fg(theme.border)),
                    Rect::new(inner.x, y + 2, inner.width, 1),
                );
            }
        }
    }

    fn footer(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let status = app
            .busy
            .as_ref()
            .map(|message| format!("◌ {message}"))
            .unwrap_or_else(|| app.status.clone());
        frame.render_widget(
            Paragraph::new(format!("  {}", clean_text(&status))).style(Style::default().fg(
                if app.busy.is_some() {
                    theme.accent
                } else {
                    theme.muted
                },
            )),
            Rect::new(area.x, area.y, area.width, 1),
        );
        let footer = Line::from(if area.width < 110 {
            "[←/→] Views · [hjkl] Move · [Tab] Focus"
        } else {
            "[Tab] Focus · [←/→] Views · [hjkl] Move · [Enter] Select · [z] Overview"
        });
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(theme.muted)),
            Rect::new(area.x, area.y + 2, area.width.saturating_sub(24), 1),
        );
        if area.width >= 48 {
            self.button(
                frame,
                Rect::new(area.right() - 23, area.y + 2, 10, 1),
                "Quit [q]",
                Action::Quit,
                theme,
                false,
            );
        }
        if area.width >= 48 {
            self.button(
                frame,
                Rect::new(area.right() - 12, area.y + 2, 11, 1),
                "Help [?]",
                Action::Help,
                theme,
                false,
            );
        }
    }

    fn modal(&mut self, frame: &mut Frame, app: &App, theme: Theme) {
        let screen = frame.area();
        let modal = app.modal.as_ref().unwrap();
        let height = match modal {
            Modal::Appearance { .. } => 10,
            Modal::Funding { .. } => 15,
            Modal::Form(form) => (form.fields.len() * 3 + 10) as u16,
            Modal::Profiles { .. } => (app.config.profiles.len() * 2 + 7) as u16,
            _ => screen
                .height
                .saturating_sub(if screen.height < 24 { 2 } else { 6 }),
        };
        let width = if matches!(modal, Modal::Inspect { .. } | Modal::Review { .. }) {
            104
        } else {
            72
        };
        let area = centered(screen, width, height);
        self.effect_area = area;
        frame.render_widget(Clear, area);
        let title = match modal {
            Modal::Appearance {
                kind: Appearance::Theme,
                ..
            } => "Choose theme",
            Modal::Appearance {
                kind: Appearance::Motion,
                ..
            } => "Choose motion",
            Modal::Funding { .. } => "Fund Devnet wallet",
            Modal::Form(form) => form.title(),
            Modal::Profiles { .. } => "RPC profiles",
            Modal::Log { .. } => "Log details",
            Modal::Wallet { .. } => "Wallet details",
            Modal::Inspect { .. } => "Transaction inspector",
            Modal::Review { .. } => "Review SOL transfer",
            Modal::Help { .. } => "Solte keyboard & mouse",
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(format!(" {title} "))
            .title_top(Line::from(Span::styled(" [Esc] × ", theme.control())).right_aligned())
            .border_style(Style::default().fg(theme.accent))
            .style(Style::default().bg(theme.panel).fg(theme.text));
        let inner = block.inner(area).inner(ratatui::layout::Margin::new(1, 0));
        frame.render_widget(block, area);
        self.target(Rect::new(area.right() - 10, area.y, 9, 1), Action::Close);
        match modal {
            Modal::Appearance { kind, selected } => {
                for (index, (id, label)) in kind.choices().iter().enumerate() {
                    let palette = if *kind == Appearance::Theme {
                        Theme::named(id)
                    } else {
                        theme
                    };
                    self.button(
                        frame,
                        Rect::new(inner.x, inner.y + index as u16, inner.width, 1),
                        &format!(
                            "{} {}{}",
                            if index == *selected { "›" } else { " " },
                            label,
                            if index == kind.current(&app.config) {
                                "  (current)"
                            } else {
                                ""
                            }
                        ),
                        Action::SelectAppearance(*kind, index),
                        palette,
                        index == *selected,
                    );
                }
                frame.render_widget(
                    Paragraph::new(
                        "[↑]/[↓] or [j]/[k] · [Enter] applies\nClick to apply · [Esc] cancels",
                    )
                    .style(Style::default().fg(theme.muted)),
                    Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 2),
                );
            }
            Modal::Funding { selected, reason } => {
                if let Some(wallet) = app.wallet() {
                    frame.render_widget(
                        Paragraph::new(format!("{}  [copy]", wallet.address))
                            .style(Style::default().fg(theme.muted)),
                        Rect::new(inner.x, inner.y, inner.width, 1),
                    );
                    self.target(
                        Rect::new(inner.x, inner.y, inner.width, 1),
                        Action::CopyAddress,
                    );
                }
                for (index, (label, action)) in [
                    (
                        "[1] Solana faucet · address prefilled",
                        Action::BrowserFaucet(crate::funding::Faucet::Solana),
                    ),
                    (
                        "[2] Quicknode faucet · copy address",
                        Action::BrowserFaucet(crate::funding::Faucet::Quicknode),
                    ),
                    ("[3] Request through current RPC", Action::RpcAirdrop),
                ]
                .into_iter()
                .enumerate()
                {
                    self.button(
                        frame,
                        Rect::new(inner.x, inner.y + 1 + index as u16, inner.width, 1),
                        label,
                        action,
                        theme,
                        *selected == index,
                    );
                }
                let message = reason.as_ref().map(|reason| format!("RPC request failed: {}\nUse a web faucet, or check your balance before retrying an uncertain request.", clean_text(reason)))
                    .unwrap_or_else(|| "Web faucets have their own limits and verification. Finish the request in your browser; Solte watches for the funds.\n[↑]/[↓] or [j]/[k] selects · [Enter] opens · [Esc] cancels".into());
                frame.render_widget(
                    Paragraph::new(message)
                        .wrap(Wrap { trim: false })
                        .style(Style::default().fg(if reason.is_some() {
                            theme.accent
                        } else {
                            theme.muted
                        })),
                    Rect::new(
                        inner.x,
                        inner.y + 4,
                        inner.width,
                        inner.height.saturating_sub(4),
                    ),
                );
            }
            Modal::Form(form) if screen.height < 20 => self.short_form(frame, form, inner, theme),
            Modal::Form(form) => {
                let capacity = (inner.height.saturating_sub(8) / 3).max(1) as usize;
                let offset = form.active.saturating_sub(capacity - 1);
                for (row, (index, field)) in form
                    .fields
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(capacity)
                    .enumerate()
                {
                    let y = inner.y + row as u16 * 3 + 1;
                    if y + 1 >= inner.bottom() {
                        break;
                    }
                    frame.render_widget(
                        Paragraph::new(field.label).style(Style::default().fg(theme.muted)),
                        Rect::new(inner.x, y, inner.width, 1),
                    );
                    let active = index == form.active;
                    let visible: String = field.value[..field.cursor]
                        .chars()
                        .rev()
                        .take(inner.width.saturating_sub(3) as usize)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    let value = format!(
                        " {visible}{}{}",
                        if active { "▏" } else { "" },
                        &field.value[field.cursor..]
                    );
                    let field_area = Rect::new(inner.x, y + 1, inner.width, 1);
                    frame.render_widget(
                        Paragraph::new(value).style(theme.input(active)),
                        field_area,
                    );
                    self.target(field_area, Action::Field(index));
                }
                if capacity < form.fields.len() {
                    let y = inner.bottom().saturating_sub(6);
                    self.button(
                        frame,
                        Rect::new(inner.x, y, 10, 1),
                        "Previous",
                        Action::Field(form.active.saturating_sub(1)),
                        theme,
                        false,
                    );
                    self.button(
                        frame,
                        Rect::new(inner.right() - 10, y, 10, 1),
                        "Next",
                        Action::Field((form.active + 1).min(form.fields.len() - 1)),
                        theme,
                        false,
                    );
                }
                let note = form.error.as_deref().unwrap_or(match form.kind {
                    crate::app::FormKind::Fund => {
                        "Faucet availability and rate limits depend on your RPC."
                    }
                    crate::app::FormKind::New => "Creates a standard keypair in .solte/keys/.",
                    crate::app::FormKind::Import => {
                        "References your existing file without copying its secret."
                    }
                    crate::app::FormKind::Transfer => {
                        "Review the simulation before signing and submission."
                    }
                    crate::app::FormKind::Profile => {
                        "Profiles are stored locally in .solte/config.toml."
                    }
                    crate::app::FormKind::Search | crate::app::FormKind::LogSearch => {
                        "Leave empty to show all captured transactions."
                    }
                });
                frame.render_widget(
                    Paragraph::new(note)
                        .style(Style::default().fg(if form.error.is_some() {
                            theme.red
                        } else {
                            theme.muted
                        }))
                        .wrap(Wrap { trim: false }),
                    Rect::new(inner.x, inner.bottom().saturating_sub(5), inner.width, 2),
                );
                self.button(
                    frame,
                    Rect::new(
                        inner.x,
                        inner.bottom().saturating_sub(2),
                        (form.submit_label().len() as u16 + 3).min(inner.width),
                        1,
                    ),
                    form.submit_label(),
                    Action::Submit,
                    theme,
                    false,
                );
            }
            Modal::Profiles { selected } => {
                frame.render_widget(
                    Paragraph::new(format!(
                        "Profile {} / {} · [j]/[k] or wheel to choose",
                        selected + 1,
                        app.config.profiles.len()
                    ))
                    .style(Style::default().fg(theme.muted)),
                    Rect::new(inner.x, inner.y, inner.width, 1),
                );
                let capacity = (inner.height.saturating_sub(4) / 2).max(1) as usize;
                let offset = selected.saturating_sub(capacity - 1);
                for (row, (index, profile)) in app
                    .config
                    .profiles
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .enumerate()
                {
                    let y = inner.y + row as u16 * 2 + 1;
                    if y >= inner.bottom().saturating_sub(3) {
                        break;
                    }
                    self.button(
                        frame,
                        Rect::new(inner.x, y, inner.width, 1),
                        &format!(
                            "{}  {}",
                            if index == app.config.selected_profile {
                                "●"
                            } else {
                                "○"
                            },
                            profile.name
                        ),
                        Action::SelectProfile(index),
                        theme,
                        index == *selected,
                    );
                }
                self.button(
                    frame,
                    Rect::new(
                        inner.x,
                        inner.bottom().saturating_sub(2),
                        24.min(inner.width),
                        1,
                    ),
                    "Add profile [a]",
                    Action::AddProfile,
                    theme,
                    false,
                );
            }
            Modal::Log { entry, scroll } => {
                let mut lines = vec![
                    format!("{} · Unix timestamp {}", entry.level, entry.timestamp),
                    String::new(),
                ];
                lines.extend(entry.message.lines().map(clean_text));
                self.inspection_body(frame, inner, lines, *scroll, theme);
                self.button(
                    frame,
                    Rect::new(inner.x, inner.bottom() - 1, 14, 1),
                    "Copy log [y]",
                    Action::CopyLog,
                    theme,
                    false,
                );
                if entry.signature().is_some() {
                    self.button(
                        frame,
                        Rect::new(inner.x + 15, inner.bottom() - 1, 15, 1),
                        "Signature [s]",
                        Action::CopySignature,
                        theme,
                        false,
                    );
                    self.button(
                        frame,
                        Rect::new(
                            inner.x + 31,
                            inner.bottom() - 1,
                            14.min(inner.width.saturating_sub(31)),
                            1,
                        ),
                        "Explorer [o]",
                        Action::ExplorerTransaction,
                        theme,
                        false,
                    );
                }
            }
            Modal::Wallet { index, scroll } => {
                if let Some(wallet) = app.wallets.get(*index) {
                    let lines = vec![
                        clean_text(&wallet.name),
                        if *index == app.selected_wallet {
                            "Active wallet".into()
                        } else {
                            "Preview only · active wallet unchanged".into()
                        },
                        "PUBLIC ADDRESS".into(),
                        wallet.address.clone(),
                        String::new(),
                        if wallet.program {
                            "Program identity · read-only".into()
                        } else {
                            "Standard Solana keypair".into()
                        },
                        String::new(),
                        "KEY FILE".into(),
                        clean_text(&wallet.path.display().to_string()),
                    ];
                    self.inspection_body(frame, inner, lines, *scroll, theme);
                    self.button(
                        frame,
                        Rect::new(inner.x, inner.bottom() - 1, 18, 1),
                        "Copy address [y]",
                        Action::CopyWallet(*index),
                        theme,
                        false,
                    );
                    self.button(
                        frame,
                        Rect::new(
                            inner.x + 20,
                            inner.bottom() - 1,
                            24.min(inner.width.saturating_sub(20)),
                            1,
                        ),
                        "Use wallet [Enter]",
                        Action::ActivateWallet(*index),
                        theme,
                        false,
                    );
                }
            }
            Modal::Inspect { signature, scroll } => {
                let mut lines = app
                    .records
                    .iter()
                    .find(|r| &r.signature == signature)
                    .map(|r| r.inspection_lines())
                    .unwrap_or_else(|| {
                        vec!["Transaction is no longer in the current view.".into()]
                    });
                if inner.height < 12
                    && lines
                        .first()
                        .is_some_and(|line| line.starts_with("Signature  "))
                {
                    lines[0] = format!("Signature  {} · copy [y]", short(signature));
                }
                self.inspection_body(frame, inner, lines, *scroll, theme);
                self.button(
                    frame,
                    Rect::new(inner.x, inner.bottom() - 1, 19, 1),
                    "Explorer [o]",
                    Action::ExplorerTransaction,
                    theme,
                    false,
                );
                self.button(
                    frame,
                    Rect::new(
                        inner.x + 21,
                        inner.bottom() - 1,
                        20.min(inner.width.saturating_sub(21)),
                        1,
                    ),
                    "Copy signature [y]",
                    Action::CopySignature,
                    theme,
                    false,
                );
            }
            Modal::Review { prepared, scroll } => {
                let mut lines = vec![
                    format!("From       {}", prepared.wallet.address),
                    format!("To         {}", prepared.recipient),
                    format!("Network    {}", prepared.profile.name),
                    format!("Amount     {} SOL", format_sol(prepared.lamports)),
                    format!("Fee        {} SOL", format_sol(prepared.fee)),
                    format!(
                        "Compute    {}",
                        prepared
                            .units
                            .map(|v| format!("{v} CU"))
                            .unwrap_or_else(|| "Unavailable".into())
                    ),
                    String::new(),
                    prepared
                        .simulation_error
                        .as_ref()
                        .map(|e| format!("SIMULATION FAILED: {e}"))
                        .unwrap_or_else(|| {
                            "SIMULATION PASSED · not yet signed or submitted".into()
                        }),
                    String::new(),
                ];
                lines.extend(prepared.logs.iter().map(|s| clean_text(s)));
                self.inspection_body(frame, inner, lines, *scroll, theme);
                if prepared.simulation_error.is_none() {
                    self.button(
                        frame,
                        Rect::new(inner.x, inner.bottom() - 1, 26.min(inner.width), 1),
                        "Sign & submit [Enter]",
                        Action::Submit,
                        theme,
                        false,
                    );
                }
            }
            Modal::Help { scroll } => {
                let lines = [
                    "NAVIGATION",
                    "[Tab]/[Shift-Tab]  Move focus within this view",
                    "[1] Overview   [2] Wallets   [3] Transactions",
                    "[4] Network    [5] Logs",
                    "[←]/[→]  Switch main views outside dialogs",
                    "[h][j][k][l] or [↑]/[↓]  Navigate inside a pane",
                    "[k] at the top focuses the main tab bar",
                    "[h]/[l] on the tab bar switches views",
                    "[j] or [Enter] enters the selected view",
                    "[Enter]  Activate the highlighted control",
                    "[z]  Visit focused summary / return to Overview",
                    "",
                    "WALLETS AND NETWORK",
                    "[n] New wallet   [i] Import   []] Cycle wallet",
                    "[y] Copy wallet address; Wallets copies the previewed identity",
                    "Logs: [Enter]/click opens entry; [y] copies its full text",
                    "Log details: [s] copies signature; [o] opens Explorer when available",
                    "Wallets: [Enter]/click opens details; confirm to use wallet",
                    "[f] Funding options   [s] Review and send SOL",
                    "[p] RPC profiles      [r]/[R] Refresh state",
                    "Refresh shows progress, then success or failure.",
                    "Offline refresh reloads cached history only.",
                    "",
                    "TRANSACTIONS AND LOGS",
                    "[/] Search this view   [x] Clear its search",
                    "[e] Open Transactions and toggle failures only",
                    "[b] Load older history   [o] Transaction explorer",
                    "[F] Follow/pause logs    [C] Clear visible logs",
                    "",
                    "APPEARANCE",
                    "[t] Choose theme   [m] Choose motion",
                    "",
                    "DIALOGS AND FORMS",
                    "Type normally in fields; [←]/[→] moves the cursor.",
                    "[Tab]/[Shift-Tab] changes fields or menu choices.",
                    "[j]/[k] or [↑]/[↓] selects menu choices.",
                    "[Enter] applies a choice or submits a form.",
                    "[Esc] cancels without changing the page.",
                    "Inspectors: [j]/[k], [PageUp]/[PageDown], [Home]/[End]",
                    "Inspector [y] copies the signature; [o] opens Explorer.",
                    "[q] closes menus/help or quits from the workspace.",
                    "[Ctrl-C] exits everywhere. [F] and [C] are uppercase.",
                    "",
                    "MOUSE",
                    "Click tabs to switch views and panel headings to focus.",
                    "Click wallet or transaction rows to select or inspect.",
                    "Buttons and fields are clickable; the wheel scrolls.",
                    "",
                    "Public Devnet history may be incomplete. Solte retains",
                    "captured records; it cannot recover pruned records.",
                ];
                self.inspection_body(
                    frame,
                    inner,
                    lines.into_iter().map(str::to_owned).collect(),
                    *scroll,
                    theme,
                );
            }
        }
    }

    fn inspection_body(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        lines: Vec<String>,
        scroll: u16,
        theme: Theme,
    ) {
        let lines: Vec<_> = lines
            .into_iter()
            .map(|line| {
                let color = if line.contains("failed")
                    || line.starts_with("Error")
                    || line.contains("FAILED")
                {
                    theme.red
                } else if line.chars().all(|c| !c.is_lowercase()) && !line.is_empty() {
                    theme.accent
                } else {
                    theme.text
                };
                Line::from(Span::styled(line, Style::default().fg(color)))
            })
            .collect();
        let content = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(2));
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let total = paragraph.line_count(content.width);
        self.modal_scroll_limit = total
            .saturating_sub(content.height as usize)
            .min(u16::MAX as usize) as u16;
        let offset = scroll.min(self.modal_scroll_limit);
        frame.render_widget(paragraph.scroll((offset, 0)), content);
        frame.render_widget(
            Paragraph::new(format!(
                "{}–{} / {} · [Home]/[End]",
                usize::from(offset) + 1,
                (usize::from(offset) + content.height as usize).min(total),
                total
            ))
            .style(Style::default().fg(theme.muted)),
            Rect::new(area.x, area.bottom() - 2, area.width.saturating_sub(13), 1),
        );
        if area.width > 14 {
            self.button(
                frame,
                Rect::new(area.right() - 12, area.bottom() - 2, 5, 1),
                "↑",
                Action::Scroll(-5),
                theme,
                false,
            );
            self.button(
                frame,
                Rect::new(area.right() - 6, area.bottom() - 2, 5, 1),
                "↓",
                Action::Scroll(5),
                theme,
                false,
            );
        }
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn age(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86400)
    }
}

pub fn snapshot(app: &App, path: &Path, width: u16, height: u16) -> Result<()> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    let mut ui = Ui::default();
    terminal.draw(|frame| ui.draw(frame, app))?;
    let buffer = terminal.backend().buffer();
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"#0c1012\"/><g font-family=\"Menlo,DejaVu Sans Mono,monospace\" font-size=\"14\">",
        width as u32 * 9,
        height as u32 * 20,
        width as u32 * 9,
        height as u32 * 20
    );
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            let color = |c: ratatui::style::Color| match c {
                ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                _ => "#d5e0e4".into(),
            };
            if let ratatui::style::Color::Rgb(..) = cell.bg {
                write!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"9\" height=\"20\" fill=\"{}\"/>",
                    x as u32 * 9,
                    y as u32 * 20,
                    color(cell.bg)
                )?;
            }
            if cell.symbol() != " " {
                let text = cell
                    .symbol()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;");
                write!(
                    svg,
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\"{}>{text}</text>",
                    x as u32 * 9,
                    y as u32 * 20 + 15,
                    color(cell.fg),
                    if cell.modifier.contains(Modifier::BOLD) {
                        " font-weight=\"bold\""
                    } else {
                        ""
                    }
                )?;
            }
        }
    }
    svg.push_str("</g></svg>");
    std::fs::write(path, svg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn assert_focus(terminal: &Terminal<TestBackend>, ui: &Ui, action: Action, theme: Theme) {
        let buffer = terminal.backend().buffer();
        let hit = ui
            .hits
            .iter()
            .find(|hit| hit.action == action && buffer[(hit.area.x, hit.area.y)].bg == theme.accent)
            .unwrap();
        for y in buffer.area.y..buffer.area.bottom() {
            for x in buffer.area.x..buffer.area.right() {
                let cell = &buffer[(x, y)];
                if cell.bg == theme.accent {
                    assert!(
                        hit.area.contains((x, y).into()),
                        "Unexpected accent background at {x},{y} for {action:?}"
                    );
                    assert_eq!(cell.fg, theme.bg);
                }
            }
        }
        assert_eq!(buffer[(hit.area.x, hit.area.y)].bg, theme.accent);
    }

    #[test]
    fn disconnected_network_does_not_present_cached_telemetry_as_current() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        crate::demo::populate(&mut app);
        app.connected = false;
        for view in [View::Overview, View::Network] {
            app.switch_view(view);
            for (width, height) in [(90, 22), (140, 42)] {
                let mut ui = Ui::default();
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(text.contains("Offline"));
                assert!(!text.contains("Healthy"));
                assert!(!text.contains("42 ms"));
                assert!(!text.contains("415239881"));
            }
        }
        assert!(
            app.network.is_some(),
            "Keep verified network identity for operation guards"
        );
    }

    #[test]
    fn action_buttons_share_one_shortcut_style_across_themes() {
        for name in ["ember", "glacier", "orchid", "neon"] {
            let mut app = App::new("/test".into(), Config::default(), vec![]);
            crate::demo::populate(&mut app);
            app.config.theme = name.into();
            app.selector_focus = true;
            let theme = Theme::named(name);
            let mut ui = Ui::default();
            let mut terminal = Terminal::new(TestBackend::new(140, 42)).unwrap();
            terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
            for action in [
                Action::New,
                Action::Import,
                Action::Profiles,
                Action::Refresh,
                Action::Follow,
                Action::ClearLogs,
                Action::CopyAddress,
                Action::ExplorerWallet,
            ] {
                for hit in ui.hits.iter().filter(|hit| hit.action == action) {
                    for x in hit.area.x..hit.area.right() {
                        let cell = &terminal.backend().buffer()[(x, hit.area.y)];
                        assert_eq!(cell.bg, theme.selected, "{name} {action:?}");
                        assert_eq!(cell.fg, theme.text, "{name} {action:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn overview_header_omits_context_while_dedicated_views_retain_it() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        crate::demo::populate(&mut app);
        for (width, height) in [(90, 22), (140, 42)] {
            for view in View::ALL {
                app.switch_view(view);
                let mut ui = Ui::default();
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                let header_rows = if height < 24 { 1 } else { 3 };
                let text: String = (0..header_rows)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .map(|pos| terminal.backend().buffer()[pos].symbol())
                    .collect();
                assert_eq!(text.contains("dev.wallet"), view != View::Overview);
                assert_eq!(text.contains("Devnet"), view != View::Overview);
                assert!(text.contains("DEMO"));
                assert!(text.contains("Theme [t]") && text.contains("Motion [m]"));
            }
        }
    }

    #[test]
    fn overview_borders_are_neutral_and_actions_only_wrap_when_necessary() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        crate::demo::populate(&mut app);
        for width in [80, 90, 99, 100] {
            let mut ui = Ui::default();
            let mut terminal = Terminal::new(TestBackend::new(width, 22)).unwrap();
            terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
            for hit in ui
                .hits
                .iter()
                .filter(|hit| matches!(hit.action, Action::Focus(_)))
            {
                assert_eq!(
                    terminal.backend().buffer()[(hit.area.x, hit.area.y)].fg,
                    Theme::named(&app.config.theme).border
                );
            }
            let actions: Vec<_> = [
                Action::Fund,
                Action::Send,
                Action::CopyAddress,
                Action::ExplorerWallet,
            ]
            .into_iter()
            .map(|action| {
                ui.hits
                    .iter()
                    .find(|hit| hit.action == action)
                    .unwrap()
                    .area
            })
            .collect();
            let panel = ui
                .hits
                .iter()
                .find(|hit| hit.action == Action::Focus(Pane::Wallet))
                .unwrap()
                .area;
            if panel.width - 2 >= 35 {
                assert!(
                    actions.iter().all(|area| area.y == actions[0].y),
                    "{width} columns"
                );
            } else {
                assert!(actions.last().unwrap().y > actions[0].y);
            }
            for area in actions {
                assert_eq!(area.intersection(panel), area);
            }
            let network = ui
                .hits
                .iter()
                .find(|hit| hit.action == Action::Focus(Pane::Network))
                .unwrap()
                .area;
            let profiles = ui
                .hits
                .iter()
                .find(|hit| hit.action == Action::Profiles)
                .unwrap()
                .area;
            assert_eq!(profiles.intersection(network), profiles);
            assert!(profiles.intersection(panel).is_empty());
        }
    }

    #[test]
    fn shortcut_hints_remain_bracketed_and_visible_at_every_layout_size() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        crate::demo::populate(&mut app);
        for (width, height) in [(60, 10), (80, 10), (90, 22), (140, 42)] {
            for view in View::ALL {
                app.switch_view(view);
                let mut ui = Ui::default();
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                for hit in &ui.hits {
                    let expected = match hit.action {
                        Action::New => "[n]",
                        Action::Import => "[i]",
                        Action::Fund => "[f]",
                        Action::Send => "[s]",
                        Action::CopyAddress => "[y]",
                        Action::Profiles => "[p]",
                        Action::Theme => "[t]",
                        Action::Motion => "[m]",
                        Action::Search => "[/]",
                        Action::Failures => "[e]",
                        Action::Older => "[b]",
                        Action::Refresh => "[r]",
                        Action::Follow => "[F]",
                        Action::ClearLogs => "[C]",
                        Action::Help => "[?]",
                        Action::Quit => "[q]",
                        _ => continue,
                    };
                    let text: String = (hit.area.y..hit.area.bottom())
                        .flat_map(|y| (hit.area.x..hit.area.right()).map(move |x| (x, y)))
                        .map(|pos| terminal.backend().buffer()[pos].symbol())
                        .collect();
                    assert!(
                        text.contains(expected),
                        "{view:?} {width}x{height}: {:?} rendered {text:?}",
                        hit.action
                    );
                }
            }
        }
    }

    #[test]
    fn undersized_window_preserves_dialog_scroll_until_it_can_render_again() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        app.modal = Some(Modal::Help { scroll: 8 });
        let mut ui = Ui::default();
        for (width, height) in [(90, 22), (45, 8), (90, 22)] {
            Terminal::new(TestBackend::new(width, height))
                .unwrap()
                .draw(|frame| ui.draw(frame, &app))
                .unwrap();
            ui.sync_scroll(&mut app);
            assert!(matches!(app.modal, Some(Modal::Help { scroll: 8 })));
        }
    }

    #[test]
    fn resize_sweep_keeps_controls_in_bounds_and_disjoint() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        crate::demo::populate(&mut app);
        app.config.theme = "neon".into();
        let mut ui = Ui::default();
        for (width, height) in [
            (160, 48),
            (100, 24),
            (99, 23),
            (90, 22),
            (80, 20),
            (80, 10),
            (79, 10),
            (68, 12),
            (67, 12),
            (60, 10),
            (45, 8),
            (60, 10),
            (90, 22),
            (140, 42),
        ] {
            for view in View::ALL {
                app.switch_view(view);
                for modal in 0..9 {
                    app.modal = match modal {
                        0 => None,
                        1 => Some(Modal::Appearance {
                            kind: Appearance::Theme,
                            selected: 3,
                        }),
                        2 => Some(Modal::Profiles { selected: 1 }),
                        3 => Some(Modal::Funding {
                            selected: 0,
                            reason: None,
                        }),
                        4 => Some(Modal::Form(crate::app::Form::new(
                            crate::app::FormKind::Profile,
                            1,
                        ))),
                        5 => Some(Modal::Inspect {
                            signature: app.records[0].signature.clone(),
                            scroll: 100,
                        }),
                        6 => Some(Modal::Wallet {
                            index: 1,
                            scroll: 100,
                        }),
                        7 => Some(Modal::Log {
                            entry: crate::model::LogEntry::new(
                                "ERROR",
                                format!("Failed {}", app.records[0].signature),
                            ),
                            scroll: 100,
                        }),
                        _ => Some(Modal::Help { scroll: 100 }),
                    };
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                    ui.sync_scroll(&mut app);
                    let controls: Vec<_> = ui
                        .hits
                        .iter()
                        .filter(|hit| !matches!(hit.action, Action::Focus(_)))
                        .collect();
                    for (index, hit) in controls.iter().enumerate() {
                        assert!(
                            hit.area.right() <= width && hit.area.bottom() <= height,
                            "{view:?} {modal} {width}x{height} {:?}",
                            hit.action
                        );
                        for other in controls.iter().skip(index + 1) {
                            assert!(
                                hit.area.intersection(other.area).is_empty(),
                                "{view:?} {modal} {width}x{height}: {:?} overlaps {:?}",
                                hit.action,
                                other.action
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn one_focus_highlight_follows_navigation_and_dialog_choices() {
        for name in ["ember", "glacier", "orchid", "neon"] {
            for (width, height) in [(60, 10), (90, 22), (140, 42)] {
                let mut app = App::new("/test".into(), Config::default(), vec![]);
                app.config.theme = name.into();
                app.pane = Pane::Wallet;
                let theme = Theme::named(name);
                let mut ui = Ui::default();
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                app.focused_control = Some(Action::New);
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                assert_focus(&terminal, &ui, Action::New, theme);
                let pane = ui
                    .hits
                    .iter()
                    .find(|hit| hit.action == Action::Focus(app.pane))
                    .unwrap()
                    .area;
                let create = ui
                    .hits
                    .iter()
                    .rev()
                    .find(|hit| {
                        hit.action == Action::New && pane.contains((hit.area.x, hit.area.y).into())
                    })
                    .unwrap()
                    .area;
                let import = ui
                    .hits
                    .iter()
                    .rev()
                    .find(|hit| {
                        hit.action == Action::Import
                            && pane.contains((hit.area.x, hit.area.y).into())
                    })
                    .unwrap()
                    .area;
                let direction = if import.y > create.y {
                    crate::app::Direction::Down
                } else {
                    crate::app::Direction::Right
                };
                ui.navigate_control(&mut app, direction);
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                assert_eq!(ui.focused_action(&app), Some(Action::Import));
                assert_focus(&terminal, &ui, Action::Import, theme);
                app.selector_focus = true;
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                assert_focus(&terminal, &ui, Action::Selector(View::Overview), theme);
                app.modal = Some(Modal::Appearance {
                    kind: Appearance::Theme,
                    selected: 0,
                });
                app.navigate(&Action::Scroll(1));
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                assert_focus(
                    &terminal,
                    &ui,
                    Action::SelectAppearance(Appearance::Theme, 1),
                    theme,
                );
                app.modal = Some(Modal::Profiles { selected: 1 });
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                assert_focus(&terminal, &ui, Action::SelectProfile(1), theme);
                app.open_form(crate::app::FormKind::Import);
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                assert_focus(&terminal, &ui, Action::Field(0), theme);
                app.navigate(&Action::Field(1));
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                assert_focus(&terminal, &ui, Action::Field(1), theme);
            }
        }
    }

    #[test]
    fn inspector_clamps_scroll_and_compact_search_remains_visible() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        crate::demo::populate(&mut app);
        app.modal = Some(Modal::Inspect {
            signature: app.records[3].signature.clone(),
            scroll: 0,
        });
        let mut ui = Ui::default();
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
        ui.sync_scroll(&mut app);
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("copy [y]"));
        assert!(text.contains("Slot"));
        assert!(text.contains("Error"));
        assert!(app.modal_scroll_limit > 0);
        app.navigate(&Action::Scroll(65535));
        app.navigate(&Action::Scroll(65535));
        assert!(
            matches!(app.modal, Some(Modal::Inspect { scroll, .. }) if scroll == app.modal_scroll_limit)
        );
        terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
        app.navigate(&Action::Scroll(-1));
        assert!(
            matches!(app.modal, Some(Modal::Inspect { scroll, .. }) if scroll + 1 == app.modal_scroll_limit)
        );
        app.navigate(&Action::Close);
        app.filter = "missing-query".into();
        terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Filter: missing-query"));
        assert!(ui.hits.iter().any(|hit| hit.action == Action::ClearFilter));
    }

    #[test]
    fn first_run_controls_stay_inside_panels_and_keep_visible_text() {
        for name in ["ember", "glacier", "orchid", "neon"] {
            for (width, height) in [
                (60, 10),
                (80, 10),
                (99, 20),
                (100, 14),
                (100, 24),
                (120, 30),
                (160, 48),
            ] {
                let mut app = App::new("/test".into(), Config::default(), vec![]);
                app.config.theme = name.into();
                app.pane = Pane::Wallet;
                for view in [View::Overview, View::Activity] {
                    app.switch_view(view);
                    let mut ui = Ui::default();
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                    let panel = ui
                        .hits
                        .iter()
                        .find(|hit| hit.action == Action::Focus(Pane::Wallet))
                        .unwrap()
                        .area;
                    for action in [Action::New, Action::Import] {
                        let hit = ui
                            .hits
                            .iter()
                            .find(|hit| {
                                hit.action == action
                                    && panel.contains((hit.area.x, hit.area.y).into())
                            })
                            .unwrap();
                        let inside = panel.inner(ratatui::layout::Margin::new(1, 1));
                        assert_eq!(
                            hit.area.intersection(inside),
                            hit.area,
                            "{width}x{height} {action:?}"
                        );
                        let cell = &terminal.backend().buffer()[(hit.area.x + 1, hit.area.y)];
                        assert_ne!(cell.symbol(), " ");
                        assert_ne!(cell.fg, cell.bg, "{name} {action:?}");
                    }
                    for hit in ui
                        .hits
                        .iter()
                        .filter(|hit| matches!(hit.action, Action::Expand(_)))
                    {
                        assert_eq!(
                            terminal.backend().buffer()[(hit.area.x, hit.area.y)].bg,
                            Theme::named(name).panel
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn layouts_render_at_supported_sizes_and_expose_mouse_actions() {
        for (width, height) in [(160, 48), (110, 32), (80, 24)] {
            let app = App::new("/test/project".into(), Config::default(), vec![]);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut ui = Ui::default();
            terminal.draw(|f| ui.draw(f, &app)).unwrap();
            assert!(
                ui.hits
                    .iter()
                    .any(|h| h.action == Action::Selector(View::Network))
            );
            assert!(
                ui.hits
                    .iter()
                    .all(|h| h.area.right() <= width && h.area.bottom() <= height)
            );
        }
    }

    #[test]
    fn modal_blocks_underlying_mouse_actions() {
        let mut app = App::new("/test/project".into(), Config::default(), vec![]);
        app.open_form(crate::app::FormKind::New);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let mut ui = Ui::default();
        terminal.draw(|f| ui.draw(f, &app)).unwrap();
        assert!(!ui.hits.iter().any(|h| matches!(h.action, Action::Focus(_))));
        assert!(ui.hits.iter().any(|h| h.action == Action::Submit));
        assert!(ui.hits.iter().any(|h| h.action == Action::Field(0)));
    }

    #[test]
    fn appearance_choices_fit_small_dialogs_and_do_not_apply_on_navigation() {
        for (width, height) in [(60, 10), (100, 20), (160, 48)] {
            for kind in [Appearance::Theme, Appearance::Motion] {
                let mut app = App::new("/test".into(), Config::default(), vec![]);
                app.modal = Some(Modal::Appearance { kind, selected: 0 });
                app.navigate(&Action::Scroll(1));
                assert_eq!(kind.current(&app.config), 0);
                let mut ui = Ui::default();
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                for index in 0..kind.choices().len() {
                    let action = Action::SelectAppearance(kind, index);
                    let hit = ui.hits.iter().find(|hit| hit.action == action).unwrap();
                    assert_eq!(ui.hit(hit.area.x, hit.area.y), Some(action));
                    assert!(hit.area.right() <= width && hit.area.bottom() <= height);
                }
                assert!(
                    ui.hits.iter().all(|hit| matches!(
                        hit.action,
                        Action::Close | Action::SelectAppearance(..)
                    ))
                );
                app.navigate(&Action::Close);
                assert!(app.modal.is_none());
                assert_eq!(kind.current(&app.config), 0);
            }
        }
    }

    #[test]
    fn close_label_fits_inside_dialog_border_and_matches_its_click_target() {
        for (width, height) in [(60, 10), (120, 14), (80, 24), (160, 48)] {
            let mut app = App::new("/test/project".into(), Config::default(), vec![]);
            app.open_form(crate::app::FormKind::Profile);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut ui = Ui::default();
            terminal.draw(|f| ui.draw(f, &app)).unwrap();
            let close = ui
                .hits
                .iter()
                .find(|hit| hit.action == Action::Close)
                .unwrap()
                .area;
            for (offset, character) in " [Esc] × ".chars().enumerate() {
                assert_eq!(
                    terminal.backend().buffer()[(close.x + offset as u16, close.y)].symbol(),
                    character.to_string()
                );
            }
            assert_eq!(
                terminal.backend().buffer()[(close.right(), close.y)].symbol(),
                "╮"
            );
            assert_eq!(ui.hit(close.x + 1, close.y), Some(Action::Close));
        }
    }

    #[test]
    fn every_panel_and_form_remains_clickable_in_compact_windows() {
        for (width, height) in [
            (60, 10),
            (80, 12),
            (120, 16),
            (180, 20),
            (80, 24),
            (100, 28),
            (160, 48),
        ] {
            let mut app = App::new("/test/project".into(), Config::default(), vec![]);
            crate::demo::populate(&mut app);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut ui = Ui::default();
            for pane in Pane::ALL {
                app.pane = pane;
                terminal.draw(|f| ui.draw(f, &app)).unwrap();
                if pane == Pane::Wallet {
                    assert!(
                        ui.hits
                            .iter()
                            .any(|hit| matches!(hit.action, Action::SelectTransaction(_))),
                        "Transactions must remain clickable at {width}x{height}"
                    );
                    assert!(ui.hits.iter().any(|hit| hit.action == Action::Fund));
                    assert!(ui.hits.iter().any(|hit| hit.action == Action::Send));
                }
                assert!(
                    ui.hits
                        .iter()
                        .all(|h| h.area.right() <= width && h.area.bottom() <= height),
                    "{pane:?} at {width}x{height}"
                );
            }
            app.open_form(crate::app::FormKind::Profile);
            for active in 0..3 {
                app.navigate(&Action::Field(active));
                terminal.draw(|f| ui.draw(f, &app)).unwrap();
                assert!(ui.hits.iter().any(|h| h.action == Action::Field(active)));
                let submit = ui.hits.iter().find(|h| h.action == Action::Submit).unwrap();
                assert!(
                    ui.hits
                        .iter()
                        .filter(|h| matches!(h.action, Action::Field(_)))
                        .all(|h| h.area.intersection(submit.area).is_empty())
                );
                assert!(
                    ui.hits
                        .iter()
                        .all(|h| h.area.right() <= width && h.area.bottom() <= height)
                );
            }
        }
    }

    #[test]
    fn short_empty_sessions_offer_mouse_wallet_creation() {
        for (width, height) in [(60, 10), (120, 14), (180, 20)] {
            let app = App::new("/test/project".into(), Config::default(), vec![]);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut ui = Ui::default();
            terminal.draw(|f| ui.draw(f, &app)).unwrap();
            for action in [Action::New, Action::Import, Action::Help, Action::Quit] {
                assert!(ui.hits.iter().any(|hit| hit.action == action));
            }
        }
    }

    #[test]
    fn faucet_choices_remain_clickable_inside_small_dialogs() {
        for (width, height) in [(60, 10), (120, 14), (160, 48)] {
            let mut app = App::new("/test".into(), Config::default(), vec![]);
            crate::demo::populate(&mut app);
            app.modal = Some(Modal::Funding {
                selected: 0,
                reason: Some("Provider rejected the request".into()),
            });
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut ui = Ui::default();
            terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
            for action in [
                Action::BrowserFaucet(crate::funding::Faucet::Solana),
                Action::BrowserFaucet(crate::funding::Faucet::Quicknode),
                Action::RpcAirdrop,
                Action::CopyAddress,
                Action::Close,
            ] {
                assert!(ui.hits.iter().any(|hit| hit.action == action));
            }
            assert!(
                ui.hits
                    .iter()
                    .all(|hit| hit.area.right() <= width && hit.area.bottom() <= height)
            );
            assert!(
                !ui.hits
                    .iter()
                    .any(|hit| matches!(hit.action, Action::Selector(_) | Action::Focus(_)))
            );
        }
    }

    #[test]
    fn resizing_switches_layout_without_losing_selection() {
        let mut app = App::new("/test/project".into(), Config::default(), vec![]);
        crate::demo::populate(&mut app);
        app.transaction_cursor = 5;
        let mut ui = Ui::default();
        for (width, height) in [(160, 48), (120, 14), (60, 10), (160, 48)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| ui.draw(f, &app)).unwrap();
            assert_eq!(app.transaction_cursor, 5);
            assert!(
                ui.hits
                    .iter()
                    .any(|hit| hit.action == Action::SelectTransaction(5))
            );
        }
    }
}
