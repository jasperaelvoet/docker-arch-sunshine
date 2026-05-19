use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::state::{AppState, LogSource, Modal};

pub enum KeyAction {
    None,
    SubmitPin,
}

pub fn handle(app: &mut AppState, key: KeyEvent) -> KeyAction {
    if app.modal == Modal::Shell {
        if key.code == KeyCode::F(10) {
            app.modal = Modal::None;
            app.shell = None;
            return KeyAction::None;
        }
        if let Some(shell) = app.shell.as_mut() {
            shell.send_key(key);
        }
        return KeyAction::None;
    }
    if app.modal == Modal::PairPin {
        return handle_modal_pin(app, key);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return KeyAction::None;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('r') => {
            app.modal = Modal::PairPin;
            app.pin_input.clear();
            app.pin_message.clear();
        }
        KeyCode::Char('s') => {
            app.should_shell = true;
        }
        KeyCode::Tab => {
            let i = LogSource::ALL.iter().position(|s| *s == app.focused_log).unwrap_or(0);
            let next = LogSource::ALL[(i + 1) % LogSource::ALL.len()];
            app.focus(next);
        }
        KeyCode::BackTab => {
            let i = LogSource::ALL.iter().position(|s| *s == app.focused_log).unwrap_or(0);
            let len = LogSource::ALL.len();
            let prev = LogSource::ALL[(i + len - 1) % len];
            app.focus(prev);
        }
        KeyCode::Up => {
            app.follow_tail = false;
            app.log_scroll = app.log_scroll.saturating_add(1);
        }
        KeyCode::Down => {
            if app.log_scroll == 0 {
                app.follow_tail = true;
            } else {
                app.log_scroll -= 1;
            }
        }
        KeyCode::PageUp => {
            app.follow_tail = false;
            app.log_scroll = app.log_scroll.saturating_add(20);
        }
        KeyCode::PageDown => {
            app.log_scroll = app.log_scroll.saturating_sub(20);
            if app.log_scroll == 0 {
                app.follow_tail = true;
            }
        }
        KeyCode::Char('f') | KeyCode::Char('F') => {
            app.follow_tail = true;
            app.log_scroll = 0;
        }
        _ => {}
    }
    KeyAction::None
}

fn handle_modal_pin(app: &mut AppState, key: KeyEvent) -> KeyAction {
    match key.code {
        KeyCode::Esc => {
            app.modal = Modal::None;
            app.pin_input.clear();
            app.pin_message.clear();
        }
        KeyCode::Enter => return KeyAction::SubmitPin,
        KeyCode::Backspace => {
            app.pin_input.pop();
        }
        KeyCode::Char(c) if c.is_ascii_digit() && app.pin_input.len() < 8 => {
            app.pin_input.push(c);
        }
        _ => {}
    }
    KeyAction::None
}
