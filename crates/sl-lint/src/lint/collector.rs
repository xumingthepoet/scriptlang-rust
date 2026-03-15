use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use regex::Regex;
use sl_compiler::CompileProjectBundleResult;
use sl_core::{
    module_namespace_symbol, preprocess_scriptlang_rhai_input, rhai_function_symbol, ChoiceEntry,
    RhaiInputMode, ScriptIr, ScriptNode, ScriptTarget, SourceLocation, SourceSpan,
};
use sl_parser::{
    parse_alias_directives, parse_import_directives, parse_xml_document, AliasDirective,
    ImportDirective, XmlElementNode, XmlNode,
};

#[derive(Debug, Clone)]
pub(crate) struct NamedDecl {
    pub(crate) name: String,
    pub(crate) file: String,
    pub(crate) span: SourceSpan,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportDecl {
    pub(crate) module_name: String,
    pub(crate) file: String,
    pub(crate) span: SourceSpan,
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleDecl {
    pub(crate) module_name: String,
    pub(crate) file: String,
    pub(crate) span: SourceSpan,
    pub(crate) imports: Vec<ImportDecl>,
}

#[derive(Debug, Clone)]
pub(crate) struct ShortNameCandidate {
    pub(crate) file: String,
    pub(crate) span: SourceSpan,
    pub(crate) qualified_name: String,
    pub(crate) short_name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct UnreachableNode {
    pub(crate) file: String,
    pub(crate) script_name: String,
    pub(crate) span: SourceSpan,
}

#[derive(Debug, Clone)]
pub(crate) struct ScriptLocals {
    pub(crate) params: Vec<NamedDecl>,
    pub(crate) vars: Vec<NamedDecl>,
    pub(crate) used_locals: HashSet<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionBodyDecl {
    pub(crate) module_name: String,
    pub(crate) file: String,
    pub(crate) span: SourceSpan,
    pub(crate) code: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LintContext {
    pub(crate) modules: BTreeMap<String, ModuleDecl>,
    pub(crate) scripts: BTreeMap<String, NamedDecl>,
    pub(crate) exported_scripts: HashSet<String>,
    pub(crate) exported_functions: HashSet<String>,
    pub(crate) functions: BTreeMap<String, NamedDecl>,
    pub(crate) function_bodies: BTreeMap<String, FunctionBodyDecl>,
    pub(crate) module_vars: BTreeMap<String, NamedDecl>,
    pub(crate) module_consts: BTreeMap<String, NamedDecl>,
    pub(crate) script_locals: BTreeMap<String, ScriptLocals>,
    pub(crate) script_edges: HashMap<String, HashSet<String>>,
    pub(crate) reachable_scripts: HashSet<String>,
    pub(crate) used_scripts: HashSet<String>,
    pub(crate) used_functions: HashSet<String>,
    pub(crate) used_module_vars: HashSet<String>,
    pub(crate) used_module_consts: HashSet<String>,
    pub(crate) used_import_modules_by_file: HashMap<String, HashSet<String>>,
    pub(crate) alias_symbol_targets: HashSet<String>,
    pub(crate) short_name_candidates: Vec<ShortNameCandidate>,
    pub(crate) unreachable_nodes: Vec<UnreachableNode>,
}

pub(crate) fn collect_context(
    xml_by_path: &BTreeMap<String, String>,
    bundle: &CompileProjectBundleResult,
    entry_script: &str,
) -> LintContext {
    let mut context = LintContext::default();
    collect_declarations(xml_by_path, &mut context);
    collect_alias_symbol_usage(&mut context);
    collect_script_usage(bundle, &mut context);
    collect_initializer_usage(bundle, &mut context);
    collect_function_usage(&mut context);
    context.reachable_scripts = collect_reachable(entry_script, &context.script_edges);
    context
}

fn collect_declarations(xml_by_path: &BTreeMap<String, String>, context: &mut LintContext) {
    for (file, source) in xml_by_path {
        let Ok(document) = parse_xml_document(source) else {
            continue;
        };
        if document.root.name != "module" {
            continue;
        }
        let Some(module_name) = document.root.attributes.get("name").cloned() else {
            continue;
        };
        collect_exports(&document.root, &module_name, context);
        let alias_directives = parse_alias_directives(source);
        collect_short_name_candidates_from_source(
            file,
            source,
            &module_name,
            &alias_directives,
            context,
        );

        let mut imports = Vec::new();
        for directive in parse_import_directives(source) {
            match directive {
                ImportDirective::File { module_name, .. } => imports.push(ImportDecl {
                    module_name,
                    file: file.clone(),
                    span: document.root.location.clone(),
                }),
                ImportDirective::Directory { module_names, .. } => {
                    for module_name in module_names {
                        imports.push(ImportDecl {
                            module_name,
                            file: file.clone(),
                            span: document.root.location.clone(),
                        });
                    }
                }
            }
        }
        track_module_token_uses(file, source, &imports, context);
        for alias in alias_directives {
            if let Some((target_module, _)) = alias.target_qualified_name.split_once('.') {
                context
                    .used_import_modules_by_file
                    .entry(file.clone())
                    .or_default()
                    .insert(target_module.to_string());
            }
            context
                .alias_symbol_targets
                .insert(alias.target_qualified_name.clone());
        }

        context.modules.insert(
            module_name.clone(),
            ModuleDecl {
                module_name: module_name.clone(),
                file: file.clone(),
                span: document.root.location.clone(),
                imports,
            },
        );

        for child in &document.root.children {
            let XmlNode::Element(node) = child else {
                continue;
            };
            let Some(name) = node.attributes.get("name").cloned() else {
                continue;
            };
            let qualified_name = format!("{}.{}", module_name, name);
            let decl = NamedDecl {
                name,
                file: file.clone(),
                span: node.location.clone(),
            };
            match node.name.as_str() {
                "script" => {
                    context.scripts.insert(qualified_name, decl);
                }
                "function" => {
                    context.functions.insert(qualified_name.clone(), decl);
                    context.function_bodies.insert(
                        qualified_name,
                        FunctionBodyDecl {
                            module_name: module_name.clone(),
                            file: file.clone(),
                            span: node.location.clone(),
                            code: collect_node_text(node),
                        },
                    );
                }
                "var" => {
                    context.module_vars.insert(qualified_name, decl);
                }
                "const" => {
                    context.module_consts.insert(qualified_name, decl);
                }
                _ => {}
            }
        }
    }
}

fn track_module_token_uses(
    file: &str,
    source: &str,
    imports: &[ImportDecl],
    context: &mut LintContext,
) {
    let source_without_comments = strip_xml_comments(source);
    for import in imports {
        let needle = format!("{}.", import.module_name);
        if source_without_comments.contains(&needle) {
            context
                .used_import_modules_by_file
                .entry(file.to_string())
                .or_default()
                .insert(import.module_name.clone());
        }
    }
}

fn strip_xml_comments(source: &str) -> String {
    xml_comment_regex().replace_all(source, " ").into_owned()
}

fn collect_short_name_candidates_from_source(
    file: &str,
    source: &str,
    module_name: &str,
    alias_directives: &[AliasDirective],
    context: &mut LintContext,
) {
    let pattern = format!(
        r"\b({}\.[A-Za-z_][A-Za-z0-9_-]*)",
        regex::escape(module_name)
    );
    let regex = Regex::new(&pattern).expect("short-name candidate regex should compile");
    let comment_spans = xml_comment_regex()
        .find_iter(source)
        .map(|item| (item.start(), item.end()))
        .collect::<Vec<_>>();

    for caps in regex.captures_iter(source) {
        let Some(matched) = caps.get(1) else {
            continue;
        };
        let start = matched.start();
        if comment_spans
            .iter()
            .any(|(begin, end)| start >= *begin && start < *end)
        {
            continue;
        }
        let text = matched.as_str().to_string();
        let Some((_, short_name)) = text.split_once('.') else {
            continue;
        };
        context.short_name_candidates.push(ShortNameCandidate {
            file: file.to_string(),
            span: source_span_from_offset(source, start, matched.end()),
            qualified_name: text.clone(),
            short_name: short_name.to_string(),
        });
    }

    for alias in alias_directives {
        let qualified_name = alias.target_qualified_name.as_str();
        if qualified_name.starts_with(&format!("{module_name}.")) {
            continue;
        }

        for (start, _) in source.match_indices(qualified_name) {
            let end = start + qualified_name.len();
            if comment_spans
                .iter()
                .any(|(begin, finish)| start >= *begin && start < *finish)
            {
                continue;
            }
            if !is_candidate_token_boundary(source, start, end) {
                continue;
            }
            context.short_name_candidates.push(ShortNameCandidate {
                file: file.to_string(),
                span: source_span_from_offset(source, start, end),
                qualified_name: qualified_name.to_string(),
                short_name: alias.alias_name.clone(),
            });
        }
    }
}

fn is_candidate_token_boundary(source: &str, start: usize, end: usize) -> bool {
    let left = source[..start].chars().next_back();
    let right = source[end..].chars().next();
    is_candidate_boundary_char(left) && is_candidate_boundary_char(right)
}

fn is_candidate_boundary_char(ch: Option<char>) -> bool {
    ch.map(|value| !(value.is_ascii_alphanumeric() || value == '_' || value == '.'))
        .unwrap_or(true)
}

fn source_span_from_offset(source: &str, start: usize, end: usize) -> SourceSpan {
    let start_loc = source_location_from_offset(source, start);
    let end_loc = source_location_from_offset(source, end.saturating_sub(1));
    SourceSpan {
        start: start_loc,
        end: end_loc,
    }
}

fn source_location_from_offset(source: &str, offset: usize) -> SourceLocation {
    let mut line = 1usize;
    let mut column = 1usize;
    for (index, ch) in source.char_indices() {
        if index >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    SourceLocation { line, column }
}

fn collect_exports(root: &XmlElementNode, module_name: &str, context: &mut LintContext) {
    let Some(export_attr) = root.attributes.get("export") else {
        return;
    };
    for segment in export_attr.split(';') {
        let trimmed = segment.trim();
        let Some((kind, values)) = trimmed.split_once(':') else {
            continue;
        };
        let kind = kind.trim();
        for item in values.split(',') {
            let name = item.trim();
            if name.is_empty() {
                continue;
            }
            let qualified = format!("{}.{}", module_name, name);
            match kind {
                "script" => {
                    context.exported_scripts.insert(qualified);
                }
                "function" => {
                    context.exported_functions.insert(qualified);
                }
                _ => {}
            }
        }
    }
}

fn collect_alias_symbol_usage(context: &mut LintContext) {
    for target in &context.alias_symbol_targets {
        if context.scripts.contains_key(target) {
            context.used_scripts.insert(target.clone());
        }
        if context.functions.contains_key(target) {
            context.used_functions.insert(target.clone());
        }
        if context.module_vars.contains_key(target) {
            context.used_module_vars.insert(target.clone());
        }
        if context.module_consts.contains_key(target) {
            context.used_module_consts.insert(target.clone());
        }
    }
}

fn collect_script_usage(bundle: &CompileProjectBundleResult, context: &mut LintContext) {
    for (script_name, script) in &bundle.scripts {
        let module_name = script.module_name.clone().unwrap_or_default();
        let file = script.script_path.clone();
        let mut locals = ScriptLocals {
            params: script
                .params
                .iter()
                .map(|param| NamedDecl {
                    name: param.name.clone(),
                    file: file.clone(),
                    span: param.location.clone(),
                })
                .collect(),
            vars: Vec::new(),
            used_locals: HashSet::new(),
        };

        for group in script.groups.values() {
            let mut terminated = false;
            for node in &group.nodes {
                if terminated {
                    context.unreachable_nodes.push(UnreachableNode {
                        file: file.clone(),
                        script_name: script_name.clone(),
                        span: node_span(node).clone(),
                    });
                }

                match node {
                    ScriptNode::Call {
                        target_script,
                        args,
                        location,
                        ..
                    } => {
                        collect_script_target_usage(
                            target_script,
                            &module_name,
                            &file,
                            location,
                            Some(script_name),
                            context,
                            &mut locals,
                        );
                        for arg in args {
                            collect_expression_usage(
                                &arg.value_expr,
                                &module_name,
                                &file,
                                location,
                                Some(script),
                                context,
                                Some(&mut locals),
                                Some(script_name),
                            );
                        }
                    }
                    ScriptNode::Goto {
                        target_script,
                        args,
                        location,
                        ..
                    } => {
                        collect_script_target_usage(
                            target_script,
                            &module_name,
                            &file,
                            location,
                            Some(script_name),
                            context,
                            &mut locals,
                        );
                        for arg in args {
                            collect_expression_usage(
                                &arg.value_expr,
                                &module_name,
                                &file,
                                location,
                                Some(script),
                                context,
                                Some(&mut locals),
                                Some(script_name),
                            );
                        }
                    }
                    ScriptNode::Return { .. } | ScriptNode::End { .. } => {}
                    ScriptNode::If {
                        when_expr,
                        location,
                        ..
                    }
                    | ScriptNode::While {
                        when_expr,
                        location,
                        ..
                    } => {
                        collect_expression_usage(
                            when_expr,
                            &module_name,
                            &file,
                            location,
                            Some(script),
                            context,
                            Some(&mut locals),
                            Some(script_name),
                        );
                    }
                    ScriptNode::Code { code, location, .. } => {
                        collect_expression_usage(
                            code,
                            &module_name,
                            &file,
                            location,
                            Some(script),
                            context,
                            Some(&mut locals),
                            Some(script_name),
                        );
                    }
                    ScriptNode::Var {
                        declaration,
                        location,
                        ..
                    } => {
                        locals.vars.push(NamedDecl {
                            name: declaration.name.clone(),
                            file: file.clone(),
                            span: location.clone(),
                        });
                        if let Some(expr) = &declaration.initial_value_expr {
                            collect_expression_usage(
                                expr,
                                &module_name,
                                &file,
                                location,
                                Some(script),
                                context,
                                Some(&mut locals),
                                Some(script_name),
                            );
                        }
                    }
                    ScriptNode::Text {
                        value, location, ..
                    }
                    | ScriptNode::Debug {
                        value, location, ..
                    } => {
                        for expr in extract_template_expressions(value) {
                            collect_expression_usage(
                                &expr,
                                &module_name,
                                &file,
                                location,
                                Some(script),
                                context,
                                Some(&mut locals),
                                Some(script_name),
                            );
                        }
                    }
                    ScriptNode::Choice {
                        prompt_text,
                        entries,
                        location,
                        ..
                    } => {
                        for expr in extract_template_expressions(prompt_text) {
                            collect_expression_usage(
                                &expr,
                                &module_name,
                                &file,
                                location,
                                Some(script),
                                context,
                                Some(&mut locals),
                                Some(script_name),
                            );
                        }
                        for entry in entries {
                            collect_choice_entry_usage(
                                entry,
                                &module_name,
                                &file,
                                script_name,
                                Some(script),
                                context,
                                &mut locals,
                            );
                        }
                    }
                    ScriptNode::Input {
                        prompt_text,
                        location,
                        target_var,
                        ..
                    } => {
                        for expr in extract_template_expressions(prompt_text) {
                            collect_expression_usage(
                                &expr,
                                &module_name,
                                &file,
                                location,
                                Some(script),
                                context,
                                Some(&mut locals),
                                Some(script_name),
                            );
                        }
                        locals.used_locals.insert(target_var.clone());
                    }
                    ScriptNode::Break { .. } | ScriptNode::Continue { .. } => {}
                }

                if matches!(
                    node,
                    ScriptNode::Goto { .. }
                        | ScriptNode::End { .. }
                        | ScriptNode::Return { .. }
                        | ScriptNode::Break { .. }
                        | ScriptNode::Continue { .. }
                ) {
                    terminated = true;
                }
            }
        }

        context.script_locals.insert(script_name.clone(), locals);
    }
}

fn collect_choice_entry_usage(
    entry: &ChoiceEntry,
    module_name: &str,
    file: &str,
    script_name: &str,
    script: Option<&ScriptIr>,
    context: &mut LintContext,
    locals: &mut ScriptLocals,
) {
    match entry {
        ChoiceEntry::Static { option } => {
            if let Some(expr) = &option.when_expr {
                collect_expression_usage(
                    expr,
                    module_name,
                    file,
                    &option.location,
                    script,
                    context,
                    Some(locals),
                    Some(script_name),
                );
            }
            for expr in extract_template_expressions(&option.text) {
                collect_expression_usage(
                    &expr,
                    module_name,
                    file,
                    &option.location,
                    script,
                    context,
                    Some(locals),
                    Some(script_name),
                );
            }
        }
        ChoiceEntry::Dynamic { block } => {
            collect_expression_usage(
                &block.array_expr,
                module_name,
                file,
                &block.location,
                script,
                context,
                Some(locals),
                Some(script_name),
            );
            if let Some(expr) = &block.template.when_expr {
                collect_expression_usage(
                    expr,
                    module_name,
                    file,
                    &block.template.location,
                    script,
                    context,
                    Some(locals),
                    Some(script_name),
                );
            }
            for expr in extract_template_expressions(&block.template.text) {
                collect_expression_usage(
                    &expr,
                    module_name,
                    file,
                    &block.template.location,
                    script,
                    context,
                    Some(locals),
                    Some(script_name),
                );
            }
        }
    }
}

fn collect_initializer_usage(bundle: &CompileProjectBundleResult, context: &mut LintContext) {
    for decl in bundle.module_var_declarations.values() {
        if let Some(expr) = &decl.initial_value_expr {
            let file = context
                .modules
                .get(&decl.namespace)
                .map(|item| item.file.clone())
                .unwrap_or_default();
            collect_initializer_expression(expr, &decl.namespace, &file, &decl.location, context);
        }
    }
    for decl in bundle.module_const_declarations.values() {
        if let Some(expr) = &decl.initial_value_expr {
            let file = context
                .modules
                .get(&decl.namespace)
                .map(|item| item.file.clone())
                .unwrap_or_default();
            collect_initializer_expression(expr, &decl.namespace, &file, &decl.location, context);
        }
    }
}

fn collect_initializer_expression(
    expr: &str,
    module_name: &str,
    file: &str,
    span: &SourceSpan,
    context: &mut LintContext,
) {
    let refs = analyze_expression(expr);
    for call in refs.calls {
        mark_function_use(&call, module_name, file, span, None, context);
    }
    for call_symbol in refs.call_symbol_targets {
        mark_function_use(&call_symbol, module_name, file, span, None, context);
    }
    for ident in refs.identifiers {
        mark_value_use(&ident, module_name, file, span, None, context);
    }
    for function_name in refs.function_literals {
        mark_function_use(&function_name, module_name, file, span, None, context);
    }
    for script_name in refs.script_literals {
        mark_script_use(&script_name, module_name, file, span, None, context);
    }
}

fn collect_function_usage(context: &mut LintContext) {
    let mut queue: VecDeque<String> = context.used_functions.iter().cloned().collect();
    let mut visited = HashSet::new();
    while let Some(function_name) = queue.pop_front() {
        if !visited.insert(function_name.clone()) {
            continue;
        }
        let Some(function) = context.function_bodies.get(&function_name).cloned() else {
            continue;
        };
        collect_expression_usage(
            &function.code,
            &function.module_name,
            &function.file,
            &function.span,
            None,
            context,
            None,
            None,
        );
        for discovered in context.used_functions.iter().cloned() {
            if !visited.contains(&discovered) {
                queue.push_back(discovered);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_script_target_usage(
    target_script: &ScriptTarget,
    module_name: &str,
    file: &str,
    location: &SourceSpan,
    from_script: Option<&str>,
    context: &mut LintContext,
    locals: &mut ScriptLocals,
) {
    match target_script {
        ScriptTarget::Literal { script_name } => {
            mark_script_use(
                script_name,
                module_name,
                file,
                location,
                from_script,
                context,
            );
        }
        ScriptTarget::Variable { var_name } => {
            locals.used_locals.insert(var_name.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_expression_usage(
    expr: &str,
    module_name: &str,
    file: &str,
    span: &SourceSpan,
    script: Option<&ScriptIr>,
    context: &mut LintContext,
    mut locals: Option<&mut ScriptLocals>,
    current_script_name: Option<&str>,
) {
    let refs = analyze_expression(expr);
    for call in refs.calls {
        mark_function_use(&call, module_name, file, span, script, context);
    }
    for call_symbol in refs.call_symbol_targets {
        mark_function_use(&call_symbol, module_name, file, span, script, context);
    }
    for ident in refs.identifiers {
        if let Some(local_state) = locals.as_mut() {
            if local_state.params.iter().any(|item| item.name == ident)
                || local_state.vars.iter().any(|item| item.name == ident)
            {
                local_state.used_locals.insert(ident.clone());
            }
        }
        mark_value_use(&ident, module_name, file, span, script, context);
    }
    for function_name in refs.function_literals {
        mark_function_use(&function_name, module_name, file, span, script, context);
    }
    for script_name in refs.script_literals {
        mark_script_use(
            &script_name,
            module_name,
            file,
            span,
            current_script_name,
            context,
        );
    }
}

fn mark_function_use(
    name: &str,
    module_name: &str,
    file: &str,
    _span: &SourceSpan,
    script: Option<&ScriptIr>,
    context: &mut LintContext,
) {
    if let Some(qualified_name) = resolve_runtime_function_symbol(name, context) {
        context.used_functions.insert(qualified_name.clone());
        track_module_and_short(&qualified_name, file, context);
        return;
    }

    if context.functions.contains_key(name) {
        context.used_functions.insert(name.to_string());
        track_module_and_short(name, file, context);
        return;
    }

    if !name.contains('.') {
        let qualified = format!("{}.{}", module_name, name);
        if context.functions.contains_key(&qualified) {
            context.used_functions.insert(qualified);
            return;
        }
    }

    if let Some(script) = script {
        if let Some(function) = script.visible_functions.get(name) {
            if context.functions.contains_key(&function.name) {
                context.used_functions.insert(function.name.clone());
                if name.contains('.') {
                    track_module_and_short(&function.name, file, context);
                }
            }
        }
    }
}

fn mark_value_use(
    name: &str,
    module_name: &str,
    file: &str,
    _span: &SourceSpan,
    script: Option<&ScriptIr>,
    context: &mut LintContext,
) {
    if let Some(resolved) = resolve_runtime_module_value_symbol(name, context) {
        match resolved {
            RuntimeModuleValue::Var(qualified_name) => {
                context.used_module_vars.insert(qualified_name.clone());
                track_module_and_short(&qualified_name, file, context);
            }
            RuntimeModuleValue::Const(qualified_name) => {
                context.used_module_consts.insert(qualified_name.clone());
                track_module_and_short(&qualified_name, file, context);
            }
        }
        return;
    }

    if context.module_vars.contains_key(name) {
        context.used_module_vars.insert(name.to_string());
        track_module_and_short(name, file, context);
        return;
    }
    if context.module_consts.contains_key(name) {
        context.used_module_consts.insert(name.to_string());
        track_module_and_short(name, file, context);
        return;
    }

    if !name.contains('.') {
        let q_var = format!("{}.{}", module_name, name);
        if context.module_vars.contains_key(&q_var) {
            context.used_module_vars.insert(q_var);
            return;
        }
        let q_const = format!("{}.{}", module_name, name);
        if context.module_consts.contains_key(&q_const) {
            context.used_module_consts.insert(q_const);
            return;
        }
    }

    if let Some(script) = script {
        if let Some(decl) = script.visible_module_vars.get(name) {
            context.used_module_vars.insert(decl.qualified_name.clone());
            if name.contains('.') {
                track_module_and_short(&decl.qualified_name, file, context);
            }
        }
        if let Some(decl) = script.visible_module_consts.get(name) {
            context
                .used_module_consts
                .insert(decl.qualified_name.clone());
            if name.contains('.') {
                track_module_and_short(&decl.qualified_name, file, context);
            }
        }
    }
}

fn track_module_and_short(qualified: &str, file: &str, context: &mut LintContext) {
    if let Some((ns, _short)) = qualified.split_once('.') {
        context
            .used_import_modules_by_file
            .entry(file.to_string())
            .or_default()
            .insert(ns.to_string());
    }
}

fn mark_script_use(
    raw_script_name: &str,
    module_name: &str,
    file: &str,
    _span: &SourceSpan,
    from_script: Option<&str>,
    context: &mut LintContext,
) {
    let Some(script_name) = normalize_script_literal(raw_script_name, module_name) else {
        return;
    };
    context.used_scripts.insert(script_name.clone());
    if let Some(source_script) = from_script {
        context
            .script_edges
            .entry(source_script.to_string())
            .or_default()
            .insert(script_name.clone());
    }
    if let Some((target_module, _short_name)) = script_name.split_once('.') {
        context
            .used_import_modules_by_file
            .entry(file.to_string())
            .or_default()
            .insert(target_module.to_string());
    }
}

fn normalize_script_literal(raw_script_name: &str, module_name: &str) -> Option<String> {
    let trimmed = raw_script_name.trim().trim_start_matches('@');
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('.') {
        return Some(trimmed.to_string());
    }
    Some(format!("{}.{}", module_name, trimmed))
}

fn resolve_runtime_function_symbol(name: &str, context: &LintContext) -> Option<String> {
    if name.contains('.') {
        return None;
    }
    let mut found: Option<String> = None;
    for qualified_name in context.functions.keys() {
        if rhai_function_symbol(qualified_name) != name {
            continue;
        }
        match &found {
            None => found = Some(qualified_name.clone()),
            Some(existing) if existing == qualified_name => {}
            Some(_) => return None,
        }
    }
    found
}

enum RuntimeModuleValue {
    Var(String),
    Const(String),
}

fn resolve_runtime_module_value_symbol(
    name: &str,
    context: &LintContext,
) -> Option<RuntimeModuleValue> {
    let (namespace_symbol, field_name) = name.split_once('.')?;
    if !namespace_symbol.starts_with("__sl_module_ns_") {
        return None;
    }

    let mut matched_vars = context
        .module_vars
        .keys()
        .filter_map(|qualified_name| {
            let (namespace, local_name) = qualified_name.rsplit_once('.')?;
            if local_name != field_name {
                return None;
            }
            (module_namespace_symbol(namespace) == namespace_symbol).then(|| qualified_name.clone())
        })
        .collect::<Vec<_>>();
    let mut matched_consts = context
        .module_consts
        .keys()
        .filter_map(|qualified_name| {
            let (namespace, local_name) = qualified_name.rsplit_once('.')?;
            if local_name != field_name {
                return None;
            }
            (module_namespace_symbol(namespace) == namespace_symbol).then(|| qualified_name.clone())
        })
        .collect::<Vec<_>>();
    matched_vars.sort();
    matched_vars.dedup();
    matched_consts.sort();
    matched_consts.dedup();

    match (matched_vars.len(), matched_consts.len()) {
        (1, 0) => Some(RuntimeModuleValue::Var(matched_vars[0].clone())),
        (0, 1) => Some(RuntimeModuleValue::Const(matched_consts[0].clone())),
        _ => None,
    }
}

fn collect_reachable(
    entry_script: &str,
    edges: &HashMap<String, HashSet<String>>,
) -> HashSet<String> {
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(entry_script.to_string());
    while let Some(current) = queue.pop_front() {
        if !reachable.insert(current.clone()) {
            continue;
        }
        if let Some(next) = edges.get(&current) {
            for target in next {
                queue.push_back(target.clone());
            }
        }
    }
    reachable
}

#[derive(Debug, Clone, Default)]
struct ExpressionRefs {
    calls: BTreeSet<String>,
    call_symbol_targets: BTreeSet<String>,
    identifiers: BTreeSet<String>,
    function_literals: BTreeSet<String>,
    script_literals: BTreeSet<String>,
}

fn analyze_expression(expr: &str) -> ExpressionRefs {
    let mut refs = ExpressionRefs::default();
    let rewritten = preprocess_scriptlang_rhai_input(expr, "lint", RhaiInputMode::CodeBlock)
        .unwrap_or_else(|_| expr.to_string());

    for caps in call_name_regex().captures_iter(&rewritten) {
        if let Some(name) = caps.get(1) {
            refs.calls.insert(name.as_str().to_string());
        }
    }
    for caps in call_symbol_target_regex().captures_iter(&rewritten) {
        if let Some(name) = caps.get(1) {
            refs.call_symbol_targets.insert(name.as_str().to_string());
        }
    }
    for caps in identifier_regex().captures_iter(&rewritten) {
        let Some(name) = caps.get(1) else {
            continue;
        };
        if is_keyword(name.as_str()) {
            continue;
        }
        refs.identifiers.insert(name.as_str().to_string());
    }
    for function_name in extract_function_literals(expr) {
        refs.function_literals.insert(function_name);
    }
    for script_name in extract_script_literals(expr) {
        refs.script_literals.insert(script_name);
    }
    refs
}

fn extract_template_expressions(value: &str) -> Vec<String> {
    template_expr_regex()
        .captures_iter(value)
        .filter_map(|caps| caps.get(1).map(|inner| inner.as_str().trim().to_string()))
        .filter(|expr| !expr.is_empty())
        .collect()
}

fn extract_script_literals(expr: &str) -> Vec<String> {
    script_literal_regex()
        .captures_iter(expr)
        .filter_map(|caps| caps.get(1).map(|inner| inner.as_str().to_string()))
        .collect()
}

fn extract_function_literals(expr: &str) -> Vec<String> {
    function_literal_regex()
        .captures_iter(expr)
        .filter_map(|caps| caps.get(1).map(|inner| inner.as_str().to_string()))
        .collect()
}

fn node_span(node: &ScriptNode) -> &SourceSpan {
    match node {
        ScriptNode::Text { location, .. }
        | ScriptNode::Debug { location, .. }
        | ScriptNode::Code { location, .. }
        | ScriptNode::Var { location, .. }
        | ScriptNode::If { location, .. }
        | ScriptNode::While { location, .. }
        | ScriptNode::Choice { location, .. }
        | ScriptNode::Input { location, .. }
        | ScriptNode::Break { location, .. }
        | ScriptNode::Continue { location, .. }
        | ScriptNode::Call { location, .. }
        | ScriptNode::Goto { location, .. }
        | ScriptNode::End { location, .. }
        | ScriptNode::Return { location, .. } => location,
    }
}

fn template_expr_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\$\{([^}]*)\}").expect("template regex should compile"))
}

fn call_name_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?)\s*\(")
            .expect("call regex should compile")
    })
}

fn call_symbol_target_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\bcall\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)\b")
            .expect("call symbol target regex should compile")
    })
}

fn identifier_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?)\b")
            .expect("identifier regex should compile")
    })
}

fn script_literal_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"@([A-Za-z_][A-Za-z0-9_-]*(?:\.[A-Za-z_][A-Za-z0-9_-]*)?)")
            .expect("script literal regex should compile")
    })
}

fn function_literal_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\*([A-Za-z_][A-Za-z0-9_-]*(?:\.[A-Za-z_][A-Za-z0-9_-]*)?)")
            .expect("function literal regex should compile")
    })
}

fn xml_comment_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").expect("xml comment regex should compile"))
}

fn collect_node_text(node: &XmlElementNode) -> String {
    node.children
        .iter()
        .filter_map(|child| match child {
            XmlNode::Text(text) => Some(text.value.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_keyword(token: &str) -> bool {
    matches!(
        token,
        "if" | "else"
            | "for"
            | "while"
            | "break"
            | "continue"
            | "return"
            | "true"
            | "false"
            | "let"
            | "const"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_expression_extracts_identifiers_and_calls() {
        let refs = analyze_expression("add(hp) + score");
        assert!(refs.calls.contains("add"));
        assert!(refs.identifiers.contains("hp"));
        assert!(refs.identifiers.contains("score"));
    }

    #[test]
    fn analyze_expression_handles_invalid_expression_without_panic() {
        let refs = analyze_expression("if (");
        assert!(refs.identifiers.is_empty());
        assert!(refs.calls.contains("if"));
    }

    #[test]
    fn analyze_expression_extracts_call_symbol_target() {
        let refs = analyze_expression("call(shared_helper, [1])");
        assert!(refs.call_symbol_targets.contains("shared_helper"));
    }

    #[test]
    fn extract_template_expressions_works() {
        let items = extract_template_expressions("x=${a} y=${b + 1}");
        assert_eq!(items, vec!["a".to_string(), "b + 1".to_string()]);
    }

    #[test]
    fn analyze_expression_extracts_script_literals() {
        let refs = analyze_expression("event_system.addListener(msg, @event.tick, *main.check)");
        assert!(refs.calls.contains("event_system.addListener"));
        assert!(refs.script_literals.contains("event.tick"));
        assert!(refs.function_literals.contains("main.check"));
    }

    #[test]
    fn collect_short_name_candidates_includes_alias_target_usage() {
        let source = r#"
<!-- alias ids.LocationId -->
<module name="event_bandit_ambush">
  <function name="can_phase_1_fn" args="ids.LocationId:location_id" return_type="boolean">
    return true;
  </function>
</module>
"#;
        let alias_directives = parse_alias_directives(source);
        let mut context = LintContext::default();
        collect_short_name_candidates_from_source(
            "events/bandit_ambush.xml",
            source,
            "event_bandit_ambush",
            &alias_directives,
            &mut context,
        );

        assert!(context
            .short_name_candidates
            .iter()
            .any(|candidate| candidate.qualified_name == "ids.LocationId"
                && candidate.short_name == "LocationId"));
    }

    #[test]
    fn mark_function_use_resolves_runtime_function_symbol() {
        let mut context = LintContext::default();
        context.functions.insert(
            "shared.helper".to_string(),
            NamedDecl {
                name: "helper".to_string(),
                file: "shared.xml".to_string(),
                span: SourceSpan::synthetic(),
            },
        );

        mark_function_use(
            "shared_helper",
            "main",
            "main.xml",
            &SourceSpan::synthetic(),
            None,
            &mut context,
        );

        assert!(context.used_functions.contains("shared.helper"));
    }

    #[test]
    fn mark_value_use_resolves_runtime_namespace_symbol() {
        let mut context = LintContext::default();
        context.module_vars.insert(
            "shared.hp".to_string(),
            NamedDecl {
                name: "hp".to_string(),
                file: "shared.xml".to_string(),
                span: SourceSpan::synthetic(),
            },
        );

        mark_value_use(
            "__sl_module_ns_shared.hp",
            "main",
            "main.xml",
            &SourceSpan::synthetic(),
            None,
            &mut context,
        );

        assert!(context.used_module_vars.contains("shared.hp"));
    }
}
