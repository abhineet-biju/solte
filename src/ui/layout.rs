use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{Ui, theme::Theme};
use crate::{
    app::{Action, App, Pane},
    model::clean_text,
};

impl Ui {
    pub(super) fn workspace(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        if app.zoomed || area.width < 80 {
            self.adaptive_pane(frame, app, area, app.pane, theme);
            return;
        }
        let logs_height = if area.height >= 22 {
            (area.height / 5).clamp(5, 9)
        } else {
            0
        };
        let top = Rect::new(
            area.x,
            area.y,
            area.width,
            area.height - if logs_height > 0 { logs_height + 1 } else { 0 },
        );
        let sidebar = Rect::new(top.right() - 26, top.y, 26, top.height);
        let left_width = if area.width >= 100 { 22 } else { 0 };
        let center_x = area.x + if left_width > 0 { left_width + 1 } else { 0 };
        let center = Rect::new(center_x, top.y, sidebar.x - center_x - 1, top.height);
        if left_width > 0 {
            self.identity_sidebar(
                frame,
                app,
                Rect::new(area.x, top.y, left_width, top.height),
                theme,
            );
        }
        let center_pane = match app.pane {
            Pane::Wallets if left_width == 0 => Pane::Wallets,
            Pane::Logs if logs_height == 0 => Pane::Logs,
            _ => Pane::Wallet,
        };
        self.adaptive_pane(frame, app, center, center_pane, theme);
        self.network_sidebar(frame, app, sidebar, theme);
        if logs_height > 0 {
            self.logs(
                frame,
                app,
                Rect::new(area.x, area.bottom() - logs_height, area.width, logs_height),
                theme,
            );
        }
    }

    fn adaptive_pane(
        &mut self,
        frame: &mut Frame,
        app: &App,
        area: Rect,
        pane: Pane,
        theme: Theme,
    ) {
        match pane {
            Pane::Wallet if area.height < 22 => self.short_wallet(frame, app, area, theme),
            Pane::Wallets if area.height < 20 => self.short_wallets(frame, app, area, theme),
            _ => self.draw_pane(frame, app, area, pane, theme),
        }
    }

    fn identity_sidebar(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self.panel(frame, app, area, Pane::Wallets, "Identities", theme);
        let count = inner.height.saturating_sub(2) as usize;
        let offset = app.wallet_cursor.saturating_sub(count.saturating_sub(1));
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
                    "{} {}{}",
                    if index == app.selected_wallet {
                        "●"
                    } else {
                        "○"
                    },
                    clean_text(&wallet.name),
                    if wallet.program { " [P]" } else { "" }
                ),
                Action::SelectWallet(index),
                theme,
                index == app.wallet_cursor,
            );
        }
        if app.wallets.is_empty() {
            frame.render_widget(
                Paragraph::new(" No identities yet").style(Style::default().fg(theme.muted)),
                inner,
            );
        }
        self.button(
            frame,
            Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
            "+ New identity n",
            Action::New,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            "Import keypair i",
            Action::Import,
            theme,
            false,
        );
    }

    fn network_sidebar(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self.panel(frame, app, area, Pane::Network, "RPC / Network", theme);
        let state = app.network.as_ref();
        let number =
            |value: Option<u64>| value.map(|v| v.to_string()).unwrap_or_else(|| "—".into());
        let fields = [
            (
                "Cluster",
                state
                    .map(|n| n.cluster.clone())
                    .unwrap_or_else(|| app.profile().name.clone()),
            ),
            (
                "RPC",
                if app.connected {
                    "Connected"
                } else {
                    "Offline"
                }
                .into(),
            ),
            (
                "Latency",
                state
                    .map(|n| format!("{} ms", n.latency_ms))
                    .unwrap_or_else(|| "—".into()),
            ),
            ("Slot", number(state.map(|n| n.slot))),
            ("Block", number(state.map(|n| n.block_height))),
            ("Epoch", number(state.map(|n| n.epoch))),
            (
                "Logs",
                if app.subscribed { "Live" } else { "Polling" }.into(),
            ),
            ("Tokens", number(state.map(|n| n.token_accounts as u64))),
            (
                "Version",
                state
                    .map(|n| n.version.clone())
                    .unwrap_or_else(|| "—".into()),
            ),
        ];
        let lines: Vec<_> = fields
            .into_iter()
            .map(|(label, value)| {
                Line::from(vec![
                    Span::styled(format!(" {label:<8}"), Style::default().fg(theme.muted)),
                    Span::styled(value, Style::default().fg(theme.text)),
                ])
            })
            .collect();
        let visible = inner.height.saturating_sub(2);
        let scroll = app
            .network_scroll
            .min((lines.len() as u16).saturating_sub(visible));
        frame.render_widget(
            Paragraph::new(lines).scroll((scroll, 0)),
            Rect::new(inner.x, inner.y, inner.width, visible),
        );
        self.button(
            frame,
            Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
            "Profiles p",
            Action::Profiles,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            "Refresh r",
            Action::Refresh,
            theme,
            false,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, demo};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn wide_short_terminals_keep_the_network_sidebar_visible() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        for (width, height) in [
            (80, 10),
            (88, 10),
            (100, 14),
            (100, 10),
            (160, 16),
            (180, 22),
        ] {
            let mut ui = Ui::default();
            Terminal::new(TestBackend::new(width, height))
                .unwrap()
                .draw(|frame| ui.draw(frame, &app))
                .unwrap();
            let network = ui
                .hits
                .iter()
                .find(|h| h.action == Action::Focus(Pane::Network))
                .unwrap();
            assert_eq!(network.area.width, 26);
            assert_eq!(network.area.right(), width);
            assert!(ui.hits.iter().any(|h| h.action == Action::Fund));
            if width >= 100 {
                assert!(
                    ui.hits
                        .iter()
                        .any(|h| h.action == Action::Focus(Pane::Wallets))
                );
            }
        }
    }

    #[test]
    fn resize_cycles_rebuild_visible_click_targets_without_overlap_or_stale_effects() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        let mut ui = Ui::default();
        for (width, height) in [
            (160, 48),
            (120, 14),
            (88, 10),
            (60, 10),
            (100, 28),
            (160, 48),
        ] {
            ui.animate(&app);
            for pane in Pane::ALL {
                app.navigate(&Action::Focus(pane));
                Terminal::new(TestBackend::new(width, height))
                    .unwrap()
                    .draw(|frame| ui.draw(frame, &app))
                    .unwrap();
                assert!(
                    ui.hits
                        .iter()
                        .all(|hit| hit.area.right() <= width && hit.area.bottom() <= height)
                );
                assert!(
                    ui.hits.iter().any(|hit| hit.action == Action::Focus(pane)),
                    "{pane:?} must remain visible"
                );
                let panels: Vec<_> = ui
                    .hits
                    .iter()
                    .filter(|hit| matches!(hit.action, Action::Focus(_)))
                    .collect();
                for (index, left) in panels.iter().enumerate() {
                    for right in panels.iter().skip(index + 1) {
                        assert!(left.area.intersection(right.area).is_empty());
                    }
                }
            }
        }
    }
}
