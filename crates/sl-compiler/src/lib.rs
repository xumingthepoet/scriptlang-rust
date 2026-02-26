include!("context.rs");
include!("pipeline.rs");
include!("source_parse.rs");
include!("include_graph.rs");
include!("defs_resolver.rs");
include!("type_expr.rs");
include!("json_symbols.rs");
include!("sanitize.rs");
include!("script_compile.rs");
include!("xml_utils.rs");
include!("macro_expand.rs");
include!("defaults.rs");

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
