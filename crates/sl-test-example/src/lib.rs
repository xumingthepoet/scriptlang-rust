mod case;
mod runner;
mod source;

pub use case::{ExpectedEvent, TestAction, TestCase, TESTCASE_SCHEMA};
pub use runner::{assert_case, run_case, RunReport};

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SlTestExampleError {
    #[error("Failed to read file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to parse testcase {path}: {source}")]
    ParseCase {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("Invalid testcase schema version \"{found}\", expected \"{expected}\".")]
    InvalidSchemaVersion { expected: String, found: String },
    #[error("No .script.xml/.defs.xml/.json files under {path}.")]
    SourceEmpty { path: PathBuf },
    #[error("Engine error: {0}")]
    Engine(#[from] sl_core::ScriptLangError),
    #[error("Action missing at event index {event_index}: expected {expected_action_kind}.")]
    MissingAction {
        event_index: usize,
        expected_action_kind: String,
    },
    #[error(
        "Action kind mismatch at event index {event_index}: expected {expected_action_kind}, got {actual_action_kind}."
    )]
    ActionKindMismatch {
        event_index: usize,
        expected_action_kind: String,
        actual_action_kind: String,
    },
    #[error("Unused actions: used {used} of {total}.")]
    UnusedActions { used: usize, total: usize },
    #[error("Guard exceeded: max_steps={max_steps}.")]
    GuardExceeded { max_steps: usize },
    #[error("Expected event count {expected}, actual {actual}. observed={observed}")]
    EventCountMismatch {
        expected: usize,
        actual: usize,
        observed: String,
    },
    #[error("Event mismatch at index {index}. expected={expected} actual={actual}")]
    EventMismatch {
        index: usize,
        expected: String,
        actual: String,
    },
    #[error("Failed to serialize event for diff: {0}")]
    EventSerialize(serde_json::Error),
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

pub fn examples_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

pub fn example_dir(name: &str) -> PathBuf {
    examples_root().join(name)
}

pub fn testcase_path(name: &str) -> PathBuf {
    example_dir(name).join("testcase.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_points_to_workspace() {
        assert!(workspace_root().join("Cargo.toml").exists());
    }

    #[test]
    fn examples_root_points_to_examples_directory() {
        assert!(examples_root().is_dir());
    }

    #[test]
    fn example_dir_joins_name() {
        assert!(example_dir("01-text-code").is_dir());
    }

    #[test]
    fn testcase_path_joins_default_filename() {
        let path = testcase_path("01-text-code");
        assert!(path.ends_with("testcase.json"));
    }
}
