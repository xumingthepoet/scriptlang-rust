use sl_core::SlValue;

pub(crate) fn parse_ref_path(path: &str) -> Vec<String> {
    path.split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn assign_nested_path(
    target: &mut SlValue,
    path: &[String],
    value: SlValue,
) -> Result<(), String> {
    if path.is_empty() {
        *target = value;
        return Ok(());
    }

    let SlValue::Map(entries) = target else {
        return Err("target is not an object/map".to_string());
    };

    let head = &path[0];
    if path.len() == 1 {
        entries.insert(head.clone(), value);
        return Ok(());
    }

    let next = match entries.get_mut(head) {
        Some(value) => value,
        None => return Err(format!("missing key \"{}\"", head)),
    };
    assign_nested_path(next, &path[1..], value)
}
