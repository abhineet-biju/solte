use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{Ui, theme::Theme};
use crate::app::{Action, Form};

impl Ui {
    pub(super) fn form_progress(
        &self,
        frame: &mut Frame,
        form: &Form,
        offset: usize,
        capacity: usize,
        area: Rect,
        theme: Theme,
    ) {
        let end = (offset + capacity).min(form.fields.len());
        let mut spans = vec![Span::styled(
            format!("Fields {}–{} of {}", offset + 1, end, form.fields.len()),
            Style::default().fg(theme.muted),
        )];
        for (count, direction) in [(offset, "↑"), (form.fields.len() - end, "↓")] {
            if count > 0 {
                spans.push(Span::styled(
                    format!(
                        " · {direction} {count} more {}",
                        if direction == "↑" { "above" } else { "below" }
                    ),
                    Style::default().fg(theme.accent),
                ));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    pub(super) fn form_navigation(
        &mut self,
        frame: &mut Frame,
        form: &Form,
        area: Rect,
        theme: Theme,
    ) {
        for (label, rect, index, enabled) in [
            (
                "Previous [↑]",
                Rect::new(area.x, area.y, 14, 1),
                form.active.saturating_sub(1),
                form.active > 0,
            ),
            (
                "Next [↓]",
                Rect::new(area.right() - 10, area.y, 10, 1),
                (form.active + 1).min(form.fields.len() - 1),
                form.active + 1 < form.fields.len(),
            ),
        ] {
            if enabled {
                self.button(frame, rect, label, Action::Field(index), theme, false);
            } else {
                frame.render_widget(
                    Paragraph::new(label).style(Style::default().fg(theme.muted)),
                    rect,
                );
            }
        }
        let middle = Rect::new(area.x + 15, area.y, area.width.saturating_sub(26), 1);
        let label = if middle.width >= 32 {
            "[Tab] or scroll to move"
        } else {
            "[Tab] moves fields"
        };
        frame.render_widget(
            Paragraph::new(label).style(Style::default().fg(theme.muted)),
            middle,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{App, FormKind, Modal},
        config::Config,
    };
    use ratatui::{Terminal, backend::TestBackend};

    fn rendered(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn hidden_form_fields_have_progress_and_end_navigation_matches_the_keyboard() {
        for (width, height) in [(60, 10), (88, 22), (160, 48), (80, 20)] {
            let mut app = App::new("/test".into(), Config::default(), vec![]);
            app.open_form(FormKind::MintCreate);
            let mut ui = Ui::default();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
            let text = rendered(&terminal);
            assert!(text.contains("of 6"));
            if height < 30 {
                assert!(text.contains("more below"), "{width}x{height}");
            }
            assert!(!ui.hits.iter().any(|hit| hit.action == Action::Field(0)
                && rendered_label(&terminal, hit.area).contains("Previous")));
            let next = ui
                .hits
                .iter()
                .find(|hit| {
                    hit.action == Action::Field(1)
                        && rendered_label(&terminal, hit.area).contains("Next")
                })
                .unwrap();
            let action = ui.click(&mut app, next.area.x, next.area.y).unwrap();
            assert_eq!(
                action,
                app.key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Down,
                    crossterm::event::KeyModifiers::NONE
                ))
                .unwrap()
            );
            app.navigate(&action);
            app.navigate(&Action::Field(5));
            terminal.draw(|frame| ui.draw(frame, &app)).unwrap();
            let text = rendered(&terminal);
            if height < 30 {
                assert!(text.contains("more above"));
                assert!(!text.contains("more below"));
            }
            assert!(text.contains("of 6"));
            let keys = crossterm::event::KeyModifiers::NONE;
            assert_eq!(
                app.key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Down,
                    keys
                )),
                Some(Action::Field(5))
            );
            assert_eq!(
                app.key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Tab,
                    keys
                )),
                Some(Action::Field(0))
            );
            assert!(!ui.hits.iter().any(|hit| hit.action == Action::Field(5)
                && rendered_label(&terminal, hit.area).contains("Next")));
            assert!(matches!(&app.modal, Some(Modal::Form(form)) if form.active == 5));
        }
    }

    fn rendered_label(terminal: &Terminal<TestBackend>, area: Rect) -> String {
        (area.x..area.right())
            .map(|x| terminal.backend().buffer()[(x, area.y)].symbol())
            .collect()
    }
}
