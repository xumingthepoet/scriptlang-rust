use crate::*;

pub(crate) fn validate_import_graph(
    sources: &BTreeMap<String, SourceFile>,
) -> Result<(), ScriptLangError> {
    for (file_path, source) in sources {
        for import in &source.imports {
            if !sources.contains_key(import) {
                return Err(ScriptLangError::new(
                    "IMPORT_NOT_FOUND",
                    format!(
                        "Import \"{}\" referenced by \"{}\" not found.",
                        import, file_path
                    ),
                ));
            }
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum State {
        Visiting,
        Done,
    }

    fn dfs(
        node: &str,
        sources: &BTreeMap<String, SourceFile>,
        states: &mut HashMap<String, State>,
        stack: &mut Vec<String>,
    ) -> Result<(), ScriptLangError> {
        if let Some(state) = states.get(node) {
            if *state == State::Visiting {
                stack.push(node.to_string());
                let cycle = stack.join(" -> ");
                return Err(ScriptLangError::new(
                    "IMPORT_CYCLE",
                    format!("Import cycle detected: {}", cycle),
                ));
            }
            return Ok(());
        }

        states.insert(node.to_string(), State::Visiting);
        stack.push(node.to_string());

        let source = sources
            .get(node)
            .expect("import graph nodes should exist after validation");
        for import in &source.imports {
            dfs(import, sources, states, stack)?;
        }

        stack.pop();
        states.insert(node.to_string(), State::Done);
        Ok(())
    }

    let mut states: HashMap<String, State> = HashMap::new();
    for file_path in sources.keys() {
        dfs(file_path, sources, &mut states, &mut Vec::new())?;
    }

    Ok(())
}

pub(crate) fn collect_reachable_imports(
    start: &str,
    sources: &BTreeMap<String, SourceFile>,
) -> BTreeSet<String> {
    let mut visited = BTreeSet::new();
    let mut stack = vec![start.to_string()];

    while let Some(path) = stack.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        if let Some(source) = sources.get(&path) {
            for import in &source.imports {
                stack.push(import.clone());
            }
        }
    }

    visited
}

#[cfg(test)]
mod import_graph_tests {
    use super::*;

    #[test]
    fn validate_import_graph_reports_missing_import() {
        let sources = BTreeMap::from([(
            "main.xml".to_string(),
            SourceFile {
                kind: SourceKind::ModuleXml,
                imports: vec!["missing.xml".to_string()],
                xml_root: None,
                json_value: None,
            },
        )]);

        let error = validate_import_graph(&sources).expect_err("missing import should fail");
        assert_eq!(error.code, "IMPORT_NOT_FOUND");
    }
}
