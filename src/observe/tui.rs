mod client;
mod render;
mod terminal;
mod types;

use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode};

use self::client::fetch_data;
use self::render::draw_dashboard;
use self::terminal::TerminalGuard;
use self::types::TuiData;
use crate::Result;

/// Runs the terminal dashboard against a debug HTTP endpoint.
///
/// # Errors
///
/// Returns an error when terminal setup, terminal rendering, or keyboard event
/// handling fails.
pub async fn run(url: String, limit: u32, refresh_secs: u64) -> Result<()> {
    let client = reqwest::Client::new();
    let mut guard = TerminalGuard::enter()?;
    let refresh = Duration::from_secs(refresh_secs.max(1));
    let now = Instant::now();
    let mut last_refresh = now.checked_sub(refresh).unwrap_or(now);
    let mut data = TuiData::default();
    let mut selected_offset = 0_usize;

    loop {
        if last_refresh.elapsed() >= refresh {
            data = fetch_data(&client, &url, limit).await;
            last_refresh = Instant::now();
        }

        guard
            .terminal
            .draw(|frame| draw_dashboard(frame, &data, selected_offset))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('r') => {
                        data = fetch_data(&client, &url, limit).await;
                        last_refresh = Instant::now();
                    }
                    KeyCode::Down => {
                        selected_offset = selected_offset.saturating_add(1);
                    }
                    KeyCode::Up => {
                        selected_offset = selected_offset.saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }
    }
}
