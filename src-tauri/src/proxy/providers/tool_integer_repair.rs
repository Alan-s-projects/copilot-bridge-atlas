//! Repair Grok's exact whole-number decimals only where a Codex tool schema
//! requires an integer. Raw JSON values keep large numbers exact and leave
//! every unrelated argument untouched.

use indexmap::IndexMap;
use serde_json::{value::RawValue, Value};
use std::collections::HashSet;

const MAX_SCHEMA_DEPTH: usize = 48;

#[derive(Clone, Copy)]
struct SchemaView<'a> {
    root: &'a Value,
    node: &'a Value,
}

pub(super) fn repair_integer_arguments<'a>(
    arguments: &str,
    schema: &'a Value,
    nested_tool_schema: &dyn Fn(&str) -> Option<&'a Value>,
) -> String {
    // RawValue validates the complete JSON without first converting its
    // numeric lexemes through f64 (which would round large tool IDs).
    if serde_json::from_str::<Box<RawValue>>(arguments).is_err() {
        return arguments.to_string();
    }
    rewrite_value(
        arguments,
        &[SchemaView {
            root: schema,
            node: schema,
        }],
        nested_tool_schema,
        0,
    )
    .unwrap_or_else(|| arguments.to_string())
}

fn rewrite_value<'a>(
    raw: &str,
    schemas: &[SchemaView<'a>],
    nested_tool_schema: &dyn Fn(&str) -> Option<&'a Value>,
    depth: usize,
) -> Option<String> {
    if depth >= MAX_SCHEMA_DEPTH {
        return None;
    }
    let schemas = expand_schemas(schemas, raw, depth)?;
    let raw = raw.trim();
    match raw.as_bytes().first()? {
        b'{' => {
            let members = serde_json::from_str::<IndexMap<String, Box<RawValue>>>(raw).ok()?;
            let recipient = members
                .get("recipient_name")
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok());
            let mut changed = false;
            let mut output = Vec::with_capacity(members.len());
            for (key, value) in &members {
                let mut child_schemas = Vec::new();
                for view in &schemas {
                    let properties = view.node.get("properties").and_then(Value::as_object);
                    let child = properties
                        .and_then(|properties| properties.get(key))
                        .or_else(|| {
                            view.node
                                .get("additionalProperties")
                                .filter(|value| value.is_object())
                        });
                    if let Some(node) = child {
                        child_schemas.push(SchemaView {
                            root: view.root,
                            node,
                        });
                    }
                }
                // A parallel tool's `parameters` field contains the selected
                // inner tool's arguments, whose schema was declared separately.
                if key == "parameters" {
                    if let Some(schema) = recipient
                        .as_deref()
                        .and_then(nested_tool_schema)
                        .filter(|_| !child_schemas.is_empty())
                    {
                        child_schemas = vec![SchemaView {
                            root: schema,
                            node: schema,
                        }];
                    }
                }
                let replacement = (!child_schemas.is_empty()).then(|| {
                    rewrite_value(value.get(), &child_schemas, nested_tool_schema, depth + 1)
                });
                let replacement = replacement.flatten();
                changed |= replacement.is_some();
                output.push(format!(
                    "{}:{}",
                    serde_json::to_string(key).ok()?,
                    replacement.as_deref().unwrap_or(value.get())
                ));
            }
            changed.then(|| format!("{{{}}}", output.join(",")))
        }
        b'[' => {
            let members = serde_json::from_str::<Vec<Box<RawValue>>>(raw).ok()?;
            let mut changed = false;
            let mut output = Vec::with_capacity(members.len());
            for (index, value) in members.iter().enumerate() {
                let mut child_schemas = Vec::new();
                for view in &schemas {
                    let child = view
                        .node
                        .get("prefixItems")
                        .and_then(Value::as_array)
                        .and_then(|items| items.get(index))
                        .or_else(|| view.node.get("items").filter(|item| item.is_object()));
                    if let Some(node) = child {
                        child_schemas.push(SchemaView {
                            root: view.root,
                            node,
                        });
                    }
                }
                let replacement = (!child_schemas.is_empty()).then(|| {
                    rewrite_value(value.get(), &child_schemas, nested_tool_schema, depth + 1)
                });
                let replacement = replacement.flatten();
                changed |= replacement.is_some();
                output.push(replacement.as_deref().unwrap_or(value.get()).to_string());
            }
            changed.then(|| format!("[{}]", output.join(",")))
        }
        b'-' | b'0'..=b'9' if schemas_require_integer(&schemas) => {
            let integer = exact_whole_number(raw)?;
            within_declared_bounds(&integer, &schemas).then_some(integer)
        }
        _ => None,
    }
}

fn expand_schemas<'a>(
    schemas: &[SchemaView<'a>],
    raw: &str,
    depth: usize,
) -> Option<Vec<SchemaView<'a>>> {
    let mut expanded = Vec::new();
    for schema in schemas {
        expand_schema(*schema, raw, depth, &mut HashSet::new(), &mut expanded)?;
    }
    Some(expanded)
}

fn expand_schema<'a>(
    schema: SchemaView<'a>,
    raw: &str,
    depth: usize,
    refs: &mut HashSet<String>,
    expanded: &mut Vec<SchemaView<'a>>,
) -> Option<()> {
    if depth >= MAX_SCHEMA_DEPTH || schema.node == &Value::Bool(false) {
        return None;
    }
    if let Some(reference) = schema.node.get("$ref").and_then(Value::as_str) {
        let pointer = reference.strip_prefix('#')?;
        if !pointer.starts_with('/') || !refs.insert(reference.to_string()) {
            return None;
        }
        let target = schema.root.pointer(pointer)?;
        let result = expand_schema(
            SchemaView {
                root: schema.root,
                node: target,
            },
            raw,
            depth + 1,
            refs,
            expanded,
        );
        refs.remove(reference);
        return result;
    }

    expanded.push(schema);
    for union in ["oneOf", "anyOf"] {
        if let Some(branches) = schema.node.get(union).and_then(Value::as_array) {
            let matching: Vec<_> = branches
                .iter()
                .filter(|branch| schema_matches_raw(branch, schema.root, raw, depth + 1))
                .collect();
            if matching.len() != 1 {
                return None;
            }
            expand_schema(
                SchemaView {
                    root: schema.root,
                    node: matching[0],
                },
                raw,
                depth + 1,
                refs,
                expanded,
            )?;
        }
    }
    if let Some(branches) = schema.node.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            expand_schema(
                SchemaView {
                    root: schema.root,
                    node: branch,
                },
                raw,
                depth + 1,
                refs,
                expanded,
            )?;
        }
    }
    Some(())
}

fn schema_matches_raw(schema: &Value, root: &Value, raw: &str, depth: usize) -> bool {
    if depth >= MAX_SCHEMA_DEPTH {
        return false;
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return reference
            .strip_prefix("#/")
            .and_then(|pointer| root.pointer(&format!("/{pointer}")))
            .is_some_and(|target| schema_matches_raw(target, root, raw, depth + 1));
    }
    let raw = raw.trim();
    let kind = match raw.as_bytes().first() {
        Some(b'{') => "object",
        Some(b'[') => "array",
        Some(b'"') => "string",
        Some(b't' | b'f') => "boolean",
        Some(b'n') => "null",
        Some(b'-' | b'0'..=b'9') => "number",
        _ => return false,
    };
    if let Some(declared) = schema.get("type") {
        let matches_type = |value: &Value| {
            value.as_str().is_some_and(|name| {
                name == kind || (kind == "number" && name == "integer" && is_whole_number(raw))
            })
        };
        let matches = match declared {
            Value::String(_) => matches_type(declared),
            Value::Array(types) => types.iter().any(matches_type),
            _ => false,
        };
        if !matches {
            return false;
        }
    }
    if kind == "object" {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            let Ok(fields) = serde_json::from_str::<IndexMap<String, Box<RawValue>>>(raw) else {
                return false;
            };
            if required
                .iter()
                .filter_map(Value::as_str)
                .any(|key| !fields.contains_key(key))
            {
                return false;
            }
        }
    }
    true
}

fn schemas_require_integer(schemas: &[SchemaView<'_>]) -> bool {
    let mut requires_integer = false;
    for schema in schemas {
        let Some(declared) = schema.node.get("type") else {
            continue;
        };
        let types = match declared {
            Value::String(name) => vec![name.as_str()],
            Value::Array(types) => types.iter().filter_map(Value::as_str).collect(),
            _ => return false,
        };
        if types.contains(&"integer") {
            if types
                .iter()
                .any(|kind| *kind != "integer" && *kind != "null")
            {
                return false;
            }
            requires_integer = true;
        } else if types
            .iter()
            .any(|kind| *kind != "number" && *kind != "null")
        {
            return false;
        }
    }
    requires_integer
}

fn is_whole_number(raw: &str) -> bool {
    (!raw.contains(['.', 'e', 'E']) && (raw.parse::<i64>().is_ok() || raw.parse::<u64>().is_ok()))
        || exact_whole_number(raw).is_some()
}

fn exact_whole_number(raw: &str) -> Option<String> {
    if !raw.contains(['.', 'e', 'E']) {
        return None;
    }
    let negative = raw.starts_with('-');
    let unsigned = raw.strip_prefix('-').unwrap_or(raw);
    let (mantissa, exponent) = unsigned
        .split_once(['e', 'E'])
        .map_or((unsigned, 0), |(mantissa, exponent)| {
            (mantissa, exponent.parse::<i32>().unwrap_or(i32::MAX))
        });
    if exponent == i32::MAX {
        return None;
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let shift = exponent.checked_sub(i32::try_from(fraction.len()).ok()?)?;
    if digits.bytes().all(|digit| digit == b'0') {
        return Some("0".to_string());
    }
    if shift >= 0 {
        let zeros = usize::try_from(shift).ok()?;
        if digits.len().checked_add(zeros)? > 20 {
            return None;
        }
        digits.extend(std::iter::repeat_n('0', zeros));
    } else {
        let removed = usize::try_from(shift.checked_neg()?).ok()?;
        if removed > digits.len()
            || !digits.as_bytes()[digits.len() - removed..]
                .iter()
                .all(|digit| *digit == b'0')
        {
            return None;
        }
        digits.truncate(digits.len() - removed);
    }
    let digits = digits.trim_start_matches('0');
    let integer = if negative {
        format!("-{digits}")
    } else {
        digits.to_string()
    };
    if (negative && integer.parse::<i64>().is_ok()) || (!negative && integer.parse::<u64>().is_ok())
    {
        Some(integer)
    } else {
        None
    }
}

fn within_declared_bounds(integer: &str, schemas: &[SchemaView<'_>]) -> bool {
    let Ok(number) = integer.parse::<i128>() else {
        return false;
    };
    for schema in schemas {
        for field in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
            if let Some(bound) = schema.node.get(field) {
                let Ok(bound) = bound.to_string().parse::<i128>() else {
                    return false;
                };
                let valid = match field {
                    "minimum" => number >= bound,
                    "maximum" => number <= bound,
                    "exclusiveMinimum" => number > bound,
                    "exclusiveMaximum" => number < bound,
                    _ => unreachable!(),
                };
                if !valid {
                    return false;
                }
            }
        }
        match schema.node.get("format").and_then(Value::as_str) {
            Some("int32") if number < i32::MIN as i128 || number > i32::MAX as i128 => {
                return false
            }
            Some("uint32") if number < 0 || number > u32::MAX as i128 => return false,
            Some("int64") if number < i64::MIN as i128 || number > i64::MAX as i128 => {
                return false
            }
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repair(input: &str, schema: &Value) -> String {
        repair_integer_arguments(input, schema, &|_| None)
    }

    #[test]
    fn repairs_only_exact_integer_fields_without_rounding_other_values() {
        let schema = json!({
            "type":"object",
            "properties":{
                "session_id":{"type":"integer", "format":"int32"},
                "yield_time_ms":{"type":"integer"},
                "fraction":{"type":"integer"},
                "text":{"type":"string"},
                "number":{"type":"number"},
                "large":{"type":"integer"},
                "too_large":{"type":"integer"}
            }
        });
        let input = r#"{"session_id":4876.0,"yield_time_ms":120000.0,"fraction":1.5,"text":"4876.0","number":2.0,"large":9007199254740993.0,"too_large":18446744073709551616.0}"#;
        let output = repair(input, &schema);
        assert_eq!(
            output,
            input
                .replace("4876.0,", "4876,")
                .replace("120000.0,", "120000,")
                .replace("9007199254740993.0,", "9007199254740993,")
        );
        assert_eq!(
            repair(r#"{"session_id":2147483648.0}"#, &schema),
            r#"{"session_id":2147483648.0}"#
        );
    }

    #[test]
    fn follows_local_references_nested_arrays_and_nullable_integers() {
        let schema = json!({
            "type":"object", "$defs":{"id":{"type":["integer","null"]}},
            "properties":{"calls":{"type":"array","items":{"type":"object",
                "properties":{"id":{"$ref":"#/$defs/id"}}}}}
        });
        assert_eq!(
            repair(
                r#"{"calls":[{"id":4876.0},{"id":null},{"id":2.5}]}"#,
                &schema
            ),
            r#"{"calls":[{"id":4876},{"id":null},{"id":2.5}]}"#
        );
        assert_eq!(
            repair(r#"{"calls":[{"id":1.0}],"extra":{"id":2.0}}"#, &schema),
            r#"{"calls":[{"id":1}],"extra":{"id":2.0}}"#
        );
    }

    #[test]
    fn leaves_ambiguous_unions_invalid_json_and_unknown_fields_unchanged() {
        let schema = json!({
            "type":"object",
            "properties":{
                "ambiguous":{"anyOf":[{"type":"integer"},{"type":"number"}]},
                "nullable":{"oneOf":[{"type":"integer"},{"type":"null"}]},
                "missing":{"$ref":"#/$defs/unknown"}
            }
        });
        assert_eq!(
            repair(
                r#"{"ambiguous":1.0,"nullable":2.0,"missing":3.0,"other":4.0}"#,
                &schema
            ),
            r#"{"ambiguous":1.0,"nullable":2,"missing":3.0,"other":4.0}"#
        );
        assert_eq!(repair(r#"{"nullable":1.0"#, &schema), r#"{"nullable":1.0"#);
    }

    #[test]
    fn preserves_fractional_values_and_out_of_range_numbers() {
        let schema = json!({
            "type":"object",
            "properties":{
                "signed":{"type":"integer","format":"int32"},
                "unsigned":{"type":"integer","format":"uint32"},
                "bounded":{"type":"integer","minimum":1,"maximum":10},
                "large":{"type":"integer"},
                "number":{"type":"number"}
            }
        });
        let input = r#"{"signed":-3.0,"unsigned":4294967296.0,"bounded":11.0,"large":18446744073709551616.0,"number":2.0}"#;
        assert_eq!(repair(input, &schema), input.replacen("-3.0", "-3", 1));
        assert_eq!(
            repair(
                r#"{"large":18446744073709551615.0,"bounded":1e1,"signed":-0.0}"#,
                &schema
            ),
            r#"{"large":18446744073709551615,"bounded":10,"signed":0}"#
        );
        assert_eq!(
            repair(r#"{"signed":3.5,"bounded":1.25}"#, &schema),
            r#"{"signed":3.5,"bounded":1.25}"#
        );
    }
}
