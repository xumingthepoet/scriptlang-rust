use crate::*;

struct ParsedSourceEntry {
    file_path: String,
    root: XmlElementNode,
    module_name: String,
    imports: Vec<ImportDirective>,
}

pub(crate) fn parse_sources(
    xml_by_path: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, SourceFile>, ScriptLangError> {
    let normalized_paths = collect_normalized_paths(xml_by_path);
    let mut parsed_entries = Vec::with_capacity(xml_by_path.len());
    let mut module_names_by_path = BTreeMap::new();

    for (raw_path, source_text) in xml_by_path {
        let parsed = parse_source_entry(raw_path, source_text)?;
        module_names_by_path.insert(parsed.file_path.clone(), parsed.module_name.clone());
        parsed_entries.push(parsed);
    }

    let mut sources = BTreeMap::new();
    for parsed in parsed_entries {
        let includes = resolve_import_directives(
            &parsed.file_path,
            &parsed.imports,
            &normalized_paths,
            &module_names_by_path,
        )
        .map_err(|error| with_file_context(error, &parsed.file_path))?;

        sources.insert(
            parsed.file_path,
            SourceFile {
                kind: SourceKind::ModuleXml,
                includes,
                xml_root: Some(parsed.root),
                json_value: None,
            },
        );
    }

    Ok(sources)
}

fn collect_normalized_paths(xml_by_path: &BTreeMap<String, String>) -> BTreeSet<String> {
    xml_by_path
        .keys()
        .map(|raw_path| normalize_virtual_path(raw_path))
        .collect::<BTreeSet<_>>()
}

fn parse_source_entry(
    raw_path: &str,
    source_text: &str,
) -> Result<ParsedSourceEntry, ScriptLangError> {
    let file_path = normalize_virtual_path(raw_path);
    detect_source_kind(&file_path)?;
    reject_legacy_include_directives(source_text, &file_path)?;

    let document =
        parse_xml_document(source_text).map_err(|error| with_file_context(error, &file_path))?;
    let module_name = extract_module_name(&document.root).map_err(|error| {
        let span = error.span.unwrap_or(SourceSpan::synthetic());
        with_file_context(
            ScriptLangError::with_span(error.code, error.message, span),
            &file_path,
        )
    })?;

    Ok(ParsedSourceEntry {
        file_path,
        root: document.root,
        module_name,
        imports: parse_import_directives(source_text),
    })
}

fn reject_legacy_include_directives(
    source_text: &str,
    file_path: &str,
) -> Result<(), ScriptLangError> {
    let legacy_includes = parse_legacy_include_directives(source_text);
    if legacy_includes.is_empty() {
        return Ok(());
    }

    Err(with_file_context(
        ScriptLangError::new(
            "IMPORT_LEGACY_INCLUDE_UNSUPPORTED",
            format!(
                "Legacy include directives are no longer supported. Replace `<!-- include: ... -->` with `<!-- import ... from ... -->` in \"{}\".",
                file_path
            ),
        ),
        file_path,
    ))
}

fn with_file_context(error: ScriptLangError, file_path: &str) -> ScriptLangError {
    with_file_context_shared(error, file_path)
}

pub(crate) fn detect_source_kind(path: &str) -> Result<SourceKind, ScriptLangError> {
    if path.ends_with(".script.xml") || path.ends_with(".defs.xml") || path.ends_with(".module.xml")
    {
        return Err(ScriptLangError::new(
            "SOURCE_KIND_LEGACY_XML_UNSUPPORTED",
            format!(
                "Legacy XML source \"{}\" is no longer supported. Migrate to a <module> root in a plain *.xml file.",
                path
            ),
        ));
    }

    if path.ends_with(".json") {
        return Err(ScriptLangError::new(
            "SOURCE_KIND_JSON_UNSUPPORTED",
            format!(
                "JSON source \"{}\" is no longer supported. Use only module *.xml sources.",
                path
            ),
        ));
    }

    if path.ends_with(".xml") {
        Ok(SourceKind::ModuleXml)
    } else {
        Err(ScriptLangError::new(
            "SOURCE_KIND_UNSUPPORTED",
            format!("Unsupported source extension: {}", path),
        ))
    }
}

pub(crate) fn resolve_import_path(current_path: &str, import_path: &str) -> String {
    let parent = match Path::new(current_path).parent() {
        Some(parent) => parent,
        None => Path::new(""),
    };
    let joined = if import_path.starts_with('/') {
        PathBuf::from(import_path)
    } else {
        parent.join(import_path)
    };
    normalize_virtual_path(joined.to_string_lossy().as_ref())
}

fn extract_module_name(root: &XmlElementNode) -> Result<String, ScriptLangError> {
    if root.name != "module" {
        return Err(ScriptLangError::with_span(
            "XML_ROOT_INVALID",
            "Module file root must be <module>.",
            root.location.clone(),
        ));
    }

    let Some(module_name) = root.attributes.get("name").map(String::as_str) else {
        return Err(ScriptLangError::with_span(
            "XML_MODULE_NAME_MISSING",
            "Module root requires non-empty name attribute.",
            root.location.clone(),
        ));
    };
    if module_name.trim().is_empty() {
        return Err(ScriptLangError::with_span(
            "XML_MODULE_NAME_MISSING",
            "Module root requires non-empty name attribute.",
            root.location.clone(),
        ));
    }

    Ok(module_name.to_string())
}

fn resolve_import_directives(
    current_path: &str,
    directives: &[ImportDirective],
    available_paths: &BTreeSet<String>,
    module_names_by_path: &BTreeMap<String, String>,
) -> Result<Vec<String>, ScriptLangError> {
    let mut includes = Vec::new();
    let mut seen = BTreeSet::new();

    for directive in directives {
        match directive {
            ImportDirective::File {
                module_name,
                from_path,
            } => {
                let resolved = resolve_import_path(current_path, from_path);
                if !available_paths.contains(&resolved) {
                    return Err(ScriptLangError::new(
                        "IMPORT_FILE_NOT_FOUND",
                        format!(
                            "Import target file \"{}\" resolved to \"{}\" in \"{}\" but was not found.",
                            from_path, resolved, current_path
                        ),
                    ));
                }
                let Some(actual_module_name) = module_names_by_path.get(&resolved) else {
                    return Err(ScriptLangError::new(
                        "IMPORT_TARGET_INVALID",
                        format!(
                            "Import target file \"{}\" resolved to \"{}\" in \"{}\" but is not a module source.",
                            from_path, resolved, current_path
                        ),
                    ));
                };
                if actual_module_name != module_name {
                    return Err(ScriptLangError::new(
                        "IMPORT_MODULE_MISMATCH",
                        format!(
                            "Import requires module \"{}\" from \"{}\", but that file declares module \"{}\".",
                            module_name, resolved, actual_module_name
                        ),
                    ));
                }
                if seen.insert(resolved.clone()) {
                    includes.push(resolved);
                }
            }
            ImportDirective::Directory {
                module_names,
                from_path,
            } => {
                if !from_path.ends_with('/') {
                    return Err(ScriptLangError::new(
                        "IMPORT_DIR_PATH_INVALID",
                        format!("Directory import from \"{}\" must end with '/'.", from_path),
                    ));
                }
                let prefix = resolve_import_directory_prefix(current_path, from_path);
                let module_paths = collect_directory_import_modules(
                    &prefix,
                    available_paths,
                    module_names_by_path,
                )
                .map_err(|error| with_file_context(error, current_path))?;
                if module_paths.is_empty() {
                    return Err(ScriptLangError::new(
                        "IMPORT_DIR_EMPTY",
                        format!(
                            "Import directory \"{}\" resolved to \"{}\" in \"{}\" but matched no module sources.",
                            from_path, prefix, current_path
                        ),
                    ));
                }

                for module_name in module_names {
                    let Some(resolved) = module_paths.get(module_name) else {
                        return Err(ScriptLangError::new(
                            "IMPORT_MODULE_NOT_FOUND",
                            format!(
                                "Import directory \"{}\" in \"{}\" does not contain module \"{}\".",
                                from_path, current_path, module_name
                            ),
                        ));
                    };
                    if seen.insert(resolved.clone()) {
                        includes.push(resolved.clone());
                    }
                }
            }
        }
    }

    Ok(includes)
}

fn resolve_import_directory_prefix(current_path: &str, import_path: &str) -> String {
    let trimmed = import_path.trim_end_matches('/');
    if trimmed.is_empty() && import_path.starts_with('/') {
        return String::new();
    }
    resolve_import_path(current_path, trimmed)
}

fn collect_directory_import_modules(
    directory_prefix: &str,
    available_paths: &BTreeSet<String>,
    module_names_by_path: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ScriptLangError> {
    let mut modules = BTreeMap::new();
    let mut matched_any = false;

    for path in available_paths {
        if !is_supported_import_path(path) || !is_path_within_directory(path, directory_prefix) {
            continue;
        }
        matched_any = true;
        let Some(module_name) = module_names_by_path.get(path) else {
            continue;
        };
        if let Some(existing_path) = modules.insert(module_name.clone(), path.clone()) {
            return Err(ScriptLangError::new(
                "IMPORT_MODULE_DUPLICATE",
                format!(
                    "Directory import prefix \"{}\" contains duplicate module name \"{}\" in \"{}\" and \"{}\".",
                    directory_prefix, module_name, existing_path, path
                ),
            ));
        }
    }

    if !matched_any {
        return Ok(BTreeMap::new());
    }

    Ok(modules)
}

fn is_supported_import_path(path: &str) -> bool {
    matches!(detect_source_kind(path), Ok(SourceKind::ModuleXml))
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
        assert_eq!(
            detect_source_kind("a.xml").expect("module kind"),
            SourceKind::ModuleXml
        );
        assert_eq!(
            detect_source_kind("a.json")
                .expect_err("json should fail")
                .code,
            "SOURCE_KIND_JSON_UNSUPPORTED"
        );
        assert_eq!(
            detect_source_kind("a.script.xml")
                .expect_err("legacy script should fail")
                .code,
            "SOURCE_KIND_LEGACY_XML_UNSUPPORTED"
        );
        assert_eq!(
            detect_source_kind("a.defs.xml")
                .expect_err("legacy defs should fail")
                .code,
            "SOURCE_KIND_LEGACY_XML_UNSUPPORTED"
        );
        assert_eq!(
            detect_source_kind("a.module.xml")
                .expect_err("legacy module should fail")
                .code,
            "SOURCE_KIND_LEGACY_XML_UNSUPPORTED"
        );
        assert_eq!(
            detect_source_kind("a.txt")
                .expect_err("txt should fail")
                .code,
            "SOURCE_KIND_UNSUPPORTED"
        );

        assert_eq!(
            resolve_import_path("nested/main.xml", "../shared.xml"),
            "shared.xml"
        );
        assert_eq!(
            resolve_import_path("/", "shared/main.xml"),
            "shared/main.xml"
        );
        assert_eq!(
            resolve_import_directory_prefix("nested/main.xml", "../shared/"),
            "shared"
        );
        assert_eq!(normalize_virtual_path("./a/./b/../c\\d.xml"), "a/c/d.xml");
        assert_eq!(stable_base("a*b?c"), "a_b_c");
        assert!(is_supported_import_path("a.xml"));
        assert!(!is_supported_import_path("a.json"));
        assert!(!is_supported_import_path("a.txt"));

        assert_eq!(normalize_virtual_path("a/b/c/../d"), "a/b/d");
        assert_eq!(normalize_virtual_path("../a"), "a");
        assert!(is_path_within_directory("shared/x.xml", "shared"));
        assert!(!is_path_within_directory("sharedx/y.xml", "shared"));
    }

    #[test]
    fn parse_sources_resolves_file_and_directory_imports() {
        let files = BTreeMap::from([
            (
                "main.xml".to_string(),
                r#"
<!-- import Helper from extras/helper.xml -->
<!-- import { Battle, Common } from shared/ -->
<module name="Main" default_access="public"><script name="main"></script></module>
"#
                .to_string(),
            ),
            (
                "shared/z-last.xml".to_string(),
                r#"<module name="Battle" default_access="public"><script name="main"></script></module>"#.to_string(),
            ),
            (
                "shared/a-first.xml".to_string(),
                r#"<module name="Common" default_access="public"><script name="main"></script></module>"#.to_string(),
            ),
            (
                "extras/helper.xml".to_string(),
                r#"<module name="Helper" default_access="public"><script name="main"></script></module>"#.to_string(),
            ),
        ]);

        let sources = parse_sources(&files).expect("imports should resolve");
        assert_eq!(
            sources.get("main.xml").expect("main source").includes,
            vec![
                "extras/helper.xml".to_string(),
                "shared/z-last.xml".to_string(),
                "shared/a-first.xml".to_string(),
            ]
        );
    }

    #[test]
    fn parse_sources_deduplicates_file_and_directory_imports() {
        let files = BTreeMap::from([
            (
                "main.xml".to_string(),
                r#"
<!-- import Base from shared/nested/base.xml -->
<!-- import { Base } from shared/ -->
<module name="Main" default_access="public"><script name="main"></script></module>
"#
                .to_string(),
            ),
            (
                "shared/nested/base.xml".to_string(),
                r#"<module name="Base" default_access="public"></module>"#.to_string(),
            ),
        ]);

        let sources = parse_sources(&files).expect("duplicate imports should dedupe");
        assert_eq!(
            sources.get("main.xml").expect("main source").includes,
            vec!["shared/nested/base.xml".to_string()]
        );
    }

    #[test]
    fn parse_sources_rejects_legacy_include_json_and_bad_import_targets() {
        let legacy = BTreeMap::from([(
            "main.xml".to_string(),
            r#"
<!-- include: shared.xml -->
<module name="Main" default_access="public"><script name="main"></script></module>
"#
            .to_string(),
        )]);
        assert_eq!(
            parse_sources(&legacy)
                .expect_err("legacy include should fail")
                .code,
            "IMPORT_LEGACY_INCLUDE_UNSUPPORTED"
        );

        let json = BTreeMap::from([("data.json".to_string(), "{}".to_string())]);
        assert_eq!(
            parse_sources(&json).expect_err("json should fail").code,
            "SOURCE_KIND_JSON_UNSUPPORTED"
        );

        let mismatch = BTreeMap::from([
            (
                "main.xml".to_string(),
                r#"
<!-- import Shared from shared.xml -->
<module name="Main" default_access="public"><script name="main"></script></module>
"#
                .to_string(),
            ),
            (
                "shared.xml".to_string(),
                r#"<module name="Other" default_access="public"><script name="main"></script></module>"#.to_string(),
            ),
        ]);
        assert_eq!(
            parse_sources(&mismatch)
                .expect_err("mismatch should fail")
                .code,
            "IMPORT_MODULE_MISMATCH"
        );
    }

    #[test]
    fn parse_sources_rejects_duplicate_or_missing_directory_modules() {
        let duplicate = BTreeMap::from([
            (
                "main.xml".to_string(),
                r#"
<!-- import { Shared } from mods/ -->
<module name="Main" default_access="public"><script name="main"></script></module>
"#
                .to_string(),
            ),
            (
                "mods/a.xml".to_string(),
                r#"<module name="Shared" default_access="public"></module>"#.to_string(),
            ),
            (
                "mods/nested/b.xml".to_string(),
                r#"<module name="Shared" default_access="public"></module>"#.to_string(),
            ),
        ]);
        assert_eq!(
            parse_sources(&duplicate)
                .expect_err("duplicate module should fail")
                .code,
            "IMPORT_MODULE_DUPLICATE"
        );

        let missing = BTreeMap::from([(
            "main.xml".to_string(),
            r#"
<!-- import { Shared } from mods/ -->
<module name="Main" default_access="public"><script name="main"></script></module>
"#
            .to_string(),
        )]);
        assert_eq!(
            parse_sources(&missing)
                .expect_err("missing dir import should fail")
                .code,
            "IMPORT_DIR_EMPTY"
        );
    }

    #[test]
    fn parse_sources_rejects_blank_module_name_and_helper_dedupes_duplicates() {
        let blank_name = BTreeMap::from([(
            "main.xml".to_string(),
            r#"<module name="   " default_access="public"><script name="main"></script></module>"#
                .to_string(),
        )]);
        assert_eq!(
            parse_sources(&blank_name)
                .expect_err("blank module name should fail")
                .code,
            "XML_MODULE_NAME_MISSING"
        );

        let includes = resolve_import_directives(
            "main.xml",
            &[
                ImportDirective::File {
                    module_name: "Shared".to_string(),
                    from_path: "shared.xml".to_string(),
                },
                ImportDirective::File {
                    module_name: "Shared".to_string(),
                    from_path: "shared.xml".to_string(),
                },
            ],
            &BTreeSet::from(["shared.xml".to_string()]),
            &BTreeMap::from([("shared.xml".to_string(), "Shared".to_string())]),
        )
        .expect("duplicate file imports should dedupe");
        assert_eq!(includes, vec!["shared.xml".to_string()]);
    }

    #[test]
    fn import_resolution_helpers_cover_private_error_branches() {
        let directives = vec![ImportDirective::File {
            module_name: "Shared".to_string(),
            from_path: "shared.xml".to_string(),
        }];
        let available_paths = BTreeSet::from(["shared.xml".to_string()]);
        let invalid_target =
            resolve_import_directives("main.xml", &directives, &available_paths, &BTreeMap::new())
                .expect_err("missing module index should fail");
        assert_eq!(invalid_target.code, "IMPORT_TARGET_INVALID");

        let invalid_dir = resolve_import_directives(
            "main.xml",
            &[ImportDirective::Directory {
                module_names: vec!["Shared".to_string()],
                from_path: "mods".to_string(),
            }],
            &available_paths,
            &BTreeMap::new(),
        )
        .expect_err("directory imports must end with slash");
        assert_eq!(invalid_dir.code, "IMPORT_DIR_PATH_INVALID");

        let missing_module = resolve_import_directives(
            "main.xml",
            &[ImportDirective::Directory {
                module_names: vec!["Shared".to_string()],
                from_path: "mods/".to_string(),
            }],
            &BTreeSet::from(["mods/other.xml".to_string()]),
            &BTreeMap::from([("mods/other.xml".to_string(), "Other".to_string())]),
        )
        .expect_err("missing named module should fail");
        assert_eq!(missing_module.code, "IMPORT_MODULE_NOT_FOUND");

        assert_eq!(resolve_import_directory_prefix("main.xml", "/"), "");
        assert!(is_path_within_directory("any.xml", ""));

        let duplicate_dir = collect_directory_import_modules(
            "mods",
            &BTreeSet::from(["mods/a.xml".to_string(), "mods/nested/b.xml".to_string()]),
            &BTreeMap::from([
                ("mods/a.xml".to_string(), "Shared".to_string()),
                ("mods/nested/b.xml".to_string(), "Shared".to_string()),
            ]),
        )
        .expect_err("duplicate names should fail");
        assert_eq!(duplicate_dir.code, "IMPORT_MODULE_DUPLICATE");

        let skipped_missing_index = collect_directory_import_modules(
            "mods",
            &BTreeSet::from(["mods/a.xml".to_string()]),
            &BTreeMap::new(),
        )
        .expect("paths without module index should be skipped");
        assert!(skipped_missing_index.is_empty());
    }

    #[test]
    fn parse_sources_attaches_file_path_for_xml_parse_error() {
        let files = BTreeMap::from([("bad.xml".to_string(), "<module>".to_string())]);
        let error = parse_sources(&files).expect_err("invalid xml should fail");
        assert_eq!(error.code, "XML_PARSE_ERROR");
        assert!(
            error.message.contains("bad.xml"),
            "message should include file path: {}",
            error.message
        );
    }
}
