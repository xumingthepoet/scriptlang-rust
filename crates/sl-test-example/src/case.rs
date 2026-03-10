use serde::{Deserialize, Serialize};

pub const TESTCASE_SCHEMA: &str = "sl-tool-case";

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
    "main.main".to_string()
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
        #[serde(default)]
        tag: Option<String>,
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
