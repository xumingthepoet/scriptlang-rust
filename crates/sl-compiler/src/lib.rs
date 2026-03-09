pub(crate) use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::sync::OnceLock;

pub(crate) use regex::Regex;
pub(crate) use serde_json::Value as JsonValue;
pub(crate) use sl_core::{
    default_value_from_type, AccessLevel, CallArgument, ChoiceEntry, ChoiceOption,
    CompiledProjectArtifact, ContinueTarget, DynamicChoiceBlock, DynamicChoiceTemplate,
    FunctionDecl, FunctionParam, FunctionReturn, ImplicitGroup, ModuleConstDecl, ModuleVarDecl,
    ScriptIr, ScriptLangError, ScriptNode, ScriptParam, ScriptType, SlValue, SourceSpan,
    VarDeclaration, COMPILED_PROJECT_SCHEMA,
};
pub(crate) use sl_parser::{
    parse_import_directives, parse_xml_document, reject_non_import_dependency_directives,
    ImportDirective, XmlElementNode, XmlNode, XmlTextNode,
};

mod artifact;
mod context;
mod defaults;
mod error_context;
mod import_graph;
mod macro_expand;
mod module_resolver;
mod pipeline;
mod sanitize;
mod script_compile;
mod source_parse;
mod type_expr;
mod xml_utils;

pub use artifact::{
    compile_artifact_from_xml_map, read_artifact_json, write_artifact_json,
    DEFAULT_COMPILER_VERSION,
};
pub use context::CompileProjectBundleResult;
pub use pipeline::{compile_project_bundle_from_xml_map, compile_project_scripts_from_xml_map};

pub(crate) use context::*;
pub(crate) use error_context::with_file_context_shared;
pub(crate) use import_graph::*;
pub(crate) use macro_expand::*;
pub(crate) use module_resolver::*;
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
            .map(|(key, value)| {
                let normalized_key = normalize_test_source_path(key);
                let normalized_value = normalize_test_source_content(value);
                (normalized_key, normalized_value)
            })
            .collect()
    }

    fn normalize_test_source_path(path: &str) -> String {
        path.replace(".script.xml", ".xml")
            .replace(".module.xml", ".xml")
    }

    fn normalize_test_source_content(source: &str) -> String {
        let mut normalized = source
            .replace(".script.xml", ".xml")
            .replace(".module.xml", ".xml");

        let trimmed = normalized.trim_start();
        if !trimmed.starts_with("<module")
            && normalized.trim_end().ends_with("</module>")
            && trimmed.starts_with("<script")
        {
            let end_regex =
                Regex::new(r"</module>\s*\z").expect("stray module close regex should compile");
            normalized = end_regex.replace(&normalized, "").into_owned();
        }

        if normalize_wrapped_root(&normalized, "module", None).is_some() {
            return normalized;
        }

        if let Some(wrapped) = normalize_wrapped_root(&normalized, "script", None) {
            return wrapped;
        }

        normalized
    }

    fn normalize_wrapped_root(
        source: &str,
        root_name: &str,
        explicit_module_name_attr: Option<&str>,
    ) -> Option<String> {
        let pattern = format!(r"\A(\s*(?:<!--.*?-->\s*)*)<{}\b([^>]*)>", root_name);
        let regex = Regex::new(&pattern).expect("test root regex should compile");
        let captures = regex.captures(source)?;
        let prefix = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
        let attrs = captures.get(2).map(|m| m.as_str()).unwrap_or_default();

        if root_name == "module" {
            return Some(source.to_string());
        }

        let attr_name = explicit_module_name_attr.unwrap_or("name");
        let attr_regex = Regex::new(&format!(r#"{attr_name}="([^"]+)""#))
            .expect("attribute regex should compile");
        let module_name = attr_regex
            .captures(attrs)
            .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))?;

        let replaced_open = regex.replace(
            source,
            format!(
                r#"{prefix}<module name="{module_name}" default_access="public">
<{root_name}{attrs}>"#
            ),
        );
        let closing = format!("</{root_name}>");
        let end_regex =
            Regex::new(&format!(r"{closing}\s*\z")).expect("closing regex should compile");
        Some(
            end_regex
                .replace(replaced_open.as_ref(), format!("{closing}\n</module>"))
                .into_owned(),
        )
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

    #[cfg(test)]
    mod compiler_test_support_tests {
        use super::*;

        #[test]
        fn normalize_test_source_content_handles_wrapped_script_and_module_roots() {
            let script = normalize_test_source_content(
                r#"
<script name="main">
  <text>x</text>
</script>
</module>
"#,
            );
            assert!(script.contains(r#"<module name="main" default_access="public">"#));
            assert!(script.contains(r#"<script name="main">"#));
            assert!(!script.trim_end().ends_with("</module>\n</module>"));

            let module = normalize_test_source_content(
                r#"
<module name="shared">
  <var name="hp" type="int">1</var>
</module>
</module>
"#,
            );
            assert!(module.contains(r#"<module name="shared">"#));
            assert!(module.trim_end().ends_with("</module>"));
        }

        #[test]
        fn normalize_test_source_content_handles_module_and_legacy_import_comments() {
            let module = normalize_test_source_content(
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
  <script name="main"/>
</module>
"#,
            );
            assert!(module.contains(r#"<!-- import shared from shared.xml -->"#));
            assert!(normalize_wrapped_root(&module, "module", None).is_some());
        }
    }
}
