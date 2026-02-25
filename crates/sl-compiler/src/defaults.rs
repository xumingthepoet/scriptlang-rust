fn slvalue_from_json(value: JsonValue) -> SlValue {
    match value {
        JsonValue::Null => SlValue::String("null".to_string()),
        JsonValue::Bool(value) => SlValue::Bool(value),
        JsonValue::Number(value) => SlValue::Number(value.as_f64().unwrap_or(0.0)),
        JsonValue::String(value) => SlValue::String(value),
        JsonValue::Array(values) => {
            SlValue::Array(values.into_iter().map(slvalue_from_json).collect())
        }
        JsonValue::Object(values) => SlValue::Map(
            values
                .into_iter()
                .map(|(key, value)| (key, slvalue_from_json(value)))
                .collect(),
        ),
    }
}

pub fn default_values_from_script_params(params: &[ScriptParam]) -> BTreeMap<String, SlValue> {
    let mut defaults = BTreeMap::new();
    for param in params {
        defaults.insert(param.name.clone(), default_value_from_type(&param.r#type));
    }
    defaults
}

#[cfg(test)]
mod defaults_tests {
    use super::*;

    #[test]
    fn default_values_from_script_params_respects_declared_types() {
        let params = vec![
            ScriptParam {
                name: "hp".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                is_ref: false,
                location: SourceSpan::synthetic(),
            },
            ScriptParam {
                name: "name".to_string(),
                r#type: ScriptType::Primitive {
                    name: "string".to_string(),
                },
                is_ref: false,
                location: SourceSpan::synthetic(),
            },
        ];
        let defaults = default_values_from_script_params(&params);
        assert_eq!(defaults.get("hp"), Some(&SlValue::Number(0.0)));
        assert_eq!(defaults.get("name"), Some(&SlValue::String(String::new())));
    }
    
}
