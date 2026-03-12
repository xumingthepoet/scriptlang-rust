use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use regex::Regex;
use rhai::Engine;
use sl_compiler::CompileProjectBundleResult;
use sl_core::{ChoiceEntry, ScriptIr, ScriptNode, ScriptTarget, SourceSpan};
use sl_parser::{parse_import_directives, parse_xml_document, ImportDirective, XmlNode};

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

#[derive(Debug, Clone, Default)]
pub(crate) struct LintContext {
    pub(crate) modules: BTreeMap<String, ModuleDecl>,
    pub(crate) scripts: BTreeMap<String, NamedDecl>,
    pub(crate) functions: BTreeMap<String, NamedDecl>,
    pub(crate) module_vars: BTreeMap<String, NamedDecl>,
    pub(crate) module_consts: BTreeMap<String, NamedDecl>,
    pub(crate) script_locals: BTreeMap<String, ScriptLocals>,
    pub(crate) script_edges: HashMap<String, HashSet<String>>,
    pub(crate) reachable_scripts: HashSet<String>,
    pub(crate) used_functions: HashSet<String>,
    pub(crate) used_module_vars: HashSet<String>,
    pub(crate) used_module_consts: HashSet<String>,
    pub(crate) used_import_modules_by_file: HashMap<String, HashSet<String>>,
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
    collect_script_usage(bundle, &mut context);
    collect_initializer_usage(bundle, &mut context);
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
                    context.functions.insert(qualified_name, decl);
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
                            script_name,
                            context,
                            &mut locals,
                        );
                        for arg in args {
                            collect_expression_usage(
                                &arg.value_expr,
                                &module_name,
                                &file,
                                location,
                                script,
                                context,
                                &mut locals,
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
                            script_name,
                            context,
                            &mut locals,
                        );
                        for arg in args {
                            collect_expression_usage(
                                &arg.value_expr,
                                &module_name,
                                &file,
                                location,
                                script,
                                context,
                                &mut locals,
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
                            script,
                            context,
                            &mut locals,
                        );
                    }
                    ScriptNode::Code { code, location, .. } => {
                        collect_expression_usage(
                            code,
                            &module_name,
                            &file,
                            location,
                            script,
                            context,
                            &mut locals,
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
                                script,
                                context,
                                &mut locals,
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
                                script,
                                context,
                                &mut locals,
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
                                script,
                                context,
                                &mut locals,
                            );
                        }
                        for entry in entries {
                            collect_choice_entry_usage(
                                entry,
                                &module_name,
                                &file,
                                script,
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
                                script,
                                context,
                                &mut locals,
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
    script: &ScriptIr,
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
                    locals,
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
                    locals,
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
                locals,
            );
            if let Some(expr) = &block.template.when_expr {
                collect_expression_usage(
                    expr,
                    module_name,
                    file,
                    &block.template.location,
                    script,
                    context,
                    locals,
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
                    locals,
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
    for ident in refs.identifiers {
        mark_value_use(&ident, module_name, file, span, None, context);
    }
    for literal in extract_invoke_literals(expr) {
        mark_function_use(&literal, module_name, file, span, None, context);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_script_target_usage(
    target_script: &ScriptTarget,
    module_name: &str,
    file: &str,
    location: &SourceSpan,
    from_script: &str,
    context: &mut LintContext,
    locals: &mut ScriptLocals,
) {
    match target_script {
        ScriptTarget::Literal { script_name } => {
            context
                .script_edges
                .entry(from_script.to_string())
                .or_default()
                .insert(script_name.clone());
            if let Some((target_module, short_name)) = script_name.split_once('.') {
                context
                    .used_import_modules_by_file
                    .entry(file.to_string())
                    .or_default()
                    .insert(target_module.to_string());
                if target_module == module_name {
                    context.short_name_candidates.push(ShortNameCandidate {
                        file: file.to_string(),
                        span: location.clone(),
                        qualified_name: script_name.clone(),
                        short_name: short_name.to_string(),
                    });
                }
            }
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
    script: &ScriptIr,
    context: &mut LintContext,
    locals: &mut ScriptLocals,
) {
    let refs = analyze_expression(expr);
    for call in refs.calls {
        mark_function_use(&call, module_name, file, span, Some(script), context);
    }
    for ident in refs.identifiers {
        if locals.params.iter().any(|item| item.name == ident)
            || locals.vars.iter().any(|item| item.name == ident)
        {
            locals.used_locals.insert(ident.clone());
        }
        mark_value_use(&ident, module_name, file, span, Some(script), context);
    }
    for literal in extract_invoke_literals(expr) {
        mark_function_use(&literal, module_name, file, span, Some(script), context);
    }
}

fn mark_function_use(
    name: &str,
    module_name: &str,
    file: &str,
    span: &SourceSpan,
    script: Option<&ScriptIr>,
    context: &mut LintContext,
) {
    if context.functions.contains_key(name) {
        context.used_functions.insert(name.to_string());
        track_module_and_short(name, module_name, file, span, context);
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
                track_module_and_short(&function.name, module_name, file, span, context);
            }
        }
    }
}

fn mark_value_use(
    name: &str,
    module_name: &str,
    file: &str,
    span: &SourceSpan,
    script: Option<&ScriptIr>,
    context: &mut LintContext,
) {
    if context.module_vars.contains_key(name) {
        context.used_module_vars.insert(name.to_string());
        track_module_and_short(name, module_name, file, span, context);
        return;
    }
    if context.module_consts.contains_key(name) {
        context.used_module_consts.insert(name.to_string());
        track_module_and_short(name, module_name, file, span, context);
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
            track_module_and_short(&decl.qualified_name, module_name, file, span, context);
        }
        if let Some(decl) = script.visible_module_consts.get(name) {
            context
                .used_module_consts
                .insert(decl.qualified_name.clone());
            track_module_and_short(&decl.qualified_name, module_name, file, span, context);
        }
    }
}

fn track_module_and_short(
    qualified: &str,
    module_name: &str,
    file: &str,
    span: &SourceSpan,
    context: &mut LintContext,
) {
    if let Some((ns, short)) = qualified.split_once('.') {
        context
            .used_import_modules_by_file
            .entry(file.to_string())
            .or_default()
            .insert(ns.to_string());
        if ns == module_name {
            context.short_name_candidates.push(ShortNameCandidate {
                file: file.to_string(),
                span: span.clone(),
                qualified_name: qualified.to_string(),
                short_name: short.to_string(),
            });
        }
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
    identifiers: BTreeSet<String>,
}

fn analyze_expression(expr: &str) -> ExpressionRefs {
    let mut refs = ExpressionRefs::default();
    let engine = Engine::new();
    if engine.compile_expression(expr).is_err() {
        return refs;
    }

    for caps in call_name_regex().captures_iter(expr) {
        if let Some(name) = caps.get(1) {
            refs.calls.insert(name.as_str().to_string());
        }
    }
    for caps in identifier_regex().captures_iter(expr) {
        let Some(name) = caps.get(1) else {
            continue;
        };
        if is_keyword(name.as_str()) {
            continue;
        }
        refs.identifiers.insert(name.as_str().to_string());
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

fn extract_invoke_literals(expr: &str) -> Vec<String> {
    invoke_literal_regex()
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

fn identifier_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?)\b")
            .expect("identifier regex should compile")
    })
}

fn invoke_literal_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"invoke\s*\(\s*["']([A-Za-z_][A-Za-z0-9_-]*\.[A-Za-z_][A-Za-z0-9_-]*)["']\s*[,)]"#,
        )
        .expect("invoke literal regex should compile")
    })
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
    fn analyze_expression_skips_invalid_expression() {
        let refs = analyze_expression("if (");
        assert!(refs.calls.is_empty());
        assert!(refs.identifiers.is_empty());
    }

    #[test]
    fn extract_template_expressions_works() {
        let items = extract_template_expressions("x=${a} y=${b + 1}");
        assert_eq!(items, vec!["a".to_string(), "b + 1".to_string()]);
    }

    #[test]
    fn extract_invoke_literals_works() {
        let items = extract_invoke_literals(r#"invoke("a.b", [])"#);
        assert_eq!(items, vec!["a.b".to_string()]);
    }
}
