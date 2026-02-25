impl ScriptLangEngine {
    fn is_choice_option_visible(
        &mut self,
        script_name: &str,
        option: &sl_core::ChoiceOption,
    ) -> Result<bool, ScriptLangError> {
        if let Some(when_expr) = &option.when_expr {
            if !self.eval_boolean(when_expr)? {
                return Ok(false);
            }
        }

        if !option.once {
            return Ok(true);
        }

        Ok(!self.has_once_state(script_name, &format!("option:{}", option.id)))
    }

    fn has_once_state(&self, script_name: &str, key: &str) -> bool {
        self.once_state_by_script
            .get(script_name)
            .map(|set| set.contains(key))
            .unwrap_or(false)
    }

    fn mark_once_state(&mut self, script_name: &str, key: &str) {
        self.once_state_by_script
            .entry(script_name.to_string())
            .or_default()
            .insert(key.to_string());
    }
}
#[derive(Debug, Clone)]
struct BindingOwner {
    value: SlValue,
}

