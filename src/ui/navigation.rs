use ratatui::{Frame, layout::Rect};

use super::{Hit, Ui, theme::Theme};
use crate::app::{Action, App, Direction, Modal, Pane, Tab, View};

impl Ui {
    fn pane_area(&self, pane: Pane) -> Option<Rect> {
        self.hits
            .iter()
            .filter(|hit| hit.action == Action::Focus(pane))
            .max_by_key(|hit| u32::from(hit.area.width) * u32::from(hit.area.height))
            .map(|hit| hit.area)
    }

    fn controls(&self, pane: Pane) -> Vec<&Hit> {
        let Some(area) = self.pane_area(pane) else {
            return vec![];
        };
        self.hits
            .iter()
            .filter(|hit| {
                hit.area.intersection(area) == hit.area
                    && !matches!(hit.action, Action::Focus(_) | Action::Expand(_))
            })
            .collect()
    }

    pub fn focused_action(&self, app: &App) -> Option<Action> {
        if app.selector_focus {
            return Some(Action::Focus(app.pane));
        }
        let controls = self.controls(app.pane);
        let default = match app.pane {
            Pane::Wallets if !app.wallets.is_empty() => Action::SelectWallet(app.wallet_cursor),
            Pane::Wallets => Action::New,
            Pane::Wallet if app.tab == Tab::Settings => Action::Theme,
            Pane::Wallet => Action::SetTab(app.tab),
            Pane::Network => Action::Profiles,
            Pane::Logs => Action::Follow,
        };
        controls
            .iter()
            .find(|hit| app.focused_control.as_ref() == Some(&hit.action))
            .or_else(|| controls.iter().find(|hit| hit.action == default))
            .or_else(|| {
                controls
                    .iter()
                    .find(|hit| hit.action == Action::Fund && app.pane == Pane::Wallet)
            })
            .or_else(|| controls.first())
            .map(|hit| hit.action.clone())
    }

    pub fn click(&self, app: &mut App, x: u16, y: u16) -> Option<Action> {
        let action = self.hit(x, y)?;
        if app.modal.is_none() && !matches!(action, Action::Focus(_) | Action::Expand(_)) {
            for pane in Pane::ALL {
                if self
                    .pane_area(pane)
                    .is_some_and(|area| area.contains((x, y).into()))
                {
                    app.pane = pane;
                    app.selector_focus = false;
                    app.focused_control = Some(action.clone());
                    break;
                }
            }
        }
        Some(action)
    }

    pub fn cycle_region(&self, app: &mut App, forward: bool) {
        let panes: Vec<_> = Pane::ALL
            .into_iter()
            .filter(|pane| self.pane_area(*pane).is_some())
            .collect();
        if app.selector_focus {
            app.selector_focus = false;
            if let Some(pane) = if forward { panes.first() } else { panes.last() } {
                app.pane = *pane;
            }
        } else if let Some(index) = panes.iter().position(|pane| *pane == app.pane) {
            if (forward && index + 1 == panes.len()) || (!forward && index == 0) {
                app.selector_focus = true;
            } else {
                app.pane = panes[if forward { index + 1 } else { index - 1 }];
            }
        }
        app.focused_control = None;
    }

    pub fn navigate_control(&self, app: &mut App, direction: Direction) {
        if app.selector_focus {
            match direction {
                Direction::Left | Direction::Right => {
                    let index = app.view as usize;
                    let next = if direction == Direction::Left {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(View::ALL.len() - 1)
                    };
                    app.switch_view(View::ALL[next]);
                }
                Direction::Down => app.selector_focus = false,
                Direction::Up => {}
            }
            return;
        }
        let Some(current) = self.focused_action(app) else {
            return;
        };
        let delta = match direction {
            Direction::Up => -1,
            Direction::Down => 1,
            _ => 0,
        };
        if delta != 0 {
            let list = match current {
                Action::SelectWallet(index) => Some((index, app.wallets.len(), true)),
                Action::SelectTransaction(index) => {
                    Some((index, app.visible_records().len(), false))
                }
                _ => None,
            };
            if let Some((index, count, wallets)) = list {
                let next = index as i64 + delta;
                if (0..count as i64).contains(&next) {
                    Self::focus(
                        app,
                        if wallets {
                            Action::SelectWallet(next as usize)
                        } else {
                            Action::SelectTransaction(next as usize)
                        },
                    );
                    return;
                }
            }
        }
        let controls = self.controls(app.pane);
        let Some(origin) = controls.iter().find(|hit| hit.action == current) else {
            return;
        };
        let next = controls
            .iter()
            .filter(|hit| hit.action != current)
            .filter_map(|hit| {
                score(origin.area, hit.area, direction).map(|score| (score, hit.action.clone()))
            })
            .min_by_key(|(score, _)| *score);
        if let Some((_, action)) = next {
            Self::focus(app, action);
        } else if direction == Direction::Up {
            app.selector_focus = true;
        } else if delta != 0 && matches!(app.pane, Pane::Network | Pane::Logs) {
            app.navigate(&Action::Move(delta as i32));
        }
    }

    fn focus(app: &mut App, action: Action) {
        match action {
            Action::SelectWallet(index) => app.wallet_cursor = index,
            Action::SelectTransaction(index) => app.transaction_cursor = index,
            _ => {}
        }
        app.focused_control = Some(action);
    }

    pub(super) fn paint_control_focus(&self, frame: &mut Frame, app: &App, theme: Theme) {
        let action = match &app.modal {
            Some(Modal::Appearance { kind, selected }) => {
                Some(Action::SelectAppearance(*kind, *selected))
            }
            Some(Modal::Profiles { selected }) => Some(Action::SelectProfile(*selected)),
            Some(Modal::Funding { selected, .. }) => Some(match selected {
                0 => Action::BrowserFaucet(crate::funding::Faucet::Solana),
                1 => Action::BrowserFaucet(crate::funding::Faucet::Quicknode),
                _ => Action::RpcAirdrop,
            }),
            Some(Modal::Form(form)) => Some(Action::Field(form.active)),
            Some(Modal::Review { prepared, .. }) if prepared.simulation_error.is_none() => {
                Some(Action::Submit)
            }
            Some(_) => None,
            None if app.selector_focus => Some(Action::Selector(app.view)),
            None => self.focused_action(app),
        };
        if let Some(action) = action {
            let candidates = if app.modal.is_some() || app.selector_focus {
                self.hits.iter().collect()
            } else {
                self.controls(app.pane)
            };
            if let Some(hit) = candidates.into_iter().find(|hit| hit.action == action) {
                frame
                    .buffer_mut()
                    .set_style(hit.area, theme.focused_control());
            }
        }
    }
}

fn score(origin: Rect, target: Rect, direction: Direction) -> Option<(u16, u16, u16)> {
    let cx = origin.x + origin.width / 2;
    let cy = origin.y + origin.height / 2;
    let tx = target.x + target.width / 2;
    let ty = target.y + target.height / 2;
    let horizontal = matches!(direction, Direction::Left | Direction::Right);
    let in_direction = match direction {
        Direction::Left => tx < cx,
        Direction::Right => tx > cx,
        Direction::Up => ty < cy,
        Direction::Down => ty > cy,
    };
    if !in_direction || (horizontal && (target.bottom() <= origin.y || target.y >= origin.bottom()))
    {
        return None;
    }
    if horizontal {
        Some((cx.abs_diff(tx), cy.abs_diff(ty), target.x))
    } else {
        let gap = if target.right() <= origin.x {
            origin.x - target.right()
        } else {
            target.x.saturating_sub(origin.right())
        };
        Some((cy.abs_diff(ty), gap, cx.abs_diff(tx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, demo};
    use ratatui::{Terminal, backend::TestBackend};

    fn render(ui: &mut Ui, app: &App, width: u16, height: u16) {
        Terminal::new(TestBackend::new(width, height))
            .unwrap()
            .draw(|frame| ui.draw(frame, app))
            .unwrap();
    }

    #[test]
    fn horizontal_keys_stay_inside_pane_and_focus_neighboring_options() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        let mut ui = Ui::default();
        render(&mut ui, &app, 160, 48);
        ui.navigate_control(&mut app, Direction::Right);
        assert_eq!(app.pane, Pane::Wallet);
        assert_eq!(app.focused_control, Some(Action::Send));
        assert_eq!(app.view, View::Overview);
        app.focused_control = Some(Action::Fund);
        render(&mut ui, &app, 160, 48);
        ui.navigate_control(&mut app, Direction::Right);
        assert_eq!(app.focused_control, Some(Action::Send));
        ui.navigate_control(&mut app, Direction::Left);
        assert_eq!(app.focused_control, Some(Action::Fund));
        assert_eq!(app.pane, Pane::Wallet);
    }

    #[test]
    fn list_navigation_scrolls_to_offscreen_rows_without_changing_panes() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        app.focused_control = Some(Action::SelectTransaction(0));
        let mut ui = Ui::default();
        for index in 1..app.records.len() {
            render(&mut ui, &app, 60, 10);
            ui.navigate_control(&mut app, Direction::Down);
            assert_eq!(app.transaction_cursor, index);
            assert_eq!(app.pane, Pane::Wallet);
        }
    }

    #[test]
    fn mouse_and_keyboard_share_the_same_focused_action() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        let mut ui = Ui::default();
        render(&mut ui, &app, 120, 16);
        let fund = ui
            .hits
            .iter()
            .find(|hit| hit.action == Action::Fund)
            .unwrap()
            .area;
        assert_eq!(ui.click(&mut app, fund.x, fund.y), Some(Action::Fund));
        ui.navigate_control(&mut app, Direction::Right);
        assert_eq!(ui.focused_action(&app), Some(Action::Send));
        assert_eq!(app.pane, Pane::Wallet);
    }

    #[test]
    fn directional_navigation_never_escapes_the_focused_pane() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        let mut ui = Ui::default();
        for (width, height) in [(60, 10), (120, 16), (160, 48)] {
            for pane in Pane::ALL {
                app.navigate(&Action::Focus(pane));
                for direction in [
                    Direction::Right,
                    Direction::Down,
                    Direction::Left,
                    Direction::Up,
                ] {
                    render(&mut ui, &app, width, height);
                    ui.navigate_control(&mut app, direction);
                    assert_eq!(app.pane, pane);
                }
            }
        }
    }

    #[test]
    fn settings_options_are_reachable_in_both_layouts() {
        let mut app = App::new("/test".into(), Config::default(), vec![]);
        demo::populate(&mut app);
        let mut ui = Ui::default();
        for (width, height) in [(60, 10), (160, 48)] {
            app.navigate(&Action::SetTab(Tab::Settings));
            app.focused_control = Some(Action::Theme);
            render(&mut ui, &app, width, height);
            ui.navigate_control(&mut app, Direction::Right);
            assert_eq!(ui.focused_action(&app), Some(Action::Motion));
            ui.navigate_control(&mut app, Direction::Left);
            assert_eq!(ui.focused_action(&app), Some(Action::Theme));
        }
    }

    #[test]
    fn up_reaches_main_selector_and_down_reenters_selected_pane() {
        for (width, height) in [(60, 10), (120, 16), (160, 48)] {
            let mut app = App::new("/test".into(), Config::default(), vec![]);
            demo::populate(&mut app);
            let mut ui = Ui::default();
            render(&mut ui, &app, width, height);
            for _ in 0..12 {
                ui.navigate_control(&mut app, Direction::Up);
                if app.selector_focus {
                    break;
                }
            }
            assert!(app.selector_focus);
            ui.navigate_control(&mut app, Direction::Right);
            assert_eq!(app.view, View::Wallets);
            assert_eq!(app.pane, Pane::Wallets);
            ui.navigate_control(&mut app, Direction::Left);
            assert_eq!(app.pane, Pane::Wallet);
            ui.navigate_control(&mut app, Direction::Down);
            assert!(!app.selector_focus);
            assert_eq!(app.pane, Pane::Wallet);
        }
    }
}
