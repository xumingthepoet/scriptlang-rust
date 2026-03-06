use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sl_api::Snapshot;

pub(crate) const PLAYER_STATE_SCHEMA: &str = "player-state";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PlayerRandomMode {
    Seeded,
    Sequence,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedScenario {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) scripts_xml: BTreeMap<String, String>,
    pub(crate) entry_script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlayerState {
    pub(crate) schema_version: String,
    pub(crate) scenario_id: String,
    pub(crate) compiler_version: String,
    pub(crate) snapshot: Snapshot,
    pub(crate) random_mode: PlayerRandomMode,
    pub(crate) random_seed_state: Option<u32>,
    #[serde(default)]
    pub(crate) random_sequence: Vec<u32>,
    pub(crate) random_sequence_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundaryEvent {
    Choices,
    Input,
    End,
}

#[derive(Debug, Clone)]
pub(crate) struct TextEvent {
    pub(crate) text: String,
    pub(crate) tag: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DebugEvent {
    pub(crate) text: String,
}

#[derive(Debug, Clone)]
pub(crate) enum OutputEvent {
    Text(TextEvent),
    Debug(DebugEvent),
}

#[derive(Debug, Clone)]
pub(crate) struct BoundaryResult {
    pub(crate) event: BoundaryEvent,
    pub(crate) outputs: Vec<OutputEvent>,
    pub(crate) choices: Vec<(usize, String)>,
    pub(crate) choice_prompt_text: Option<String>,
    pub(crate) input_prompt_text: Option<String>,
    pub(crate) input_default_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiCommandAction {
    NotHandled,
    Continue,
    RefreshBoundary,
    Quit,
}

pub(crate) struct TuiCommandContext<'a> {
    pub(crate) state_file: &'a str,
    pub(crate) scenario: &'a LoadedScenario,
    pub(crate) entry_script: &'a str,
    pub(crate) random_sequence: Option<Vec<u32>>,
}
