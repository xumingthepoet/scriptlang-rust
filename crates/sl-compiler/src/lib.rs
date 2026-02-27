pub(crate) use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::sync::OnceLock;

pub(crate) use regex::Regex;
pub(crate) use serde_json::Value as JsonValue;
pub(crate) use sl_core::{
    default_value_from_type, CallArgument, ChoiceOption, ContinueTarget, DefsGlobalVarDecl,
    FunctionDecl, FunctionParam, FunctionReturn, ImplicitGroup, ScriptIr, ScriptLangError,
    ScriptNode, ScriptParam, ScriptType, SlValue, SourceSpan, VarDeclaration,
};
pub(crate) use sl_parser::{
    parse_include_directives, parse_xml_document, XmlElementNode, XmlNode, XmlTextNode,
};

mod context;
mod defaults;
mod defs_resolver;
mod include_graph;
mod json_symbols;
mod macro_expand;
mod pipeline;
mod sanitize;
mod script_compile;
mod source_parse;
mod type_expr;
mod xml_utils;

pub use context::CompileProjectBundleResult;
pub use pipeline::{compile_project_bundle_from_xml_map, compile_project_scripts_from_xml_map};

pub(crate) use context::*;
pub(crate) use defaults::*;
pub(crate) use defs_resolver::*;
pub(crate) use include_graph::*;
pub(crate) use json_symbols::*;
pub(crate) use macro_expand::*;
pub(crate) use sanitize::*;
pub(crate) use script_compile::*;
pub(crate) use source_parse::*;
pub(crate) use type_expr::*;
pub(crate) use xml_utils::*;

#[cfg(test)]
pub(crate) mod compiler_test_support {
    use super::*;

    pub(crate) fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    pub(crate) fn xml_text(value: &str) -> XmlNode {
        XmlNode::Text(XmlTextNode {
            value: value.to_string(),
            location: SourceSpan::synthetic(),
        })
    }

    pub(crate) fn xml_element(
        name: &str,
        attrs: &[(&str, &str)],
        children: Vec<XmlNode>,
    ) -> XmlElementNode {
        XmlElementNode {
            name: name.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            children,
            location: SourceSpan::synthetic(),
        }
    }
}
