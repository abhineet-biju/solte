use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, BorderType, Paragraph, Wrap},
};

use super::{Ui, theme::Theme};
use crate::{
    app::{Action, App, Pane},
    model::short,
    tokens::format_amount,
};

impl Ui {
    pub(super) fn token_buttons(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        actions: &[(&str, Action)],
        theme: Theme,
    ) {
        let labels_width = actions
            .iter()
            .map(|(label, _)| label.len() as u16)
            .sum::<u16>();
        let gaps = actions.len().saturating_sub(1) as u16;
        let spacious = labels_width + 2 * actions.len() as u16 + 3 * gaps <= area.width;
        let padding = if spacious { 2 } else { 0 };
        let gap = if spacious { 3 } else { 2 };
        let mut x = area.x;
        for (label, action) in actions {
            let width = label.len() as u16 + padding;
            if x + width > area.right() {
                break;
            }
            self.button(
                frame,
                Rect::new(x, area.y, width, 1),
                label,
                action.clone(),
                theme,
                false,
            );
            x += width + gap;
        }
    }

    pub(super) fn tokens(&mut self, frame: &mut Frame, app: &App, area: Rect, theme: Theme) {
        let inner = self
            .panel(frame, app, area, Pane::Tokens, "Token accounts", theme)
            .inner(ratatui::layout::Margin::new(1, 0));
        if inner.height < 3 {
            return;
        }
        let state = if app.token_loading {
            if app.config.reduced_motion {
                "Refreshing…".into()
            } else {
                format!(
                    "Refreshing {}",
                    ['|', '/', '-', '\\'][app
                        .token_started
                        .map_or(0, |at| at.elapsed().as_millis() / 120 % 4)
                        as usize]
                )
            }
        } else if let Some(error) = &app.token_error {
            format!("Unavailable · {error}")
        } else if !app.token_warnings.is_empty() {
            format!("Partial coverage · {}", app.token_warnings.join(" · "))
        } else if let Some(at) = app.token_updated {
            format!("Updated {}s ago", at.elapsed().as_secs())
        } else if app.demo {
            "Demo token accounts".into()
        } else {
            "Waiting for account discovery".into()
        };
        let owner = app
            .wallet()
            .map(|w| w.name.as_str())
            .unwrap_or("No wallet selected");
        frame.render_widget(
            Paragraph::new(format!("{owner} · {} accounts · {state}", app.tokens.len()))
                .style(Style::default().fg(theme.muted)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let roomy = inner.height >= 9;
        let selected = app.selected_token().is_some();
        let reserved = if roomy {
            if selected { 5 } else { 3 }
        } else {
            1
        };
        let body_y = inner.y + if roomy { 3 } else { 1 };
        let body_bottom = inner.bottom() - reserved;
        let split = inner.width >= 104 && roomy;
        let list_width = if split {
            inner.width * 55 / 100
        } else {
            inner.width
        };
        let list = Rect::new(
            inner.x,
            body_y,
            list_width,
            body_bottom.saturating_sub(body_y),
        );
        let accounts = app.visible_tokens();
        if !app.token_filter.is_empty() && roomy {
            frame.render_widget(
                Paragraph::new(format!(
                    "Filter: {} · [x] clear",
                    crate::model::clean_text(&app.token_filter)
                ))
                .style(Style::default().fg(theme.accent)),
                Rect::new(inner.x, inner.y + 1, inner.width, 1),
            );
        }
        let detailed = list.width >= 76;
        let row_height = if detailed { 2 } else { 1 };
        let count = usize::from(list.height / row_height);
        let start = app.token_cursor.saturating_sub(count.saturating_sub(1));
        if accounts.is_empty() {
            let empty = if app.wallet().is_none() {
                "Create or select a wallet to inspect its token accounts."
            } else if app.token_loading {
                "Discovering SPL Token and Token-2022 accounts…"
            } else if app.token_error.is_some() {
                "Discovery unavailable. [r] retries; existing accounts are retained."
            } else if !app.token_filter.is_empty() {
                "No matching accounts. [x] clears the filter."
            } else if app.token_updated.is_some() || app.demo {
                "No token accounts yet.\n\nCreate a mint or associated account with [c]."
            } else {
                "Token account discovery has not completed."
            };
            frame.render_widget(
                Paragraph::new(empty)
                    .wrap(Wrap { trim: true })
                    .style(Style::default().fg(theme.muted)),
                list,
            );
        }
        for (row, (index, account)) in accounts
            .iter()
            .enumerate()
            .skip(start)
            .take(count)
            .enumerate()
        {
            let y = list.y + row as u16 * row_height;
            let balance = format_amount(account.amount, account.decimals);
            let flags = format!(
                "{}{}",
                if account.associated { "ATA" } else { "Custom" },
                if account.info().get("delegate").is_some() {
                    " · delegated"
                } else {
                    ""
                }
            );
            let color = if account.state == "frozen" {
                theme.red
            } else if account.amount > 0 {
                theme.green
            } else {
                theme.muted
            };
            let text = if detailed {
                format!(
                    "{}  {}  {} · {}\nAccount {}  Mint {}  {}",
                    account.label(),
                    balance,
                    account.program_label(),
                    account.state,
                    short(&account.address),
                    short(&account.mint),
                    flags
                )
            } else {
                format!(
                    "{}  {}  {}  {}",
                    short(&account.mint),
                    balance,
                    if account.program == crate::tokens::TOKEN_2022 {
                        "T22"
                    } else {
                        "SPL"
                    },
                    account.state
                )
            };
            let rect = Rect::new(list.x, y, list.width, row_height);
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(color).bg(
                    if index == app.token_cursor {
                        theme.selected
                    } else {
                        theme.panel
                    },
                )),
                rect,
            );
            self.target(rect, Action::SelectToken(index));
        }
        if split {
            let details = Rect::new(
                list.right() + 1,
                body_y,
                inner.right() - list.right() - 1,
                list.height,
            );
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Selected account · [Enter] details ")
                .border_style(Style::default().fg(theme.border));
            let content = block.inner(details);
            frame.render_widget(block, details);
            if let Some(account) = app.selected_token() {
                frame.render_widget(
                    Paragraph::new(account.lines().join("\n"))
                        .wrap(Wrap { trim: false })
                        .style(Style::default().fg(theme.text)),
                    content,
                );
            }
        }
        if roomy {
            frame.render_widget(
                Paragraph::new("─".repeat(inner.width as usize))
                    .style(Style::default().fg(theme.border)),
                Rect::new(inner.x, body_bottom, inner.width, 1),
            );
            if selected {
                self.token_buttons(
                    frame,
                    Rect::new(inner.x, inner.bottom() - 3, inner.width, 1),
                    &[
                        ("Inspect [Enter]", Action::SelectToken(app.token_cursor)),
                        ("Send [s]", Action::Send),
                    ],
                    theme,
                );
            }
        }
        let mut tools = Vec::new();
        if !app.tokens.is_empty() || !app.token_filter.is_empty() {
            tools.push(if app.token_filter.is_empty() {
                ("Find [/]", Action::Search)
            } else {
                ("Clear [x]", Action::ClearFilter)
            });
        }
        tools.push(("Refresh [r]", Action::Refresh));
        let tools_width = tools
            .iter()
            .map(|(label, _)| label.len() as u16 + 2)
            .sum::<u16>()
            + 3 * tools.len().saturating_sub(1) as u16;
        let y = inner.bottom() - 1;
        let mut primary = vec![
            ("Create [c]", Action::TokenCreation),
            ("Mints [v]", Action::ProjectMints),
        ];
        if !roomy && selected && inner.width >= 80 {
            primary.push(("Send [s]", Action::Send));
        }
        self.token_buttons(
            frame,
            Rect::new(inner.x, y, inner.width - tools_width - 2, 1),
            &primary,
            theme,
        );
        self.token_buttons(
            frame,
            Rect::new(inner.right() - tools_width, y, tools_width, 1),
            &tools,
            theme,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{Direction, View},
        config::Config,
        demo,
    };
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn token_forms_keep_every_field_and_submission_inside_compact_dialogs() {
        for (width, height) in [(60, 10), (80, 20), (160, 48)] {
            let mut app = App::new("/test".into(), Config::default(), vec![]);
            demo::populate(&mut app);
            let mut ui = Ui::default();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for kind in [
                crate::app::FormKind::TokenCreate,
                crate::app::FormKind::MintCreate,
                crate::app::FormKind::MintMore,
                crate::app::FormKind::TokenTransfer,
                crate::app::FormKind::TokenExport,
                crate::app::FormKind::TokenSearch,
            ] {
                app.open_form(kind);
                let count = if let Some(crate::app::Modal::Form(form)) = &app.modal {
                    form.fields.len()
                } else {
                    0
                };
                for index in 0..count {
                    app.navigate(&Action::Field(index));
                    terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                    assert!(ui.hits.iter().any(|hit| hit.action == Action::Field(index)));
                    let submit = ui
                        .hits
                        .iter()
                        .find(|hit| hit.action == Action::Submit)
                        .unwrap();
                    assert!(
                        ui.hits
                            .iter()
                            .filter(|hit| matches!(hit.action, Action::Field(_)))
                            .all(|hit| hit.area.intersection(submit.area).is_empty())
                    );
                    assert!(
                        ui.hits
                            .iter()
                            .all(|hit| hit.area.right() <= width && hit.area.bottom() <= height)
                    );
                }
            }
        }
    }

    #[test]
    fn tokens_keep_navigation_copy_and_inspection_accessible_after_resize() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        app.switch_view(View::Tokens);
        let mut ui = Ui::default();
        for (width, height) in [(160, 48), (100, 28), (80, 20), (60, 10), (160, 48)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
            for hit in &ui.hits {
                assert!(
                    hit.area.right() <= width && hit.area.bottom() <= height,
                    "{:?}",
                    hit.action
                );
            }
            assert!(
                ui.hits
                    .iter()
                    .any(|hit| hit.action == Action::SelectToken(0))
            );
            app.focused_control = Some(Action::SelectToken(0));
            ui.navigate_control(&mut app, Direction::Down);
            assert_eq!(app.token_cursor, 1);
            app.token_cursor = 0;
            app.focused_control = None;
            app.modal = Some(crate::app::Modal::Token {
                account: Box::new(app.tokens[0].clone()),
                scroll: 0,
            });
            terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
            for action in [
                Action::CopyToken(false),
                Action::CopyToken(true),
                Action::ExportToken,
                Action::ExplorerToken(false),
            ] {
                assert!(
                    ui.hits.iter().any(|hit| hit.action == action),
                    "{width}x{height}: {action:?}"
                );
            }
            app.modal = None;
        }
    }
    #[test]
    fn mint_dialogs_keep_actions_separate_and_selection_visible_when_resized() {
        use crate::app::Modal;
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        app.switch_view(View::Tokens);
        let mut ui = Ui::default();
        for (width, height) in [(60, 10), (80, 20), (120, 32), (160, 48)] {
            for selected in [0, 1] {
                app.modal = Some(Modal::TokenCreation { selected });
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
                let chosen = if selected == 0 {
                    Action::CreateMint
                } else {
                    Action::CreateTokenAccount
                };
                let hit = ui.hits.iter().find(|hit| hit.action == chosen).unwrap();
                assert_eq!(
                    terminal.backend().buffer()[(hit.area.x, hit.area.y)].bg,
                    crate::ui::theme::Theme::named(&app.config.theme).accent
                );
            }
            app.modal = Some(Modal::Mint {
                mint: Box::new(app.project_mints[0].clone()),
                scroll: 0,
            });
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
            for action in [
                Action::CopyToken(true),
                Action::ViewMintAccount,
                Action::ExplorerToken(true),
                Action::MintMore,
                Action::CreateTokenAccount,
                Action::Refresh,
            ] {
                assert!(
                    ui.hits.iter().any(|hit| hit.action == action),
                    "{width}x{height} {action:?}"
                );
            }
            assert!(
                ui.hits
                    .iter()
                    .all(|hit| hit.area.right() <= width && hit.area.bottom() <= height)
            );
        }
    }
}
