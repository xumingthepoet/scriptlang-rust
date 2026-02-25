use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value as JsonValue;
use sl_core::{
    default_value_from_type, CallArgument, ChoiceOption, ContinueTarget, FunctionDecl,
    FunctionParam, FunctionReturn, ImplicitGroup, ScriptIr, ScriptLangError, ScriptNode,
    ScriptParam, ScriptType, SlValue, SourceSpan, VarDeclaration,
};
use sl_parser::{
    parse_include_directives, parse_xml_document, XmlElementNode, XmlNode, XmlTextNode,
};

pub const INTERNAL_RESERVED_NAME_PREFIX: &str = "__";
const LOOP_TEMP_VAR_PREFIX: &str = "__sl_loop_";

#[derive(Debug, Clone)]
pub struct CompileProjectBundleResult {
    pub scripts: BTreeMap<String, ScriptIr>,
    pub global_json: BTreeMap<String, SlValue>,
}

#[derive(Debug, Clone)]
enum SourceKind {
    ScriptXml,
    DefsXml,
    Json,
}

#[derive(Debug, Clone)]
struct SourceFile {
    kind: SourceKind,
    includes: Vec<String>,
    xml_root: Option<XmlElementNode>,
    json_value: Option<SlValue>,
}

#[derive(Debug, Clone)]
struct ParsedTypeDecl {
    name: String,
    qualified_name: String,
    fields: Vec<ParsedTypeFieldDecl>,
    location: SourceSpan,
}

#[derive(Debug, Clone)]
struct ParsedTypeFieldDecl {
    name: String,
    type_expr: ParsedTypeExpr,
    location: SourceSpan,
}

#[derive(Debug, Clone)]
struct ParsedFunctionDecl {
    name: String,
    qualified_name: String,
    params: Vec<ParsedFunctionParamDecl>,
    return_binding: ParsedFunctionParamDecl,
    code: String,
    location: SourceSpan,
}

#[derive(Debug, Clone)]
struct ParsedFunctionParamDecl {
    name: String,
    type_expr: ParsedTypeExpr,
    location: SourceSpan,
}

#[derive(Debug, Clone)]
enum ParsedTypeExpr {
    Primitive(String),
    Array(Box<ParsedTypeExpr>),
    Map(Box<ParsedTypeExpr>),
    Custom(String),
}

#[derive(Debug, Clone)]
struct DefsDeclarations {
    type_decls: Vec<ParsedTypeDecl>,
    function_decls: Vec<ParsedFunctionDecl>,
}

type VisibleTypeMap = BTreeMap<String, ScriptType>;
type VisibleFunctionMap = BTreeMap<String, FunctionDecl>;

#[derive(Debug, Clone)]
struct MacroExpansionContext {
    used_var_names: BTreeSet<String>,
    loop_counter: usize,
}

#[derive(Debug, Clone)]
struct GroupBuilder {
    script_path: String,
    group_counter: usize,
    node_counter: usize,
    choice_counter: usize,
    groups: BTreeMap<String, ImplicitGroup>,
}

impl GroupBuilder {
    fn new(script_path: impl Into<String>) -> Self {
        Self {
            script_path: script_path.into(),
            group_counter: 0,
            node_counter: 0,
            choice_counter: 0,
            groups: BTreeMap::new(),
        }
    }

    fn next_group_id(&mut self) -> String {
        let id = format!(
            "{}::g{}",
            stable_base(&self.script_path),
            self.group_counter
        );
        self.group_counter += 1;
        id
    }

    fn next_node_id(&mut self, kind: &str) -> String {
        let id = format!(
            "{}::n{}:{}",
            stable_base(&self.script_path),
            self.node_counter,
            kind
        );
        self.node_counter += 1;
        id
    }

    fn next_choice_id(&mut self) -> String {
        let id = format!(
            "{}::c{}",
            stable_base(&self.script_path),
            self.choice_counter
        );
        self.choice_counter += 1;
        id
    }
}

