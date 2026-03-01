use serde::{Deserialize, Serialize};

pub const TESTCASE_SCHEMA_V1: &str = "sl-tool-case.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCase {
    pub schema_version: String,
    #[serde(default = "default_entry_script")]
    pub entry_script: String,
    #[serde(default)]
    pub actions: Vec<TestAction>,
    #[serde(default)]
    pub expected_events: Vec<ExpectedEvent>,
}

fn default_entry_script() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TestAction {
    Choose { index: usize },
    Input { text: String },
}

impl TestAction {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Choose { .. } => "choose",
            Self::Input { .. } => "input",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ExpectedEvent {
    Text {
        text: String,
    },
    Choices {
        #[serde(default, rename = "promptText")]
        prompt_text: Option<String>,
        choices: Vec<String>,
    },
    Input {
        #[serde(rename = "promptText")]
        prompt_text: String,
        #[serde(rename = "defaultText")]
        default_text: String,
    },
    End,
}

#[cfg(test)]
mod case_tests {
    use super::*;

    #[test]
    fn default_entry_script_returns_main() {
        assert_eq!(default_entry_script(), "main");
    }

    #[test]
    fn test_action_kind_name_reports_expected_value() {
        assert_eq!(TestAction::Choose { index: 0 }.kind_name(), "choose");
        assert_eq!(
            TestAction::Input {
                text: "x".to_string()
            }
            .kind_name(),
            "input"
        );
    }

    #[test]
    fn testcase_deserialize_applies_defaults() {
        let parsed: TestCase = serde_json::from_str(
            r#"{
  "schemaVersion": "sl-tool-case.v1",
  "actions": [],
  "expectedEvents": []
}"#,
        )
        .expect("testcase should deserialize");

        assert_eq!(parsed.schema_version, TESTCASE_SCHEMA_V1);
        assert_eq!(parsed.entry_script, "main");
        assert!(parsed.actions.is_empty());
        assert!(parsed.expected_events.is_empty());
    }

    #[test]
    fn expected_event_deserialize_supports_all_variants() {
        let parsed: Vec<ExpectedEvent> = serde_json::from_str(
            r#"[
  {"kind":"text","text":"a"},
  {"kind":"choices","promptText":"pick","choices":["A"]},
  {"kind":"input","promptText":"name","defaultText":"Traveler"},
  {"kind":"end"}
]"#,
        )
        .expect("events should deserialize");

        assert_eq!(parsed.len(), 4);
        assert!(matches!(parsed[0], ExpectedEvent::Text { .. }));
        assert!(matches!(parsed[1], ExpectedEvent::Choices { .. }));
        assert!(matches!(parsed[2], ExpectedEvent::Input { .. }));
        assert!(matches!(parsed[3], ExpectedEvent::End));
    }
}
