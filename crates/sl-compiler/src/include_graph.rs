use crate::*;

pub(crate) fn validate_include_graph(
    sources: &BTreeMap<String, SourceFile>,
) -> Result<(), ScriptLangError> {
    for (file_path, source) in sources {
        for include in &source.includes {
            if !sources.contains_key(include) {
                return Err(ScriptLangError::new(
                    "INCLUDE_NOT_FOUND",
                    format!(
                        "Include \"{}\" referenced by \"{}\" not found.",
                        include, file_path
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
                    "INCLUDE_CYCLE",
                    format!("Include cycle detected: {}", cycle),
                ));
            }
            return Ok(());
        }

        states.insert(node.to_string(), State::Visiting);
        stack.push(node.to_string());

        let source = sources
            .get(node)
            .expect("include graph nodes should exist after validation");
        for include in &source.includes {
            dfs(include, sources, states, stack)?;
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

pub(crate) fn collect_reachable_files(
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
            for include in &source.includes {
                stack.push(include.clone());
            }
        }
    }

    visited
}
