//! Core traits for TUI component abstraction and event dispatching.
//!
//! These traits allow the massive `handle_event` match in app.rs to be
//! decomposed into per-component handlers, reducing App from a God-struct
//! to a thin coordinator.

use crate::tui::app::AppEvent;
use crossterm::event::KeyEvent;
use ratatui::{layout::Rect, Frame};

/// A stateful panel component that can handle key events and render itself.
///
/// Implementors own their state and can be composed into larger containers.
/// The `handle_key` method returns `true` if the key was consumed,
/// indicating that no further handlers should process it.
pub trait Component {
    /// Handle a key event. Returns `true` if consumed (stops event bubbling).
    fn handle_key(&mut self, _key: &KeyEvent) -> bool {
        false
    }

    /// Render the component into the given frame area.
    fn render(&self, _f: &mut Frame, _area: Rect) {}
}

/// A handler for AppEvent variants.
///
/// Instead of a single giant match statement, events can be dispatched
/// to typed handlers that each own a cohesive subset of App state.
/// Returns `true` if the event was handled.
pub trait EventHandler {
    /// Handle an AppEvent. Returns `true` if consumed.
    fn handle_event(&mut self, _event: &AppEvent) -> bool {
        false
    }
}
