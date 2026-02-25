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
            "XML_EMPTY_ROOT",
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
