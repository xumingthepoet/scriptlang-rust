use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sl_core::SnapshotV3;

pub(crate) const PLAYER_STATE_SCHEMA: &str = "player-state.v3";

#[derive(Debug, Clone)]
pub(crate) struct LoadedScenario {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) scripts_xml: BTreeMap<String, String>,
    pub(crate) entry_script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlayerStateV3 {
    pub(crate) schema_version: String,
    pub(crate) scenario_id: String,
    pub(crate) compiler_version: String,
    pub(crate) snapshot: SnapshotV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundaryEvent {
    Choices,
    Input,
    End,
}

#[derive(Debug, Clone)]
pub(crate) struct BoundaryResult {
    pub(crate) event: BoundaryEvent,
    pub(crate) texts: Vec<String>,
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
}
