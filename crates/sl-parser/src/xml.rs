use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use roxmltree::{Document, Node, NodeType};
use sl_core::{ScriptLangError, SourceLocation, SourceSpan};

#[derive(Debug, Clone, PartialEq)]
pub struct XmlDocument {
    pub root: XmlElementNode,
}

#[derive(Debug, Clone, PartialEq)]
pub enum XmlNode {
    Element(XmlElementNode),
    Text(XmlTextNode),
}

#[derive(Debug, Clone, PartialEq)]
pub struct XmlElementNode {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub children: Vec<XmlNode>,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct XmlTextNode {
    pub value: String,
    pub location: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportDirective {
    File {
        module_name: String,
        from_path: String,
    },
    Directory {
        module_names: Vec<String>,
        from_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasDirective {
    pub target_qualified_name: String,
    pub alias_name: String,
}

pub fn parse_import_directives(source: &str) -> Vec<ImportDirective> {
    let mut directives = Vec::new();

    for caps in import_directive_regex().captures_iter(source) {
        let raw = caps
            .get(1)
            .expect("import directive regex should always capture body")
            .as_str()
            .trim();
        if let Some(file_caps) = file_import_body_regex().captures(raw) {
            let module_name = file_caps
                .get(1)
                .expect("file import regex should capture module name")
                .as_str()
                .trim();
            let from_path = file_caps
                .get(2)
                .expect("file import regex should capture import path")
                .as_str()
                .trim();
            directives.extend((!module_name.is_empty() && !from_path.is_empty()).then(|| {
                ImportDirective::File {
                    module_name: module_name.to_string(),
                    from_path: from_path.to_string(),
                }
            }));
            continue;
        }

        let Some(dir_caps) = directory_import_body_regex().captures(raw) else {
            continue;
        };
        let module_names = dir_caps
            .get(1)
            .expect("directory import regex should capture module list")
            .as_str();
        let from_path = dir_caps
            .get(2)
            .expect("directory import regex should capture import path")
            .as_str()
            .trim();
        let module_names = module_names
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        directives.extend(
            (!from_path.is_empty() && !module_names.is_empty()).then(|| {
                ImportDirective::Directory {
                    module_names,
                    from_path: from_path.to_string(),
                }
            }),
        );
    }

    directives
}

pub fn parse_alias_directives(source: &str) -> Vec<AliasDirective> {
    let mut directives = Vec::new();

    for caps in alias_directive_regex().captures_iter(source) {
        let raw = caps
            .get(1)
            .expect("alias directive regex should always capture body")
            .as_str()
            .trim();
        let Some(alias_caps) = alias_directive_body_regex().captures(raw) else {
            continue;
        };
        let target_qualified_name = alias_caps
            .get(1)
            .expect("alias directive body regex should capture target")
            .as_str()
            .trim();
        let alias_name = alias_caps
            .get(2)
            .map(|value| value.as_str().trim().to_string())
            .unwrap_or_else(|| {
                target_qualified_name
                    .split('.')
                    .nth(1)
                    .unwrap_or_default()
                    .to_string()
            });
        if target_qualified_name.is_empty() || alias_name.is_empty() {
            continue;
        }
        directives.push(AliasDirective {
            target_qualified_name: target_qualified_name.to_string(),
            alias_name,
        });
    }

    directives
}

pub fn reject_non_import_dependency_directives(source: &str) -> Result<(), ScriptLangError> {
    if let Some(caps) = non_import_dependency_directive_regex().captures(source) {
        let keyword = caps
            .get(1)
            .expect("non-import dependency directive regex should capture keyword")
            .as_str()
            .trim();
        (keyword == "import").then_some(()).ok_or_else(|| {
            ScriptLangError::new(
                "IMPORT_DIRECTIVE_UNSUPPORTED",
                format!(
                    "Unsupported dependency directive \"{}\". Only `import` directives are allowed.",
                    keyword
                ),
            )
        })?;
    }
    Ok(())
}

fn import_directive_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?m)^\s*<!--\s*(import.+?)\s*-->\s*$").expect("import regex must compile")
    })
}

fn file_import_body_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^import\s+([A-Za-z_][A-Za-z0-9_-]*)\s+from\s+(.+?)$")
            .expect("file import body regex must compile")
    })
}

fn directory_import_body_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^import\s*\{\s*(.+?)\s*\}\s+from\s+(.+?)$")
            .expect("directory import body regex must compile")
    })
}

fn alias_directive_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?m)^\s*<!--\s*(alias.+?)\s*-->\s*$")
            .expect("alias directive regex must compile")
    })
}

fn alias_directive_body_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"^alias\s+([A-Za-z_][A-Za-z0-9_-]*\.[A-Za-z_][A-Za-z0-9_-]*)(?:\s+as\s+([A-Za-z_][A-Za-z0-9_-]*))?$",
        )
        .expect("alias directive body regex must compile")
    })
}

fn non_import_dependency_directive_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?m)^\s*<!--\s*([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.+?)\s*-->\s*$")
            .expect("non-import dependency directive regex must compile")
    })
}

pub fn parse_xml_document(source: &str) -> Result<XmlDocument, ScriptLangError> {
    let document = Document::parse(source)
        .map_err(|error| ScriptLangError::new("XML_PARSE_ERROR", error.to_string()))?;

    let root = document.root_element();

    Ok(XmlDocument {
        root: parse_element(&document, root),
    })
}

fn parse_element(document: &Document<'_>, node: Node<'_, '_>) -> XmlElementNode {
    let mut attributes = BTreeMap::new();
    for attribute in node.attributes() {
        attributes.insert(attribute.name().to_string(), attribute.value().to_string());
    }

    let mut children = Vec::new();
    for child in node.children() {
        match child.node_type() {
            NodeType::Element => children.push(XmlNode::Element(parse_element(document, child))),
            NodeType::Text => {
                let value = child.text().unwrap_or_default().to_string();
                if value.is_empty() {
                    continue;
                }
                children.push(XmlNode::Text(XmlTextNode {
                    value,
                    location: node_span(document, child.range().start, child.range().end),
                }));
            }
            _ => {}
        }
    }

    XmlElementNode {
        name: node.tag_name().name().to_string(),
        attributes,
        children,
        location: node_span(document, node.range().start, node.range().end),
    }
}

fn node_span(document: &Document<'_>, start: usize, end: usize) -> SourceSpan {
    let start_pos = document.text_pos_at(start);
    let end_pos = document.text_pos_at(end);
    SourceSpan {
        start: SourceLocation {
            line: start_pos.row as usize,
            column: start_pos.col as usize,
        },
        end: SourceLocation {
            line: end_pos.row as usize,
            column: end_pos.col as usize,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_import_directives_extracts_file_and_directory_imports() {
        let source = r#"
<!-- import Shared from shared.xml -->
<!-- import { Battle, Common } from modules/ -->
<!-- import {    } from ignored/ -->
<script name="main"></script>
"#;

        let imports = parse_import_directives(source);
        assert_eq!(
            imports,
            vec![
                ImportDirective::File {
                    module_name: "Shared".to_string(),
                    from_path: "shared.xml".to_string(),
                },
                ImportDirective::Directory {
                    module_names: vec!["Battle".to_string(), "Common".to_string()],
                    from_path: "modules/".to_string(),
                }
            ]
        );
    }

    #[test]
    fn parse_import_directives_ignores_malformed_entries() {
        let source = r#"
<!-- import from shared.xml -->
<!-- import Shared from -->
<!-- import { } from modules/ -->
<!-- import { Shared } from -->
<!-- import -->
<!-- import Shared from shared.xml -->
<!-- import { Battle, Common } from modules/ -->
"#;

        let imports = parse_import_directives(source);
        assert_eq!(
            imports,
            vec![
                ImportDirective::File {
                    module_name: "Shared".to_string(),
                    from_path: "shared.xml".to_string(),
                },
                ImportDirective::Directory {
                    module_names: vec!["Battle".to_string(), "Common".to_string()],
                    from_path: "modules/".to_string(),
                }
            ]
        );

        assert!(parse_import_directives("<!-- import Shared from    -->").is_empty());
        assert!(parse_import_directives("<!-- import { Shared } from    -->").is_empty());
    }

    #[test]
    fn parse_alias_directives_extracts_with_and_without_as() {
        let source = r#"
<!-- alias shared.hp -->
<!-- alias shared.maxHp as max -->
<!-- alias shared -->
<!-- alias shared.hp as -->
"#;

        let aliases = parse_alias_directives(source);
        assert_eq!(
            aliases,
            vec![
                AliasDirective {
                    target_qualified_name: "shared.hp".to_string(),
                    alias_name: "hp".to_string(),
                },
                AliasDirective {
                    target_qualified_name: "shared.maxHp".to_string(),
                    alias_name: "max".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_alias_directives_ignores_malformed_and_non_alias_comments() {
        let source = r#"
<!--! alias shared.hp as hp -->
<!-- dependency: shared.hp -->
<!-- alias shared as hp -->
<!-- alias shared.hp as -->
<!-- alias shared.hp as hp -->
"#;
        let aliases = parse_alias_directives(source);
        assert_eq!(
            aliases,
            vec![AliasDirective {
                target_qualified_name: "shared.hp".to_string(),
                alias_name: "hp".to_string(),
            }]
        );
    }

    #[test]
    fn reject_non_import_dependency_directives_reports_unsupported_directive() {
        let source = r#"
<!-- dependency: a.xml -->
<module name="main" default_access="public"></module>
"#;

        let error = reject_non_import_dependency_directives(source)
            .expect_err("non-import dependency directive should fail");
        assert_eq!(error.code, "IMPORT_DIRECTIVE_UNSUPPORTED");

        let valid = r#"
<!-- import Shared from shared.xml -->
<module name="main" default_access="public"></module>
"#;
        reject_non_import_dependency_directives(valid)
            .expect("import directive should pass whitelist");
    }

    #[test]
    fn parse_xml_document_builds_tree_with_attributes_and_text() {
        let source = r#"<script name="main">prefix<text id="t1"><inner/>Hello</text></script>"#;
        let document = parse_xml_document(source).expect("xml should parse");
        assert_eq!(document.root.name, "script");
        assert_eq!(
            document.root.attributes.get("name"),
            Some(&"main".to_string())
        );
        assert_eq!(document.root.children.len(), 2);

        let text_node = document
            .root
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(node) => Some(node),
                XmlNode::Text(_) => None,
            })
            .expect("expected first child to be element");
        assert_eq!(text_node.name, "text");
        assert_eq!(text_node.attributes.get("id"), Some(&"t1".to_string()));

        let text_value = text_node
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Text(value) => Some(value),
                XmlNode::Element(_) => None,
            })
            .expect("expected first text child");
        assert_eq!(text_value.value, "Hello");
        assert!(text_value.location.start.line >= 1);
        assert!(document.root.location.end.column >= document.root.location.start.column);
    }

    #[test]
    fn parse_xml_document_handles_comment_and_empty_text_nodes() {
        let source = r#"<script name="main"><text><!--c-->A</text><text></text></script>"#;
        let document = parse_xml_document(source).expect("xml should parse");
        assert_eq!(document.root.children.len(), 2);
    }

    #[test]
    fn parse_xml_document_handles_empty_cdata_node() {
        let source = r#"<script name="main"><text><![CDATA[]]></text></script>"#;
        let document = parse_xml_document(source).expect("xml should parse");
        assert_eq!(document.root.children.len(), 1);
    }

    #[test]
    fn parse_xml_document_returns_parse_error_for_invalid_xml() {
        let error = parse_xml_document("<script>").expect_err("invalid xml should fail");
        assert_eq!(error.code, "XML_PARSE_ERROR");
    }

    #[test]
    fn parse_xml_document_returns_parse_error_when_root_element_is_missing() {
        let error = parse_xml_document("<?xml version=\"1.0\"?><!---->")
            .expect_err("missing root element should fail");
        assert_eq!(error.code, "XML_PARSE_ERROR");
    }

    #[test]
    fn parse_xml_document_can_return_empty_root_for_element_less_document() {
        let parsed = parse_xml_document("<?xml version=\"1.0\"?><?pi test?>");
        assert!(parsed.is_err());
    }
}
