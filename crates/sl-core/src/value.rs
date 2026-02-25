use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::ScriptType;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SlValue {
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<SlValue>),
    Map(BTreeMap<String, SlValue>),
}

impl SlValue {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Map(_) => "map",
        }
    }
}

pub fn default_value_from_type(ty: &ScriptType) -> SlValue {
    match ty {
        ScriptType::Primitive { name } => match name.as_str() {
            "number" => SlValue::Number(0.0),
            "string" => SlValue::String(String::new()),
            "boolean" => SlValue::Bool(false),
            _ => SlValue::String(String::new()),
        },
        ScriptType::Array { .. } => SlValue::Array(Vec::new()),
        ScriptType::Map { .. } => SlValue::Map(BTreeMap::new()),
        ScriptType::Object { fields, .. } => {
            let mut map = BTreeMap::new();
            for (field_name, field_type) in fields {
                map.insert(field_name.clone(), default_value_from_type(field_type));
            }
            SlValue::Map(map)
        }
    }
}

pub fn is_type_compatible(value: &SlValue, ty: &ScriptType) -> bool {
    match ty {
        ScriptType::Primitive { name } => matches!(
            (name.as_str(), value),
            ("number", SlValue::Number(_))
                | ("string", SlValue::String(_))
                | ("boolean", SlValue::Bool(_))
        ),
        ScriptType::Array { element_type } => match value {
            SlValue::Array(values) => values
                .iter()
                .all(|entry| is_type_compatible(entry, element_type)),
            _ => false,
        },
        ScriptType::Map { value_type, .. } => match value {
            SlValue::Map(values) => values
                .values()
                .all(|entry| is_type_compatible(entry, value_type)),
            _ => false,
        },
        ScriptType::Object { fields, .. } => match value {
            SlValue::Map(values) => {
                if values.len() != fields.len() {
                    return false;
                }
                for (field_name, field_type) in fields {
                    let Some(field_value) = values.get(field_name) else {
                        return false;
                    };
                    if !is_type_compatible(field_value, field_type) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        },
    }
}
