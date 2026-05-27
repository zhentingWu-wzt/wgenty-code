//! IME-aware key event handler for CJK input support
//!
//! crossterm 0.28 does not yet have `Event::Ime` (it's in a future release).
//! This module provides a workaround: detect CJK characters arriving as `KeyCode::Char`
//! and treat them as committed IME text.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};

/// Action returned after processing an event
pub enum ImeAction {
    /// Committed text (from IME or direct CJK character input)
    Committed(String),
    /// Event should be ignored
    Ignored,
    /// Event is not text input, pass through to normal handling
    Passthrough,
}

/// Handles IME-aware key event processing
pub struct ImeHandler {
    composing: bool,
}

impl ImeHandler {
    pub fn new() -> Self {
        Self { composing: false }
    }

    pub fn is_composing(&self) -> bool {
        self.composing
    }

    /// Mark that an IME composition has started (called externally if needed)
    pub fn start_composition(&mut self) {
        self.composing = true;
    }

    /// Mark that an IME composition has ended
    pub fn end_composition(&mut self) {
        self.composing = false;
    }

    pub fn handle_event(&mut self, event: &Event) -> ImeAction {
        match event {
            Event::Key(key) => self.handle_key_event(key),
            _ => ImeAction::Passthrough,
        }
    }

    fn handle_key_event(&mut self, key: &KeyEvent) -> ImeAction {
        // Only process Press events
        if key.kind != KeyEventKind::Press {
            return ImeAction::Ignored;
        }

        if let KeyCode::Char(c) = key.code {
            if is_cjk_char(c) {
                // CJK character arriving as KeyCode::Char — this is committed
                // IME text on terminals where Event::Ime is not available
                self.composing = false;
                return ImeAction::Committed(c.to_string());
            }
        }

        // Space during composition could be IME confirmation
        if self.composing && key.code == KeyCode::Char(' ') {
            self.composing = false;
            return ImeAction::Ignored;
        }

        ImeAction::Passthrough
    }
}

/// Check if a character is a CJK character
fn is_cjk_char(c: char) -> bool {
    let cp = c as u32;
    (0x4E00..=0x9FFF).contains(&cp)       // CJK Unified Ideographs
        || (0x3400..=0x4DBF).contains(&cp) // CJK Extension A
        || (0xF900..=0xFAFF).contains(&cp) // CJK Compatibility Ideographs
        || (0x3040..=0x309F).contains(&cp) // Hiragana
        || (0x30A0..=0x30FF).contains(&cp) // Katakana
        || (0xAC00..=0xD7AF).contains(&cp) // Hangul Syllables
        || (0x1100..=0x11FF).contains(&cp) // Hangul Jamo
        || (0x20000..=0x2FA1F).contains(&cp) // CJK Extension B-F
        || (0xFF01..=0xFF60).contains(&cp) // Fullwidth forms
}

impl Default for ImeHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ime_handler_default() {
        let handler = ImeHandler::default();
        assert!(!handler.is_composing());
    }

    #[test]
    fn test_is_cjk_char() {
        assert!(is_cjk_char('你'));
        assert!(is_cjk_char('の'));
        assert!(is_cjk_char('한'));
        assert!(!is_cjk_char('a'));
        assert!(!is_cjk_char('1'));
    }
}
