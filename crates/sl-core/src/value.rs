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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_string_and_as_number_match_expected_variant() {
        let string_value = SlValue::String("name".to_string());
        let number_value = SlValue::Number(3.5);
        let bool_value = SlValue::Bool(true);

        assert_eq!(string_value.as_string(), Some("name"));
        assert_eq!(number_value.as_number(), Some(3.5));
        assert_eq!(bool_value.as_string(), None);
        assert_eq!(bool_value.as_number(), None);
    }

    #[test]
    fn type_name_reports_all_variants() {
        assert_eq!(SlValue::Bool(true).type_name(), "boolean");
        assert_eq!(SlValue::Number(1.0).type_name(), "number");
        assert_eq!(SlValue::String("x".to_string()).type_name(), "string");
        assert_eq!(SlValue::Array(Vec::new()).type_name(), "array");
        assert_eq!(SlValue::Map(BTreeMap::new()).type_name(), "map");
    }

    #[test]
    fn default_value_from_type_builds_expected_defaults() {
        let unknown = ScriptType::Primitive {
            name: "unknown".to_string(),
        };
        assert_eq!(
            default_value_from_type(&ScriptType::Primitive {
                name: "number".to_string()
            }),
            SlValue::Number(0.0)
        );
        assert_eq!(
            default_value_from_type(&ScriptType::Primitive {
                name: "string".to_string()
            }),
            SlValue::String(String::new())
        );
        assert_eq!(
            default_value_from_type(&ScriptType::Primitive {
                name: "boolean".to_string()
            }),
            SlValue::Bool(false)
        );
        assert_eq!(
            default_value_from_type(&unknown),
            SlValue::String(String::new())
        );

        let array_type = ScriptType::Array {
            element_type: Box::new(ScriptType::Primitive {
                name: "number".to_string(),
            }),
        };
        assert_eq!(
            default_value_from_type(&array_type),
            SlValue::Array(Vec::new())
        );

        let map_type = ScriptType::Map {
            key_type: "string".to_string(),
            value_type: Box::new(ScriptType::Primitive {
                name: "number".to_string(),
            }),
        };
        assert_eq!(
            default_value_from_type(&map_type),
            SlValue::Map(BTreeMap::new())
        );

        let object_type = ScriptType::Object {
            type_name: "Hero".to_string(),
            fields: BTreeMap::from([
                (
                    "name".to_string(),
                    ScriptType::Primitive {
                        name: "string".to_string(),
                    },
                ),
                (
                    "hp".to_string(),
                    ScriptType::Primitive {
                        name: "number".to_string(),
                    },
                ),
            ]),
        };

        let object_default = default_value_from_type(&object_type);
        assert!(matches!(object_default, SlValue::Map(_)));
        let fields = match object_default {
            SlValue::Map(fields) => fields,
            _ => unreachable!("already asserted map variant"),
        };
        assert_eq!(fields.get("name"), Some(&SlValue::String(String::new())));
        assert_eq!(fields.get("hp"), Some(&SlValue::Number(0.0)));
    }

    #[test]
    fn is_type_compatible_validates_primitive_array_map_and_object() {
        let number_type = ScriptType::Primitive {
            name: "number".to_string(),
        };
        assert!(is_type_compatible(&SlValue::Number(1.0), &number_type));
        assert!(!is_type_compatible(
            &SlValue::String("1".to_string()),
            &number_type
        ));

        let array_type = ScriptType::Array {
            element_type: Box::new(ScriptType::Primitive {
                name: "number".to_string(),
            }),
        };
        assert!(is_type_compatible(
            &SlValue::Array(vec![SlValue::Number(1.0), SlValue::Number(2.0)]),
            &array_type
        ));
        assert!(!is_type_compatible(
            &SlValue::Array(vec![SlValue::Number(1.0), SlValue::String("x".to_string())]),
            &array_type
        ));
        assert!(!is_type_compatible(
            &SlValue::String("x".to_string()),
            &array_type
        ));

        let map_type = ScriptType::Map {
            key_type: "string".to_string(),
            value_type: Box::new(ScriptType::Primitive {
                name: "boolean".to_string(),
            }),
        };
        assert!(is_type_compatible(
            &SlValue::Map(BTreeMap::from([
                ("a".to_string(), SlValue::Bool(true)),
                ("b".to_string(), SlValue::Bool(false))
            ])),
            &map_type
        ));
        assert!(!is_type_compatible(
            &SlValue::Map(BTreeMap::from([("a".to_string(), SlValue::Number(1.0))])),
            &map_type
        ));
        assert!(!is_type_compatible(&SlValue::Bool(true), &map_type));

        let object_type = ScriptType::Object {
            type_name: "Obj".to_string(),
            fields: BTreeMap::from([
                (
                    "name".to_string(),
                    ScriptType::Primitive {
                        name: "string".to_string(),
                    },
                ),
                (
                    "alive".to_string(),
                    ScriptType::Primitive {
                        name: "boolean".to_string(),
                    },
                ),
            ]),
        };

        assert!(is_type_compatible(
            &SlValue::Map(BTreeMap::from([
                ("name".to_string(), SlValue::String("Rin".to_string())),
                ("alive".to_string(), SlValue::Bool(true))
            ])),
            &object_type
        ));
        assert!(!is_type_compatible(
            &SlValue::Map(BTreeMap::from([(
                "name".to_string(),
                SlValue::String("Rin".to_string())
            )])),
            &object_type
        ));
        assert!(!is_type_compatible(
            &SlValue::Map(BTreeMap::from([
                ("title".to_string(), SlValue::String("Rin".to_string())),
                ("alive".to_string(), SlValue::Bool(true))
            ])),
            &object_type
        ));
        assert!(!is_type_compatible(
            &SlValue::Map(BTreeMap::from([
                ("name".to_string(), SlValue::String("Rin".to_string())),
                ("alive".to_string(), SlValue::String("yes".to_string()))
            ])),
            &object_type
        ));
        assert!(!is_type_compatible(
            &SlValue::Map(BTreeMap::from([
                ("name".to_string(), SlValue::String("Rin".to_string())),
                ("alive".to_string(), SlValue::Bool(true)),
                ("extra".to_string(), SlValue::Bool(false))
            ])),
            &object_type
        ));
        assert!(!is_type_compatible(&SlValue::Bool(true), &object_type));
    }
}
