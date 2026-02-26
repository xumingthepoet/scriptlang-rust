mod rich {
    use std::io;
    use std::time::{Duration, Instant};

    use crossterm::event::{self, Event, KeyEventKind};
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use crossterm::ExecutableCommand;
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use sl_core::ScriptLangError;

    use crate::tui_actions::{handle_key, TuiActionContext};
    use crate::tui_render::render_tui;
    use crate::tui_state::TuiUiState;
    use crate::{map_tui_io, run_to_boundary, LoadedScenario};

    const TYPEWRITER_CHARS_PER_SECOND: usize = 60;
    const TYPEWRITER_TICK_MS: u64 = (1000 / TYPEWRITER_CHARS_PER_SECOND) as u64;

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
        let action_context = TuiActionContext {
            state_file,
            scenario,
            entry_script,
        };

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
                let should_quit = match handle_key(key, &action_context, engine, &mut ui) {
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
}

fn should_force_line_mode() -> bool {
    cfg!(test) || std::env::var_os("RUST_TEST_THREADS").is_some()
}

pub(super) fn run_tui_ratatui_mode(
    state_file: &str,
    scenario: &super::LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<i32, sl_core::ScriptLangError> {
    use std::io::IsTerminal;

    if should_force_line_mode()
        || !std::io::stdin().is_terminal()
        || !std::io::stdout().is_terminal()
    {
        return super::run_tui_line_mode(state_file, scenario, entry_script, engine);
    }
    rich::run_tui_ratatui_mode(state_file, scenario, entry_script, engine)
}
