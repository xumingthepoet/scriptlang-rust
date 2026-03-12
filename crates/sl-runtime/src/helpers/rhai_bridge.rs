use std::collections::BTreeMap;

use rhai::{Array, Dynamic, ImmutableString, Map, FLOAT, INT};
use sl_core::{ScriptLangError, ScriptType, SlValue};

pub(crate) fn slvalue_to_text(value: &SlValue) -> String {
    match value {
        SlValue::Bool(value) => value.to_string(),
        SlValue::Number(value) => {
            if value.fract().abs() < f64::EPSILON {
                (*value as i64).to_string()
            } else {
                value.to_string()
            }
        }
        SlValue::String(value) => value.clone(),
        SlValue::Array(_) | SlValue::Map(_) => format!("{:?}", value),
    }
}

pub(crate) fn slvalue_to_dynamic(value: &SlValue) -> Dynamic {
    slvalue_to_dynamic_with_type(value, None)
}

pub(crate) fn slvalue_to_dynamic_with_type(value: &SlValue, ty: Option<&ScriptType>) -> Dynamic {
    match value {
        SlValue::Bool(value) => Dynamic::from_bool(*value),
        SlValue::Number(value) => {
            if matches!(
                ty,
                Some(ScriptType::Primitive { name }) if name == "int"
            ) && value.is_finite()
                && value.fract().abs() < f64::EPSILON
            {
                return Dynamic::from_int(*value as INT);
            }
            Dynamic::from_float(*value as FLOAT)
        }
        SlValue::String(value) => Dynamic::from(value.clone()),
        SlValue::Array(values) => {
            let mut array = Array::new();
            for value in values {
                let element_type = match ty {
                    Some(ScriptType::Array { element_type }) => Some(element_type.as_ref()),
                    _ => None,
                };
                array.push(slvalue_to_dynamic_with_type(value, element_type));
            }
            Dynamic::from_array(array)
        }
        SlValue::Map(values) => {
            let mut map = Map::new();
            for (key, value) in values {
                let value_type = match ty {
                    Some(ScriptType::Map { value_type, .. }) => Some(value_type.as_ref()),
                    Some(ScriptType::Object { fields, .. }) => fields.get(key),
                    _ => None,
                };
                map.insert(
                    key.clone().into(),
                    slvalue_to_dynamic_with_type(value, value_type),
                );
            }
            Dynamic::from_map(map)
        }
    }
}

pub(crate) fn dynamic_to_slvalue(value: Dynamic) -> Result<SlValue, ScriptLangError> {
    if value.is::<bool>() {
        return Ok(SlValue::Bool(value.cast::<bool>()));
    }
    if value.is::<INT>() {
        return Ok(SlValue::Number(value.cast::<INT>() as f64));
    }
    if value.is::<FLOAT>() {
        return Ok(SlValue::Number(value.cast::<FLOAT>()));
    }
    if value.is::<ImmutableString>() {
        return Ok(SlValue::String(value.cast::<ImmutableString>().to_string()));
    }
    if value.is::<Array>() {
        let array = value.cast::<Array>();
        let mut out = Vec::with_capacity(array.len());
        for item in array {
            out.push(dynamic_to_slvalue(item)?);
        }
        return Ok(SlValue::Array(out));
    }
    if value.is::<Map>() {
        let map = value.cast::<Map>();
        let mut out = BTreeMap::new();
        for (key, value) in map {
            out.insert(key.to_string(), dynamic_to_slvalue(value)?);
        }
        return Ok(SlValue::Map(out));
    }

    Err(ScriptLangError::new(
        "ENGINE_VALUE_UNSUPPORTED",
        "Unsupported Rhai value type.",
    ))
}

pub(crate) fn slvalue_to_rhai_literal(value: &SlValue) -> String {
    match value {
        SlValue::Bool(value) => value.to_string(),
        SlValue::Number(value) => {
            if value.fract().abs() < f64::EPSILON {
                (*value as i64).to_string()
            } else {
                value.to_string()
            }
        }
        SlValue::String(value) => format!("\"{}\"", value.replace('"', "\\\"")),
        SlValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(slvalue_to_rhai_literal)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SlValue::Map(values) => {
            let entries = values
                .iter()
                .map(|(key, value)| format!("{}: {}", key, slvalue_to_rhai_literal(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("#{{{}}}", entries)
        }
    }
}

#[cfg(test)]
mod rhai_bridge_tests {
    use super::*;

    #[test]
    fn literal_helpers_cover_decimal_and_array_paths() {
        assert_eq!(slvalue_to_text(&SlValue::Number(2.5)), "2.5");
        assert_eq!(slvalue_to_rhai_literal(&SlValue::Number(2.5)), "2.5");
        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::Array(vec![
                SlValue::Number(1.0),
                SlValue::Number(2.5),
            ])),
            "[1, 2.5]"
        );
    }

    #[test]
    fn conversion_helpers_cover_remaining_paths() {
        assert_eq!(slvalue_to_text(&SlValue::Bool(true)), "true");
        assert!(slvalue_to_text(&SlValue::Array(vec![SlValue::Number(1.0)])).contains("Array"));

        let dynamic_map = slvalue_to_dynamic(&SlValue::Map(BTreeMap::from([(
            "k".to_string(),
            SlValue::Array(vec![SlValue::Bool(false)]),
        )])));
        let roundtrip = dynamic_to_slvalue(dynamic_map).expect("roundtrip");
        assert_eq!(
            roundtrip,
            SlValue::Map(BTreeMap::from([(
                "k".to_string(),
                SlValue::Array(vec![SlValue::Bool(false)]),
            )]))
        );

        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::String("A\"B".to_string())),
            "\"A\\\"B\""
        );
        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::Map(BTreeMap::from([(
                "k".to_string(),
                SlValue::Number(1.0),
            )]))),
            "#{k: 1}"
        );
    }

    #[test]
    fn slvalue_to_dynamic_with_type_int_uses_rhai_int() {
        let int_ty = ScriptType::Primitive {
            name: "int".to_string(),
        };
        let dynamic = slvalue_to_dynamic_with_type(&SlValue::Number(2.0), Some(&int_ty));
        assert!(dynamic.is::<INT>());
        assert_eq!(dynamic.cast::<INT>(), 2);
    }

    #[test]
    fn slvalue_to_dynamic_with_object_type_uses_field_types() {
        let object_ty = ScriptType::Object {
            type_name: "Obj".to_string(),
            fields: BTreeMap::from([(
                "idx".to_string(),
                ScriptType::Primitive {
                    name: "int".to_string(),
                },
            )]),
        };
        let value = SlValue::Map(BTreeMap::from([("idx".to_string(), SlValue::Number(3.0))]));
        let dynamic = slvalue_to_dynamic_with_type(&value, Some(&object_ty));
        let map = dynamic.cast::<Map>();
        let idx = map.get("idx").expect("idx field");
        assert!(idx.is::<INT>());
        assert_eq!(idx.clone().cast::<INT>(), 3);
    }

    #[test]
    fn dynamic_to_slvalue_array_recursive_covered() {
        let arr = Array::from([Dynamic::from_array(Array::from([Dynamic::from_bool(true)]))]);
        let dynamic = Dynamic::from_array(arr);
        let result = dynamic_to_slvalue(dynamic).expect("array recursive");
        assert!(matches!(result, SlValue::Array(vec) if vec.len() == 1));

        let bad = Dynamic::from_array(Array::from([Dynamic::UNIT]));
        let error = dynamic_to_slvalue(bad).expect_err("nested unsupported array value");
        assert_eq!(error.code, "ENGINE_VALUE_UNSUPPORTED");
    }

    #[test]
    fn dynamic_to_slvalue_map_recursive_covered() {
        let mut map = Map::new();
        map.insert(
            "arr".into(),
            Dynamic::from_array(Array::from([Dynamic::from_bool(false)])),
        );
        let dynamic = Dynamic::from_map(map);
        let result = dynamic_to_slvalue(dynamic).expect("map recursive");
        assert!(matches!(result, SlValue::Map(m) if m.contains_key("arr")));

        let mut bad = Map::new();
        bad.insert("bad".into(), Dynamic::UNIT);
        let error =
            dynamic_to_slvalue(Dynamic::from_map(bad)).expect_err("nested unsupported map value");
        assert_eq!(error.code, "ENGINE_VALUE_UNSUPPORTED");
    }

    #[test]
    fn dynamic_to_slvalue_error_covered() {
        let result = dynamic_to_slvalue(Dynamic::UNIT);
        assert!(result.is_err());
    }
}
