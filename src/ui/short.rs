use ratatui::{Frame, layout::Rect, style::Style, widgets::Paragraph};

use super::{Ui, theme::Theme};
use crate::{
    amount::format_sol,
    app::{Action, App, Form, Pane, Tab},
    model::{clean_text, short},
};

impl Ui {
    pub(super) fn short_layout(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        frame.render_widget(
            Paragraph::new(format!(
                " SOLTE  ·  {}{}",
                app.profile().name,
                if app.demo { "  ·  DEMO" } else { "" }
            ))
            .style(Style::default().fg(theme.accent)),
            Rect::new(area.x, area.y, area.width.saturating_sub(22), 1),
        );
        self.button(
            frame,
            Rect::new(area.right() - 21, area.y, 10, 1),
            "Theme t",
            Action::Theme,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(area.right() - 10, area.y, 10, 1),
            "Motion m",
            Action::Motion,
            theme,
            false,
        );
        let mut x = area.x;
        for (i, pane) in Pane::ALL.iter().enumerate() {
            let label = format!("{} {}", i + 1, pane.name());
            let width = label.len() as u16 + 2;
            self.button(
                frame,
                Rect::new(x, area.y + 1, width, 1),
                &label,
                Action::Selector(*pane),
                theme,
                app.pane == *pane,
            );
            x += width + 1;
        }
        let body = Rect::new(area.x, area.y + 2, area.width, area.height - 3);
        self.workspace(frame, app, body, theme);
        let y = area.bottom() - 1;
        let message = app.busy.as_ref().unwrap_or(&app.status);
        frame.render_widget(
            Paragraph::new(clean_text(message)).style(Style::default().fg(theme.muted)),
            Rect::new(area.x, y, area.width - 19, 1),
        );
        self.button(
            frame,
            Rect::new(area.right() - 18, y, 9, 1),
            "? Help",
            Action::Help,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(area.right() - 8, y, 8, 1),
            "q Quit",
            Action::Quit,
            theme,
            false,
        );
    }

    pub(super) fn short_wallets(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self.panel(frame, app, area, Pane::Wallets, "Project identities", theme);
        let count = inner.height.saturating_sub(1) as usize;
        let offset = app.wallet_cursor.saturating_sub(count.saturating_sub(1));
        if app.wallets.is_empty() {
            frame.render_widget(
                Paragraph::new(" No wallets. Create or import a keypair below.")
                    .style(Style::default().fg(theme.muted)),
                inner,
            );
        }
        for (row, (index, wallet)) in app
            .wallets
            .iter()
            .enumerate()
            .skip(offset)
            .take(count)
            .enumerate()
        {
            self.button(
                frame,
                Rect::new(inner.x, inner.y + row as u16, inner.width, 1),
                &format!(
                    "{} {}  {}{}",
                    if index == app.selected_wallet {
                        "●"
                    } else {
                        "○"
                    },
                    clean_text(&wallet.name),
                    short(&wallet.address),
                    if wallet.program { " · program" } else { "" }
                ),
                Action::SelectWallet(index),
                theme,
                index == app.wallet_cursor,
            );
        }
        let y = inner.bottom() - 1;
        self.button(
            frame,
            Rect::new(inner.x, y, 17, 1),
            "New identity n",
            Action::New,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(inner.x + 18, y, 12, 1),
            "Import i",
            Action::Import,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(inner.x + 31, y, 12, 1),
            "Cycle ]",
            Action::NextWallet,
            theme,
            false,
        );
    }

    pub(super) fn short_wallet(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self.panel(frame, app, area, Pane::Wallet, "Wallet activity", theme);
        if app.tab == Tab::Settings {
            frame.render_widget(
                Paragraph::new(format!(
                    " Theme: {} · Motion: {}",
                    app.config.theme,
                    if app.config.reduced_motion {
                        "reduced"
                    } else {
                        "enabled"
                    }
                ))
                .style(Style::default().fg(theme.text)),
                inner,
            );
            self.button(
                frame,
                Rect::new(inner.x, inner.y + 1, 15, 1),
                "Theme t",
                Action::Theme,
                theme,
                false,
            );
            self.button(
                frame,
                Rect::new(inner.x + 17, inner.y + 1, 15, 1),
                "Motion m",
                Action::Motion,
                theme,
                false,
            );
            self.button(
                frame,
                Rect::new(inner.x, inner.y + 3, 18, 1),
                "Back to activity",
                Action::SetTab(Tab::Overview),
                theme,
                false,
            );
            return;
        }
        let Some(wallet) = app.wallet() else {
            self.welcome(frame, inner, theme);
            return;
        };
        let balance = app.balance.map(format_sol).unwrap_or_else(|| "—".into());
        frame.render_widget(
            Paragraph::new(format!(
                " {} · {balance} SOL{} · {}",
                clean_text(&wallet.name),
                if !app.connected { " · stale" } else { "" },
                short(&wallet.address)
            ))
            .style(Style::default().fg(theme.green)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let mut x = inner.x;
        for (label, action) in [
            ("Fund f", Action::Fund),
            ("Send s", Action::Send),
            ("Copy y", Action::CopyAddress),
            ("Explorer", Action::ExplorerWallet),
            ("RPC p", Action::Profiles),
        ] {
            let width = label.len() as u16 + 2;
            self.button(
                frame,
                Rect::new(x, inner.y + 1, width, 1),
                label,
                action,
                theme,
                false,
            );
            x += width + 1;
        }
        let visible = app.visible_records();
        let filtered = u16::from(!app.filter.is_empty());
        if filtered > 0 {
            frame.render_widget(
                Paragraph::new(format!(" Filter: {}", clean_text(&app.filter)))
                    .style(Style::default().fg(theme.accent)),
                Rect::new(inner.x, inner.y + 2, inner.width, 1),
            );
        }
        let count = inner.height.saturating_sub(3 + filtered) as usize;
        let offset = app
            .transaction_cursor
            .saturating_sub(count.saturating_sub(1));
        for (row, (index, record)) in visible
            .iter()
            .enumerate()
            .skip(offset)
            .take(count)
            .enumerate()
        {
            let rect = Rect::new(inner.x, inner.y + 2 + filtered + row as u16, inner.width, 1);
            frame.render_widget(
                Paragraph::new(format!(
                    " {} {}  {}  slot {}",
                    if record.error.is_some() { "×" } else { "✓" },
                    short(&record.signature),
                    record.kind(),
                    record.slot
                ))
                .style(
                    Style::default()
                        .fg(if record.error.is_some() {
                            theme.red
                        } else {
                            theme.text
                        })
                        .bg(if index == app.transaction_cursor {
                            theme.selected
                        } else {
                            theme.panel
                        }),
                ),
                rect,
            );
            self.target(rect, Action::SelectTransaction(index));
        }
        if visible.is_empty() {
            frame.render_widget(
                Paragraph::new(if app.failures_only {
                    " No matching failed transactions. e shows all."
                } else {
                    " No matching captured transactions."
                })
                .style(Style::default().fg(theme.muted)),
                Rect::new(inner.x, inner.y + 2 + filtered, inner.width, 1),
            );
        }
        let y = inner.bottom() - 1;
        self.button(
            frame,
            Rect::new(inner.x, y, 10, 1),
            if app.filter.is_empty() {
                "Find /"
            } else {
                "Clear x"
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
            Rect::new(inner.x + 11, y, 12, 1),
            if app.failures_only {
                "Errors ✓ e"
            } else {
                "Errors e"
            },
            Action::Failures,
            theme,
            app.failures_only,
        );
        self.button(
            frame,
            Rect::new(inner.x + 24, y, 12, 1),
            if app.history_loading {
                "Loading…"
            } else {
                "Older b"
            },
            Action::Older,
            theme,
            false,
        );
    }

    pub(super) fn short_form(&mut self, frame: &mut Frame, form: &Form, inner: Rect, theme: Theme) {
        let field = &form.fields[form.active];
        frame.render_widget(
            Paragraph::new(format!(
                "{}  ({}/{})",
                field.label,
                form.active + 1,
                form.fields.len()
            ))
            .style(Style::default().fg(theme.muted)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let prefix: String = field.value[..field.cursor]
            .chars()
            .rev()
            .take(inner.width.saturating_sub(3) as usize)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let field_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
        frame.render_widget(
            Paragraph::new(format!(" {prefix}▏{}", &field.value[field.cursor..]))
                .style(Style::default().fg(theme.accent).bg(theme.selected)),
            field_area,
        );
        self.target(field_area, Action::Field(form.active));
        if form.fields.len() > 1 {
            self.button(
                frame,
                Rect::new(inner.x, inner.y + 2, 12, 1),
                "Previous",
                Action::Field(form.active.saturating_sub(1)),
                theme,
                false,
            );
            self.button(
                frame,
                Rect::new(inner.right() - 10, inner.y + 2, 10, 1),
                "Next",
                Action::Field((form.active + 1).min(form.fields.len() - 1)),
                theme,
                false,
            );
        }
        let note = form
            .error
            .as_deref()
            .unwrap_or("Tab: next field · Enter: submit · Esc: cancel");
        frame.render_widget(
            Paragraph::new(note).style(Style::default().fg(if form.error.is_some() {
                theme.red
            } else {
                theme.muted
            })),
            Rect::new(inner.x, inner.y + 3, inner.width, 1),
        );
        self.button(
            frame,
            Rect::new(
                inner.x,
                inner.bottom() - 1,
                (form.submit_label().len() as u16 + 2).min(inner.width),
                1,
            ),
            form.submit_label(),
            Action::Submit,
            theme,
            true,
        );
    }
}
