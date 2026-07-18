//! Blocking input reader running on a dedicated OS thread.
//!
//! Polls crossterm events and converts them to AppEvent messages.
//! Extracted from app.rs to reduce coupling.

use super::app::AppEvent;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::io;
use tokio::sync::mpsc;

pub fn read_input(
    tx: mpsc::UnboundedSender<AppEvent>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> io::Result<()> {
    use std::sync::atomic::Ordering;
    while !shutdown.load(Ordering::SeqCst) {
        // Poll with 100ms timeout so we can check shutdown flag frequently
        if event::poll(std::time::Duration::from_millis(100))? {
            let ev = event::read()?;
            if let Event::Mouse(mouse) = &ev {
                use crossterm::event::MouseEventKind;
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        let _ = tx.send(AppEvent::MouseScrolled(5));
                    }
                    MouseEventKind::ScrollDown => {
                        let _ = tx.send(AppEvent::MouseScrolled(-5));
                    }
                    _ => {}
                }
                continue;
            }
            if let Event::Paste(text) = &ev {
                let _ = tx.send(AppEvent::Paste(text.clone()));
                continue;
            }
            if let Event::Key(key) = ev {
                if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::CtrlCPressed);
                        continue;
                    }
                    // Session / memory panels are opened via slash commands
                    // (`/session`, `/memory`) — no Ctrl bindings (avoids terminal
                    // Ctrl+letter collisions like Ctrl+M == Enter).
                    if key.code == KeyCode::Char('t')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::ToggleTaskPanel);
                        continue;
                    }
                    if key.code == KeyCode::Char('e')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::ToggleCollapseAll);
                        continue;
                    }
                    if key.code == KeyCode::Char('o')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::ToggleCollapseLatest);
                        continue;
                    }
                    let _ = tx.send(AppEvent::KeyEvent(Box::new(key)));
                }
            }
        }
    }
    Ok(())
}
