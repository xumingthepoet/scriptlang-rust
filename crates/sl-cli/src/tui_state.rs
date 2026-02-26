use crate::{BoundaryEvent, BoundaryResult};

#[derive(Debug, Clone)]
pub(crate) struct ChoiceRow {
    pub(crate) index: usize,
    pub(crate) text: String,
}

#[derive(Debug, Default)]
pub(crate) struct TuiUiState {
    pub(crate) rendered_lines: Vec<String>,
    pub(crate) pending_lines: Vec<String>,
    pub(crate) typing_line: Option<String>,
    pub(crate) typing_chars: usize,
    pub(crate) choices: Vec<ChoiceRow>,
    pub(crate) choice_prompt_text: Option<String>,
    pub(crate) input_prompt_text: Option<String>,
    pub(crate) input_default_text: Option<String>,
    pub(crate) input_buffer: String,
    pub(crate) selected_choice_index: usize,
    pub(crate) choice_scroll_offset: usize,
    pub(crate) ended: bool,
    pub(crate) help_visible: bool,
    pub(crate) status: String,
}

impl TuiUiState {
    pub(crate) fn typing_in_progress(&self) -> bool {
        self.typing_line.is_some() || !self.pending_lines.is_empty()
    }

    pub(crate) fn set_boundary_state(&mut self, boundary: BoundaryResult) {
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

    pub(crate) fn append_boundary(&mut self, boundary: BoundaryResult) {
        if !boundary.texts.is_empty() {
            self.pending_lines.extend(boundary.texts.clone());
        }
        self.set_boundary_state(boundary);
    }

    pub(crate) fn replace_boundary(&mut self, boundary: BoundaryResult) {
        self.rendered_lines.clear();
        self.pending_lines = boundary.texts.clone();
        self.typing_line = None;
        self.typing_chars = 0;
        self.set_boundary_state(boundary);
    }

    pub(crate) fn advance_typewriter(&mut self) -> bool {
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
