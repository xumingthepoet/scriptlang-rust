use crate::*;

pub const INTERNAL_RESERVED_NAME_PREFIX: &str = "__";
pub(crate) const LOOP_TEMP_VAR_PREFIX: &str = "__sl_loop_";

#[derive(Debug, Clone)]
pub struct CompileProjectBundleResult {
    pub scripts: BTreeMap<String, ScriptIr>,
    pub global_json: BTreeMap<String, SlValue>,
    pub defs_global_declarations: BTreeMap<String, DefsGlobalVarDecl>,
    pub defs_global_init_order: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum SourceKind {
    ScriptXml,
    DefsXml,
    Json,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceFile {
    pub(crate) kind: SourceKind,
    pub(crate) includes: Vec<String>,
    pub(crate) xml_root: Option<XmlElementNode>,
    pub(crate) json_value: Option<SlValue>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedTypeDecl {
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) fields: Vec<ParsedTypeFieldDecl>,
    pub(crate) location: SourceSpan,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedTypeFieldDecl {
    pub(crate) name: String,
    pub(crate) type_expr: ParsedTypeExpr,
    pub(crate) location: SourceSpan,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedFunctionDecl {
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) params: Vec<ParsedFunctionParamDecl>,
    pub(crate) return_binding: ParsedFunctionParamDecl,
    pub(crate) code: String,
    pub(crate) location: SourceSpan,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedDefsGlobalVarDecl {
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) type_expr: ParsedTypeExpr,
    pub(crate) initial_value_expr: Option<String>,
    pub(crate) location: SourceSpan,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedFunctionParamDecl {
    pub(crate) name: String,
    pub(crate) type_expr: ParsedTypeExpr,
    pub(crate) location: SourceSpan,
}

#[derive(Debug, Clone)]
pub(crate) enum ParsedTypeExpr {
    Primitive(String),
    Array(Box<ParsedTypeExpr>),
    Map(Box<ParsedTypeExpr>),
    Custom(String),
}

#[derive(Debug, Clone)]
pub(crate) struct DefsDeclarations {
    pub(crate) type_decls: Vec<ParsedTypeDecl>,
    pub(crate) function_decls: Vec<ParsedFunctionDecl>,
    pub(crate) defs_global_var_decls: Vec<ParsedDefsGlobalVarDecl>,
}

pub(crate) type VisibleTypeMap = BTreeMap<String, ScriptType>;
pub(crate) type VisibleFunctionMap = BTreeMap<String, FunctionDecl>;

#[derive(Debug, Clone)]
pub(crate) struct MacroExpansionContext {
    pub(crate) used_var_names: BTreeSet<String>,
    pub(crate) loop_counter: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct GroupBuilder {
    pub(crate) script_path: String,
    pub(crate) group_counter: usize,
    pub(crate) node_counter: usize,
    pub(crate) choice_counter: usize,
    pub(crate) groups: BTreeMap<String, ImplicitGroup>,
}

impl GroupBuilder {
    pub(crate) fn new(script_path: impl Into<String>) -> Self {
        Self {
            script_path: script_path.into(),
            group_counter: 0,
            node_counter: 0,
            choice_counter: 0,
            groups: BTreeMap::new(),
        }
    }

    pub(crate) fn next_group_id(&mut self) -> String {
        let id = format!(
            "{}::g{}",
            stable_base(&self.script_path),
            self.group_counter
        );
        self.group_counter += 1;
        id
    }

    pub(crate) fn next_node_id(&mut self, kind: &str) -> String {
        let id = format!(
            "{}::n{}:{}",
            stable_base(&self.script_path),
            self.node_counter,
            kind
        );
        self.node_counter += 1;
        id
    }

    pub(crate) fn next_choice_id(&mut self) -> String {
        let id = format!(
            "{}::c{}",
            stable_base(&self.script_path),
            self.choice_counter
        );
        self.choice_counter += 1;
        id
    }
}
