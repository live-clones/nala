use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};

pub mod fetch;
pub mod progress;

pub use progress::NalaProgressBar;

pub fn poll_exit_event() -> Result<bool> {
	if crossterm::event::poll(Duration::from_millis(0))? {
		if let Event::Key(key) = event::read()? {
			if KeyCode::Char('q') == key.code {
				return Ok(true);
			}

			if KeyCode::Char('c') == key.code && key.modifiers.contains(KeyModifiers::CONTROL) {
				return Ok(true);
			}
		}
	}
	Ok(false)
}
