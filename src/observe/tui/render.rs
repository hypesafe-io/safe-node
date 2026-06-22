use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use serde_json::Value;

use super::types::TuiData;

pub(super) fn draw_dashboard(
    frame: &mut ratatui::Frame<'_>,
    data: &TuiData,
    selected_offset: usize,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let status = pretty_panel("Status", data.status.as_ref(), data.error.as_deref());
    frame.render_widget(status, chunks[0]);

    let config_policy = Text::from(vec![
        Line::from(format_value("Config", data.config.as_ref())),
        Line::from(format_value("Policy", data.policy.as_ref())),
    ]);
    frame.render_widget(
        Paragraph::new(config_policy)
            .block(Block::new().title("Config / Policy").borders(Borders::ALL)),
        chunks[1],
    );

    let items = data
        .transactions
        .iter()
        .skip(selected_offset)
        .take(20)
        .map(|tx| ListItem::new(transaction_line(tx)))
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(
            Block::new()
                .title("Recent transactions")
                .borders(Borders::ALL),
        ),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new("q quit | r refresh | up/down scroll")
            .block(Block::new().borders(Borders::ALL)),
        chunks[3],
    );
}

fn pretty_panel<'a>(title: &'a str, value: Option<&Value>, error: Option<&str>) -> Paragraph<'a> {
    let text = match (error, value) {
        (Some(err), _) => Text::from(format!("connection error: {err}")),
        (None, Some(value)) => Text::from(pretty_json(value)),
        (None, None) => Text::from("loading"),
    };
    Paragraph::new(text).block(Block::new().title(title).borders(Borders::ALL))
}

fn format_value(label: &str, value: Option<&Value>) -> String {
    match value {
        Some(value) => format!("{label}: {}", compact_json(value)),
        None => format!("{label}: loading"),
    }
}

fn transaction_line(value: &Value) -> String {
    let task_id = value.get("task_id").and_then(Value::as_str).unwrap_or("-");
    let status = value
        .get("local_status")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let template = value
        .get("template_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let reason = value
        .get("reject_reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    format!("{task_id} | {status} | {template} | {reason}")
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}
