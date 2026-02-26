#![cfg(not(coverage))]

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::tui_state::TuiUiState;
use crate::LoadedScenario;

const CHOICE_VIEWPORT_ROWS: usize = 5;
const ELLIPSIS: &str = "…";

pub(crate) fn render_tui(
    frame: &mut Frame<'_>,
    ui: &TuiUiState,
    scenario: &LoadedScenario,
    state_file: &str,
) {
    let terminal_width = frame.area().width as usize;
    let terminal_rows = frame.area().height as usize;

    let typing_in_progress = ui.typing_in_progress();
    let lines = if let Some(typing) = &ui.typing_line {
        let typing_part = typing.chars().take(ui.typing_chars).collect::<String>();
        let mut out = ui.rendered_lines.clone();
        out.push(typing_part);
        out
    } else {
        ui.rendered_lines.clone()
    };

    let content_width = (terminal_width.saturating_sub(2)).max(16);
    let wrapped_text_rows = lines
        .iter()
        .flat_map(|line| wrap_line_to_width(line, content_width))
        .collect::<Vec<_>>();

    let input_mode_enabled = !typing_in_progress && ui.input_prompt_text.is_some();
    let choice_display_enabled =
        !typing_in_progress && !input_mode_enabled && !ui.choices.is_empty();
    let choice_rows_source = if choice_display_enabled {
        ui.choices.clone()
    } else {
        Vec::new()
    };

    let interaction_header_raw = if input_mode_enabled {
        ui.input_prompt_text.clone().unwrap_or_default()
    } else if choice_display_enabled {
        ui.choice_prompt_text
            .clone()
            .unwrap_or_else(|| "choices (up/down + enter):".to_string())
    } else {
        String::new()
    };
    let choice_header_text = truncate_to_width(&interaction_header_raw, content_width);

    let mut reserved_rows = 3usize + 1usize + CHOICE_VIEWPORT_ROWS + 1usize + 1usize;
    if ui.ended {
        reserved_rows += 1;
    }
    if ui.help_visible {
        reserved_rows += 1;
    }
    if !choice_header_text.is_empty() {
        reserved_rows += 1;
    }
    let visible_text_rows = terminal_rows.saturating_sub(reserved_rows).max(1);
    let clipped_text_rows = if wrapped_text_rows.len() <= visible_text_rows {
        wrapped_text_rows
    } else {
        wrapped_text_rows[wrapped_text_rows.len() - visible_text_rows..].to_vec()
    };

    let choice_text_width = content_width.saturating_sub(2).max(8);
    let visible_choice_rows = (0..CHOICE_VIEWPORT_ROWS)
        .map(|row_index| {
            if input_mode_enabled {
                if row_index == 0 {
                    return (
                        truncate_to_width(ui.input_buffer.as_str(), choice_text_width),
                        true,
                    );
                }
                return (" ".to_string(), false);
            }
            let absolute_index = ui.choice_scroll_offset + row_index;
            let Some(choice) = choice_rows_source.get(absolute_index) else {
                return (" ".to_string(), false);
            };
            (
                truncate_to_width(choice.text.as_str(), choice_text_width),
                absolute_index == ui.selected_choice_index,
            )
        })
        .collect::<Vec<_>>();

    let window_start = if choice_rows_source.is_empty() {
        0
    } else {
        ui.choice_scroll_offset + 1
    };
    let window_end = if choice_rows_source.is_empty() {
        0
    } else {
        (ui.choice_scroll_offset + CHOICE_VIEWPORT_ROWS).min(choice_rows_source.len())
    };
    let choice_window_text = if input_mode_enabled {
        truncate_to_width(
            format!(
                "default: {}",
                ui.input_default_text.clone().unwrap_or_default()
            )
            .as_str(),
            content_width,
        )
    } else if choice_rows_source.len() > CHOICE_VIEWPORT_ROWS {
        truncate_to_width(
            format!(
                "window {}-{} / {}",
                window_start,
                window_end,
                choice_rows_source.len()
            )
            .as_str(),
            content_width,
        )
    } else {
        " ".to_string()
    };

    let header_text = truncate_to_width(
        format!("{} | {}", scenario.id, scenario.title).as_str(),
        content_width,
    );
    let state_text = truncate_to_width(format!("state: {}", state_file).as_str(), content_width);
    let status_text = truncate_to_width(format!("status: {}", ui.status).as_str(), content_width);
    let divider_line = "─".repeat(content_width);
    let key_text = truncate_to_width(
        "keys: up/down move | type+backspace input | enter submit/choose | s save | l load | r restart | h help | q quit",
        content_width,
    );
    let help_text = truncate_to_width(
        "snapshot is valid only when waiting at choices/input. if save fails, continue until interaction appears.",
        content_width,
    );

    let mut lines_out: Vec<Line<'_>> = Vec::new();
    lines_out.push(Line::from(header_text));
    lines_out.push(Line::from(Span::styled(
        state_text,
        Style::default().fg(Color::Gray),
    )));
    lines_out.push(Line::from(Span::styled(
        status_text,
        Style::default().fg(Color::Gray),
    )));
    for row in clipped_text_rows {
        lines_out.push(Line::from(row));
    }
    lines_out.push(Line::from(Span::styled(
        divider_line,
        Style::default().fg(Color::Gray),
    )));
    if !choice_header_text.is_empty() {
        lines_out.push(Line::from(Span::styled(
            choice_header_text,
            Style::default().fg(Color::Cyan),
        )));
    }
    for (text, selected) in visible_choice_rows {
        let prefix = if selected { "> " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };
        lines_out.push(Line::from(Span::styled(
            format!("{}{}", prefix, text),
            style,
        )));
    }
    lines_out.push(Line::from(Span::styled(
        choice_window_text,
        Style::default().fg(Color::Gray),
    )));
    if ui.ended {
        lines_out.push(Line::from(Span::styled(
            "[end]".to_string(),
            Style::default().fg(Color::Green),
        )));
    }
    lines_out.push(Line::from(Span::styled(
        key_text,
        Style::default().fg(Color::Yellow),
    )));
    if ui.help_visible {
        lines_out.push(Line::from(Span::styled(
            help_text,
            Style::default().fg(Color::Magenta),
        )));
    }

    let paragraph = Paragraph::new(lines_out).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, frame.area());
}

fn truncate_to_width(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= width {
        return value.to_string();
    }
    if width == 1 {
        return ELLIPSIS.to_string();
    }
    let mut out = chars.into_iter().take(width - 1).collect::<String>();
    out.push_str(ELLIPSIS);
    out
}

fn wrap_line_to_width(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let chars = value.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return vec![String::new()];
    }
    let mut rows = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        rows.push(
            chars[index..(index + width).min(chars.len())]
                .iter()
                .collect(),
        );
        index += width;
    }
    rows
}
