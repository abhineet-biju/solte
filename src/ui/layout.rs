use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Wrap},
};

use super::{Ui, theme::Theme};
use crate::{
    app::{Action, App, Pane, View},
    model::clean_text,
};

impl Ui {
    pub(super) fn workspace(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        if app.view == View::Wallets && area.width >= 90 {
            let left = Rect::new(area.x, area.y, 34, area.height);
            self.adaptive_pane(frame, app, left, Pane::Wallets, theme);
            let right = Rect::new(left.right() + 1, area.y, area.width - 35, area.height);
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Identity details ")
                .border_style(Style::default().fg(theme.border))
                .style(Style::default().bg(theme.panel));
            let inner = block.inner(right).inner(ratatui::layout::Margin::new(1, 0));
            frame.render_widget(block, right);
            let lines = if let Some(wallet) = app.wallets.get(app.wallet_cursor) {
                vec![
                    Line::from(Span::styled(
                        clean_text(&wallet.name),
                        Style::default().fg(theme.accent).bold(),
                    )),
                    Line::from(if app.wallet_cursor == app.selected_wallet {
                        "Active wallet"
                    } else {
                        "Enter or click to make active"
                    }),
                    Line::from(""),
                    Line::from("PUBLIC ADDRESS"),
                    Line::from(wallet.address.clone()),
                    Line::from(""),
                    Line::from(if wallet.program {
                        "Program identity · read-only"
                    } else {
                        "Standard Solana keypair"
                    }),
                    Line::from(""),
                    Line::from("KEY FILE"),
                    Line::from(clean_text(&wallet.path.display().to_string())),
                ]
            } else {
                vec![Line::from("Create or import an identity to get started.")]
            };
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(theme.text)),
                inner,
            );
            return;
        }
        if app.view != View::Overview {
            self.adaptive_pane(frame, app, area, app.view.pane(), theme);
            return;
        }
        let single_pane = area.width < 80;
        if single_pane && app.pane == Pane::Logs {
            self.adaptive_pane(frame, app, area, app.pane, theme);
            return;
        }
        let logs_height = match area.height {
            22.. => (area.height / 5).clamp(5, 9),
            14..=21 => 4,
            11..=13 => 3,
            _ => 0,
        };
        let top = Rect::new(
            area.x,
            area.y,
            area.width,
            area.height - if logs_height > 0 { logs_height + 1 } else { 0 },
        );
        if logs_height > 0 {
            self.logs(
                frame,
                app,
                Rect::new(area.x, area.bottom() - logs_height, area.width, logs_height),
                theme,
            );
        }
        if single_pane {
            self.adaptive_pane(frame, app, top, app.pane, theme);
            return;
        }
        let (left_width, right_width) = if area.width >= 100 {
            (22, 26)
        } else {
            (
                (area.width / 5).clamp(16, 20),
                (area.width / 4).clamp(22, 25),
            )
        };
        let sidebar = Rect::new(top.right() - right_width, top.y, right_width, top.height);
        let center_x = area.x + left_width + 1;
        let center = Rect::new(center_x, top.y, sidebar.x - center_x - 1, top.height);
        self.identity_sidebar(
            frame,
            app,
            Rect::new(area.x, top.y, left_width, top.height),
            theme,
        );
        let center_pane = if app.pane == Pane::Logs && logs_height == 0 {
            Pane::Logs
        } else {
            Pane::Wallet
        };
        self.adaptive_pane(frame, app, center, center_pane, theme);
        self.network_sidebar(frame, app, sidebar, theme);
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
            Pane::Wallet if area.height < 22 || area.width < 52 => {
                self.short_wallet(frame, app, area, theme)
            }
            Pane::Wallets if area.height < 20 => self.short_wallets(frame, app, area, theme),
            _ => self.draw_pane(frame, app, area, pane, theme),
        }
    }

    fn identity_sidebar(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self.panel(
            frame,
            app,
            area,
            Pane::Wallets,
            &format!("Wallets · {}", app.wallets.len()),
            theme,
        );
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
            if inner.width < 20 {
                "+ New n"
            } else {
                "+ New identity n"
            },
            Action::New,
            theme,
            false,
        );
        self.button(
            frame,
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            if inner.width < 20 {
                "Import i"
            } else {
                "Import keypair i"
            },
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
            ("Logs", app.log_transport().into()),
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
    fn overview_preserves_left_wallets_and_right_network_with_contained_controls() {
        for (width, height) in [(80, 10), (80, 20), (90, 22), (99, 24), (140, 42)] {
            let mut app = App::new("/test".into(), Config::default(), vec![]);
            demo::populate(&mut app);
            let mut ui = Ui::default();
            Terminal::new(TestBackend::new(width, height))
                .unwrap()
                .draw(|frame| ui.draw(frame, &app))
                .unwrap();
            let area = |pane| {
                ui.hits
                    .iter()
                    .find(|hit| hit.action == Action::Focus(pane))
                    .unwrap()
                    .area
            };
            let wallets = area(Pane::Wallets);
            let activity = area(Pane::Wallet);
            let network = area(Pane::Network);
            assert_eq!(wallets.x, 0);
            assert!(wallets.right() < activity.x && activity.right() < network.x);
            assert_eq!(network.right(), width);
            for hit in &ui.hits {
                if hit.area.y >= activity.y && hit.area.y < activity.bottom() {
                    assert!(
                        [wallets, activity, network]
                            .iter()
                            .any(|panel| panel.intersection(hit.area) == hit.area),
                        "{width}x{height} {:?}",
                        hit.action
                    );
                }
            }
        }
    }

    #[test]
    fn zoomed_overview_keeps_all_four_summaries_without_overlap() {
        for (width, height) in [(80, 20), (90, 22), (99, 24), (100, 22), (140, 42)] {
            let mut app = App::new("/test".into(), Config::default(), vec![]);
            demo::populate(&mut app);
            let mut ui = Ui::default();
            Terminal::new(TestBackend::new(width, height))
                .unwrap()
                .draw(|frame| ui.draw(frame, &app))
                .unwrap();
            let panels: Vec<_> = Pane::ALL
                .into_iter()
                .map(|pane| {
                    ui.hits
                        .iter()
                        .find(|hit| hit.action == Action::Focus(pane))
                        .unwrap()
                        .area
                })
                .collect();
            for (index, panel) in panels.iter().enumerate() {
                assert!(panel.height >= 3);
                for other in panels.iter().skip(index + 1) {
                    assert!(panel.intersection(*other).is_empty());
                }
            }
            assert!(
                ui.hits
                    .iter()
                    .any(|hit| hit.action == Action::SelectWallet(0))
            );
            assert!(ui.hits.iter().any(|hit| hit.action == Action::Fund));
        }
    }

    #[test]
    fn log_filter_is_independent_of_transaction_filter() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        app.filter = "transfer".into();
        app.log_filter = "insufficient".into();
        app.switch_view(View::Logs);
        let mut ui = Ui::default();
        let mut terminal = Terminal::new(TestBackend::new(90, 22)).unwrap();
        terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Filter: insufficient"));
        assert!(!text.contains("Project loaded"));
        app.navigate(&Action::ClearFilter);
        assert!(app.log_filter.is_empty());
        assert_eq!(app.filter, "transfer");
    }

    #[test]
    fn dedicated_views_show_only_their_own_content_and_tab_does_not_change_view() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        for (width, height) in [(60, 10), (90, 22), (140, 42)] {
            for view in [View::Wallets, View::Activity, View::Network, View::Logs] {
                app.switch_view(view);
                let mut ui = Ui::default();
                Terminal::new(TestBackend::new(width, height))
                    .unwrap()
                    .draw(|frame| ui.draw(frame, &app))
                    .unwrap();
                let panels: Vec<_> = ui
                    .hits
                    .iter()
                    .filter_map(|hit| {
                        if let Action::Focus(pane) = hit.action {
                            Some(pane)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(panels, vec![view.pane()]);
                let panel = ui
                    .hits
                    .iter()
                    .find(|hit| hit.action == Action::Focus(view.pane()))
                    .unwrap()
                    .area;
                for hit in &ui.hits {
                    if hit.area.y >= panel.y && hit.area.y < panel.bottom() {
                        assert_eq!(
                            hit.area.intersection(panel),
                            hit.area,
                            "{view:?} {width}x{height} {:?}",
                            hit.action
                        );
                    }
                }
                assert!(
                    !ui.hits
                        .iter()
                        .any(|hit| matches!(hit.action, Action::Expand(_)))
                );
                for _ in 0..8 {
                    ui.cycle_region(&mut app, true);
                    assert_eq!(app.view, view);
                }
            }
        }
    }

    #[test]
    fn overview_keeps_readable_log_context() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        app.pane = Pane::Wallet;
        for (width, height) in [(60, 17), (100, 14), (100, 20), (140, 42)] {
            {
                let mut ui = Ui::default();
                Terminal::new(TestBackend::new(width, height))
                    .unwrap()
                    .draw(|frame| ui.draw(frame, &app))
                    .unwrap();
                let logs = ui
                    .hits
                    .iter()
                    .find(|hit| hit.action == Action::Focus(Pane::Logs))
                    .unwrap();
                assert!(logs.area.height >= 3);
                let wallet = ui
                    .hits
                    .iter()
                    .find(|hit| hit.action == Action::Focus(Pane::Wallet))
                    .unwrap();
                assert!(wallet.area.height >= 7);
                assert!(wallet.area.intersection(logs.area).is_empty());
                assert!(ui.hits.iter().any(|hit| hit.action == Action::Fund));
            }
        }
    }

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
            assert!((22..=26).contains(&network.area.width));
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
