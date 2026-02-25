fn parse_sources(
    xml_by_path: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, SourceFile>, ScriptLangError> {
    let mut sources = BTreeMap::new();

    for (raw_path, source_text) in xml_by_path {
        let file_path = normalize_virtual_path(raw_path);
        let kind = detect_source_kind(&file_path).ok_or_else(|| {
            ScriptLangError::new(
                "SOURCE_KIND_UNSUPPORTED",
                format!("Unsupported source extension: {}", file_path),
            )
        })?;

        let source = match kind {
            SourceKind::Json => {
                let parsed = serde_json::from_str::<JsonValue>(source_text).map_err(|error| {
                    ScriptLangError::new(
                        "JSON_PARSE_ERROR",
                        format!("Failed to parse JSON include \"{}\": {}", file_path, error),
                    )
                })?;

                SourceFile {
                    kind,
                    includes: Vec::new(),
                    xml_root: None,
                    json_value: Some(slvalue_from_json(parsed)),
                }
            }
            SourceKind::ScriptXml | SourceKind::DefsXml => {
                let document = parse_xml_document(source_text)?;
                let includes = parse_include_directives(source_text)
                    .into_iter()
                    .map(|include| resolve_include_path(&file_path, &include))
                    .collect::<Vec<_>>();

                SourceFile {
                    kind,
                    includes,
                    xml_root: Some(document.root),
                    json_value: None,
                }
            }
        };

        sources.insert(file_path, source);
    }

    Ok(sources)
}

fn detect_source_kind(path: &str) -> Option<SourceKind> {
    if path.ends_with(".script.xml") {
        Some(SourceKind::ScriptXml)
    } else if path.ends_with(".defs.xml") {
        Some(SourceKind::DefsXml)
    } else if path.ends_with(".json") {
        Some(SourceKind::Json)
    } else {
        None
    }
}

fn resolve_include_path(current_path: &str, include: &str) -> String {
    let parent = match Path::new(current_path).parent() {
        Some(parent) => parent,
        None => Path::new(""),
    };
    let joined = if include.starts_with('/') {
        PathBuf::from(include)
    } else {
        parent.join(include)
    };
    normalize_virtual_path(joined.to_string_lossy().as_ref())
}

fn normalize_virtual_path(path: &str) -> String {
    let mut stack: Vec<String> = Vec::new();
    for part in path.replace('\\', "/").split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if !stack.is_empty() {
                stack.pop();
            }
            continue;
        }
        stack.push(part.to_string());
    }
    stack.join("/")
}

#[cfg(test)]
mod source_parse_tests {
    use super::*;

    #[test]
    fn source_kind_and_path_helpers_cover_common_cases() {
        assert!(matches!(
            detect_source_kind("a.script.xml"),
            Some(SourceKind::ScriptXml)
        ));
        assert!(matches!(
            detect_source_kind("a.defs.xml"),
            Some(SourceKind::DefsXml)
        ));
        assert!(matches!(
            detect_source_kind("a.json"),
            Some(SourceKind::Json)
        ));
        assert!(detect_source_kind("a.txt").is_none());
    
        assert_eq!(
            resolve_include_path("nested/main.script.xml", "../shared.defs.xml"),
            "shared.defs.xml"
        );
        assert_eq!(
            resolve_include_path("/", "shared/main.script.xml"),
            "shared/main.script.xml"
        );
        assert_eq!(
            normalize_virtual_path("./a/./b/../c\\d.script.xml"),
            "a/c/d.script.xml"
        );
        assert_eq!(stable_base("a*b?c"), "a_b_c");
    }

}
