#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::bool_assert_comparison,
    clippy::approx_constant
)]
//! End-to-end integration tests for `agent_first_data::document`.
//!
//! Library-level coverage: format read/write, dot-path get/set, keyed lists,
//! coercion, and source preservation. The CLI-invoking cases live in
//! `tests/cli_document.rs` (the `afdata` command surface).
//!
//! JSON support is a core (always-compiled) dependency of `agent_first_data`,
//! not an optional `json` feature — so JSON-only tests below carry no feature
//! gate.

use agent_first_data::document::{
    DocumentError, Format, KeyedList, Value, add_keyed, get_path, remove_keyed, set_path,
};
use std::collections::BTreeMap;

#[test]
fn test_json_round_trip() {
    let json_str = r#"{"imap": {"host": "mail.example.com", "port": 993}}"#;

    let value = Format::Json.load(json_str).unwrap();

    let host = get_path(&value, "imap.host", &[]).unwrap();
    assert_eq!(host.as_str().unwrap(), "mail.example.com");

    let mut value = value;
    set_path(&mut value, "imap.port", &Value::Integer(587), &[]).unwrap();

    let output = Format::Json.save(&value).unwrap();
    let reloaded = Format::Json.load(&output).unwrap();
    let port = get_path(&reloaded, "imap.port", &[]).unwrap();
    assert_eq!(port.as_integer().unwrap(), 587);
}

#[test]
fn test_json_scalar_edit_preserves_unrelated_source() {
    let source = "{\n  \"z\": 1e+3,\n  \"nested\": { \"keep\": \"\\u0061\", \"target\": 2 }\n}\n";
    let edited = agent_first_data::document::format::json::set_preserving(
        source,
        "nested.target",
        &Value::Integer(7),
    )
    .unwrap();
    assert_eq!(
        edited,
        "{\n  \"z\": 1e+3,\n  \"nested\": { \"keep\": \"\\u0061\", \"target\": 7 }\n}\n"
    );
}

#[test]
fn test_json_unset_preserves_unrelated_source() {
    let source = "{\n  \"keep\": 1e+3,\n  \"remove\": 2,\n  \"last\": \"x\"\n}\n";
    let edited =
        agent_first_data::document::format::json::unset_preserving(source, "remove").unwrap();
    assert_eq!(edited, "{\n  \"keep\": 1e+3,\n  \"last\": \"x\"\n}\n");
}

#[test]
fn test_json_unset_last_member_stays_inside_parent() {
    // Regression: removing the sole/last member of a nested object must not
    // grab a comma from an enclosing container. Here the comma after `arr`
    // belongs to the root, not to `b`.
    let source = r#"{"arr": [{"n": 1}], "b": {"x": 2}}"#;
    let edited = agent_first_data::document::format::json::unset_preserving(source, "b.x").unwrap();
    assert_eq!(edited, r#"{"arr": [{"n": 1}], "b": {}}"#);
}

#[test]
fn test_json_golden_variants_preserve_untouched_source() {
    let compact = "{\"keep\":\"\\u0061\",\"target\":1e+3,\"tail\":[1,2]}";
    let edited = agent_first_data::document::format::json::set_preserving(
        compact,
        "target",
        &Value::Integer(7),
    )
    .unwrap();
    assert!(edited.contains("\\u0061"));
    assert!(edited.contains("[1,2]"));
    assert_eq!(edited, "{\"keep\":\"\\u0061\",\"target\":7,\"tail\":[1,2]}");

    let crlf = "{\r\n  \"keep\": 1e+3,\r\n  \"target\": 2\r\n}\r\n";
    let edited =
        agent_first_data::document::format::json::unset_preserving(crlf, "target").unwrap();
    assert_eq!(edited, "{\r\n  \"keep\": 1e+3\r\n}\r\n");
}

#[test]
fn test_json_set_creates_missing_parent_objects() {
    // Sparse config: `imap` does not exist yet, so setting `imap.host` must
    // create the intermediate object, indented to match the sibling member.
    let source = "{\n  \"a\": 1\n}\n";
    let edited = agent_first_data::document::format::json::set_preserving(
        source,
        "imap.host",
        &Value::String("mail.example.com".to_string()),
    )
    .unwrap();
    assert_eq!(
        edited,
        "{\n  \"a\": 1,\n  \"imap\": {\n    \"host\": \"mail.example.com\"\n  }\n}\n"
    );
}

#[test]
fn test_json_set_creates_deep_missing_parents_in_empty_object() {
    let source = "{\n  \"a\": {}\n}\n";
    let edited = agent_first_data::document::format::json::set_preserving(
        source,
        "a.b.c",
        &Value::Integer(1),
    )
    .unwrap();
    // The whole nested chain is created; unrelated source is untouched.
    let parsed: serde_json::Value = serde_json::from_str(&edited).unwrap();
    assert_eq!(parsed["a"]["b"]["c"], serde_json::json!(1));
}

#[test]
fn test_json_set_replaces_existing_array_value() {
    let source = "{\n  \"steps\": [1, 2]\n}\n";
    let edited = agent_first_data::document::format::json::set_preserving(
        source,
        "steps",
        &Value::Array(vec![Value::Integer(3), Value::Integer(4)]),
    )
    .unwrap();
    assert_eq!(edited, "{\n  \"steps\": [3,4]\n}\n");
}

#[test]
fn test_json_set_replaces_scalar_with_empty_array() {
    let source = "{\n  \"steps\": \"none\"\n}\n";
    let edited = agent_first_data::document::format::json::set_preserving(
        source,
        "steps",
        &Value::Array(vec![]),
    )
    .unwrap();
    assert_eq!(edited, "{\n  \"steps\": []\n}\n");
}

#[test]
fn test_json_set_rejects_child_under_scalar() {
    let source = "{\n  \"a\": 1\n}\n";
    let error =
        agent_first_data::document::format::json::set_preserving(source, "a.b", &Value::Integer(2));
    assert!(matches!(
        error,
        Err(DocumentError::UnsupportedOperation { .. })
    ));
}

#[cfg(feature = "toml")]
#[test]
fn test_toml_round_trip() {
    let toml_str = r#"
[database]
host = "localhost"
port = 5432
"#;

    let value = Format::Toml.load(toml_str).unwrap();

    let host = get_path(&value, "database.host", &[]).unwrap();
    assert_eq!(host.as_str().unwrap(), "localhost");

    let mut value = value;
    set_path(&mut value, "database.port", &Value::Integer(3306), &[]).unwrap();

    let output = Format::Toml.save(&value).unwrap();
    let reloaded = Format::Toml.load(&output).unwrap();
    let port = get_path(&reloaded, "database.port", &[]).unwrap();
    assert_eq!(port.as_integer().unwrap(), 3306);
}

#[cfg(feature = "toml")]
#[test]
fn test_toml_scalar_edit_preserves_comments_and_datetime() {
    let source = "# keep\n[database]\nport = 5432 # note\nwhen = 2024-01-01T00:00:00Z\n";
    let edited = agent_first_data::document::format::toml::set_preserving(
        source,
        "database.port",
        &Value::Integer(3306),
    )
    .unwrap();
    assert_eq!(
        edited,
        "# keep\n[database]\nport = 3306 # note\nwhen = 2024-01-01T00:00:00Z\n"
    );
}

#[cfg(feature = "toml")]
#[test]
fn test_toml_unset_preserves_comments() {
    let source = "# keep\n[database]\nremove = 1 # remove\nkeep = 2024-01-01T00:00:00Z\n";
    let edited =
        agent_first_data::document::format::toml::unset_preserving(source, "database.remove")
            .unwrap();
    assert_eq!(edited, "# keep\n[database]\nkeep = 2024-01-01T00:00:00Z\n");
}

#[cfg(feature = "toml")]
#[test]
fn test_toml_golden_array_and_datetime_bytes() {
    let source = "# keep\nwhen = 2024-01-01T00:00:00Z\nvalues = [1, 2, 3]\ntarget = 1\n";
    let edited = agent_first_data::document::format::toml::set_preserving(
        source,
        "target",
        &Value::Integer(2),
    )
    .unwrap();
    assert!(edited.contains("when = 2024-01-01T00:00:00Z"));
    assert!(edited.contains("values = [1, 2, 3]"));
    assert!(edited.ends_with("target = 2\n"));

    let arrays =
        "global_target = 1\n\n[[servers]]\nname = \"one\"\n\n[[servers]]\nname = \"two\"\n";
    let edited = agent_first_data::document::format::toml::set_preserving(
        arrays,
        "global_target",
        &Value::Integer(2),
    )
    .unwrap();
    assert!(edited.contains("[[servers]]\nname = \"one\""));
    assert!(edited.contains("global_target = 2"), "{edited}");
    assert!(edited.contains("[[servers]]\nname = \"two\""), "{edited}");
}

#[cfg(feature = "toml")]
#[test]
fn test_toml_rejects_unrepresentable_null_and_u64() {
    let null_error = agent_first_data::document::format::toml::set_preserving(
        "value = 1\n",
        "value",
        &Value::Null,
    )
    .unwrap_err();
    assert!(null_error.to_string().contains("no null"));
    let unsigned_error = agent_first_data::document::format::toml::set_preserving(
        "value = 1\n",
        "value",
        &Value::Unsigned(u64::MAX),
    )
    .unwrap_err();
    assert!(unsigned_error.to_string().contains("exceeds TOML"));
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_round_trip() {
    let yaml_str = r#"
server:
  host: localhost
  port: 8080
"#;

    let value = Format::Yaml.load(yaml_str).unwrap();

    let host = get_path(&value, "server.host", &[]).unwrap();
    assert_eq!(host.as_str().unwrap(), "localhost");

    let mut value = value;
    set_path(&mut value, "server.port", &Value::Integer(9000), &[]).unwrap();

    let output = Format::Yaml.save(&value).unwrap();
    let reloaded = Format::Yaml.load(&output).unwrap();
    let port = get_path(&reloaded, "server.port", &[]).unwrap();
    assert_eq!(port.as_integer().unwrap(), 9000);
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_scalar_edit_preserves_comments_and_float() {
    let source = "# keep\nserver:\n  host: localhost # host\n  ratio: 1.0\n";
    let edited = agent_first_data::document::format::yaml::set_preserving(
        source,
        "server.host",
        &Value::String("example.com".to_string()),
    )
    .unwrap();
    assert_eq!(
        edited,
        "# keep\nserver:\n  host: example.com # host\n  ratio: 1.0\n"
    );
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_unset_preserves_comments() {
    let source = "# keep\nserver:\n  remove: 1 # remove\n  keep: 1.0\n";
    let edited =
        agent_first_data::document::format::yaml::unset_preserving(source, "server.remove")
            .unwrap();
    assert_eq!(edited, "# keep\nserver:\n  keep: 1.0\n");
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_golden_styles_and_crlf() {
    let source =
        "# keep\r\nroot:\r\n  quoted: 'old'\r\n  literal: |\r\n    unchanged\r\n  target: 1.0\r\n";
    let edited = agent_first_data::document::format::yaml::set_preserving(
        source,
        "root.target",
        &Value::Float(2.0),
    )
    .unwrap();
    assert!(edited.contains("# keep\r\n"));
    assert!(edited.contains("quoted: 'old'\r\n"));
    assert!(edited.contains("literal: |\r\n    unchanged\r\n"));
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_golden_flow_tag_anchor_and_alias_untouched() {
    let source = "defaults: &defaults {name: old}\ncopy: *defaults\ntarget: 1\nflow: [1, 2]\n";
    let edited = agent_first_data::document::format::yaml::set_preserving(
        source,
        "target",
        &Value::Integer(2),
    )
    .unwrap();
    assert!(edited.contains("defaults: &defaults {name: old}"));
    assert!(edited.contains("copy: *defaults"));
    assert!(edited.contains("flow: [1, 2]"));
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_keyed_collection_edit_preserves_unrelated_source() {
    let source = "# keep\nitems:\n  - id: a\n    name: A\nkeep: 1.0\n";
    let item = Value::Object(BTreeMap::from([
        ("id".to_string(), Value::String("b".to_string())),
        ("name".to_string(), Value::String("B".to_string())),
    ]));
    let added = agent_first_data::document::format::yaml::append_array_item_preserving(
        source, "items", &item,
    )
    .unwrap();
    assert!(added.contains("keep: 1.0"));
    assert_eq!(
        Format::Yaml
            .load(&added)
            .unwrap()
            .get("items")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let removed =
        agent_first_data::document::format::yaml::remove_array_item_preserving(&added, "items", 1)
            .unwrap();
    assert!(removed.contains("keep: 1.0"));
    assert_eq!(
        Format::Yaml
            .load(&removed)
            .unwrap()
            .get("items")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_cst_numeric_path_adapter_and_unsupported_escaped_keys() {
    let source = "items:\n  - name: first\n  - name: second\nkeep: 1.0\n";
    let edited = agent_first_data::document::format::yaml::set_preserving(
        source,
        "items.1.name",
        &Value::String("changed".to_string()),
    )
    .unwrap();
    assert!(edited.contains("name: changed"));
    assert!(edited.contains("keep: 1.0"));
    let error = agent_first_data::document::format::yaml::set_preserving(
        source,
        r"items.key\.with.dot",
        &Value::String("x".to_string()),
    )
    .unwrap_err();
    assert!(matches!(error, DocumentError::UnsupportedOperation { .. }));
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_uses_strict_yaml_1_2_parsing() {
    let value = Format::Yaml.load("country: NO\nenabled: true\n").unwrap();
    assert_eq!(
        get_path(&value, "country", &[]).unwrap().as_str(),
        Some("NO")
    );
    assert_eq!(
        get_path(&value, "enabled", &[]).unwrap().as_bool(),
        Some(true)
    );

    assert!(Format::Yaml.load("name: first\nname: second\n").is_err());
}

#[test]
fn test_keyed_list_add_and_remove() {
    let mut root = Value::Object(BTreeMap::new());

    let keyed_lists = [KeyedList {
        prefix: "identities",
        slug_field: "identity",
    }];

    if let Some(obj) = root.as_object_mut() {
        obj.insert("identities".to_string(), Value::Array(vec![]));
    }

    add_keyed(
        &mut root,
        "identities",
        "alice",
        &keyed_lists,
        None,
        &[
            (
                "email".to_string(),
                Value::String("alice@example.com".to_string()),
            ),
            ("name".to_string(), Value::String("Alice".to_string())),
        ],
    )
    .unwrap();

    let alice_email = get_path(&root, "identities.alice.email", &keyed_lists).unwrap();
    assert_eq!(alice_email.as_str().unwrap(), "alice@example.com");

    add_keyed(
        &mut root,
        "identities",
        "bob",
        &keyed_lists,
        None,
        &[(
            "email".to_string(),
            Value::String("bob@example.com".to_string()),
        )],
    )
    .unwrap();

    let arr = root.get("identities").unwrap().as_array().unwrap();
    assert_eq!(arr.len(), 2);

    remove_keyed(&mut root, "identities", "alice", &keyed_lists).unwrap();

    let arr = root.get("identities").unwrap().as_array().unwrap();
    assert_eq!(arr.len(), 1);

    let bob_email = get_path(&root, "identities.bob.email", &keyed_lists).unwrap();
    assert_eq!(bob_email.as_str().unwrap(), "bob@example.com");
}

#[test]
fn test_escaped_dotted_key_matching() {
    let json_str = r#"{"actions":{"case.add":{"steps":[{"move":"archive"}]}}}"#;

    let value = Format::Json.load(json_str).unwrap();

    let steps = get_path(&value, r"actions.case\.add.steps", &[]).unwrap();
    assert!(steps.is_array());
}

#[test]
fn test_escaped_keyed_list_prefix_routes_consistently() {
    let mut root = Value::Object(BTreeMap::from([(
        "group.list".to_string(),
        Value::Object(BTreeMap::from([(
            "items".to_string(),
            Value::Array(vec![]),
        )])),
    )]));
    let keyed_lists = [KeyedList {
        prefix: r"group\.list.items",
        slug_field: "id",
    }];
    add_keyed(
        &mut root,
        r"group\.list.items",
        "one",
        &keyed_lists,
        None,
        &[("name".to_string(), Value::String("first".to_string()))],
    )
    .unwrap();
    assert_eq!(
        get_path(&root, r"group\.list.items.one.name", &keyed_lists)
            .unwrap()
            .as_str(),
        Some("first")
    );
    set_path(
        &mut root,
        r"group\.list.items.one.name",
        &Value::String("second".to_string()),
        &keyed_lists,
    )
    .unwrap();
    assert_eq!(
        get_path(&root, r"group\.list.items.one.name", &keyed_lists)
            .unwrap()
            .as_str(),
        Some("second")
    );
}

#[test]
fn test_type_coercion() {
    let mut root = Value::Object(BTreeMap::new());

    set_path(&mut root, "port", &Value::Integer(993), &[]).unwrap();
    let port = get_path(&root, "port", &[]).unwrap();
    assert_eq!(port.as_integer(), Some(993));

    set_path(&mut root, "enabled", &Value::Bool(true), &[]).unwrap();
    let enabled = get_path(&root, "enabled", &[]).unwrap();
    assert_eq!(enabled.as_bool(), Some(true));

    set_path(&mut root, "timeout", &Value::Float(3.14), &[]).unwrap();
    let timeout = get_path(&root, "timeout", &[]).unwrap();
    match timeout.as_float() {
        Some(f) => assert!((f - 3.14).abs() < 0.01),
        None => panic!("expected float"),
    }

    set_path(&mut root, "name", &Value::String("Alice".to_string()), &[]).unwrap();
    let name = get_path(&root, "name", &[]).unwrap();
    assert_eq!(name.as_str(), Some("Alice"));
}

#[test]
fn test_scalar_array_replacement() {
    let mut root = Value::Object(BTreeMap::new());

    set_path(
        &mut root,
        "tags",
        &Value::Array(vec![
            Value::String("dev".to_string()),
            Value::String("staging".to_string()),
            Value::String("prod".to_string()),
        ]),
        &[],
    )
    .unwrap();

    let tags = get_path(&root, "tags", &[]).unwrap();
    let arr = tags.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_str(), Some("dev"));
    assert_eq!(arr[1].as_str(), Some("staging"));
    assert_eq!(arr[2].as_str(), Some("prod"));
}

#[test]
fn test_nested_object_creation() {
    let mut root = Value::Object(BTreeMap::new());

    set_path(
        &mut root,
        "server.database.connection.host",
        &Value::String("localhost".to_string()),
        &[],
    )
    .unwrap();

    let host = get_path(&root, "server.database.connection.host", &[]).unwrap();
    assert_eq!(host.as_str().unwrap(), "localhost");
}

#[test]
fn test_json_array_coercion() {
    let mut root = Value::Object(BTreeMap::new());

    set_path(
        &mut root,
        "config",
        &Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("c".to_string()),
        ]),
        &[],
    )
    .unwrap();

    let config = get_path(&root, "config", &[]).unwrap();
    let arr = config.as_array().unwrap();
    assert_eq!(arr.len(), 3);
}

#[test]
fn test_type_prefix_coercion() {
    let mut root = Value::Object(BTreeMap::new());

    set_path(&mut root, "field1", &Value::String("true".to_string()), &[]).unwrap();
    let val = get_path(&root, "field1", &[]).unwrap();
    assert_eq!(val.as_str(), Some("true"));

    set_path(&mut root, "field2", &Value::Bool(true), &[]).unwrap();
    let val = get_path(&root, "field2", &[]).unwrap();
    assert_eq!(val.as_bool(), Some(true));
}

#[test]
fn test_numeric_boundaries_do_not_narrow_unsigned_or_float() {
    // §4: `coerce_scalar`'s shape-guessing is gone (superseded by §3's
    // "bare value is always a string" rule); the literal-faithful numeric
    // entry point is now `value_from_type(ValueType::Number, ...)`.
    use agent_first_data::document::{ValueType, value_from_type};

    let max = value_from_type(ValueType::Number, Some("18446744073709551615")).unwrap();
    assert_eq!(max.as_unsigned(), Some(u64::MAX));
    let precise = value_from_type(ValueType::Number, Some("9007199254740993")).unwrap();
    assert_eq!(precise.as_integer(), Some(9_007_199_254_740_993));
    // A literal written with a decimal point stays literal-faithful
    // (`Value::Number`), not silently narrowed to `Value::Float` — even
    // when, as here, its magnitude would fit exactly.
    let float = value_from_type(ValueType::Number, Some("3.0")).unwrap();
    assert_eq!(float, Value::Number("3.0".to_string()));
    assert_eq!(
        value_from_type(ValueType::Number, Some(&i64::MIN.to_string()))
            .unwrap()
            .as_integer(),
        Some(i64::MIN)
    );
    assert_eq!(
        value_from_type(ValueType::Number, Some(&i64::MAX.to_string()))
            .unwrap()
            .as_integer(),
        Some(i64::MAX)
    );
    assert_eq!(
        value_from_type(ValueType::Number, Some("9007199254740991"))
            .unwrap()
            .as_integer(),
        Some(9_007_199_254_740_991)
    );
    assert_eq!(
        value_from_type(ValueType::Number, Some("9007199254740993"))
            .unwrap()
            .as_integer(),
        Some(9_007_199_254_740_993)
    );
}

#[test]
fn test_json_unsigned_boundary_round_trip() {
    let value = Format::Json
        .load("{\"n\":18446744073709551615,\"f\":3.0}")
        .unwrap();
    assert_eq!(
        get_path(&value, "n", &[]).unwrap().as_unsigned(),
        Some(u64::MAX)
    );
    // §4: a float literal is preserved verbatim as `Value::Number`, not
    // narrowed through `f64` — even a value whose magnitude, like `3.0`,
    // would survive that narrowing exactly. The point is the literal
    // *spelling* survives too.
    assert_eq!(
        get_path(&value, "f", &[]).unwrap(),
        Value::Number("3.0".to_string())
    );
    // §4: an exponent so large it would overflow `f64` no longer fails to
    // parse — arbitrary-precision JSON numbers never force an eager `f64`
    // evaluation, so the literal is simply preserved (serde_json normalizes
    // a positive exponent to an explicit `+`, `1e400` -> `1e+400`; every
    // digit is still exactly as written).
    let huge_exponent = Format::Json.load("1e400").unwrap();
    assert_eq!(huge_exponent, Value::Number("1e+400".to_string()));
}

/// §4 fixture matrix: oversized integers, high-precision floats, exponent
/// notation, and `-0` through the JSON backend's `get`/`set`-facing reader.
/// `value` (CLI byte emission) is covered separately in
/// `tests/cli_document.rs`; this is the structural (`document::Value`)
/// layer both `get` and `value` build on.
#[test]
fn test_json_number_literal_fidelity_matrix() {
    let huge_int = "123456789012345678901234567890";
    let precise_float = "0.1000000000000000055511151231257827";
    let source = format!(
        "{{\"huge\":{huge_int},\"precise\":{precise_float},\"exp\":1e+140,\"neg_zero_int\":-0,\"neg_zero_float\":-0.0}}"
    );
    let value = Format::Json.load(&source).unwrap();

    assert_eq!(
        get_path(&value, "huge", &[]).unwrap(),
        Value::Number(huge_int.to_string())
    );
    assert_eq!(
        get_path(&value, "precise", &[]).unwrap(),
        Value::Number(precise_float.to_string())
    );
    assert_eq!(
        get_path(&value, "exp", &[]).unwrap(),
        Value::Number("1e+140".to_string())
    );
    // An integer `-0` has no distinct representation from `0` in `i64` —
    // there is no signed zero for integers, so this collapses to a plain
    // `Integer(0)`, not a fidelity violation (no digits are lost; the sign
    // was mathematically redundant).
    assert_eq!(
        get_path(&value, "neg_zero_int", &[]).unwrap(),
        Value::Integer(0)
    );
    // A *float* `-0.0` is a distinct IEEE 754 value from `0.0`, and its
    // literal is preserved exactly (not collapsed).
    assert_eq!(
        get_path(&value, "neg_zero_float", &[]).unwrap(),
        Value::Number("-0.0".to_string())
    );

    // set_path/get_path round-trip the literal exactly (no f64 detour).
    let mut root = value;
    set_path(
        &mut root,
        "set_huge",
        &Value::Number(huge_int.to_string()),
        &[],
    )
    .unwrap();
    assert_eq!(
        get_path(&root, "set_huge", &[]).unwrap(),
        Value::Number(huge_int.to_string())
    );

    // Regression: values that fit i64/u64 stay Integer/Unsigned, unaffected
    // by the Number variant's introduction.
    assert_eq!(
        get_path(
            &Format::Json.load("{\"u\":18446744073709551615}").unwrap(),
            "u",
            &[]
        )
        .unwrap()
        .as_unsigned(),
        Some(u64::MAX)
    );
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_unsigned_boundary_round_trip() {
    let value = Format::Yaml
        .load("n: 18446744073709551615\nf: 3.0\n")
        .unwrap();
    assert_eq!(
        get_path(&value, "n", &[]).unwrap().as_unsigned(),
        Some(u64::MAX)
    );
    assert!(matches!(get_path(&value, "f", &[]).unwrap(), Value::Float(value) if value == 3.0));
}

#[test]
fn test_json_numeric_boundary_matrix() {
    let source = format!(
        "{{\"min\":{},\"max\":{},\"above\":{},\"u\":{},\"p\":9007199254740993,\"f\":1e-10}}",
        i64::MIN,
        i64::MAX,
        i64::MAX as u128 + 1,
        u64::MAX
    );
    let value = Format::Json.load(&source).unwrap();
    assert_eq!(
        get_path(&value, "min", &[]).unwrap().as_integer(),
        Some(i64::MIN)
    );
    assert_eq!(
        get_path(&value, "max", &[]).unwrap().as_integer(),
        Some(i64::MAX)
    );
    assert_eq!(
        get_path(&value, "above", &[]).unwrap().as_unsigned(),
        Some(i64::MAX as u64 + 1)
    );
    assert_eq!(
        get_path(&value, "u", &[]).unwrap().as_unsigned(),
        Some(u64::MAX)
    );
    // §4: a decimal-exponent literal is preserved verbatim as `Value::Number`.
    assert_eq!(
        get_path(&value, "f", &[]).unwrap(),
        Value::Number("1e-10".to_string())
    );
    // §4: no longer an error — see `test_json_unsigned_boundary_round_trip`.
    assert_eq!(
        Format::Json.load("{\"bad\":1e400}").unwrap().get("bad"),
        Some(&Value::Number("1e+400".to_string()))
    );
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_numeric_boundary_matrix() {
    let source = format!(
        "min: {}\nmax: {}\nabove: {}\nu: {}\nprecise: 9007199254740993\n",
        i64::MIN,
        i64::MAX,
        i64::MAX as u128 + 1,
        u64::MAX
    );
    let value = Format::Yaml.load(&source).unwrap();
    assert_eq!(
        get_path(&value, "min", &[]).unwrap().as_integer(),
        Some(i64::MIN)
    );
    assert_eq!(
        get_path(&value, "max", &[]).unwrap().as_integer(),
        Some(i64::MAX)
    );
    assert_eq!(
        get_path(&value, "above", &[]).unwrap().as_unsigned(),
        Some(i64::MAX as u64 + 1)
    );
    assert_eq!(
        get_path(&value, "u", &[]).unwrap().as_unsigned(),
        Some(u64::MAX)
    );
}

#[cfg(feature = "toml")]
#[test]
fn test_toml_numeric_boundary_matrix() {
    let value = Format::Toml
        .load(&format!(
            "min = {}\nmax = {}\nprecise = 9007199254740993\n",
            i64::MIN,
            i64::MAX
        ))
        .unwrap();
    assert_eq!(
        get_path(&value, "min", &[]).unwrap().as_integer(),
        Some(i64::MIN)
    );
    assert_eq!(
        get_path(&value, "max", &[]).unwrap().as_integer(),
        Some(i64::MAX)
    );
    assert_eq!(
        get_path(&value, "precise", &[]).unwrap().as_integer(),
        Some(9_007_199_254_740_993)
    );
    assert!(Format::Toml.load("bad = 1e9999\n").is_err());
    assert!(
        agent_first_data::document::format::toml::set_preserving(
            "value = 1\n",
            "value",
            &Value::Float(f64::NAN)
        )
        .is_err()
    );
}

#[test]
fn test_error_on_nonexistent_slug() {
    let mut root = Value::Object(BTreeMap::new());

    let keyed_lists = [KeyedList {
        prefix: "identities",
        slug_field: "identity",
    }];

    if let Some(obj) = root.as_object_mut() {
        obj.insert("identities".to_string(), Value::Array(vec![]));
    }

    let result = remove_keyed(&mut root, "identities", "nonexistent", &keyed_lists);
    assert!(result.is_err());
}

#[test]
fn test_format_detection_json() {
    assert_eq!(
        Format::detect(std::path::Path::new("config.json")),
        Some(Format::Json)
    );
    assert_eq!(Format::detect(std::path::Path::new("config.txt")), None);
}

#[cfg(feature = "toml")]
#[test]
fn test_format_detection_toml() {
    assert_eq!(
        Format::detect(std::path::Path::new("config.toml")),
        Some(Format::Toml)
    );
}

#[cfg(feature = "yaml")]
#[test]
fn test_format_detection_yaml() {
    assert_eq!(
        Format::detect(std::path::Path::new("config.yaml")),
        Some(Format::Yaml)
    );
    assert_eq!(
        Format::detect(std::path::Path::new("config.yml")),
        Some(Format::Yaml)
    );
}

#[cfg(feature = "dotenv")]
#[test]
fn test_format_detection_dotenv() {
    for path in [
        ".env",
        ".env.local",
        ".env.test",
        ".env.example",
        "config.env",
        "CONFIG.ENV",
    ] {
        assert_eq!(
            Format::detect(std::path::Path::new(path)),
            Some(Format::Dotenv),
            "failed to detect {path}"
        );
    }
    assert_eq!(Format::detect(std::path::Path::new("config.txt")), None);
}

#[cfg(feature = "ini")]
#[test]
fn test_ini_core_v1_strings_and_duplicates() {
    let value = Format::Ini
        .load("[database]\r\nhost = localhost\r\nport=5432\r\n")
        .unwrap();
    assert_eq!(
        get_path(&value, "database.host", &[]).unwrap().as_str(),
        Some("localhost")
    );
    assert_eq!(
        get_path(&value, "database.port", &[]).unwrap().as_str(),
        Some("5432")
    );
    assert!(Format::Ini.load("[database]\na=1\na=2\n").is_err());
    assert!(Format::Ini.load("[database]\n[database]\n").is_err());
}

#[cfg(feature = "ini")]
#[test]
fn test_ini_fixtures_and_source_editor() {
    let fixture = include_str!("fixtures/ini/core.ini");
    let invalid_fixture = include_str!("fixtures/ini/invalid.ini");
    assert!(Format::Ini.load(fixture).is_ok());
    assert!(Format::Ini.load(invalid_fixture).is_err());
    let source =
        "; comment\r\n[Database]\r\nkey.with.dot = value # literal\r\nempty=\r\n\r\n[empty]\r\n";
    let value = Format::Ini.load(source).unwrap();
    assert_eq!(
        get_path(&value, r"Database.key\.with\.dot", &[])
            .unwrap()
            .as_str(),
        Some("value # literal")
    );
    assert!(Format::Ini.load("root=value\n").is_err());
    assert!(Format::Ini.load("[s]\na: b\n").is_err());
    let edited = agent_first_data::document::format::ini::set_preserving(
        source,
        r"Database.key\.with\.dot",
        &Value::String("changed".to_string()),
    )
    .unwrap();
    assert!(edited.contains("; comment\r\n"));
    assert!(edited.contains("key.with.dot = changed\r\n"));
    let removed =
        agent_first_data::document::format::ini::unset_preserving(&edited, "Database.empty")
            .unwrap();
    assert!(removed.contains("[empty]\r\n"));

    let no_final_newline = "[section]\r\nkey = old";
    let edited = agent_first_data::document::format::ini::set_preserving(
        no_final_newline,
        "section.key",
        &Value::String("new".to_string()),
    )
    .unwrap();
    assert_eq!(edited, "[section]\r\nkey = new");
}

#[cfg(feature = "dotenv")]
#[test]
fn test_dotenv_read_semantics() {
    let fixture = include_str!("fixtures/dotenv/core.env");
    assert!(Format::Dotenv.load(fixture).is_ok());
    assert!(
        Format::Dotenv
            .load(include_str!("fixtures/dotenv/invalid.env"))
            .is_err()
    );
    let content = concat!(
        "# comment\r\n",
        " BASIC = value with spaces  # comment\r\n",
        "export EMPTY=\r\n",
        "SINGLE='literal # value'\r\n",
        "DOUBLE=\"line\\nquoted\\t\\\"value\\\"\\\\ # value\" # comment\r\n",
        "NUMBER=5432\r\n",
        "UNICODE=你好\r\n",
        "DUPLICATE=first\r\n",
        "DUPLICATE=last\r\n",
        "REFERENCE=${AFDATA_TEST_PROCESS_VALUE}\r\n",
    );
    let error = Format::Dotenv
        .load(content)
        .expect_err("duplicate keys must fail");
    assert!(error.to_string().contains("duplicate"));
    let content = content
        .replace("DUPLICATE=first\r\n", "")
        .replace("DUPLICATE=last\r\n", "");
    let value = Format::Dotenv.load(&content).expect("dotenv should parse");

    let expected = [
        ("BASIC", "value with spaces"),
        ("EMPTY", ""),
        ("SINGLE", "literal # value"),
        ("DOUBLE", "line\nquoted\t\"value\"\\ # value"),
        ("NUMBER", "5432"),
        ("UNICODE", "你好"),
        ("REFERENCE", "${AFDATA_TEST_PROCESS_VALUE}"),
    ];
    for (key, expected_value) in expected {
        let actual = get_path(&value, key, &[]).expect("key should exist");
        assert_eq!(actual.as_str(), Some(expected_value));
        assert!(actual.is_string());
    }
}

#[cfg(feature = "dotenv")]
#[test]
fn test_dotenv_multiline_and_missing_set_preserve_source() {
    let source = "# keep\nMULTI=\"first\nsecond\"\nOTHER=abc#def\n";
    let value = Format::Dotenv.load(source).unwrap();
    assert_eq!(
        get_path(&value, "MULTI", &[]).unwrap().as_str(),
        Some("first\nsecond")
    );
    let edited = agent_first_data::document::format::dotenv::set_preserving(
        source,
        "NEW",
        &Value::String("value".to_string()),
    )
    .unwrap();
    assert!(edited.starts_with(source));

    let no_final_newline = "export KEY='old'";
    let edited = agent_first_data::document::format::dotenv::set_preserving(
        no_final_newline,
        "KEY",
        &Value::String("new value".to_string()),
    )
    .unwrap();
    assert_eq!(edited, "export KEY=\"new value\"");
}

#[cfg(feature = "dotenv")]
#[test]
fn test_dotenv_rejects_invalid_assignments_without_source_text() {
    let error = Format::Dotenv
        .load("SECRET_VALUE_WITHOUT_EQUALS")
        .expect_err("invalid assignment should fail");
    let message = error.to_string();
    assert!(message.contains("line 1"));
    assert!(!message.contains("SECRET_VALUE_WITHOUT_EQUALS"));
}

#[cfg(feature = "dotenv")]
#[test]
fn test_dotenv_save_is_typed_unsupported_operation() {
    let value = Format::Dotenv
        .load("KEY=value\n")
        .expect("dotenv should parse");
    let error = Format::Dotenv
        .save(&value)
        .expect_err("dotenv save should fail");
    assert!(matches!(error, DocumentError::UnsupportedOperation { .. }));
}

#[test]
fn test_json_set_missing_key_inserts_preserving_layout() {
    // A new leaf under an existing object is spliced with sibling indentation;
    // every untouched byte (including number spelling) is preserved.
    let source = "{\n  \"keep\": 1e+3,\n  \"obj\": {\n    \"a\": 1\n  }\n}\n";
    let edited = agent_first_data::document::format::json::set_preserving(
        source,
        "obj.b",
        &Value::Integer(2),
    )
    .unwrap();
    assert_eq!(
        edited,
        "{\n  \"keep\": 1e+3,\n  \"obj\": {\n    \"a\": 1,\n    \"b\": 2\n  }\n}\n"
    );
    // A missing intermediate parent is now created under the deepest existing
    // object, leaving every untouched byte (including number spelling) intact.
    let created = agent_first_data::document::format::json::set_preserving(
        source,
        "nope.deep",
        &Value::Integer(1),
    )
    .unwrap();
    assert_eq!(
        created,
        "{\n  \"keep\": 1e+3,\n  \"obj\": {\n    \"a\": 1\n  },\n  \"nope\": {\n    \"deep\": 1\n  }\n}\n"
    );
}

#[test]
fn test_file_operations() {
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_path = temp_dir.path().join("config.json");

    let initial = r#"{"app":{"name":"test","version":"1.0"}}"#;
    fs::write(&config_path, initial).expect("failed to write initial config");

    let content = fs::read_to_string(&config_path).expect("failed to read config");
    let mut value = Format::Json.load(&content).expect("failed to load JSON");
    set_path(
        &mut value,
        "app.version",
        &Value::String("2.0".to_string()),
        &[],
    )
    .expect("failed to set path");

    let output = Format::Json.save(&value).expect("failed to save JSON");
    fs::write(&config_path, output).expect("failed to write updated config");

    let updated = fs::read_to_string(&config_path).expect("failed to read updated config");
    let reloaded = Format::Json
        .load(&updated)
        .expect("failed to load updated JSON");
    let version = get_path(&reloaded, "app.version", &[]).expect("failed to get version");
    assert_eq!(version.as_str().expect("version should be string"), "2.0");
}

#[test]
fn test_document_file_stages_edits_until_save() {
    use agent_first_data::document::DocumentFile;
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.json");
    let initial =
        "{\n  \"imap\": {\n    \"host\": \"old\",\n    \"password_secret_env\": \"OLD\"\n  }\n}\n";
    fs::write(&path, initial).unwrap();

    let mut doc = DocumentFile::open(&path, Some(Format::Json)).unwrap();
    // Two staged edits in afmail's config-set shape: replace a scalar and clear
    // a mutually-exclusive sibling.
    doc.set("imap.host", Value::String("new".to_string()))
        .unwrap();
    doc.unset("imap.password_secret_env").unwrap();

    // Nothing has touched disk yet — both edits are staged in memory.
    assert_eq!(fs::read_to_string(&path).unwrap(), initial);

    // The in-memory value already reflects both edits, so a caller can
    // deserialize-and-validate the result before committing.
    assert_eq!(
        get_path(doc.value(), "imap.host", &[])
            .unwrap()
            .as_str()
            .unwrap(),
        "new"
    );
    assert!(get_path(doc.value(), "imap.password_secret_env", &[]).is_err());

    // A single atomic commit lands both edits.
    doc.save().unwrap();
    let reloaded = Format::Json
        .load(&fs::read_to_string(&path).unwrap())
        .unwrap();
    assert_eq!(
        get_path(&reloaded, "imap.host", &[])
            .unwrap()
            .as_str()
            .unwrap(),
        "new"
    );
    assert!(get_path(&reloaded, "imap.password_secret_env", &[]).is_err());
}

#[test]
fn test_document_unset_is_idempotent() {
    use agent_first_data::document::Document;

    let mut doc = Document::parse("{\n  \"a\": 1,\n  \"b\": 2\n}\n", Format::Json).unwrap();
    assert!(doc.unset("a").unwrap()); // removed → true
    assert!(!doc.unset("a").unwrap()); // already gone → false, no error
    assert!(!doc.unset("missing").unwrap()); // never existed → false
    assert_eq!(doc.source(), "{\n  \"b\": 2\n}\n");
}

#[cfg(feature = "toml")]
#[test]
fn test_toml_set_creates_missing_parent_table() {
    let source = "title = \"cfg\"\n";
    let edited = agent_first_data::document::format::toml::set_preserving(
        source,
        "imap.host",
        &Value::String("mail.example.com".to_string()),
    )
    .unwrap();
    // Parent table created; existing content preserved; parses back correctly.
    let parsed = Format::Toml.load(&edited).unwrap();
    assert_eq!(
        get_path(&parsed, "imap.host", &[])
            .unwrap()
            .as_str()
            .unwrap(),
        "mail.example.com"
    );
    assert_eq!(
        get_path(&parsed, "title", &[]).unwrap().as_str().unwrap(),
        "cfg"
    );
}
