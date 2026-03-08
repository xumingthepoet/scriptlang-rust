use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::value::SlValue;

pub const COMPILED_PROJECT_SCHEMA: &str = "compiled-project";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AccessLevel {
    Public,
    #[default]
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

impl SourceSpan {
    pub fn synthetic() -> Self {
        Self {
            start: SourceLocation { line: 1, column: 1 },
            end: SourceLocation { line: 1, column: 1 },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScriptType {
    Primitive {
        name: String,
    },
    Array {
        element_type: Box<ScriptType>,
    },
    Map {
        key_type: String,
        value_type: Box<ScriptType>,
    },
    Object {
        type_name: String,
        fields: BTreeMap<String, ScriptType>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VarDeclaration {
    pub name: String,
    pub r#type: ScriptType,
    pub initial_value_expr: Option<String>,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefsGlobalVarDecl {
    pub namespace: String,
    pub name: String,
    pub qualified_name: String,
    #[serde(default)]
    pub access: AccessLevel,
    pub r#type: ScriptType,
    pub initial_value_expr: Option<String>,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptParam {
    pub name: String,
    pub r#type: ScriptType,
    pub is_ref: bool,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionParam {
    pub name: String,
    pub r#type: ScriptType,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionReturn {
    pub name: String,
    pub r#type: ScriptType,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDecl {
    pub name: String,
    pub params: Vec<FunctionParam>,
    pub return_binding: FunctionReturn,
    pub code: String,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallArgument {
    pub value_expr: String,
    pub is_ref: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub id: String,
    pub text: String,
    pub when_expr: Option<String>,
    pub once: bool,
    pub fall_over: bool,
    pub group_id: String,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicChoiceTemplate {
    pub text: String,
    pub when_expr: Option<String>,
    pub group_id: String,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicChoiceBlock {
    pub id: String,
    pub array_expr: String,
    pub item_name: String,
    pub index_name: Option<String>,
    pub template: DynamicChoiceTemplate,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChoiceEntry {
    Static { option: ChoiceOption },
    Dynamic { block: DynamicChoiceBlock },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScriptNode {
    Text {
        id: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        once: bool,
        location: SourceSpan,
    },
    Debug {
        id: String,
        value: String,
        location: SourceSpan,
    },
    Code {
        id: String,
        code: String,
        location: SourceSpan,
    },
    Var {
        id: String,
        declaration: VarDeclaration,
        location: SourceSpan,
    },
    If {
        id: String,
        when_expr: String,
        then_group_id: String,
        else_group_id: Option<String>,
        location: SourceSpan,
    },
    While {
        id: String,
        when_expr: String,
        body_group_id: String,
        location: SourceSpan,
    },
    Choice {
        id: String,
        prompt_text: String,
        entries: Vec<ChoiceEntry>,
        location: SourceSpan,
    },
    Input {
        id: String,
        target_var: String,
        prompt_text: String,
        location: SourceSpan,
    },
    Break {
        id: String,
        location: SourceSpan,
    },
    Continue {
        id: String,
        target: ContinueTarget,
        location: SourceSpan,
    },
    Call {
        id: String,
        target_script: String,
        args: Vec<CallArgument>,
        location: SourceSpan,
    },
    Return {
        id: String,
        target_script: Option<String>,
        args: Vec<CallArgument>,
        location: SourceSpan,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinueTarget {
    While,
    Choice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplicitGroup {
    pub group_id: String,
    pub parent_group_id: Option<String>,
    pub entry_node_id: Option<String>,
    pub nodes: Vec<ScriptNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptIr {
    pub script_path: String,
    pub script_name: String,
    #[serde(default)]
    pub access: AccessLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_script_name: Option<String>,
    pub params: Vec<ScriptParam>,
    pub root_group_id: String,
    pub groups: BTreeMap<String, ImplicitGroup>,
    pub visible_json_globals: Vec<String>,
    pub visible_functions: BTreeMap<String, FunctionDecl>,
    pub visible_defs_globals: BTreeMap<String, DefsGlobalVarDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuationFrame {
    pub resume_frame_id: u64,
    pub next_node_index: usize,
    pub ref_bindings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFrame {
    pub frame_id: u64,
    pub group_id: String,
    pub node_index: usize,
    pub scope: BTreeMap<String, SlValue>,
    pub var_types: BTreeMap<String, ScriptType>,
    pub completion: SnapshotCompletion,
    pub script_root: bool,
    pub return_continuation: Option<ContinuationFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SnapshotCompletion {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "whileBody")]
    WhileBody,
    #[serde(rename = "resumeAfterChild")]
    ResumeAfterChild,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceItem {
    pub index: usize,
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDynamicChoiceBinding {
    pub group_id: String,
    pub item_name: String,
    pub item_value: SlValue,
    pub index_name: Option<String>,
    pub index_value: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PendingBoundary {
    #[serde(rename_all = "camelCase")]
    Choice {
        node_id: String,
        items: Vec<ChoiceItem>,
        prompt_text: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        dynamic_bindings: BTreeMap<String, PendingDynamicChoiceBinding>,
    },
    #[serde(rename_all = "camelCase")]
    Input {
        node_id: String,
        target_var: String,
        prompt_text: String,
        default_text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema_version: String,
    pub compiler_version: String,
    pub runtime_frames: Vec<SnapshotFrame>,
    pub rng_state: u32,
    pub pending_boundary: PendingBoundary,
    #[serde(default)]
    pub defs_globals: BTreeMap<String, SlValue>,
    pub once_state_by_script: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineOutput {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
    },
    Debug {
        text: String,
    },
    Choices {
        items: Vec<ChoiceItem>,
        prompt_text: Option<String>,
    },
    Input {
        prompt_text: String,
        default_text: String,
    },
    End,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompileProjectResult {
    pub scripts: BTreeMap<String, ScriptIr>,
    pub entry_script: String,
    pub global_json: BTreeMap<String, SlValue>,
    pub defs_global_declarations: BTreeMap<String, DefsGlobalVarDecl>,
    pub defs_global_init_order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledProjectArtifact {
    pub schema_version: String,
    pub compiler_version: String,
    pub entry_script: String,
    pub scripts: BTreeMap<String, ScriptIr>,
    pub global_json: BTreeMap<String, SlValue>,
    pub defs_global_declarations: BTreeMap<String, DefsGlobalVarDecl>,
    pub defs_global_init_order: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_span_synthetic_uses_1_based_defaults() {
        let span = SourceSpan::synthetic();
        assert_eq!(span.start.line, 1);
        assert_eq!(span.start.column, 1);
        assert_eq!(span.end.line, 1);
        assert_eq!(span.end.column, 1);
    }

    #[test]
    fn compiled_project_schema_constant_matches_v1() {
        assert_eq!(COMPILED_PROJECT_SCHEMA, "compiled-project");
    }

    #[test]
    fn compiled_project_artifact_roundtrip_json() {
        let artifact = CompiledProjectArtifact {
            schema_version: COMPILED_PROJECT_SCHEMA.to_string(),
            compiler_version: "player".to_string(),
            entry_script: "main".to_string(),
            scripts: BTreeMap::new(),
            global_json: BTreeMap::new(),
            defs_global_declarations: BTreeMap::new(),
            defs_global_init_order: Vec::new(),
        };

        let encoded = serde_json::to_string(&artifact).expect("artifact serialize");
        let decoded: CompiledProjectArtifact =
            serde_json::from_str(&encoded).expect("artifact deserialize");
        assert_eq!(decoded, artifact);
    }
}
