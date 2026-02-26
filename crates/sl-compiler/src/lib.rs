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
    use std::fs;
    use std::path::{Path, PathBuf};

    pub(crate) fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    pub(crate) fn read_sources_recursive(
        root: &Path,
        current: &Path,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), std::io::Error> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                read_sources_recursive(root, &path, out)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path should be under root")
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(&path)?;
            out.insert(relative, text);
        }
        Ok(())
    }

    pub(crate) fn sources_from_example_dir(name: &str) -> BTreeMap<String, String> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .join("..")
            .join("..")
            .join("examples")
            .join(name);
        let mut out = BTreeMap::new();
        read_sources_recursive(&root, &root, &mut out).expect("example sources should read");
        out
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
