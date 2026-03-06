use crate::*;

pub(crate) fn parse_sources(
    xml_by_path: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, SourceFile>, ScriptLangError> {
    let normalized_paths = xml_by_path
        .keys()
        .map(|raw_path| normalize_virtual_path(raw_path))
        .collect::<BTreeSet<_>>();
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
            SourceKind::ScriptXml | SourceKind::DefsXml | SourceKind::ModuleXml => {
                let document = parse_xml_document(source_text)
                    .map_err(|error| with_file_context(error, &file_path))?;
                let includes =
                    expand_include_directives(&file_path, source_text, &normalized_paths)
                        .map_err(|error| with_file_context(error, &file_path))?;

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

fn with_file_context(error: ScriptLangError, file_path: &str) -> ScriptLangError {
    let message = format!("In file \"{}\": {}", file_path, error.message);
    ScriptLangError::with_span(
        error.code,
        message,
        error.span.unwrap_or(SourceSpan::synthetic()),
    )
}

pub(crate) fn detect_source_kind(path: &str) -> Option<SourceKind> {
    if path.ends_with(".script.xml") {
        Some(SourceKind::ScriptXml)
    } else if path.ends_with(".defs.xml") {
        Some(SourceKind::DefsXml)
    } else if path.ends_with(".module.xml") {
        Some(SourceKind::ModuleXml)
    } else if path.ends_with(".json") {
        Some(SourceKind::Json)
    } else {
        None
    }
}

pub(crate) fn resolve_include_path(current_path: &str, include: &str) -> String {
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

fn expand_include_directives(
    current_path: &str,
    source_text: &str,
    available_paths: &BTreeSet<String>,
) -> Result<Vec<String>, ScriptLangError> {
    let mut includes = Vec::new();
    let mut seen = BTreeSet::new();

    for include in parse_include_directives(source_text) {
        if include.ends_with('/') {
            let prefix = resolve_include_directory_prefix(current_path, &include);
            let matches = collect_directory_include_matches(&prefix, available_paths);
            if matches.is_empty() {
                return Err(ScriptLangError::new(
                    "INCLUDE_DIR_EMPTY",
                    format!(
                        "Include directory \"{}\" resolved to \"{}\" in \"{}\" but matched no supported source files.",
                        include, prefix, current_path
                    ),
                ));
            }

            for matched in matches {
                if seen.insert(matched.clone()) {
                    includes.push(matched);
                }
            }
            continue;
        }

        let resolved = resolve_include_path(current_path, &include);
        if seen.insert(resolved.clone()) {
            includes.push(resolved);
        }
    }

    Ok(includes)
}

fn resolve_include_directory_prefix(current_path: &str, include: &str) -> String {
    let trimmed = include.trim_end_matches('/');
    if trimmed.is_empty() && include.starts_with('/') {
        return String::new();
    }
    resolve_include_path(current_path, trimmed)
}

fn collect_directory_include_matches(
    directory_prefix: &str,
    available_paths: &BTreeSet<String>,
) -> Vec<String> {
    let mut matches = available_paths
        .iter()
        .filter(|path| {
            detect_source_kind(path).is_some() && is_path_within_directory(path, directory_prefix)
        })
        .cloned()
        .collect::<Vec<_>>();
    matches.sort();
    matches
}

fn is_path_within_directory(path: &str, directory_prefix: &str) -> bool {
    if directory_prefix.is_empty() {
        return true;
    }

    path == directory_prefix
        || path
            .strip_prefix(directory_prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(crate) fn normalize_virtual_path(path: &str) -> String {
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
        let kind_name = |kind: SourceKind| match kind {
            SourceKind::ScriptXml => "script",
            SourceKind::DefsXml => "defs",
            SourceKind::ModuleXml => "module",
            SourceKind::Json => "json",
        };
        let script_kind = detect_source_kind("a.script.xml").expect("script kind");
        let defs_kind = detect_source_kind("a.defs.xml").expect("defs kind");
        let module_kind = detect_source_kind("a.module.xml").expect("module kind");
        let json_kind = detect_source_kind("a.json").expect("json kind");
        assert_eq!(kind_name(script_kind), "script");
        assert_eq!(kind_name(defs_kind), "defs");
        assert_eq!(kind_name(module_kind), "module");
        assert_eq!(kind_name(json_kind), "json");
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
            resolve_include_directory_prefix("nested/main.script.xml", "../shared/"),
            "shared"
        );
        assert_eq!(
            normalize_virtual_path("./a/./b/../c\\d.script.xml"),
            "a/c/d.script.xml"
        );
        assert_eq!(stable_base("a*b?c"), "a_b_c");

        // Test .. path handling explicitly (covers line 87)
        assert_eq!(normalize_virtual_path("a/b/c/../d"), "a/b/d");
        assert_eq!(normalize_virtual_path("../a"), "a");
        assert!(is_path_within_directory("shared/x.script.xml", "shared"));
        assert!(!is_path_within_directory("sharedx/y.script.xml", "shared"));
    }

    #[test]
    fn parse_sources_expands_directory_includes_in_sorted_order() {
        let files = BTreeMap::from([
            (
                "main.script.xml".to_string(),
                r#"
<!-- include: shared/ -->
<!-- include: extras/helper.script.xml -->
<script name="main"></script>
"#
                .to_string(),
            ),
            (
                "shared/z-last.script.xml".to_string(),
                r#"<script name="z-last"></script>"#.to_string(),
            ),
            (
                "shared/a-first.script.xml".to_string(),
                r#"<script name="a-first"></script>"#.to_string(),
            ),
            ("shared/data.json".to_string(), r#"{"ok":true}"#.to_string()),
            (
                "shared/nested/base.defs.xml".to_string(),
                r#"<defs name="base"></defs>"#.to_string(),
            ),
            (
                "extras/helper.script.xml".to_string(),
                r#"<script name="helper"></script>"#.to_string(),
            ),
        ]);

        let sources = parse_sources(&files).expect("directory includes should expand");
        assert_eq!(
            sources
                .get("main.script.xml")
                .expect("main source")
                .includes,
            vec![
                "shared/a-first.script.xml".to_string(),
                "shared/data.json".to_string(),
                "shared/nested/base.defs.xml".to_string(),
                "shared/z-last.script.xml".to_string(),
                "extras/helper.script.xml".to_string(),
            ]
        );
    }

    #[test]
    fn parse_sources_deduplicates_directory_and_file_includes() {
        let files = BTreeMap::from([
            (
                "main.script.xml".to_string(),
                r#"
<!-- include: shared/ -->
<!-- include: shared/nested/base.defs.xml -->
<script name="main"></script>
"#
                .to_string(),
            ),
            (
                "shared/nested/base.defs.xml".to_string(),
                r#"<defs name="base"></defs>"#.to_string(),
            ),
        ]);

        let sources = parse_sources(&files).expect("duplicate includes should dedupe");
        assert_eq!(
            sources
                .get("main.script.xml")
                .expect("main source")
                .includes,
            vec!["shared/nested/base.defs.xml".to_string()]
        );
    }

    #[test]
    fn parse_sources_deduplicates_overlapping_directory_includes_and_supports_root_prefix() {
        let files = BTreeMap::from([
            (
                "nested/main.script.xml".to_string(),
                r#"
<!-- include: / -->
<!-- include: ../shared/ -->
<script name="main"></script>
"#
                .to_string(),
            ),
            (
                "shared/base.defs.xml".to_string(),
                r#"<defs name="base"></defs>"#.to_string(),
            ),
            ("shared/data.json".to_string(), r#"{"ok":true}"#.to_string()),
        ]);

        let sources = parse_sources(&files).expect("root directory include should expand");
        assert_eq!(
            sources
                .get("nested/main.script.xml")
                .expect("main source")
                .includes,
            vec![
                "nested/main.script.xml".to_string(),
                "shared/base.defs.xml".to_string(),
                "shared/data.json".to_string(),
            ]
        );
    }

    #[test]
    fn parse_sources_rejects_empty_directory_includes() {
        let files = BTreeMap::from([(
            "main.script.xml".to_string(),
            r#"
<!-- include: shared/ -->
<script name="main"></script>
"#
            .to_string(),
        )]);

        let error = parse_sources(&files).expect_err("empty directory include should fail");
        assert_eq!(error.code, "INCLUDE_DIR_EMPTY");
        assert!(error.message.contains("shared/"));
        assert!(error.message.contains("main.script.xml"));
    }

    #[test]
    fn parse_sources_attaches_file_path_for_xml_parse_error() {
        let files = BTreeMap::from([("bad.script.xml".to_string(), "<script>".to_string())]);
        let error = parse_sources(&files).expect_err("invalid xml should fail");
        assert_eq!(error.code, "XML_PARSE_ERROR");
        assert!(
            error.message.contains("bad.script.xml"),
            "message should include file path: {}",
            error.message
        );
    }
}
