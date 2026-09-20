//! Ratatui modals: y/N confirm and free-text input (non-secret).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

/// Outcome of a confirm modal key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    /// Enter / y
    Yes,
    /// Esc / n
    No,
    /// Key ignored
    Ignore,
}

/// Handle y/N / Enter / Esc for a confirm overlay.
#[must_use]
pub fn confirm_key(key: KeyEvent) -> ConfirmResult {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmResult::Yes,
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => ConfirmResult::No,
        _ => ConfirmResult::Ignore,
    }
}

/// Free-text modal (non-secret). Global shortcuts must not fire while open.
#[derive(Debug, Clone)]
pub struct TextInput {
    title: String,
    buffer: String,
}

/// Result of handling a key in [`TextInput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputResult {
    /// Still editing
    Continue,
    /// Enter: submit current buffer (trimmed by caller if desired)
    Submit(String),
    /// Esc: cancel
    Cancel,
}

impl TextInput {
    /// Create a text modal with an optional initial value.
    #[must_use]
    pub fn new(title: impl Into<String>, initial: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            buffer: initial.into(),
        }
    }

    /// Apply a key. Printable chars edit; Backspace deletes; Enter/Esc finish.
    pub fn handle_key(&mut self, key: KeyEvent) -> TextInputResult {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return TextInputResult::Continue;
        }
        match key.code {
            KeyCode::Enter => TextInputResult::Submit(self.buffer.clone()),
            KeyCode::Esc => TextInputResult::Cancel,
            KeyCode::Backspace => {
                self.buffer.pop();
                TextInputResult::Continue
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.buffer.push(c);
                TextInputResult::Continue
            }
            _ => TextInputResult::Continue,
        }
    }
}

/// Draw a centered confirm dialog.
pub fn draw_confirm(f: &mut Frame, title: &str, body: &str) {
    let area = centered_rect(60, 40, f.area());
    f.render_widget(Clear, area);
    let text = format!("{body}\n\nEnter/y confirm · Esc/n cancel");
    let p = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(p, area);
}

/// Draw a centered text-input dialog.
pub fn draw_text_input(f: &mut Frame, input: &TextInput) {
    let area = centered_rect(70, 35, f.area());
    f.render_widget(Clear, area);
    let text = format!("{}\n\nEnter submit · Esc cancel", input.buffer);
    let p = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(input.title.as_str())
            .border_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    f.render_widget(p, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn confirm_yes_no() {
        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        let n = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(confirm_key(y), ConfirmResult::Yes);
        assert_eq!(confirm_key(n), ConfirmResult::No);
    }

    #[test]
    fn text_input_type_backspace_submit_cancel() {
        let mut t = TextInput::new("Host", "ho");
        assert_eq!(
            t.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
            TextInputResult::Continue
        );
        assert_eq!(t.buffer, "hor");
        assert_eq!(
            t.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            TextInputResult::Continue
        );
        assert_eq!(t.buffer, "ho");
        assert_eq!(
            t.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            TextInputResult::Submit("ho".into())
        );
        let mut t2 = TextInput::new("Host", "x");
        assert_eq!(
            t2.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            TextInputResult::Cancel
        );
    }

    #[test]
    fn text_input_ignores_control_chars_as_shortcuts() {
        let mut t = TextInput::new("Host", "");
        // Ctrl+c should not inject a character (handled as Continue).
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(t.handle_key(ctrl_c), TextInputResult::Continue);
        assert!(t.buffer.is_empty());
    }
}
