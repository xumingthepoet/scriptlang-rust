use std::collections::BTreeMap;

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

pub fn parse_include_directives(source: &str) -> Vec<String> {
    let regex = Regex::new(r"(?m)^\s*<!--\s*include:\s*(.+?)\s*-->\s*$")
        .expect("include regex must compile");
    regex
        .captures_iter(source)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().trim().to_string()))
        .filter(|value| !value.is_empty())
        .collect()
}

pub fn parse_xml_document(source: &str) -> Result<XmlDocument, ScriptLangError> {
    let document = Document::parse(source)
        .map_err(|error| ScriptLangError::new("XML_PARSE_ERROR", error.to_string()))?;

    let Some(root) = document.root().children().find(|node| node.is_element()) else {
        return Err(ScriptLangError::new(
            "XML_PARSE_ERROR",
            "XML document must contain a root element.",
        ));
    };

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
    fn parse_include_directives_extracts_non_empty_paths() {
        let source = r#"
<!-- include: a.script.xml -->
<!-- include:   nested/b.script.xml   -->
<!-- include:    -->
<script name="main"></script>
"#;

        let includes = parse_include_directives(source);
        assert_eq!(
            includes,
            vec![
                "a.script.xml".to_string(),
                "nested/b.script.xml".to_string()
            ]
        );
    }

    #[test]
    fn parse_xml_document_builds_tree_with_attributes_and_text() {
        let source = r#"<script name="main"><text id="t1">Hello</text></script>"#;
        let document = parse_xml_document(source).expect("xml should parse");
        assert_eq!(document.root.name, "script");
        assert_eq!(
            document.root.attributes.get("name"),
            Some(&"main".to_string())
        );
        assert_eq!(document.root.children.len(), 1);

        assert!(matches!(document.root.children[0], XmlNode::Element(_)));
        let text_node = match &document.root.children[0] {
            XmlNode::Element(node) => node,
            XmlNode::Text(_) => unreachable!("already asserted element"),
        };
        assert_eq!(text_node.name, "text");
        assert_eq!(text_node.attributes.get("id"), Some(&"t1".to_string()));

        assert!(matches!(text_node.children[0], XmlNode::Text(_)));
        let text_value = match &text_node.children[0] {
            XmlNode::Text(value) => value,
            XmlNode::Element(_) => unreachable!("already asserted text"),
        };
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
