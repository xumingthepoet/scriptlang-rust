#[cfg(coverage)]
pub(super) fn run_tui_ratatui_mode(
    state_file: &str,
    scenario: &super::LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<i32, sl_core::ScriptLangError> {
    super::run_tui_line_mode(state_file, scenario, entry_script, engine)
}

#[cfg(not(coverage))]
mod rich {
    use std::io;
    use std::time::{Duration, Instant};

    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use crossterm::ExecutableCommand;
    use ratatui::backend::CrosstermBackend;
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Paragraph, Wrap};
    use ratatui::{Frame, Terminal};
    use sl_api::{
        create_engine_from_xml, resume_engine_from_xml, CreateEngineFromXmlOptions,
        ResumeEngineFromXmlOptions,
    };
    use sl_core::ScriptLangError;
    use sl_runtime::DEFAULT_COMPILER_VERSION;

    use crate::{
        load_player_state, run_to_boundary, save_player_state, BoundaryEvent, BoundaryResult,
        LoadedScenario, PlayerStateV3, PLAYER_STATE_SCHEMA,
    };

    const CHOICE_VIEWPORT_ROWS: usize = 5;
    const TYPEWRITER_CHARS_PER_SECOND: usize = 60;
    const TYPEWRITER_TICK_MS: u64 = (1000 / TYPEWRITER_CHARS_PER_SECOND) as u64;
    const ELLIPSIS: &str = "…";

    #[derive(Debug, Clone)]
    struct ChoiceRow {
        index: usize,
        text: String,
    }

    #[derive(Debug, Default)]
    struct TuiUiState {
        rendered_lines: Vec<String>,
        pending_lines: Vec<String>,
        typing_line: Option<String>,
        typing_chars: usize,
        choices: Vec<ChoiceRow>,
        choice_prompt_text: Option<String>,
        input_prompt_text: Option<String>,
        input_default_text: Option<String>,
        input_buffer: String,
        selected_choice_index: usize,
        choice_scroll_offset: usize,
        ended: bool,
        help_visible: bool,
        status: String,
    }

    struct TuiTerminal {
        terminal: Terminal<CrosstermBackend<io::Stdout>>,
    }

    impl TuiTerminal {
        fn new() -> Result<Self, ScriptLangError> {
            enable_raw_mode().map_err(map_tui_io)?;
            io::stdout()
                .execute(EnterAlternateScreen)
                .map_err(map_tui_io)?;
            let backend = CrosstermBackend::new(io::stdout());
            let terminal = Terminal::new(backend).map_err(map_tui_io)?;
            Ok(Self { terminal })
        }

        fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
            &mut self.terminal
        }
    }

    impl Drop for TuiTerminal {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let _ = io::stdout().execute(LeaveAlternateScreen);
        }
    }

    pub(super) fn run_tui_ratatui_mode(
        state_file: &str,
        scenario: &LoadedScenario,
        entry_script: &str,
        engine: &mut sl_runtime::ScriptLangEngine,
    ) -> Result<i32, ScriptLangError> {
        let mut terminal = TuiTerminal::new()?;
        let mut ui = TuiUiState {
            status: "ready".to_string(),
            ..TuiUiState::default()
        };
        let boundary = run_to_boundary(engine)?;
        ui.replace_boundary(boundary);

        let tick = Duration::from_millis(TYPEWRITER_TICK_MS);
        let mut last_tick = Instant::now();

        loop {
            terminal
                .terminal_mut()
                .draw(|frame| render_tui(frame, &ui, scenario, state_file))
                .map_err(map_tui_io)?;

            if last_tick.elapsed() >= tick && ui.advance_typewriter() {
                last_tick = Instant::now();
            }

            let timeout = tick.saturating_sub(last_tick.elapsed());
            if !event::poll(timeout).map_err(map_tui_io)? {
                continue;
            }

            let evt = event::read().map_err(map_tui_io)?;
            if let Event::Key(key) = evt {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let should_quit =
                    match handle_key(key, state_file, scenario, entry_script, engine, &mut ui) {
                        Ok(should_quit) => should_quit,
                        Err(error) => {
                            ui.status = error.message;
                            false
                        }
                    };
                if should_quit {
                    break;
                }
            }
        }

        Ok(0)
    }

    impl TuiUiState {
        fn typing_in_progress(&self) -> bool {
            self.typing_line.is_some() || !self.pending_lines.is_empty()
        }

        fn set_boundary_state(&mut self, boundary: BoundaryResult) {
            match boundary.event {
                BoundaryEvent::Choices => {
                    self.choices = boundary
                        .choices
                        .into_iter()
                        .map(|(index, text)| ChoiceRow { index, text })
                        .collect();
                    self.choice_prompt_text = boundary.choice_prompt_text;
                    self.input_prompt_text = None;
                    self.input_default_text = None;
                    self.input_buffer.clear();
                    self.ended = false;
                    self.selected_choice_index = 0;
                    self.choice_scroll_offset = 0;
                }
                BoundaryEvent::Input => {
                    self.choices.clear();
                    self.choice_prompt_text = None;
                    self.input_prompt_text = boundary.input_prompt_text;
                    self.input_default_text = boundary.input_default_text;
                    self.input_buffer = self.input_default_text.clone().unwrap_or_default();
                    self.ended = false;
                    self.selected_choice_index = 0;
                    self.choice_scroll_offset = 0;
                }
                BoundaryEvent::End => {
                    self.choices.clear();
                    self.choice_prompt_text = None;
                    self.input_prompt_text = None;
                    self.input_default_text = None;
                    self.input_buffer.clear();
                    self.ended = true;
                    self.selected_choice_index = 0;
                    self.choice_scroll_offset = 0;
                }
            }
        }

        fn append_boundary(&mut self, boundary: BoundaryResult) {
            if !boundary.texts.is_empty() {
                self.pending_lines.extend(boundary.texts.clone());
            }
            self.set_boundary_state(boundary);
        }

        fn replace_boundary(&mut self, boundary: BoundaryResult) {
            self.rendered_lines.clear();
            self.pending_lines = boundary.texts.clone();
            self.typing_line = None;
            self.typing_chars = 0;
            self.set_boundary_state(boundary);
        }

        fn advance_typewriter(&mut self) -> bool {
            if self.typing_line.is_none() {
                if self.pending_lines.is_empty() {
                    return false;
                }
                let next_line = self.pending_lines.remove(0);
                if next_line.is_empty() {
                    self.rendered_lines.push(next_line);
                    return true;
                }
                self.typing_line = Some(next_line);
                self.typing_chars = 1;
                return true;
            }

            let line = self
                .typing_line
                .as_ref()
                .expect("typing line should exist when typing");
            let total_chars = line.chars().count();
            if self.typing_chars >= total_chars {
                self.rendered_lines.push(line.clone());
                self.typing_line = None;
                self.typing_chars = 0;
                return true;
            }
            self.typing_chars += 1;
            true
        }
    }

    fn handle_key(
        key: crossterm::event::KeyEvent,
        state_file: &str,
        scenario: &LoadedScenario,
        entry_script: &str,
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
                let next = create_engine_from_xml(CreateEngineFromXmlOptions {
                    scripts_xml: scenario.scripts_xml.clone(),
                    entry_script: Some(entry_script.to_string()),
                    entry_args: None,
                    host_functions: None,
                    random_seed: None,
                    compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
                })?;
                *engine = next;
                let boundary = run_to_boundary(engine)?;
                ui.replace_boundary(boundary);
                ui.status = "restarted".to_string();
                return Ok(false);
            }
            KeyCode::Char('s') => {
                let snapshot = engine.snapshot()?;
                let state = PlayerStateV3 {
                    schema_version: PLAYER_STATE_SCHEMA.to_string(),
                    scenario_id: scenario.id.clone(),
                    compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
                    snapshot,
                };
                save_player_state(std::path::Path::new(state_file), &state)?;
                ui.status = format!("saved to {}", state_file);
                return Ok(false);
            }
            KeyCode::Char('l') => {
                let state = load_player_state(std::path::Path::new(state_file))?;
                if state.scenario_id != scenario.id {
                    return Err(ScriptLangError::new(
                        "TUI_STATE_SCENARIO_MISMATCH",
                        format!(
                            "State scenario mismatch. expected={} actual={}",
                            scenario.id, state.scenario_id
                        ),
                    ));
                }
                let resumed = resume_engine_from_xml(ResumeEngineFromXmlOptions {
                    scripts_xml: scenario.scripts_xml.clone(),
                    snapshot: state.snapshot,
                    host_functions: None,
                    compiler_version: Some(state.compiler_version),
                })?;
                *engine = resumed;
                let boundary = run_to_boundary(engine)?;
                ui.append_boundary(boundary);
                ui.status = format!("loaded from {}", state_file);
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
                let selected = ui.choices.get(ui.selected_choice_index).ok_or_else(|| {
                    ScriptLangError::new("TUI_CHOICE_PARSE", "No choices available")
                })?;
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
    ) -> Result<BoundaryResult, ScriptLangError> {
        engine.choose(choice_index)?;
        run_to_boundary(engine)
    }

    fn submit_current_input(
        engine: &mut sl_runtime::ScriptLangEngine,
        input: &str,
    ) -> Result<BoundaryResult, ScriptLangError> {
        engine.submit_input(input)?;
        run_to_boundary(engine)
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

    fn render_tui(
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
        let state_text =
            truncate_to_width(format!("state: {}", state_file).as_str(), content_width);
        let status_text =
            truncate_to_width(format!("status: {}", ui.status).as_str(), content_width);
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

    fn map_tui_io(error: std::io::Error) -> ScriptLangError {
        ScriptLangError::new("TUI_IO", error.to_string())
    }
}

#[cfg(not(coverage))]
pub(super) fn run_tui_ratatui_mode(
    state_file: &str,
    scenario: &super::LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<i32, sl_core::ScriptLangError> {
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return super::run_tui_line_mode(state_file, scenario, entry_script, engine);
    }
    rich::run_tui_ratatui_mode(state_file, scenario, entry_script, engine)
}
