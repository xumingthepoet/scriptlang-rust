use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sl_core::ScriptLangError;
use sl_runtime::DEFAULT_COMPILER_VERSION;

use crate::tui_state::TuiUiState;
use crate::{
    create_engine_for_scenario, load_engine_from_state_for_scenario, run_to_boundary,
    save_engine_state, LoadedScenario,
};

const CHOICE_VIEWPORT_ROWS: usize = 5;

pub(crate) struct TuiActionContext<'a> {
    pub(crate) state_file: &'a str,
    pub(crate) scenario: &'a LoadedScenario,
    pub(crate) entry_script: &'a str,
}

pub(crate) fn handle_key(
    key: KeyEvent,
    context: &TuiActionContext<'_>,
    engine: &mut sl_runtime::ScriptLangEngine,
    ui: &mut TuiUiState,
) -> Result<bool, ScriptLangError> {
    if key.code == KeyCode::Esc || matches!(key.code, KeyCode::Char('q')) {
        return Ok(true);
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(true);
    }

    match key.code {
        KeyCode::Char('h') => {
            ui.help_visible = !ui.help_visible;
            return Ok(false);
        }
        KeyCode::Char('r') => {
            let next = create_engine_for_scenario(context.scenario, context.entry_script)?;
            *engine = next;
            let boundary = run_to_boundary(engine)?;
            ui.replace_boundary(boundary);
            ui.status = "restarted".to_string();
            return Ok(false);
        }
        KeyCode::Char('s') => {
            save_engine_state(
                Path::new(context.state_file),
                engine,
                &context.scenario.id,
                DEFAULT_COMPILER_VERSION,
            )?;
            ui.status = format!("saved to {}", context.state_file);
            return Ok(false);
        }
        KeyCode::Char('l') => {
            let (_state, resumed) = load_engine_from_state_for_scenario(
                Path::new(context.state_file),
                context.scenario,
            )?;
            *engine = resumed;
            let boundary = run_to_boundary(engine)?;
            ui.append_boundary(boundary);
            ui.status = format!("loaded from {}", context.state_file);
            return Ok(false);
        }
        _ => {}
    }

    let typing_in_progress = ui.typing_in_progress();
    let input_pending = ui.input_prompt_text.is_some();
    let input_mode = ui.input_prompt_text.is_some() && !typing_in_progress;

    match key.code {
        KeyCode::Up => {
            if typing_in_progress {
                ui.status = "text streaming...".to_string();
                return Ok(false);
            }
            if input_mode {
                ui.status = "input mode".to_string();
                return Ok(false);
            }
            if ui.choices.is_empty() {
                ui.status = "no pending choice".to_string();
                return Ok(false);
            }
            ui.selected_choice_index = ui.selected_choice_index.saturating_sub(1);
            if ui.selected_choice_index < ui.choice_scroll_offset {
                ui.choice_scroll_offset = ui.selected_choice_index;
            }
        }
        KeyCode::Down => {
            if typing_in_progress {
                ui.status = "text streaming...".to_string();
                return Ok(false);
            }
            if input_mode {
                ui.status = "input mode".to_string();
                return Ok(false);
            }
            if ui.choices.is_empty() {
                ui.status = "no pending choice".to_string();
                return Ok(false);
            }
            let last = ui.choices.len().saturating_sub(1);
            ui.selected_choice_index = (ui.selected_choice_index + 1).min(last);
            if ui.choices.len() > CHOICE_VIEWPORT_ROWS
                && ui.selected_choice_index >= ui.choice_scroll_offset + CHOICE_VIEWPORT_ROWS
            {
                ui.choice_scroll_offset = ui.selected_choice_index - CHOICE_VIEWPORT_ROWS + 1;
            }
        }
        KeyCode::Backspace | KeyCode::Delete => {
            if input_pending {
                ui.input_buffer.pop();
            }
        }
        KeyCode::Enter => {
            if typing_in_progress {
                ui.status = "text streaming...".to_string();
                return Ok(false);
            }
            if input_mode {
                let boundary = submit_current_input(engine, ui.input_buffer.as_str())?;
                ui.append_boundary(boundary);
                ui.status = "submitted input".to_string();
                return Ok(false);
            }
            if ui.choices.is_empty() {
                ui.status = "no pending choice".to_string();
                return Ok(false);
            }
            let selected = ui
                .choices
                .get(ui.selected_choice_index)
                .ok_or_else(|| ScriptLangError::new("TUI_CHOICE_PARSE", "No choices available"))?;
            let boundary = choose_current(engine, selected.index)?;
            ui.append_boundary(boundary);
            ui.status = format!("chose {}", ui.selected_choice_index);
            return Ok(false);
        }
        KeyCode::Char(ch) => {
            if input_pending
                && !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                ui.input_buffer.push(ch);
            }
        }
        _ => {}
    }

    Ok(false)
}

fn choose_current(
    engine: &mut sl_runtime::ScriptLangEngine,
    choice_index: usize,
) -> Result<crate::BoundaryResult, ScriptLangError> {
    engine.choose(choice_index)?;
    run_to_boundary(engine)
}

fn submit_current_input(
    engine: &mut sl_runtime::ScriptLangEngine,
    input: &str,
) -> Result<crate::BoundaryResult, ScriptLangError> {
    engine.submit_input(input)?;
    run_to_boundary(engine)
}
