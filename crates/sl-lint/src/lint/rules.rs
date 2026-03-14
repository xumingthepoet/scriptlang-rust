use std::collections::HashSet;

use crate::lint::collector::LintContext;
use crate::lint::diagnostic::LintDiagnostic;

pub(crate) fn run_rules(context: &LintContext) -> Vec<LintDiagnostic> {
    let mut diagnostics = Vec::new();
    collect_unused_script(context, &mut diagnostics);
    collect_unused_module(context, &mut diagnostics);
    collect_unused_function(context, &mut diagnostics);
    collect_unused_module_var(context, &mut diagnostics);
    collect_unused_module_const(context, &mut diagnostics);
    collect_unused_locals(context, &mut diagnostics);
    collect_prefer_short_name(context, &mut diagnostics);
    collect_unused_import(context, &mut diagnostics);
    collect_unreachable_node(context, &mut diagnostics);
    diagnostics
}

fn collect_unused_script(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    for (name, decl) in &context.scripts {
        if context.reachable_scripts.contains(name) || context.used_scripts.contains(name) {
            continue;
        }
        diagnostics.push(LintDiagnostic::warning(
            "unused-script",
            decl.file.clone(),
            Some(decl.span.clone()),
            format!("Script \"{}\" is not reachable from entry.", name),
            Some("Remove it or reference it from call/return flow.".to_string()),
        ));
    }
}

fn collect_unused_module(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    for (module_name, module) in &context.modules {
        let has_reachable_script = context
            .reachable_scripts
            .iter()
            .any(|script| script.starts_with(&format!("{}.", module_name)));
        let has_used_script = context
            .used_scripts
            .iter()
            .any(|script| script.starts_with(&format!("{}.", module_name)));
        let has_used_function = context
            .used_functions
            .iter()
            .any(|name| name.starts_with(&format!("{}.", module_name)));
        let has_used_var = context
            .used_module_vars
            .iter()
            .any(|name| name.starts_with(&format!("{}.", module_name)));
        let has_used_const = context
            .used_module_consts
            .iter()
            .any(|name| name.starts_with(&format!("{}.", module_name)));
        let has_import_reference = context
            .used_import_modules_by_file
            .values()
            .any(|used_modules| used_modules.contains(module_name));

        if has_reachable_script
            || has_used_script
            || has_used_function
            || has_used_var
            || has_used_const
            || has_import_reference
        {
            continue;
        }

        diagnostics.push(LintDiagnostic::warning(
            "unused-module",
            module.file.clone(),
            Some(module.span.clone()),
            format!("Module \"{}\" has no used symbols.", module.module_name),
            Some("Remove module or reference its symbols from active scripts.".to_string()),
        ));
    }
}

fn collect_unused_function(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    for (name, decl) in &context.functions {
        if context.used_functions.contains(name) {
            continue;
        }
        diagnostics.push(LintDiagnostic::warning(
            "unused-function",
            decl.file.clone(),
            Some(decl.span.clone()),
            format!("Function \"{}\" is never called.", name),
            Some("Remove it or call it from script expressions/invoke.".to_string()),
        ));
    }
}

fn collect_unused_module_var(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    for (name, decl) in &context.module_vars {
        if context.used_module_vars.contains(name) {
            continue;
        }
        diagnostics.push(LintDiagnostic::warning(
            "unused-module-var",
            decl.file.clone(),
            Some(decl.span.clone()),
            format!("Module var \"{}\" is never read.", name),
            Some("Remove it or reference it from script expressions.".to_string()),
        ));
    }
}

fn collect_unused_module_const(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    for (name, decl) in &context.module_consts {
        if context.used_module_consts.contains(name) {
            continue;
        }
        diagnostics.push(LintDiagnostic::warning(
            "unused-module-const",
            decl.file.clone(),
            Some(decl.span.clone()),
            format!("Module const \"{}\" is never read.", name),
            Some("Remove it or reference it from script expressions.".to_string()),
        ));
    }
}

fn collect_unused_locals(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    for locals in context.script_locals.values() {
        for param in &locals.params {
            if locals.used_locals.contains(&param.name) {
                continue;
            }
            diagnostics.push(LintDiagnostic::warning(
                "unused-param",
                param.file.clone(),
                Some(param.span.clone()),
                format!("Param \"{}\" is never read.", param.name),
                Some("Remove it or use it in expressions.".to_string()),
            ));
        }
        for var in &locals.vars {
            if locals.used_locals.contains(&var.name) {
                continue;
            }
            diagnostics.push(LintDiagnostic::warning(
                "unused-local-var",
                var.file.clone(),
                Some(var.span.clone()),
                format!("Local var \"{}\" is never read.", var.name),
                Some("Remove it or use it in expressions.".to_string()),
            ));
        }
    }
}

fn collect_prefer_short_name(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    let mut seen = HashSet::new();
    for candidate in &context.short_name_candidates {
        if candidate.qualified_name == candidate.short_name {
            continue;
        }
        let key = format!(
            "{}:{}:{}:{}",
            candidate.file,
            candidate.span.start.line,
            candidate.span.start.column,
            candidate.qualified_name
        );
        if !seen.insert(key) {
            continue;
        }

        diagnostics.push(LintDiagnostic::warning(
            "prefer-short-name",
            candidate.file.clone(),
            Some(candidate.span.clone()),
            format!(
                "Use short name \"{}\" instead of \"{}\" in same module.",
                candidate.short_name, candidate.qualified_name
            ),
            Some("Replace with short alias for readability.".to_string()),
        ));
    }
}

fn collect_unused_import(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    for module in context.modules.values() {
        let used = context
            .used_import_modules_by_file
            .get(&module.file)
            .cloned()
            .unwrap_or_default();
        for import in &module.imports {
            if used.contains(&import.module_name) {
                continue;
            }
            diagnostics.push(LintDiagnostic::warning(
                "unused-import",
                import.file.clone(),
                Some(import.span.clone()),
                format!(
                    "Imported module \"{}\" is not used in this file.",
                    import.module_name
                ),
                Some("Remove unused import directive.".to_string()),
            ));
        }
    }
}

fn collect_unreachable_node(context: &LintContext, diagnostics: &mut Vec<LintDiagnostic>) {
    for unreachable in &context.unreachable_nodes {
        diagnostics.push(LintDiagnostic::warning(
            "unreachable-node",
            unreachable.file.clone(),
            Some(unreachable.span.clone()),
            format!(
                "Node in script \"{}\" is unreachable due to earlier control flow terminator.",
                unreachable.script_name
            ),
            Some("Remove dead node or move it before return/break/continue.".to_string()),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::collector::{
        ImportDecl, ModuleDecl, NamedDecl, ScriptLocals, ShortNameCandidate, UnreachableNode,
    };
    use sl_core::SourceSpan;
    use std::collections::HashSet;

    fn base_context() -> LintContext {
        LintContext::default()
    }

    #[test]
    fn run_rules_emits_unused_script() {
        let mut ctx = base_context();
        ctx.scripts.insert(
            "main.unused".to_string(),
            NamedDecl {
                name: "unused".to_string(),
                file: "main.xml".to_string(),
                span: SourceSpan::synthetic(),
            },
        );
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-script"));
    }

    #[test]
    fn run_rules_emits_unused_module() {
        let mut ctx = base_context();
        ctx.modules.insert(
            "main".to_string(),
            ModuleDecl {
                module_name: "main".to_string(),
                file: "main.xml".to_string(),
                span: SourceSpan::synthetic(),
                imports: Vec::new(),
            },
        );
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-module"));
    }

    #[test]
    fn run_rules_skips_unused_module_when_import_referenced() {
        let mut ctx = base_context();
        ctx.modules.insert(
            "shared".to_string(),
            ModuleDecl {
                module_name: "shared".to_string(),
                file: "shared.xml".to_string(),
                span: SourceSpan::synthetic(),
                imports: Vec::new(),
            },
        );
        ctx.used_import_modules_by_file.insert(
            "main.xml".to_string(),
            std::collections::HashSet::from(["shared".to_string()]),
        );
        let result = run_rules(&ctx);
        assert!(!result.iter().any(|d| d.code == "unused-module"));
    }

    #[test]
    fn run_rules_emits_unused_function() {
        let mut ctx = base_context();
        ctx.functions.insert(
            "main.add".to_string(),
            NamedDecl {
                name: "add".to_string(),
                file: "main.xml".to_string(),
                span: SourceSpan::synthetic(),
            },
        );
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-function"));
    }

    #[test]
    fn run_rules_emits_unused_function_even_if_exported() {
        let mut ctx = base_context();
        ctx.functions.insert(
            "main.add".to_string(),
            NamedDecl {
                name: "add".to_string(),
                file: "main.xml".to_string(),
                span: SourceSpan::synthetic(),
            },
        );
        ctx.exported_functions.insert("main.add".to_string());
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-function"));
    }

    #[test]
    fn run_rules_emits_unused_module_var() {
        let mut ctx = base_context();
        ctx.module_vars.insert(
            "main.hp".to_string(),
            NamedDecl {
                name: "hp".to_string(),
                file: "main.xml".to_string(),
                span: SourceSpan::synthetic(),
            },
        );
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-module-var"));
    }

    #[test]
    fn run_rules_emits_unused_module_const() {
        let mut ctx = base_context();
        ctx.module_consts.insert(
            "main.BASE".to_string(),
            NamedDecl {
                name: "BASE".to_string(),
                file: "main.xml".to_string(),
                span: SourceSpan::synthetic(),
            },
        );
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-module-const"));
    }

    #[test]
    fn run_rules_emits_unused_param_and_local_var() {
        let mut ctx = base_context();
        ctx.script_locals.insert(
            "main.main".to_string(),
            ScriptLocals {
                params: vec![NamedDecl {
                    name: "p".to_string(),
                    file: "main.xml".to_string(),
                    span: SourceSpan::synthetic(),
                }],
                vars: vec![NamedDecl {
                    name: "v".to_string(),
                    file: "main.xml".to_string(),
                    span: SourceSpan::synthetic(),
                }],
                used_locals: HashSet::new(),
            },
        );
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-param"));
        assert!(result.iter().any(|d| d.code == "unused-local-var"));
    }

    #[test]
    fn run_rules_emits_unused_script_even_if_exported() {
        let mut ctx = base_context();
        ctx.scripts.insert(
            "main.unused".to_string(),
            NamedDecl {
                name: "unused".to_string(),
                file: "main.xml".to_string(),
                span: SourceSpan::synthetic(),
            },
        );
        ctx.exported_scripts.insert("main.unused".to_string());
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-script"));
    }

    #[test]
    fn run_rules_emits_prefer_short_name() {
        let mut ctx = base_context();
        ctx.short_name_candidates.push(ShortNameCandidate {
            file: "main.xml".to_string(),
            span: SourceSpan::synthetic(),
            qualified_name: "main.next".to_string(),
            short_name: "next".to_string(),
        });
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "prefer-short-name"));
    }

    #[test]
    fn run_rules_emits_unused_import() {
        let mut ctx = base_context();
        ctx.modules.insert(
            "main".to_string(),
            ModuleDecl {
                module_name: "main".to_string(),
                file: "main.xml".to_string(),
                span: SourceSpan::synthetic(),
                imports: vec![ImportDecl {
                    module_name: "shared".to_string(),
                    file: "main.xml".to_string(),
                    span: SourceSpan::synthetic(),
                }],
            },
        );
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unused-import"));
    }

    #[test]
    fn run_rules_emits_unreachable_node() {
        let mut ctx = base_context();
        ctx.unreachable_nodes.push(UnreachableNode {
            file: "main.xml".to_string(),
            script_name: "main.main".to_string(),
            span: SourceSpan::synthetic(),
        });
        let result = run_rules(&ctx);
        assert!(result.iter().any(|d| d.code == "unreachable-node"));
    }

    #[test]
    fn run_rules_handles_empty_context() {
        let ctx = base_context();
        let result = run_rules(&ctx);
        assert!(result.is_empty());
    }
}
