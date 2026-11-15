//! Additive type overrides (§3.3). Every op only widens a schema, so an
//! item valid before stays valid; nothing may add, remove or change an
//! `x-halp-*` key or `required`.

use halp_domain::{TaskType, limits};
use serde_json::{Map, Value};

use crate::error::{PackError, Result};
use crate::manifest::TypeOverride;

const MAX_ADDED: usize = 32;

/// Whether any object key at any depth starts with `x-halp-`.
fn has_halp_key(v: &Value) -> Option<String> {
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(o) => {
                if let Some(k) = o.keys().find(|k| k.starts_with("x-halp-")) {
                    return Some(k.clone());
                }
                stack.extend(o.values());
            }
            Value::Array(a) => stack.extend(a.iter()),
            _ => {}
        }
    }
    None
}

/// The checks that need no parent type: names, sizes, text, and no
/// `x-halp-*` key anywhere in what is added.
pub fn check_override_shape(o: &TypeOverride) -> Result<()> {
    let t = &o.type_id;
    limits::simple_id("override type", t)?;
    if o.add_properties.len() > MAX_ADDED
        || o.add_enum.len() > MAX_ADDED
        || o.property_descriptions.len() > MAX_ADDED
    {
        return Err(PackError::invalid(format!(
            "override of {t}: too many changes"
        )));
    }
    let nothing = o.add_properties.is_empty()
        && o.add_enum.is_empty()
        && o.title.is_none()
        && o.icon.is_none()
        && o.display.is_none()
        && o.property_descriptions.is_empty();
    if nothing {
        return Err(PackError::invalid(format!(
            "override of {t} changes nothing"
        )));
    }
    for (name, schema) in &o.add_properties {
        limits::simple_id("added property", name)?;
        if name.starts_with("x-halp-") {
            return Err(PackError::invalid(format!(
                "override of {t}: {name} is reserved"
            )));
        }
        if !(schema.is_object() || schema.is_boolean()) {
            return Err(PackError::invalid(format!(
                "override of {t}: property {name} must be a schema"
            )));
        }
        if let Some(k) = has_halp_key(schema) {
            return Err(PackError::invalid(format!(
                "override of {t}: {k} may not be added by an override"
            )));
        }
    }
    for (name, vals) in &o.add_enum {
        limits::simple_id("enum property", name)?;
        if vals.is_empty() || vals.len() > 64 {
            return Err(PackError::invalid(format!(
                "override of {t}: add_enum {name} needs 1 to 64 values"
            )));
        }
    }
    for (name, d) in &o.property_descriptions {
        limits::simple_id("described property", name)?;
        limits::label("property description", d, 500)?;
    }
    let all = serde_json::to_value(o).map_err(|e| PackError::invalid(e.to_string()))?;
    limits::check_json_text(&format!("override of {t}"), &all)?;
    Ok(())
}

/// Every `x-halp-*` key in a schema with its value, by JSON pointer.
fn halp_keys(v: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut stack: Vec<(String, &Value)> = vec![(String::new(), v)];
    while let Some((p, v)) = stack.pop() {
        match v {
            Value::Object(o) => {
                for (k, c) in o {
                    let path = format!("{p}/{k}");
                    if k.starts_with("x-halp-") {
                        out.push((path.clone(), c.clone()));
                    }
                    stack.push((path, c));
                }
            }
            Value::Array(a) => {
                for (i, c) in a.iter().enumerate() {
                    stack.push((format!("{p}/{i}"), c));
                }
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Applies an additive override to a type, returning the new definition.
pub fn apply_override(base: &TaskType, o: &TypeOverride) -> Result<TaskType> {
    check_override_shape(o)?;
    let id = &base.id;
    if o.type_id != *id {
        return Err(PackError::invalid(format!(
            "override of {} applied to {id}",
            o.type_id
        )));
    }
    let mut t = base.clone();
    let schema = t
        .schema
        .as_object_mut()
        .ok_or_else(|| PackError::invalid(format!("type {id}: schema is not an object")))?;
    if !o.add_properties.is_empty() {
        // Only when no valid item can already hold the new name.
        if schema.get("additionalProperties") != Some(&Value::Bool(false))
            || schema.contains_key("patternProperties")
        {
            return Err(PackError::invalid(format!(
                "override of {id}: properties can only be added to a schema with \
                 \"additionalProperties\": false and no patternProperties"
            )));
        }
    }
    let props = match schema
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()))
    {
        Value::Object(p) => p,
        _ => {
            return Err(PackError::invalid(format!(
                "type {id}: properties is not an object"
            )));
        }
    };
    for (name, s) in &o.add_properties {
        if props.contains_key(name) {
            return Err(PackError::invalid(format!(
                "override of {id}: property {name} already exists (overrides only add)"
            )));
        }
        props.insert(name.clone(), s.clone());
    }
    for (name, vals) in &o.add_enum {
        let e = props
            .get_mut(name)
            .and_then(Value::as_object_mut)
            .and_then(|p| p.get_mut("enum"))
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                PackError::invalid(format!(
                    "override of {id}: property {name} has no enum to add to"
                ))
            })?;
        for v in vals {
            if !e.contains(v) {
                e.push(v.clone());
            }
        }
    }
    for (name, d) in &o.property_descriptions {
        let p = props
            .get_mut(name)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                PackError::invalid(format!("override of {id}: no property {name} to describe"))
            })?;
        p.insert("description".into(), Value::String(d.clone()));
    }
    if let Some(v) = &o.title {
        t.title = v.clone();
    }
    if let Some(v) = &o.icon {
        t.icon = v.clone();
    }
    if let Some(v) = &o.display {
        t.display = v.clone();
    }
    // Defense in depth: what must not change did not.
    if halp_keys(&t.schema) != halp_keys(&base.schema) {
        return Err(PackError::invalid(format!(
            "override of {id} would change an x-halp-* key"
        )));
    }
    if t.schema.get("required") != base.schema.get("required") {
        return Err(PackError::invalid(format!(
            "override of {id} would change required"
        )));
    }
    t.validate()?;
    limits::check_json_text(&format!("type {id} schema"), &t.schema)?;
    Ok(t)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use halp_domain::{TaskTypeDisplay, TaskTypeOrigin, TaskTypeRegistry};
    use serde_json::json;

    use super::*;

    fn pr_type() -> TaskType {
        TaskType {
            id: "dev.pull_request".into(),
            title: "Pull request".into(),
            icon: "git-pull-request".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "repo": {"type": "string"},
                    "state": {"enum": ["open", "merged"]}
                },
                "required": ["repo"],
                "additionalProperties": false,
                "x-halp-immutable-when": {"state": ["merged"]}
            }),
            display: TaskTypeDisplay::default(),
        }
    }

    fn ov(v: Value) -> Result<TypeOverride> {
        serde_json::from_value(v).map_err(|e| PackError::invalid(e.to_string()))
    }

    #[test]
    fn additive_override_applies() {
        let o = ov(json!({
            "type": "dev.pull_request",
            "add_properties": {"team": {"type": "string", "maxLength": 50}},
            "add_enum": {"state": ["closed", "open"]},
            "title": "PR",
            "icon": "git",
            "display": {"primary_fields": ["repo", "team"]},
            "property_descriptions": {"repo": "owner/name"}
        }))
        .unwrap();
        let t = apply_override(&pr_type(), &o).unwrap();
        assert_eq!(t.title, "PR");
        assert_eq!(
            t.schema["properties"]["state"]["enum"],
            json!(["open", "merged", "closed"])
        );
        assert_eq!(t.schema["properties"]["repo"]["description"], "owner/name");
        assert_eq!(t.schema["required"], json!(["repo"]));
        assert_eq!(
            t.schema["x-halp-immutable-when"],
            json!({"state": ["merged"]})
        );
    }

    #[test]
    fn halp_keys_and_required_are_untouchable() {
        let base = pr_type();
        let bad = [
            // An unknown op such as `required` or `remove_properties` fails to parse.
            json!({"type": "dev.pull_request", "required": ["team"]}),
            json!({"type": "dev.pull_request", "add_required": ["team"]}),
            json!({"type": "dev.pull_request", "remove_properties": ["repo"]}),
            json!({"type": "dev.pull_request", "schema": {}}),
        ];
        for b in bad {
            assert!(ov(b.clone()).is_err(), "{b}");
        }
        let rejected = [
            json!({"type": "dev.pull_request", "add_properties": {"team": {"type": "string", "x-halp-data-class": "secrets"}}}),
            json!({"type": "dev.pull_request", "add_properties": {"team": {"type": "object", "properties": {"a": {"x-halp-ext-writable": ["a"]}}}}}),
            json!({"type": "dev.pull_request", "add_properties": {"x-halp-immutable-when": {}}}),
            // Existing property: would be a change, not an addition.
            json!({"type": "dev.pull_request", "add_properties": {"repo": {"type": "integer"}}}),
            // No enum on that property.
            json!({"type": "dev.pull_request", "add_enum": {"repo": ["x"]}}),
            json!({"type": "dev.pull_request", "add_enum": {"nope": ["x"]}}),
            json!({"type": "dev.pull_request", "property_descriptions": {"nope": "x"}}),
            json!({"type": "dev.pull_request"}),
            json!({"type": "dev.pull_request", "title": "a\u{202E}b"}),
        ];
        for r in rejected {
            let o = ov(r.clone()).unwrap();
            assert!(apply_override(&base, &o).is_err(), "{r}");
        }
    }

    #[test]
    fn adding_properties_needs_closed_schema() {
        let mut open = pr_type();
        open.schema
            .as_object_mut()
            .unwrap()
            .remove("additionalProperties");
        let o =
            ov(json!({"type": "dev.pull_request", "add_properties": {"team": {"type": "string"}}}))
                .unwrap();
        assert!(apply_override(&open, &o).is_err());
        let mut patterned = pr_type();
        patterned.schema["patternProperties"] = json!({"^t": {"type": "integer"}});
        assert!(apply_override(&patterned, &o).is_err());
        assert!(apply_override(&pr_type(), &o).is_ok());
    }

    /// A small deterministic PRNG (xorshift64*); no new dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn chance(&mut self, pct: u64) -> bool {
            self.next() % 100 < pct
        }
        fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
            &xs[self.below(xs.len())]
        }
    }

    const NAMES: &[&str] = &["a", "b", "c", "d", "e", "zz"];
    const WORDS: &[&str] = &["x", "y", "open", "done", "zz", ""];

    fn random_value(r: &mut Rng) -> Value {
        match r.below(6) {
            0 => json!(r.pick(WORDS)),
            1 => json!(r.below(20) as i64 - 5),
            2 => json!(r.chance(50)),
            3 => Value::Null,
            4 => json!([r.pick(WORDS)]),
            _ => json!({"k": r.pick(WORDS)}),
        }
    }

    fn random_prop(r: &mut Rng) -> Value {
        match r.below(5) {
            0 => {
                let n = 1 + r.below(3);
                let vals: Vec<Value> = (0..n).map(|_| json!(r.pick(WORDS))).collect();
                json!({ "enum": vals })
            }
            1 => json!({"type": "string", "maxLength": 1 + r.below(4)}),
            2 => json!({"type": "integer", "minimum": 0, "maximum": r.below(10)}),
            3 => json!({"type": "boolean"}),
            _ => json!({"enum": [r.below(3), "x"], "type": ["string", "integer"]}),
        }
    }

    fn random_type(r: &mut Rng) -> TaskType {
        let mut props = Map::new();
        let mut required = Vec::new();
        for n in &NAMES[..5] {
            if r.chance(70) {
                props.insert((*n).into(), random_prop(r));
                if r.chance(25) {
                    required.push(json!(n));
                }
            }
        }
        let mut schema = json!({"type": "object", "properties": props});
        if !required.is_empty() {
            schema["required"] = Value::Array(required);
        }
        match r.below(3) {
            0 => schema["additionalProperties"] = json!(false),
            1 => schema["additionalProperties"] = json!(true),
            _ => {}
        }
        if r.chance(10) {
            schema["patternProperties"] = json!({"^z": {"type": "integer"}});
        }
        TaskType {
            id: "acme.thing".into(),
            title: "Thing".into(),
            icon: String::new(),
            schema,
            display: TaskTypeDisplay::default(),
        }
    }

    fn random_override(r: &mut Rng) -> TypeOverride {
        let mut o = TypeOverride {
            type_id: "acme.thing".into(),
            ..Default::default()
        };
        for _ in 0..r.below(3) {
            o.add_properties
                .insert((*r.pick(NAMES)).into(), random_prop(r));
        }
        for _ in 0..r.below(3) {
            let vals: Vec<Value> = (0..1 + r.below(3)).map(|_| random_value(r)).collect();
            o.add_enum.insert((*r.pick(NAMES)).into(), vals);
        }
        if r.chance(30) {
            o.property_descriptions
                .insert((*r.pick(NAMES)).into(), "about".into());
        }
        if r.chance(30) {
            o.title = Some("Renamed".into());
        }
        if o.add_properties.is_empty() && o.add_enum.is_empty() && o.title.is_none() {
            o.icon = Some("star".into());
        }
        o
    }

    fn random_item(r: &mut Rng) -> Map<String, Value> {
        let mut m = Map::new();
        for n in NAMES {
            if r.chance(45) {
                m.insert((*n).into(), random_value(r));
            }
        }
        m
    }

    fn registry(t: &TaskType) -> TaskTypeRegistry {
        let mut reg = TaskTypeRegistry::default();
        reg.insert(t.clone(), TaskTypeOrigin::Builtin).unwrap();
        reg
    }

    /// Property: for any additive override that applies, every item valid
    /// before is valid after. Hand-rolled generator (proptest is not in
    /// the lock), 3000 seeded cases.
    #[test]
    fn additive_overrides_keep_valid_items_valid() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut applied, mut checked_items) = (0, 0);
        for case in 0..3000 {
            let base = random_type(&mut r);
            if base.validate().is_err() {
                continue;
            }
            let o = random_override(&mut r);
            let Ok(after) = apply_override(&base, &o) else {
                continue;
            };
            applied += 1;
            let before_reg = registry(&base);
            let after_reg = registry(&after);
            for _ in 0..100 {
                let item = random_item(&mut r);
                if before_reg
                    .validate_fields(Some("acme.thing"), &item)
                    .is_ok()
                {
                    checked_items += 1;
                    assert!(
                        after_reg.validate_fields(Some("acme.thing"), &item).is_ok(),
                        "case {case}: {item:?} valid under {} but not under {} (override {o:?})",
                        base.schema,
                        after.schema
                    );
                }
            }
        }
        assert!(applied > 300, "only {applied} overrides applied");
        assert!(
            checked_items > 2000,
            "only {checked_items} valid items checked"
        );
    }
}
