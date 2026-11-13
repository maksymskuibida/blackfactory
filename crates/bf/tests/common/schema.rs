//! A small JSON Schema checker: enough for `views/contract/view.schema.json`.
//! Supports $ref (local), type, enum, const, required, properties, additionalProperties,
//! items, minItems, maxItems, minLength, minimum, oneOf, anyOf, allOf.

use serde_json::Value;

pub fn validate(schema: &Value, data: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    check(schema, schema, data, "$", &mut errs);
    errs
}

fn resolve<'a>(root: &'a Value, r: &str) -> &'a Value {
    let mut cur = root;
    for part in r.trim_start_matches("#/").split('/') {
        cur = &cur[part];
    }
    cur
}

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "integer" => {
            v.is_i64() || v.is_u64() || v.as_f64().map(|f| f.fract() == 0.0).unwrap_or(false)
        }
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn check(root: &Value, sch: &Value, v: &Value, path: &str, errs: &mut Vec<String>) {
    if let Some(r) = sch.get("$ref").and_then(|r| r.as_str()) {
        return check(root, resolve(root, r), v, path, errs);
    }
    if let Some(t) = sch.get("type") {
        let ok = match t {
            Value::String(t) => type_ok(t, v),
            Value::Array(a) => a.iter().filter_map(|x| x.as_str()).any(|t| type_ok(t, v)),
            _ => true,
        };
        if !ok {
            errs.push(format!("{path}: expected type {t}, got {v}"));
            return;
        }
    }
    if let Some(e) = sch.get("enum").and_then(|e| e.as_array()) {
        if !e.contains(v) {
            errs.push(format!("{path}: {v} is not one of {e:?}"));
        }
    }
    if let Some(c) = sch.get("const") {
        if c != v {
            errs.push(format!("{path}: must be {c}"));
        }
    }
    if let (Some(n), Some(s)) = (sch.get("minLength").and_then(|n| n.as_u64()), v.as_str()) {
        if (s.chars().count() as u64) < n {
            errs.push(format!("{path}: shorter than {n}"));
        }
    }
    if let (Some(n), Some(x)) = (sch.get("minimum").and_then(|n| n.as_f64()), v.as_f64()) {
        if x < n {
            errs.push(format!("{path}: below {n}"));
        }
    }
    if let Some(obj) = v.as_object() {
        if let Some(req) = sch.get("required").and_then(|r| r.as_array()) {
            for r in req.iter().filter_map(|x| x.as_str()) {
                if !obj.contains_key(r) {
                    errs.push(format!("{path}: missing required `{r}`"));
                }
            }
        }
        let props = sch.get("properties").and_then(|p| p.as_object());
        for (k, val) in obj {
            match props.and_then(|p| p.get(k)) {
                Some(ps) => check(root, ps, val, &format!("{path}.{k}"), errs),
                None => match sch.get("additionalProperties") {
                    Some(Value::Bool(false)) => errs.push(format!("{path}: unexpected `{k}`")),
                    Some(s @ Value::Object(_)) => check(root, s, val, &format!("{path}.{k}"), errs),
                    _ => {}
                },
            }
        }
    }
    if let Some(arr) = v.as_array() {
        if let Some(n) = sch.get("minItems").and_then(|n| n.as_u64()) {
            if (arr.len() as u64) < n {
                errs.push(format!("{path}: fewer than {n} items"));
            }
        }
        if let Some(n) = sch.get("maxItems").and_then(|n| n.as_u64()) {
            if (arr.len() as u64) > n {
                errs.push(format!("{path}: more than {n} items"));
            }
        }
        if let Some(items) = sch.get("items") {
            for (i, x) in arr.iter().enumerate() {
                check(root, items, x, &format!("{path}[{i}]"), errs);
            }
        }
    }
    if let Some(one) = sch.get("oneOf").and_then(|o| o.as_array()) {
        let n = one
            .iter()
            .filter(|s| {
                let mut e = Vec::new();
                check(root, s, v, path, &mut e);
                e.is_empty()
            })
            .count();
        if n != 1 {
            errs.push(format!(
                "{path}: matches {n} of the oneOf alternatives (type {:?})",
                v.get("type")
            ));
        }
    }
    if let Some(any) = sch.get("anyOf").and_then(|o| o.as_array()) {
        let n = any
            .iter()
            .filter(|s| {
                let mut e = Vec::new();
                check(root, s, v, path, &mut e);
                e.is_empty()
            })
            .count();
        if n == 0 {
            errs.push(format!("{path}: matches none of the anyOf alternatives"));
        }
    }
    if let Some(all) = sch.get("allOf").and_then(|o| o.as_array()) {
        for s in all {
            check(root, s, v, path, errs);
        }
    }
}
