#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr
)]

use super::*;
use serde_json::{Value, json};

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/spec/fixtures");

fn load_fixture(name: &str) -> Value {
    let path = format!("{}/{}", FIXTURES_DIR, name);
    let data =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
    serde_json::from_str(&data).unwrap_or_else(|e| panic!("failed to parse {}: {}", path, e))
}

fn redactor_from_case(case: &Value) -> Redactor {
    let options = case.get("options").and_then(Value::as_object);
    let policy = options
        .and_then(|obj| obj.get("policy"))
        .and_then(Value::as_str)
        .map(|policy| match policy {
            "TraceOnly" => RedactionPolicy::TraceOnly,
            "Off" => RedactionPolicy::Off,
            other => panic!("unknown redaction policy: {other}"),
        });
    let secret_names = options
        .and_then(|obj| obj.get("secret_names"))
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .map(|name| {
                    name.as_str()
                        .unwrap_or_else(|| panic!("secret_names entries must be strings"))
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut redactor = Redactor::new();
    if !secret_names.is_empty() {
        redactor = redactor.secret_names(secret_names);
    }
    if let Some(policy) = policy {
        redactor = redactor.policy(policy);
    }
    redactor
}

// ═══════════════════════════════════════════
// Fixture-driven tests (cross-language spec)
// ═══════════════════════════════════════════

#[test]
fn test_redact_url_fixtures() {
    let cases = load_fixture("redact_url.json");
    for case in cases.as_array().expect("redact_url.json must be an array") {
        let name = case["name"].as_str().expect("missing name");
        let input = case["input"].as_str().expect("input must be a string");
        let expected = case["expected"]
            .as_str()
            .expect("expected must be a string");
        let redactor = redactor_from_case(case);
        let got = redactor.url(input);
        assert_eq!(got, expected, "[redact_url/{name}]");
    }
}

#[test]
fn test_redact_fixtures() {
    let cases = load_fixture("redact.json");
    for case in cases.as_array().expect("redact.json must be an array") {
        let name = case["name"].as_str().expect("missing name");
        let input = case["input"].clone();
        let expected = &case["expected"];
        let got = redacted_value(&input);
        assert_eq!(&got, expected, "[redact/{name}]");
    }
}

#[test]
fn test_redaction_options_fixtures() {
    let cases = load_fixture("redaction_options.json");
    for case in cases
        .as_array()
        .expect("redaction_options.json must be an array")
    {
        let name = case["name"].as_str().expect("missing name");
        let redactor = redactor_from_case(case);
        let output_options = OutputOptions {
            redaction: redactor.clone(),
            style: PlainStyle::Readable,
        };
        let expected = &case["expected"];

        let got = redactor.value(&case["input"]);
        assert_eq!(&got, expected, "[redaction_options/{name}] value");

        let mut input = case["input"].clone();
        output_options.redaction.redact_in_place(&mut input);
        assert_eq!(&input, expected, "[redaction_options/{name}] in-place");

        let json_out = render(&case["input"], OutputFormat::Json, &output_options);
        let parsed_json: Value = serde_json::from_str(&json_out)
            .unwrap_or_else(|e| panic!("[redaction_options/{name}] invalid json output: {e}"));
        assert_eq!(parsed_json, *expected, "[redaction_options/{name}] json");

        if let Some(expected_yaml) = case.get("expected_yaml").and_then(Value::as_str) {
            assert_eq!(
                render(&case["input"], OutputFormat::Yaml, &output_options),
                expected_yaml,
                "[redaction_options/{name}] yaml"
            );
        }
        if let Some(expected_plain) = case.get("expected_plain").and_then(Value::as_str) {
            assert_eq!(
                render(&case["input"], OutputFormat::Plain, &output_options),
                expected_plain,
                "[redaction_options/{name}] plain"
            );
        }
    }
}

#[test]
fn test_security_fixtures() {
    let fixture = load_fixture("security.json");
    for case in fixture["redaction_cases"]
        .as_array()
        .expect("security redaction_cases must be an array")
    {
        let name = case["name"].as_str().expect("missing name");
        let redactor = redactor_from_case(case);
        let output_options = OutputOptions {
            redaction: redactor.clone(),
            style: PlainStyle::Readable,
        };
        let expected = &case["expected"];
        assert_eq!(
            redactor.value(&case["input"]),
            *expected,
            "[security/{name}] redacted value"
        );

        let mut outputs = vec![
            render(&case["input"], OutputFormat::Json, &output_options),
            render(&case["input"], OutputFormat::Plain, &output_options),
        ];
        outputs.push(render(&case["input"], OutputFormat::Yaml, &output_options));
        for output in outputs {
            for needle in case["must_contain"]
                .as_array()
                .expect("must_contain must be an array")
            {
                let needle = needle
                    .as_str()
                    .expect("must_contain entries must be strings");
                assert!(
                    output.contains(needle),
                    "[security/{name}] output missing {needle:?}: {output}"
                );
            }
            for needle in case["must_not_contain"]
                .as_array()
                .expect("must_not_contain must be an array")
            {
                let needle = needle
                    .as_str()
                    .expect("must_not_contain entries must be strings");
                assert!(
                    !output.contains(needle),
                    "[security/{name}] output leaked {needle:?}: {output}"
                );
            }
        }
    }
}

// ═══════════════════════════════════════════
// Generated property-style correctness tests
// ═══════════════════════════════════════════

#[derive(Clone, Copy)]
struct TestRng(u64);

impl TestRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn next_usize(&mut self, modulo: usize) -> usize {
        (self.next_u64() as usize) % modulo
    }
}

fn generated_property_value(seed: u64) -> Value {
    let mut rng = TestRng::new(seed ^ 0xA17D_A7A5_EED5_EED5);
    let rows = (0..(4 + rng.next_usize(12)))
        .map(|idx| {
            json!({
                "row_id": idx,
                "duration_ms": rng.next_usize(25_000),
                "payload_size_bytes": rng.next_usize(8_000_000),
                "created_at_epoch_ms": 1_738_886_400_000i64 + rng.next_usize(250_000) as i64,
                "price_usd_cents": rng.next_usize(50_000),
                "ratio_percent": (rng.next_usize(20_000) as f64) / 100.0,
                "visible_note": format!("visible-note-{seed}-{idx}"),
                "item_secret": format!("prop-secret-item-{seed}-{idx}"),
                "endpoint_url": format!(
                    "https://user:prop-secret-password-{seed}-{idx}@example.test/callback?trace={idx}&token_secret=prop-secret-query-{seed}-{idx}"
                ),
                "nested": {
                    "legacy_token": format!("prop-secret-legacy-{seed}-{idx}"),
                    "safe_url": format!("https://example.test/public/{seed}/{idx}"),
                    "array": [
                        idx,
                        format!("visible-array-{seed}-{idx}"),
                        {"deep_secret": format!("prop-secret-deep-{seed}-{idx}")}
                    ]
                }
            })
        })
        .collect::<Vec<_>>();

    json!({
        "batch_id": seed,
        "root_secret": format!("prop-secret-root-{seed}"),
        "legacy_token": format!("prop-secret-root-legacy-{seed}"),
        "rows": rows
    })
}

fn assert_no_property_secret(output: &str, context: &str) {
    assert!(
        !output.contains("prop-secret-"),
        "{context} leaked generated secret: {output}"
    );
}

#[test]
fn generated_outputs_are_deterministic_and_do_not_reintroduce_secrets() {
    let options = OutputOptions {
        redaction: Redactor::new().secret_names(vec!["legacy_token".to_string()]),
        style: PlainStyle::Readable,
    };

    for seed in 0..64 {
        let input = generated_property_value(seed);
        let json_a = render(&input, OutputFormat::Json, &options);
        let json_b = render(&input, OutputFormat::Json, &options);
        assert_eq!(json_a, json_b, "[seed {seed}] json output changed");
        assert_no_property_secret(&json_a, &format!("[seed {seed}] json"));

        let yaml_a = render(&input, OutputFormat::Yaml, &options);
        let yaml_b = render(&input, OutputFormat::Yaml, &options);
        assert_eq!(yaml_a, yaml_b, "[seed {seed}] yaml output changed");
        assert_no_property_secret(&yaml_a, &format!("[seed {seed}] yaml"));

        let plain_a = render(&input, OutputFormat::Plain, &options);
        let plain_b = render(&input, OutputFormat::Plain, &options);
        assert_eq!(plain_a, plain_b, "[seed {seed}] plain output changed");
        assert_no_property_secret(&plain_a, &format!("[seed {seed}] plain"));
    }
}

#[test]
fn generated_redaction_is_idempotent_for_copy_and_in_place_paths() {
    let redactor = Redactor::new().secret_names(vec!["legacy_token".to_string()]);

    for seed in 0..64 {
        let input = generated_property_value(seed);
        let first = redactor.value(&input);
        let second = redactor.value(&first);
        assert_eq!(
            first, second,
            "[seed {seed}] copy redaction is not idempotent"
        );

        let mut in_place = input.clone();
        redactor.redact_in_place(&mut in_place);
        let once = in_place.clone();
        redactor.redact_in_place(&mut in_place);
        assert_eq!(
            in_place, once,
            "[seed {seed}] in-place redaction is not idempotent"
        );
        assert_eq!(
            in_place, first,
            "[seed {seed}] copy and in-place redaction diverged"
        );

        let serialized =
            serde_json::to_string(&in_place).expect("redacted generated value must serialize");
        assert_no_property_secret(&serialized, &format!("[seed {seed}] redacted value"));
    }
}

#[test]
fn test_protocol_fixtures() {
    let cases = load_fixture("protocol.json");
    for case in cases.as_array().expect("protocol.json must be an array") {
        let name = case["name"].as_str().expect("missing name");
        if let Some(invalid) = case.get("invalid") {
            assert!(
                validate_protocol_event(invalid, false).is_err(),
                "[protocol/{name}] invalid event unexpectedly passed"
            );
            continue;
        }
        let typ = case["type"].as_str().expect("missing type");
        let args = &case["args"];
        let result = match typ {
            "result" => crate::protocol::json_result(args["result"].clone())
                .build()
                .into_value(),
            "result_trace" => crate::protocol::json_result(args["result"].clone())
                .trace(args["trace"].clone())
                .build()
                .into_value(),
            "error" => crate::protocol::json_error(
                args["code"].as_str().expect("missing code"),
                args["message"].as_str().expect("missing message"),
            )
            .build()
            .expect("builder failed")
            .into_value(),
            "error_trace" => crate::protocol::json_error(
                args["code"].as_str().expect("missing code"),
                args["message"].as_str().expect("missing message"),
            )
            .trace(args["trace"].clone())
            .build()
            .expect("builder failed")
            .into_value(),
            "error_hint" => crate::protocol::json_error(
                args["code"].as_str().expect("missing code"),
                args["message"].as_str().expect("missing message"),
            )
            .hint_if_some(args["hint"].as_str())
            .build()
            .expect("builder failed")
            .into_value(),
            "error_retryable" => crate::protocol::json_error(
                args["code"].as_str().expect("missing code"),
                args["message"].as_str().expect("missing message"),
            )
            .retryable_if(args["retryable"].as_bool().expect("retryable must be bool"))
            .build()
            .expect("builder failed")
            .into_value(),
            "error_extension_fields" => crate::protocol::json_error(
                args["code"].as_str().expect("missing code"),
                args["message"].as_str().expect("missing message"),
            )
            .fields(args["fields"].clone())
            .build()
            .expect("builder failed")
            .into_value(),
            "progress" => {
                let mut payload = serde_json::Map::new();
                payload.insert("message".to_string(), args["message"].clone());
                if let Some(fields) = args.get("fields").and_then(Value::as_object) {
                    for (k, v) in fields {
                        payload.insert(k.clone(), v.clone());
                    }
                }
                crate::protocol::json_progress(Value::Object(payload))
                    .build()
                    .into_value()
            }
            "log" => {
                let mut payload = serde_json::Map::new();
                payload.insert("level".to_string(), args["level"].clone());
                payload.insert("message".to_string(), args["message"].clone());
                if let Some(fields) = args.get("fields").and_then(Value::as_object) {
                    for (k, v) in fields {
                        payload.insert(k.clone(), v.clone());
                    }
                }
                crate::protocol::json_log(Value::Object(payload))
                    .build()
                    .into_value()
            }
            other => panic!("unknown protocol type: {other}"),
        };
        validate_protocol_event(&result, false)
            .unwrap_or_else(|err| panic!("[protocol/{name}] invalid event: {err}"));
        let expected = case.get("expected").expect("missing expected");
        assert_eq!(&result, expected, "[protocol/{name}]");
    }
}

#[test]
fn test_protocol_stream_fixtures() {
    let cases = load_fixture("protocol_streams.json");
    for case in cases
        .as_array()
        .expect("protocol_streams.json must be an array")
    {
        let name = case["name"].as_str().expect("missing name");
        let valid = case["valid"].as_bool().expect("missing valid");
        let events = case["events"]
            .as_array()
            .expect("events must be array")
            .to_vec();
        let result = validate_protocol_stream(&events, false);
        assert_eq!(
            result.is_ok(),
            valid,
            "[protocol_streams/{name}] got {result:?}"
        );
    }
}

#[test]
fn test_protocol_strict_fixtures() {
    let cases = load_fixture("protocol_strict.json");
    for case in cases
        .as_array()
        .expect("protocol_strict.json must be an array")
    {
        let name = case["name"].as_str().expect("missing name");
        let events = case["events"]
            .as_array()
            .expect("events must be array")
            .to_vec();
        assert_eq!(
            validate_protocol_stream(&events, true).is_ok(),
            case["valid"].as_bool().expect("missing valid"),
            "[protocol_strict/{name}]"
        );
    }
}

#[test]
fn test_error_builder_rejects_reserved_extension_fields() {
    // 0.16 API: reserved fields cause build error, not silent filtering
    let builder = crate::protocol::json_error("explicit", "message")
        .fields(json!({"code":"wrong","message":"wrong","hint":"wrong","detail":1}));
    // The fields call records the error internally
    let result = builder.build();
    assert!(
        result.is_err(),
        "builder should reject reserved field overwrite"
    );
    match result {
        Err(crate::protocol::BuildError::ReservedField(_)) => {
            // Expected
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn test_helper_fixtures() {
    let cases = load_fixture("helpers.json");
    for case in cases.as_array().expect("helpers.json must be an array") {
        let name = case["name"].as_str().expect("missing name");
        let test_cases = case["cases"].as_array().expect("missing cases");
        match name {
            "format_bytes_human" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_u64().expect("input must be u64");
                    let expected = arr[1].as_str().expect("expected must be string");
                    assert_eq!(
                        format_bytes_human(input),
                        expected,
                        "[helpers/format_bytes_human({input})]"
                    );
                }
            }
            "format_with_commas" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_u64().expect("input must be u64");
                    let expected = arr[1].as_str().expect("expected must be string");
                    assert_eq!(
                        format_with_commas(input),
                        expected,
                        "[helpers/format_with_commas({input})]"
                    );
                }
            }
            "extract_currency_code" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = if arr[1].is_null() {
                        None
                    } else {
                        arr[1].as_str()
                    };
                    assert_eq!(
                        extract_currency_code(input),
                        expected,
                        "[helpers/extract_currency_code({input})]"
                    );
                }
            }
            "normalize_utc_offset" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = if arr[1].is_null() {
                        None
                    } else {
                        arr[1].as_str().map(str::to_string)
                    };
                    assert_eq!(
                        normalize_utc_offset(input),
                        expected,
                        "[helpers/normalize_utc_offset({input:?})]"
                    );
                }
            }
            "is_valid_rfc3339_date" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = arr[1].as_bool().expect("expected must be bool");
                    assert_eq!(
                        is_valid_rfc3339_date(input),
                        expected,
                        "[helpers/is_valid_rfc3339_date({input:?})]"
                    );
                }
            }
            "is_valid_rfc3339_time" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = arr[1].as_bool().expect("expected must be bool");
                    assert_eq!(
                        is_valid_rfc3339_time(input),
                        expected,
                        "[helpers/is_valid_rfc3339_time({input:?})]"
                    );
                }
            }
            "is_valid_bcp47" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = arr[1].as_bool().expect("expected must be bool");
                    assert_eq!(
                        is_valid_bcp47(input),
                        expected,
                        "[helpers/is_valid_bcp47({input:?})]"
                    );
                }
            }
            "is_valid_rfc3339" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = arr[1].as_bool().expect("expected must be bool");
                    assert_eq!(
                        is_valid_rfc3339(input),
                        expected,
                        "[helpers/is_valid_rfc3339({input:?})]"
                    );
                }
            }
            other => panic!("unknown helper: {other}"),
        }
    }
}

#[test]
fn test_output_format_fixtures() {
    let cases = load_fixture("output_formats.json");
    for case in cases
        .as_array()
        .expect("output_formats.json must be an array")
    {
        let name = case["name"].as_str().expect("missing name");
        let input = case["input"].clone();
        let expected_json = case["expected_json"].clone();
        let expected_plain = case["expected_plain"]
            .as_str()
            .expect("expected_plain must be string");

        let json_out = render(&input, OutputFormat::Json, &OutputOptions::default());
        let parsed_json: Value = serde_json::from_str(&json_out)
            .unwrap_or_else(|e| panic!("[output/{name}] invalid json output: {e}"));
        assert_eq!(parsed_json, expected_json, "[output/{name}] json mismatch");

        let expected_yaml = case["expected_yaml"]
            .as_str()
            .expect("expected_yaml must be string");
        let yaml_out = render(&input, OutputFormat::Yaml, &OutputOptions::default());
        assert_eq!(yaml_out, expected_yaml, "[output/{name}] yaml mismatch");

        let plain_out = render(&input, OutputFormat::Plain, &OutputOptions::default());
        assert_eq!(plain_out, expected_plain, "[output/{name}] plain mismatch");
    }
}

// ═══════════════════════════════════════════
// render: OutputFormat::Json
// ═══════════════════════════════════════════

#[test]
fn json_single_line() {
    let out = render(
        &json!({"a": 1, "b": {"c": 2}}),
        OutputFormat::Json,
        &OutputOptions::default(),
    );
    assert!(!out.contains('\n'));
}

#[test]
fn json_secrets_redacted() {
    let out = render(
        &json!({"api_key_secret": "sk-123", "name": "test"}),
        OutputFormat::Json,
        &OutputOptions::default(),
    );
    assert!(out.contains("\"***\""));
    assert!(!out.contains("sk-123"));
    assert!(out.contains("\"name\""));
}

#[test]
fn json_nested_secrets_redacted() {
    let out = render(
        &json!({"config": {"password_secret": "real"}}),
        OutputFormat::Json,
        &OutputOptions::default(),
    );
    assert!(!out.contains("real"));
    assert!(out.contains("***"));
}

#[test]
fn json_original_keys_preserved() {
    let out = render(
        &json!({"duration_ms": 1280}),
        OutputFormat::Json,
        &OutputOptions::default(),
    );
    assert!(out.contains("\"duration_ms\""));
    assert!(out.contains("1280"));
    assert!(!out.contains("\"duration\":"));
}

#[test]
fn json_raw_values_not_formatted() {
    let out = render(
        &json!({"size_bytes": 5242880}),
        OutputFormat::Json,
        &OutputOptions::default(),
    );
    assert!(out.contains("5242880"));
    assert!(!out.contains("MiB"));
}

#[test]
fn json_non_string_secret_redacted() {
    let out = render(
        &json!({"count_secret": 42}),
        OutputFormat::Json,
        &OutputOptions::default(),
    );
    assert!(out.contains("\"***\""));
    assert!(!out.contains("42"));
}

#[test]
fn json_with_trace_only_redacts_trace_only() {
    let out = render(
        &json!({
            "code": "ok",
            "result": {"api_key_secret": "sk-live-123"},
            "trace": {"request_secret": "top-secret"}
        }),
        OutputFormat::Json,
        &RedactionPolicy::TraceOnly.into(),
    );
    assert!(out.contains("\"request_secret\":\"***\""));
    assert!(out.contains("\"api_key_secret\":\"sk-live-123\""));
}

#[test]
fn json_with_none_keeps_secret_values() {
    let out = render(
        &json!({"api_key_secret": "sk-live-123"}),
        OutputFormat::Json,
        &RedactionPolicy::Off.into(),
    );
    assert!(out.contains("\"api_key_secret\":\"sk-live-123\""));
    assert!(!out.contains("\"***\""));
}

#[test]
fn redaction_policy_into_redactor() {
    let redactor: Redactor = RedactionPolicy::TraceOnly.into();
    assert_eq!(redactor, Redactor::new().policy(RedactionPolicy::TraceOnly));
}

#[test]
fn redaction_policy_into_output_options() {
    let options: OutputOptions = RedactionPolicy::Off.into();
    assert_eq!(
        options.redaction,
        Redactor::new().policy(RedactionPolicy::Off)
    );
    assert_eq!(options.style, PlainStyle::Readable);
}

#[test]
fn redacted_value_returns_safe_copy() {
    let input = json!({"api_key_secret": "sk-live-123", "nested": {"token_secret": "tok"}});
    let got = redacted_value(&input);
    assert_eq!(got["api_key_secret"], "***");
    assert_eq!(got["nested"]["token_secret"], "***");
    assert_eq!(input["api_key_secret"], "sk-live-123");
}

#[test]
fn redacted_value_redacts_secret_subtree_by_default() {
    let input = json!({"db_secret": {"password_secret": "real", "host": "localhost"}});
    let default = redacted_value(&input);
    assert_eq!(default["db_secret"], "***");
}

#[test]
fn max_depth_marker_is_not_secret_redaction_marker() {
    let mut input = json!("leaf");
    for _ in 0..300 {
        input = json!({"next": input});
    }
    let out = render(&input, OutputFormat::Json, &OutputOptions::default());
    assert!(out.contains("<afdata:max-depth>"), "{out}");
    assert!(!out.contains("***"), "{out}");
}

#[test]
fn json_default_output_redacts_secrets() {
    let out = render(
        &json!({"api_key_secret": "sk-live-123"}),
        OutputFormat::Json,
        &OutputOptions::default(),
    );
    assert!(out.contains("\"api_key_secret\":\"***\""));
}

// ═══════════════════════════════════════════
// render: OutputFormat::Yaml — structure-preserving (same semantics as JSON)
// ═══════════════════════════════════════════
mod yaml_output_tests {
    use super::*;

    #[test]
    fn yaml_starts_with_separator() {
        let out = render(
            &json!({"a": 1}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.starts_with("---\n"));
    }

    #[test]
    fn yaml_keeps_suffix_keys_unstripped() {
        // No suffix is stripped from any key: unlike `plain`, YAML never applies
        // suffix-driven key rewriting.
        let out = render(
            &json!({
                "duration_ms": 42,
                "timeout_s": 30,
                "gc_pause_ns": 450000,
                "query_us": 830,
                "file_size_bytes": 5242880,
                "cpu_percent": 85,
                "balance_msats": 50000,
                "withdrawn_sats": 1234,
                "reserve_btc": 0.5,
                "price_usd_cents": 999,
                "price_eur_cents": 850,
                "price_jpy": 1500,
                "fare_thb_cents": 15050,
                "timeout_minutes": 30,
                "validity_hours": 24,
                "cert_days": 365,
                "created_at_epoch_ms": 1738886400000i64,
                "cached_epoch_s": 1738886400,
                "created_epoch_ns": "1707868800000000000",
                "expires_rfc3339": "2026-02-14T10:30:00Z",
                "api_key_secret": "sk-123",
                "user_id": 123,
                "config_path": "a.yml"
            }),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        for key in [
            "duration_ms",
            "timeout_s",
            "gc_pause_ns",
            "query_us",
            "file_size_bytes",
            "cpu_percent",
            "balance_msats",
            "withdrawn_sats",
            "reserve_btc",
            "price_usd_cents",
            "price_eur_cents",
            "price_jpy",
            "fare_thb_cents",
            "timeout_minutes",
            "validity_hours",
            "cert_days",
            "created_at_epoch_ms",
            "cached_epoch_s",
            "created_epoch_ns",
            "expires_rfc3339",
            "api_key_secret",
            "user_id",
            "config_path",
        ] {
            assert!(out.contains(&format!("{key}:")), "missing key {key}: {out}");
        }
        // The secret value is still redacted; only its key is left alone.
        assert!(out.contains("api_key_secret: \"***\""));
        assert!(!out.contains("sk-123"));
    }

    #[test]
    fn yaml_strip_uppercase_secret_key_unstripped() {
        let out = render(
            &json!({"DATABASE_URL_SECRET": "postgres://..."}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("DATABASE_URL_SECRET: \"***\""));
        assert!(!out.contains("postgres://..."));
    }

    #[test]
    fn yaml_uppercase_suffix_key_unstripped() {
        let out = render(
            &json!({"CACHE_TTL_S": 3600}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("CACHE_TTL_S: 3600"));
    }

    #[test]
    fn yaml_no_suffix_keys_pass_through() {
        let out = render(
            &json!({"user_id": 123, "config_path": "a.yml"}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("user_id: 123"));
        assert!(out.contains("config_path: \"a.yml\""));
    }

    #[test]
    fn yaml_raw_keeps_suffix_keys_and_structure() {
        let options = OutputOptions {
            redaction: Redactor::new().policy(RedactionPolicy::TraceOnly),
            style: PlainStyle::Raw,
        };
        let out = render(
            &json!({
                "code": "result",
                "rows": [{"api_key_secret": "sk-live-1", "duration_ms": 42}],
                "trace": {"request_secret": "top-secret"}
            }),
            OutputFormat::Yaml,
            &options,
        );

        assert!(out.contains("rows:\n  -"));
        assert!(out.contains("api_key_secret: \"sk-live-1\""));
        assert!(out.contains("duration_ms: 42"));
        assert!(out.contains("request_secret: \"***\""));
        assert!(!out.contains("duration: \"42ms\""));
        assert!(!out.contains("rows: \"["));
    }

    #[test]
    fn yaml_ignores_output_style() {
        // Unlike `plain`, YAML renders identically regardless of `PlainStyle`: it
        // is always structure-preserving.
        let base_redaction = Redactor::new().policy(RedactionPolicy::Off);
        let value = json!({"duration_ms": 42, "name": "alice"});
        let readable = render(
            &value,
            OutputFormat::Yaml,
            &OutputOptions {
                redaction: base_redaction.clone(),
                style: PlainStyle::Readable,
            },
        );
        let raw = render(
            &value,
            OutputFormat::Yaml,
            &OutputOptions {
                redaction: base_redaction,
                style: PlainStyle::Raw,
            },
        );
        assert_eq!(readable, raw);
        assert!(readable.contains("duration_ms: 42"));
        assert!(!readable.contains("42ms"));
    }

    #[test]
    fn yaml_key_collision_keeps_originals() {
        let out = render(
            &json!({"response_ms": 150, "response_s": 1}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("response_ms: 150"));
        assert!(out.contains("response_s: 1"));
    }

    #[test]
    fn yaml_numbers_stay_raw_numbers_not_formatted() {
        // Every one of these would be reformatted into a human string by `plain`;
        // YAML must keep them as plain JSON-equivalent numbers.
        let out = render(
            &json!({
                "duration_ms": 1280,
                "cpu_percent": 85,
                "price_usd_cents": 9999,
                "file_size_bytes": 5242880,
                "balance_msats": 50000000
            }),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("duration_ms: 1280"));
        assert!(out.contains("cpu_percent: 85"));
        assert!(out.contains("price_usd_cents: 9999"));
        assert!(out.contains("file_size_bytes: 5242880"));
        assert!(out.contains("balance_msats: 50000000"));
        for lossy in ["1.28s", "85%", "$99.99", "5.0MiB", "50000000msats"] {
            assert!(
                !out.contains(lossy),
                "unexpected lossy formatting {lossy}: {out}"
            );
        }
    }

    #[test]
    fn yaml_epoch_and_rfc3339_stay_as_written_not_reformatted() {
        let out = render(
            &json!({
                "created_at_epoch_ms": 1738886400000i64,
                "expires_rfc3339": "2026-02-14T10:30:00Z"
            }),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("created_at_epoch_ms: 1738886400000"));
        assert!(out.contains("expires_rfc3339: \"2026-02-14T10:30:00Z\""));
        assert!(!out.contains("2025-02-07T00:00:00.000Z"));
    }

    #[test]
    fn yaml_fmt_secret() {
        let out = render(
            &json!({"api_key_secret": "sk-1234567890abcdef"}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("\"***\""));
        assert!(!out.contains("sk-1234567890abcdef"));
    }

    #[test]
    fn yaml_strings_always_quoted() {
        let out = render(
            &json!({"name": "alice"}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("\"alice\""));
    }

    #[test]
    fn yaml_numbers_unquoted() {
        let out = render(
            &json!({"count": 42}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("count: 42"));
        assert!(!out.contains("\"42\""));
    }

    #[test]
    fn yaml_nested_keys_not_stripped() {
        let out = render(
            &json!({
                "config": {
                    "api_key_secret": "sk-123",
                    "timeout_s": 30
                }
            }),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        assert!(out.contains("config:"));
        assert!(out.contains("  api_key_secret: \"***\""));
        assert!(out.contains("  timeout_s: 30"));
    }

    #[test]
    fn yaml_stream_of_records_has_stable_separator_framing() {
        // Simulates how a CLI streams multiple AFDATA records: each record is
        // rendered independently and concatenated. `---` framing must stay
        // stable and each record's raw keys must stay intact and in order.
        let first = render(
            &json!({"kind": "log", "duration_ms": 1}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        let second = render(
            &json!({"kind": "result", "duration_ms": 2}),
            OutputFormat::Yaml,
            &OutputOptions::default(),
        );
        let stream = format!("{first}\n{second}\n");

        assert_eq!(stream.matches("---").count(), 2);
        let first_idx = stream.find("duration_ms: 1").expect("first record present");
        let second_idx = stream
            .find("duration_ms: 2")
            .expect("second record present");
        assert!(first_idx < second_idx, "records out of order: {stream}");
    }
}

// ═══════════════════════════════════════════
// Key collision detection
// ═══════════════════════════════════════════

#[test]
fn plain_key_collision_keeps_originals() {
    let out = render(
        &json!({"response_ms": 150, "response_s": 1}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("response_ms=150"));
    assert!(out.contains("response_s=1"));
}

#[test]
fn plain_raw_keeps_suffix_keys_and_redacts_trace() {
    let options = OutputOptions {
        redaction: Redactor::new().policy(RedactionPolicy::TraceOnly),
        style: PlainStyle::Raw,
    };
    let out = render(
        &json!({
            "duration_ms": 42,
            "trace": {"request_secret": "top-secret"}
        }),
        OutputFormat::Plain,
        &options,
    );
    assert!(out.contains("duration_ms=42"));
    assert!(out.contains("trace.request_secret=***"));
    assert!(!out.contains("duration=42ms"));
}

// ═══════════════════════════════════════════
// render: OutputFormat::Plain — logfmt format
// ═══════════════════════════════════════════

#[test]
fn plain_single_line() {
    let out = render(
        &json!({"a": 1, "b": 2, "c": 3}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(!out.contains('\n'));
}

#[test]
fn plain_key_value_pair() {
    let out = render(
        &json!({"user_id": 123}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "user_id=123");
}

#[test]
fn plain_sorted_keys() {
    let out = render(
        &json!({"z": 1, "a": 2, "m": 3}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "a=2 m=3 z=1");
}

#[test]
fn plain_dot_notation_nesting() {
    let out = render(
        &json!({"trace": {"duration_ms": 150, "source": "db"}}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("trace.duration=150ms"));
    assert!(out.contains("trace.source=db"));
}

#[test]
fn plain_sorted_by_dot_path() {
    let out = render(
        &json!({
            "kind": "result",
            "result": {"count": 3},
            "trace": {"duration_ms": 12}
        }),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "kind=result result.count=3 trace.duration=12ms");
}

#[test]
fn plain_quoted_spaces() {
    let out = render(
        &json!({"message": "uploading chunks"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("message=\"uploading chunks\""));
}

#[test]
fn plain_arrays_comma_joined() {
    let out = render(
        &json!({"fields": ["email", "age"]}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("fields=email,age"));
}

#[test]
fn plain_null_empty() {
    let out = render(
        &json!({"RUST_LOG": null}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("RUST_LOG="));
}

#[test]
fn plain_key_stripping_and_formatting() {
    let out = render(
        &json!({"duration_ms": 1280, "api_key_secret": "sk-123"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "api_key=*** duration=1.28s");
}

#[test]
fn plain_deep_nesting() {
    let out = render(
        &json!({"a": {"b": {"c": "deep"}}}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "a.b.c=deep");
}

#[test]
fn plain_secrets_redacted() {
    let out = render(
        &json!({"api_key_secret": "real-key"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("api_key=***"));
    assert!(!out.contains("real-key"));
}

#[test]
fn plain_empty_object() {
    let out = render(&json!({}), OutputFormat::Plain, &OutputOptions::default());
    assert_eq!(out, "");
}

#[test]
fn plain_bool_unquoted() {
    let out = render(
        &json!({"active": true, "disabled": false}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "active=true disabled=false");
}

#[test]
fn plain_nested_secrets() {
    let out = render(
        &json!({"config": {"api_key_secret": "real", "host": "localhost"}}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("config.api_key=***"));
    assert!(out.contains("config.host=localhost"));
    assert!(!out.contains("real"));
}

// ═══════════════════════════════════════════
// Type constraint fall-through
// Wrong type -> raw value with ORIGINAL key
// ═══════════════════════════════════════════

#[test]
fn fallthrough_bytes_float() {
    let out = render(
        &json!({"size_bytes": 1024.5}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "size_bytes=1024.5");
}

#[test]
fn fallthrough_bytes_string() {
    let out = render(
        &json!({"size_bytes": "unknown"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "size_bytes=unknown");
}

#[test]
fn fallthrough_bytes_bool() {
    let out = render(
        &json!({"size_bytes": false}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "size_bytes=false");
}

#[test]
fn fallthrough_epoch_ms_float() {
    let out = render(
        &json!({"created_epoch_ms": 1707868800000.5}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "created_epoch_ms=1707868800000.5");
}

#[test]
fn fallthrough_epoch_ms_bool() {
    let out = render(
        &json!({"created_epoch_ms": true}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "created_epoch_ms=true");
}

#[test]
fn fallthrough_epoch_ms_string() {
    let out = render(
        &json!({"created_epoch_ms": "yesterday"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "created_epoch_ms=yesterday");
}

#[test]
fn fallthrough_ms_string() {
    let out = render(
        &json!({"latency_ms": "fast"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "latency_ms=fast");
}

#[test]
fn fallthrough_ms_bool() {
    let out = render(
        &json!({"latency_ms": true}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "latency_ms=true");
}

#[test]
fn fallthrough_s_string() {
    let out = render(
        &json!({"dns_ttl_s": "auto"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "dns_ttl_s=auto");
}

#[test]
fn fallthrough_usd_cents_negative() {
    let out = render(
        &json!({"refund_usd_cents": -499}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "refund_usd_cents=-499");
}

#[test]
fn fallthrough_eur_cents_negative() {
    let out = render(
        &json!({"refund_eur_cents": -100}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "refund_eur_cents=-100");
}

#[test]
fn fallthrough_jpy_negative() {
    let out = render(
        &json!({"refund_jpy": -1500}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "refund_jpy=-1500");
}

#[test]
fn fallthrough_percent_string() {
    let out = render(
        &json!({"cpu_percent": "high"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "cpu_percent=high");
}

#[test]
fn fallthrough_percent_bool() {
    let out = render(
        &json!({"cpu_percent": true}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "cpu_percent=true");
}

#[test]
fn fallthrough_btc_string() {
    let out = render(
        &json!({"reserve_btc": "pending"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "reserve_btc=pending");
}

#[test]
fn fallthrough_msats_string() {
    let out = render(
        &json!({"cost_msats": "pending"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "cost_msats=pending");
}

#[test]
fn fallthrough_minutes_string() {
    let out = render(
        &json!({"timeout_minutes": "infinite"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "timeout_minutes=infinite");
}

// ═══════════════════════════════════════════
// Case sensitivity
// Only _secret/_SECRET, NOT _Secret/_sEcReT
// ═══════════════════════════════════════════

#[test]
fn case_lowercase_secret() {
    let out = render(
        &json!({"api_key_secret": "real"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("***"));
    assert!(!out.contains("real"));
}

#[test]
fn case_uppercase_secret() {
    let out = render(
        &json!({"DATABASE_URL_SECRET": "postgres://..."}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("DATABASE_URL=***"));
}

#[test]
fn case_mixed_secret_not_matched() {
    let out = render(
        &json!({"api_key_Secret": "real"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("api_key_Secret=real"));
    assert!(!out.contains("***"));
}

#[test]
fn case_uppercase_s() {
    let out = render(
        &json!({"CACHE_TTL_S": 3600}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("CACHE_TTL=3600s"));
}

#[test]
fn case_mixed_ms_not_matched() {
    let out = render(
        &json!({"latency_Ms": 42}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "latency_Ms=42");
}

// ═══════════════════════════════════════════
// Negative epoch timestamps
// ═══════════════════════════════════════════

#[test]
fn negative_epoch_ms() {
    let out = render(
        &json!({"created_epoch_ms": -1000}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "created=1969-12-31T23:59:59.000Z");
}

#[test]
fn negative_epoch_s() {
    let out = render(
        &json!({"cached_epoch_s": -60}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "cached=1969-12-31T23:59:00.000Z");
}

#[test]
fn negative_epoch_ns() {
    let out = render(
        &json!({"created_epoch_ns": "-60000000000"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "created=1969-12-31T23:59:00.000Z");
}

#[test]
fn negative_epoch_ns_minus_one() {
    let out = render(
        &json!({"created_epoch_ns": "-1"}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "created=1969-12-31T23:59:59.999Z");
}

// ═══════════════════════════════════════════
// Negative bytes
// ═══════════════════════════════════════════

#[test]
fn negative_bytes_small() {
    let out = render(
        &json!({"delta_bytes": -100}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "delta_bytes=-100");
}

#[test]
fn negative_bytes_mb() {
    let out = render(
        &json!({"delta_bytes": -5242880}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "delta_bytes=-5242880");
}

// ═══════════════════════════════════════════
// Duration _ms boundary
// ═══════════════════════════════════════════

#[test]
fn ms_boundary_999() {
    let out = render(
        &json!({"latency_ms": 999}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "latency=999ms");
}

#[test]
fn ms_boundary_1000() {
    let out = render(
        &json!({"latency_ms": 1000}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "latency=1.0s");
}

#[test]
fn ms_boundary_1001() {
    let out = render(
        &json!({"latency_ms": 1001}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "latency=1.001s");
}

#[test]
fn ms_float_small() {
    let out = render(
        &json!({"latency_ms": 0.5}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "latency=0.5ms");
}

#[test]
fn ms_zero() {
    let out = render(
        &json!({"latency_ms": 0}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "latency=0ms");
}

// ═══════════════════════════════════════════
// Zero values
// ═══════════════════════════════════════════

#[test]
fn zero_bytes() {
    let out = render(
        &json!({"size_bytes": 0}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "size=0B");
}

#[test]
fn zero_percent() {
    let out = render(
        &json!({"cpu_percent": 0}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "cpu=0%");
}

#[test]
fn zero_usd_cents() {
    let out = render(
        &json!({"price_usd_cents": 0}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "price=$0.00");
}

#[test]
fn zero_s() {
    let out = render(
        &json!({"timeout_s": 0}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "timeout=0s");
}

// ═══════════════════════════════════════════
// Suffix priority (longest match first)
// ═══════════════════════════════════════════

#[test]
fn suffix_priority_epoch_ms_over_ms() {
    let out = render(
        &json!({"created_at_epoch_ms": 1738886400000i64}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("2025-02-07"));
    assert!(!out.contains("ms"));
}

#[test]
fn suffix_priority_usd_cents_over_s() {
    let out = render(
        &json!({"price_usd_cents": 999}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "price=$9.99");
}

#[test]
fn suffix_priority_msats_over_s() {
    let out = render(
        &json!({"cost_msats": 2056}),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert_eq!(out, "cost=2056msats");
}

// ═══════════════════════════════════════════
// redacted_value
// ═══════════════════════════════════════════

#[test]
fn redact_flat() {
    let v = json!({"api_key_secret": "sk-123", "name": "test"});
    let redacted = redacted_value(&v);
    assert_eq!(redacted["api_key_secret"], "***");
    assert_eq!(redacted["name"], "test");
}

#[test]
fn redact_nested() {
    let v = json!({"config": {"password_secret": "real"}});
    let redacted = redacted_value(&v);
    assert_eq!(redacted["config"]["password_secret"], "***");
}

#[test]
fn redact_array_traversal() {
    let v = json!([{"api_key_secret": "a"}, {"token_secret": "b"}]);
    let redacted = redacted_value(&v);
    assert_eq!(redacted[0]["api_key_secret"], "***");
    assert_eq!(redacted[1]["token_secret"], "***");
}

#[test]
fn redact_non_string_redacted() {
    let v = json!({"count_secret": 42});
    let redacted = redacted_value(&v);
    assert_eq!(redacted["count_secret"], "***");
}

// ═══════════════════════════════════════════
// CLI helpers
// ═══════════════════════════════════════════

#[test]
fn cli_parse_output_all_formats() {
    assert!(matches!(cli_parse_output("json"), Ok(OutputFormat::Json)));
    assert!(matches!(cli_parse_output("yaml"), Ok(OutputFormat::Yaml)));
    assert!(matches!(cli_parse_output("plain"), Ok(OutputFormat::Plain)));
}

#[test]
fn cli_parse_output_rejects_unknown() {
    assert!(cli_parse_output("xml").is_err());
    assert!(cli_parse_output("JSON").is_err());
    assert!(cli_parse_output("").is_err());
}

#[test]
fn cli_parse_output_error_message_contains_value() {
    let e = cli_parse_output("toml").unwrap_err();
    assert!(e.contains("toml"));
    assert!(e.contains("json"));
}

#[test]
fn cli_parse_log_filters_trims_and_lowercases() {
    let f = cli_parse_log_filters(&["  Query  ", "ERROR"]);
    assert_eq!(f.as_slice(), &["query", "error"]);
}

#[test]
fn cli_parse_log_filters_deduplicates() {
    let f = cli_parse_log_filters(&["query", "error", "Query", "query"]);
    assert_eq!(f.as_slice(), &["query", "error"]);
}

#[test]
fn cli_parse_log_filters_removes_empty() {
    let f = cli_parse_log_filters(&["", "query", "  "]);
    assert_eq!(f.as_slice(), &["query"]);
}

#[test]
fn cli_parse_log_filters_empty_slice() {
    let f = cli_parse_log_filters::<String>(&[]);
    assert!(f.is_empty());
}

#[test]
fn cli_parse_log_filters_preserves_order() {
    let f = cli_parse_log_filters(&["startup", "request", "retry"]);
    assert_eq!(f.as_slice(), &["startup", "request", "retry"]);
}

#[test]
fn build_cli_error_required_fields() {
    let v = build_cli_error("missing --sql", None).into_value();
    assert_eq!(v["kind"], "error");
    assert_eq!(v["error"]["code"], "cli_error");
    assert_eq!(v["error"]["message"], "missing --sql");
    assert_eq!(v["error"]["retryable"], false);
    assert!(v.get("error_code").is_none());
    assert!(v.get("retryable").is_none());
    assert!(v["trace"].is_object());
    validate_protocol_event(&v, true).expect("strict CLI error");
}

#[test]
fn build_cli_error_is_valid_json() {
    let v = build_cli_error("oops", None);
    assert!(serde_json::to_string(&v).is_ok());
}

#[test]
fn build_cli_error_with_hint() {
    let v = build_cli_error("bad flag", Some("try --help")).into_value();
    assert_eq!(v["kind"], "error");
    assert_eq!(v["error"]["hint"], "try --help");
}

#[test]
fn build_cli_error_without_hint_has_no_hint_key() {
    let v = build_cli_error("oops", None);
    assert!(v.as_value()["error"].get("hint").is_none());
}

#[test]
fn render_dispatches_json() {
    let v = crate::protocol::json_result(json!({"size_bytes": 1024})).build();
    let out = render(v.as_value(), OutputFormat::Json, &OutputOptions::default());
    assert!(out.contains("size_bytes")); // json: raw keys, no suffix processing
    assert!(!out.contains('\n'));
}

#[test]
fn render_dispatches_yaml() {
    let v = crate::protocol::json_result(json!({"size_bytes": 1024})).build();
    let out = render(v.as_value(), OutputFormat::Yaml, &OutputOptions::default());
    assert!(out.starts_with("---"));
    assert!(out.contains("size_bytes: 1024")); // yaml: structure-preserving, like json
}

#[test]
fn render_dispatches_plain() {
    let v = crate::protocol::json_result(json!({"size_bytes": 1024})).build();
    let out = render(v.as_value(), OutputFormat::Plain, &OutputOptions::default());
    assert!(!out.contains('\n'));
    assert!(out.contains("kind=result"));
    assert!(out.contains("result.size=1.0KiB")); // plain: suffix processed
}

#[test]
fn cli_emitter_writes_events_and_tracks_terminal() {
    let mut emitter = CliEmitter::new(Vec::new(), OutputFormat::Json);
    emitter
        .emit_log(LogLevel::Info, "startup")
        .expect("log emit");
    emitter
        .emit_result(json!({"rows": 2}))
        .expect("result emit");
    let out = String::from_utf8(emitter.into_inner()).expect("utf8");
    let lines = out.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(lines[0]).expect("log json")["kind"],
        "log"
    );
    assert_eq!(
        serde_json::from_str::<Value>(lines[1]).expect("result json")["kind"],
        "result"
    );
}

#[test]
fn cli_emitter_frames_all_formats() {
    let formats = [OutputFormat::Json, OutputFormat::Plain, OutputFormat::Yaml];

    for format in formats {
        let mut emitter = CliEmitter::new(Vec::new(), format);
        emitter
            .emit_log(LogLevel::Info, "startup")
            .expect("log emit");
        emitter
            .emit_result(json!({"rows": 2}))
            .expect("result emit");
        let out = String::from_utf8(emitter.into_inner()).expect("utf8");
        match format {
            OutputFormat::Json => {
                let lines = out.lines().collect::<Vec<_>>();
                assert_eq!(lines.len(), 2);
                let kinds = lines
                    .iter()
                    .map(|line| serde_json::from_str::<Value>(line).expect("json")["kind"].clone())
                    .collect::<Vec<_>>();
                assert_eq!(kinds, vec![json!("log"), json!("result")]);
            }
            OutputFormat::Plain => {
                let lines = out.lines().collect::<Vec<_>>();
                assert_eq!(lines.len(), 2);
                assert!(lines[0].starts_with("kind=log"), "{out}");
                assert!(lines[1].starts_with("kind=result"), "{out}");
            }
            OutputFormat::Yaml => {
                assert_eq!(out.matches("---").count(), 2, "{out}");
            }
        }
    }
}

#[test]
fn cli_emitter_rejects_duplicate_terminal() {
    let mut emitter = CliEmitter::new(Vec::new(), OutputFormat::Json);
    emitter
        .emit_result(json!({"rows": 2}))
        .expect("result emit");
    let err = emitter
        .emit_error("late_error", "too late")
        .expect_err("duplicate terminal must fail");
    assert!(err.to_string().contains("duplicate terminal"));
}

#[test]
fn cli_emitter_rejects_non_terminal_after_terminal() {
    let mut emitter = CliEmitter::new(Vec::new(), OutputFormat::Json);
    emitter
        .emit_result(json!({"rows": 2}))
        .expect("result emit");
    let err = emitter
        .emit_progress("progress after terminal")
        .expect_err("progress after terminal must fail");
    assert!(err.to_string().contains("after terminal"));
}

struct FailingWriter;

impl std::io::Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "closed",
        ))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn cli_emitter_returns_writer_errors() {
    let mut emitter = CliEmitter::new(FailingWriter, OutputFormat::Json);
    let err = emitter
        .emit_result(json!({"rows": 2}))
        .expect_err("writer failure must be returned");
    assert!(err.to_string().contains("failed to write"));
    assert_eq!(err.io_error_kind(), Some(std::io::ErrorKind::BrokenPipe));
    assert!(std::error::Error::source(&err).is_some());
}

struct OtherFailWriter;

impl std::io::Write for OtherFailWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("boom"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn cli_emitter_finish_maps_outcomes_to_exit_codes() {
    // Success writes the terminal event and returns the caller's code.
    let mut ok = CliEmitter::new(Vec::new(), OutputFormat::Json);
    assert_eq!(ok.finish_result(json!({"ok": true})), 0);
    assert!(
        String::from_utf8(ok.into_inner())
            .expect("utf8")
            .contains("\"kind\":\"result\"")
    );

    // A rich error is built with the json_error builder (the error "type") and
    // handed to finish — no separate error-emitting convenience method.
    let mut err = CliEmitter::new(Vec::new(), OutputFormat::Json);
    let event = crate::protocol::json_error("bad_thing", "nope")
        .hint("try again")
        .build()
        .expect("valid error");
    assert_eq!(err.finish(event, 2), 2);
    let out = String::from_utf8(err.into_inner()).expect("utf8");
    assert!(out.contains("\"code\":\"bad_thing\""), "{out}");
    assert!(out.contains("\"hint\":\"try again\""), "{out}");

    // A broken pipe (reader hung up) maps to 0, not a failure code.
    let mut broken = CliEmitter::new(FailingWriter, OutputFormat::Json);
    let event = crate::protocol::json_error("x", "y")
        .build()
        .expect("valid error");
    assert_eq!(broken.finish(event, 2), 0);

    // Any other write failure maps to 4.
    let mut other = CliEmitter::new(OtherFailWriter, OutputFormat::Json);
    assert_eq!(other.finish_result(json!({})), 4);
}

struct FailOnceWriter {
    failed: bool,
    bytes: Vec<u8>,
}

impl std::io::Write for FailOnceWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if !self.failed {
            self.failed = true;
            return Err(std::io::Error::other("retry"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn cli_emitter_does_not_commit_terminal_state_when_write_fails() {
    let writer = FailOnceWriter {
        failed: false,
        bytes: Vec::new(),
    };
    let mut emitter = CliEmitter::new(writer, OutputFormat::Json);
    emitter
        .emit_result(json!({"rows": 2}))
        .expect_err("first write must fail");
    emitter
        .emit_result(json!({"rows": 2}))
        .expect("terminal event should remain retryable");
    let output = String::from_utf8(emitter.into_inner().bytes).expect("utf8");
    assert_eq!(output.lines().count(), 1);
}

#[test]
fn build_cli_version_has_standard_shape() {
    let v = build_cli_version(
        "agent-cli",
        Some("Agent CLI Example"),
        "1.2.3",
        Some("abc1234"),
    );
    assert_eq!(v.as_value()["kind"], "result");
    assert_eq!(v.as_value()["result"]["code"], "version");
    assert_eq!(v.as_value()["result"]["name"], "agent-cli");
    assert_eq!(v.as_value()["result"]["display_name"], "Agent CLI Example");
    assert_eq!(v.as_value()["result"]["version"], "1.2.3");
    assert_eq!(v.as_value()["result"]["build"], "abc1234");
    assert!(v.as_value().get("trace").is_some());
}

#[test]
fn build_cli_version_omits_absent_display_name_and_build() {
    let v = build_cli_version("agent-cli", None, "1.2.3", None);
    let result = &v.as_value()["result"];
    assert_eq!(result["name"], "agent-cli");
    assert_eq!(result["version"], "1.2.3");
    assert!(result.get("display_name").is_none());
    assert!(result.get("build").is_none());
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
fn version_test_command() -> clap::Command {
    clap::Command::new("agent-cli")
        .arg(
            clap::Arg::new("stdout-file")
                .long("stdout-file")
                .action(clap::ArgAction::Set),
        )
        .arg(
            clap::Arg::new("stderr-file")
                .long("stderr-file")
                .action(clap::ArgAction::Set),
        )
        .subcommand(clap::Command::new("hatch"))
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_bare_defaults_to_json() {
    // The one blessed behavior: `--version` always answers with a protocol-v1
    // event, JSON by default — no more conventional bare-text special case.
    let raw = vec!["agent-cli".to_string(), "--version".to_string()];
    let out = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        Some("Agent CLI Example"),
        "1.2.3",
        None,
    )
    .expect("valid version request")
    .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["kind"], "result");
    assert_eq!(parsed["result"]["code"], "version");
    assert_eq!(parsed["result"]["name"], "agent-cli");
    assert_eq!(parsed["result"]["display_name"], "Agent CLI Example");
    assert_eq!(parsed["result"]["version"], "1.2.3");
    assert_eq!(parsed["trace"], serde_json::json!({}));
    validate_protocol_event(&parsed, true).expect("strict protocol event");
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_honors_explicit_plain_output() {
    let raw = vec![
        "agent-cli".to_string(),
        "--version".to_string(),
        "--output".to_string(),
        "plain".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        None,
        "1.2.3",
        None,
    )
    .expect("valid version request")
    .expect("version should render");
    assert!(out.contains("kind=result"), "{out}");
    assert!(out.contains("result.version=1.2.3"), "{out}");
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_skips_output_to_space_value() {
    // A preceding `--output-to <value>` (space form) must not be mistaken for
    // the subcommand boundary; the later `--version --output json` must still
    // be detected. Regression for the pre-clap version scanner.
    let raw = vec![
        "agent-cli".to_string(),
        "--output-to".to_string(),
        "stdout".to_string(),
        "--version".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        None,
        "1.2.3",
        None,
    )
    .expect("valid version request")
    .expect("version should render");
    assert!(out.contains("\"kind\":\"result\""), "{out}");
    assert!(out.contains("\"version\":\"1.2.3\""), "{out}");
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_skips_stdout_file_space_value() {
    // Same regression as --output-to, for stream_redirect's own global flags:
    // the path value of a preceding `--stdout-file`/`--stderr-file` must not be
    // mistaken for the subcommand boundary either.
    let raw = vec![
        "agent-cli".to_string(),
        "--stdout-file".to_string(),
        "/tmp/out.log".to_string(),
        "--stderr-file".to_string(),
        "/tmp/err.log".to_string(),
        "--version".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        None,
        "1.2.3",
        None,
    )
    .expect("valid version request")
    .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["result"]["name"], "agent-cli");
    assert_eq!(parsed["result"]["version"], "1.2.3");
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_skips_stdout_file_inline_value() {
    let raw = vec![
        "agent-cli".to_string(),
        "--stdout-file=/tmp/out.log".to_string(),
        "--version".to_string(),
    ];
    assert!(
        cli_handle_version_or_continue(
            &raw,
            &version_test_command(),
            "agent-cli",
            None,
            "1.2.3",
            None
        )
        .expect("valid version request")
        .is_some()
    );
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_skips_caller_defined_value_flag() {
    // A consumer's *own* value-taking global flag that afdata's pre-parser has
    // no special knowledge of — here a hypha-style comma-list `--log` — must
    // have its space-separated value recognized through the passed
    // `clap::Command`, not a hardcoded flag list. If `flag_takes_value` ever
    // regressed to a fixed stdout-file/stderr-file allowlist, `request,startup`
    // below would be mistaken for the subcommand boundary and `--version` would
    // be dropped. This locks the hypha usage shape against that drift.
    let cmd = clap::Command::new("hypha")
        .arg(
            clap::Arg::new("log")
                .long("log")
                .value_delimiter(',')
                .action(clap::ArgAction::Append),
        )
        .subcommand(clap::Command::new("sense"));
    let raw = vec![
        "hypha".to_string(),
        "--log".to_string(),
        "request,startup".to_string(),
        "--version".to_string(),
    ];
    let out = cli_handle_version_or_continue(&raw, &cmd, "hypha", None, "1.2.3", None)
        .expect("valid version request")
        .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["result"]["name"], "hypha");
    assert_eq!(parsed["result"]["version"], "1.2.3");
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_boolean_global_flag_does_not_over_consume() {
    // The mirror of the case above: a caller's boolean global flag takes no
    // value, so the following positional is the subcommand boundary. A
    // `--version` after that boundary belongs to the subcommand and must not be
    // hijacked. If `flag_takes_value` wrongly reported the boolean flag as
    // value-taking, it would swallow `sense` and misread this as a top-level
    // version request.
    let cmd = clap::Command::new("hypha")
        .arg(
            clap::Arg::new("verbose")
                .long("verbose")
                .action(clap::ArgAction::SetTrue),
        )
        .subcommand(clap::Command::new("sense"));
    let raw = vec![
        "hypha".to_string(),
        "--verbose".to_string(),
        "sense".to_string(),
        "--version".to_string(),
    ];
    assert!(
        cli_handle_version_or_continue(&raw, &cmd, "hypha", None, "1.2.3", None)
            .expect("subcommand --version must not be a top-level version request")
            .is_none()
    );
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_supports_inline_output_format() {
    let raw = vec![
        "agent-cli".to_string(),
        "--output=yaml".to_string(),
        "--version".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        None,
        "1.2.3",
        None,
    )
    .expect("valid version request")
    .expect("version should render");
    assert!(out.starts_with("---\n"), "{out}");
    assert!(out.contains("kind: \"result\""), "{out}");
    assert!(out.contains("version: \"1.2.3\""), "{out}");
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_supports_json_alias() {
    let raw = vec![
        "agent-cli".to_string(),
        "--version".to_string(),
        "--json".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        None,
        "1.2.3",
        None,
    )
    .expect("valid version request")
    .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["kind"], "result");
    assert_eq!(parsed["result"]["version"], "1.2.3");
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_rejects_json_alias_conflict() {
    let raw = vec![
        "agent-cli".to_string(),
        "--version".to_string(),
        "--json".to_string(),
        "--output".to_string(),
        "yaml".to_string(),
    ];
    let err = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        None,
        "1.2.3",
        None,
    )
    .expect_err("conflicting formats must return error")
    .into_value();
    assert_eq!(err["kind"], "error");
    assert!(
        err["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("conflicting output formats")),
        "error should mention conflict: {}",
        err["error"]["message"]
    );
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_reports_invalid_output_as_cli_error() {
    let raw = vec![
        "agent-cli".to_string(),
        "--version".to_string(),
        "--output".to_string(),
        "xml".to_string(),
    ];
    let err = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        None,
        "1.2.3",
        None,
    )
    .expect_err("invalid version output must return error")
    .into_value();
    assert_eq!(err["kind"], "error");
    assert_eq!(err["error"]["code"], "cli_error");
    assert!(
        err["error"]["message"]
            .as_str()
            .is_some_and(|s| s.contains("xml")),
        "error should mention invalid value: {err}"
    );
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_without_version_returns_none() {
    let raw = vec!["agent-cli".to_string(), "ping".to_string()];
    assert!(
        cli_handle_version_or_continue(
            &raw,
            &version_test_command(),
            "agent-cli",
            None,
            "1.2.3",
            None
        )
        .expect("valid non-version request")
        .is_none()
    );
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_ignores_version_flag_after_subcommand() {
    // A subcommand that takes its own `--version <value>` must not be hijacked
    // by the top-level pre-parser.
    let raw = vec![
        "agent-cli".to_string(),
        "hatch".to_string(),
        "--version".to_string(),
        "1.3.0".to_string(),
    ];
    assert!(
        cli_handle_version_or_continue(
            &raw,
            &version_test_command(),
            "agent-cli",
            None,
            "1.2.3",
            None
        )
        .expect("subcommand --version must not be a version request")
        .is_none()
    );
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_ignores_short_version_flag_after_subcommand() {
    let raw = vec![
        "agent-cli".to_string(),
        "hatch".to_string(),
        "-V".to_string(),
        "1.3.0".to_string(),
    ];
    assert!(
        cli_handle_version_or_continue(
            &raw,
            &version_test_command(),
            "agent-cli",
            None,
            "1.2.3",
            None
        )
        .expect("subcommand -V must not be a version request")
        .is_none()
    );
}

#[cfg(any(feature = "cli", feature = "cli-help"))]
#[test]
fn cli_handle_version_honors_output_flag_before_top_level_version() {
    // Known output flags consume their value, so a trailing top-level
    // `--version` is still recognized.
    let raw = vec![
        "agent-cli".to_string(),
        "--output".to_string(),
        "json".to_string(),
        "--version".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        &version_test_command(),
        "agent-cli",
        None,
        "1.2.3",
        None,
    )
    .expect("valid version request")
    .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["result"]["version"], "1.2.3");
}

// ═══════════════════════════════════════════
// Complete integration: README examples
// ═══════════════════════════════════════════

#[test]
fn readme_complete_suffix_yaml() {
    let data = json!({
        "created_at_epoch_ms": 1738886400000i64,
        "request_timeout_ms": 5000,
        "cache_ttl_s": 3600,
        "file_size_bytes": 5242880,
        "payment_msats": 50000000,
        "price_usd_cents": 9999,
        "success_rate_percent": 95.5,
        "api_key_secret": "sk-1234567890abcdef",
        "user_name": "alice",
        "count": 42
    });
    let out = render(&data, OutputFormat::Yaml, &OutputOptions::default());
    assert!(out.starts_with("---\n"));
    // Structure-preserving: original keys and raw values/types survive after
    // redaction; only the secret value is masked, just like `OutputFormat::Json`.
    assert!(out.contains("api_key_secret: \"***\""));
    assert!(out.contains("cache_ttl_s: 3600"));
    assert!(out.contains("count: 42"));
    assert!(out.contains("created_at_epoch_ms: 1738886400000"));
    assert!(out.contains("file_size_bytes: 5242880"));
    assert!(out.contains("payment_msats: 50000000"));
    assert!(out.contains("price_usd_cents: 9999"));
    assert!(out.contains("request_timeout_ms: 5000"));
    assert!(out.contains("success_rate_percent: 95.5"));
    assert!(out.contains("user_name: \"alice\""));
}

#[test]
fn readme_complete_suffix_plain() {
    let data = json!({
        "created_at_epoch_ms": 1738886400000i64,
        "request_timeout_ms": 5000,
        "cache_ttl_s": 3600,
        "file_size_bytes": 5242880,
        "payment_msats": 50000000,
        "price_usd_cents": 9999,
        "success_rate_percent": 95.5,
        "api_key_secret": "sk-1234567890abcdef",
        "user_name": "alice",
        "count": 42
    });
    let out = render(&data, OutputFormat::Plain, &OutputOptions::default());
    assert_eq!(
        out,
        "api_key=*** cache_ttl=3600s count=42 created_at=2025-02-07T00:00:00.000Z file_size=5.0MiB payment=50000000msats price=$99.99 request_timeout=5.0s success_rate=95.5% user_name=alice"
    );
}

#[test]
fn readme_json_output() {
    let data = json!({
        "user_id": 123,
        "api_key_secret": "sk-1234567890abcdef",
        "created_at_epoch_ms": 1738886400000i64,
        "file_size_bytes": 5242880
    });
    let out = render(&data, OutputFormat::Json, &OutputOptions::default());
    assert!(out.contains("\"api_key_secret\":\"***\""));
    assert!(out.contains("\"created_at_epoch_ms\":1738886400000"));
    assert!(out.contains("\"file_size_bytes\":5242880"));
    assert!(out.contains("\"user_id\":123"));
    assert!(!out.contains("sk-1234567890abcdef"));
    assert!(!out.contains('\n'));
}

#[test]
fn readme_cli_startup_yaml() {
    let startup_val = crate::protocol::json_log(json!({
        "level": "info",
        "message": "startup",
        "config": {"api_key_secret": "sk-sensitive-key", "timeout_s": 30},
        "args": {"input_path": "data.json"},
        "env": {"RUST_LOG": "info"}
    }))
    .build();
    let out = render(
        startup_val.as_value(),
        OutputFormat::Yaml,
        &OutputOptions::default(),
    );
    assert!(out.contains("kind: \"log\""));
    assert!(out.contains("api_key_secret: \"***\""));
    assert!(out.contains("timeout_s: 30"));
    assert!(out.contains("input_path: \"data.json\""));
    assert!(out.contains("RUST_LOG: \"info\""));
    assert!(!out.contains("sk-sensitive-key"));
}

#[test]
fn readme_cli_progress_plain() {
    let progress = crate::protocol::json_progress(json!({
        "message": "processing",
        "current": 3,
        "total": 10,
    }))
    .trace(json!({"duration_ms": 1500}))
    .build();
    let out = render(
        progress.as_value(),
        OutputFormat::Plain,
        &OutputOptions::default(),
    );
    assert!(out.contains("kind=progress"));
    assert!(out.contains("progress.current=3"));
    assert!(out.contains("progress.message=processing"));
    assert!(out.contains("progress.total=10"));
    assert!(out.contains("trace.duration=1.5s"));
}

#[test]
fn readme_jsonl_output() {
    let result = crate::protocol::json_result(json!({"status": "success"}))
        .trace(json!({"duration_ms": 250, "api_key_secret": "sk-123"}))
        .build();
    let out = render(
        result.as_value(),
        OutputFormat::Json,
        &OutputOptions::default(),
    );
    assert!(out.contains("\"kind\":\"result\""));
    assert!(out.contains("\"status\":\"success\""));
    assert!(out.contains("\"api_key_secret\":\"***\""));
    assert!(!out.contains("sk-123"));
    assert!(!out.contains('\n'));
}

// ═══════════════════════════════════════════
// decode_protocol_event
// ═══════════════════════════════════════════

#[test]
fn decode_result_event() {
    let event = crate::protocol::json_result(json!({"rows": 2}))
        .trace(json!({"duration_ms": 10}))
        .build();
    let text = serde_json::to_string(&event).expect("serialize");
    match decode_protocol_event(&text).expect("decode result") {
        DecodedEvent::Result(decoded) => {
            assert_eq!(decoded.result, json!({"rows": 2}));
            assert_eq!(decoded.trace, Some(json!({"duration_ms": 10})));
        }
        other => panic!("expected DecodedEvent::Result, got {other:?}"),
    }
}

#[test]
fn decode_error_event_with_extension_fields() {
    let event = crate::protocol::json_error("not_found", "missing")
        .hint("try again")
        .retryable()
        .field("attempt", json!(3))
        .build()
        .expect("builder failed");
    let text = serde_json::to_string(&event).expect("serialize");
    match decode_protocol_event(&text).expect("decode error") {
        DecodedEvent::Error(decoded) => {
            assert_eq!(decoded.code, "not_found");
            assert_eq!(decoded.message, "missing");
            assert!(decoded.retryable);
            assert_eq!(decoded.hint.as_deref(), Some("try again"));
            assert_eq!(decoded.fields.get("attempt"), Some(&json!(3)));
            assert_eq!(decoded.trace, Some(json!({})));
        }
        other => panic!("expected DecodedEvent::Error, got {other:?}"),
    }
}

#[test]
fn decode_progress_event_with_extension_fields() {
    let event = crate::protocol::json_progress(json!({
        "message": "uploading",
        "current": 3,
        "total": 10,
    }))
    .build();
    let text = serde_json::to_string(&event).expect("serialize");
    match decode_protocol_event(&text).expect("decode progress") {
        DecodedEvent::Progress(decoded) => {
            assert_eq!(decoded.progress["message"], "uploading");
            assert_eq!(decoded.progress["current"], 3);
            assert_eq!(decoded.progress["total"], 10);
        }
        other => panic!("expected DecodedEvent::Progress, got {other:?}"),
    }
}

#[test]
fn decode_log_event_with_extension_fields() {
    let event = crate::protocol::json_log(json!({
        "level": "warn",
        "message": "disk low",
        "free_bytes": 1024,
    }))
    .build();
    let text = serde_json::to_string(&event).expect("serialize");
    match decode_protocol_event(&text).expect("decode log") {
        DecodedEvent::Log(decoded) => {
            assert_eq!(decoded.log["level"], "warn");
            assert_eq!(decoded.log["message"], "disk low");
            assert_eq!(decoded.log["free_bytes"], 1024);
        }
        other => panic!("expected DecodedEvent::Log, got {other:?}"),
    }
}

#[test]
fn decode_protocol_event_rejects_invalid_json() {
    let err = decode_protocol_event("not json").expect_err("invalid JSON must fail");
    assert!(matches!(err, EventDecodeError::InvalidJson(_)));
    assert!(err.to_string().contains("invalid JSON"));
}

#[test]
fn decode_protocol_event_rejects_non_strict_event() {
    // Missing trace fails the strict profile that decode_protocol_event enforces.
    let err = decode_protocol_event(r#"{"kind":"result","result":{}}"#)
        .expect_err("non-strict event must fail");
    assert!(matches!(err, EventDecodeError::InvalidEvent(_)));
    assert!(err.to_string().contains("invalid protocol event"));
}

#[test]
fn decode_protocol_event_rejects_unsupported_kind() {
    let err = decode_protocol_event(r#"{"kind":"ping","ping":{},"trace":{}}"#)
        .expect_err("unsupported kind must fail");
    assert!(matches!(err, EventDecodeError::InvalidEvent(_)));
}

// ═══════════════════════════════════════════
// Number literal fidelity (shared spec/fixtures/number_fidelity.json)
// ═══════════════════════════════════════════
//
// Phase 1 (rust/src/document, cli/src/main.rs) already made the Rust CLI and
// document layer digit-faithful via Value::Number(String) and crate-wide
// serde_json arbitrary_precision. This suite verifies the *library* layer —
// decode_protocol_event + formatting::render — inherits that fidelity for
// free on the JSON path: arbitrary_precision makes serde_json::Value::Number
// retain the exact source literal regardless of call site (parsed via
// decode_protocol_event or hand-built via json!()), and serialize_json_output
// serializes that stored text directly, so no library code change was needed
// for JSON.
//
// YAML/plain are a narrower story: format_number() (this file's
// yaml_scalar/plain_scalar helper) canonicalizes an *integral-valued* float
// by reformatting through f64 and dropping a trailing ".0", to match
// Go/TS/Python's own native-number formatting for hand-built values (see its
// doc comment). Go/TS/Python can tell a *decoded* number apart from a
// hand-built one (json.Number / LosslessNumber / _RawNumber wrapper types)
// and skip that canonicalization for decoded values, preserving exact
// spelling even in YAML. serde_json::Number carries no such decoded-vs-hand-
// built distinction, so format_number cannot do the same. This is invisible
// for every fixture case here except one: a decoded "-0.0" loses its trailing
// zero in YAML/plain ("-0.0" -> "-0", value-preserving — IEEE-754 negative
// zero survives — but spelling-lossy), which is why number_fidelity.json's
// "negative_zero" case has no expected_yaml. Every other case's YAML is
// asserted and exact, since only integral-valued floats hit this path.

#[test]
fn number_fidelity_fixtures() {
    let cases = load_fixture("number_fidelity.json");
    for case in cases
        .as_array()
        .expect("number_fidelity.json must be an array")
    {
        let name = case["name"].as_str().expect("missing name");
        let input_line = case["input_line"]
            .as_str()
            .expect("input_line must be a string");
        let expected_json = case["expected_json"]
            .as_str()
            .expect("expected_json must be a string");

        let decoded = decode_protocol_event(input_line)
            .unwrap_or_else(|e| panic!("[number_fidelity/{name}] decode failed: {e}"));
        let result = match decoded {
            DecodedEvent::Result(r) => r.result,
            other => {
                panic!("[number_fidelity/{name}] expected DecodedEvent::Result, got {other:?}")
            }
        };

        let got_json = render(&result, OutputFormat::Json, &OutputOptions::default());
        assert_eq!(
            got_json, expected_json,
            "[number_fidelity/{name}] json mismatch"
        );

        if let Some(expected_yaml) = case["expected_yaml"].as_str() {
            let got_yaml = render(&result, OutputFormat::Yaml, &OutputOptions::default());
            assert_eq!(
                got_yaml, expected_yaml,
                "[number_fidelity/{name}] yaml mismatch"
            );
        }
    }
}

// Guards the pre-existing (not newly introduced) arbitrary_precision Number
// handling in as_f64/as_i64/format_number: decode_protocol_event wraps every
// decoded number in the same Value::Number regardless of magnitude, so
// Plain's suffix arithmetic must keep working for a routine decoded event.
#[test]
fn number_fidelity_does_not_regress_ordinary_decoded_numbers_in_plain_output() {
    let text = r#"{"kind":"result","result":{"duration_ms":42,"size_bytes":5242880,"cpu_percent":85.5},"trace":{}}"#;
    let decoded = decode_protocol_event(text).expect("decode");
    let result = match decoded {
        DecodedEvent::Result(r) => r.result,
        other => panic!("expected DecodedEvent::Result, got {other:?}"),
    };
    let got = render(&result, OutputFormat::Plain, &OutputOptions::default());
    assert_eq!(got, "cpu=85.5% duration=42ms size=5.0MiB");
}

// Known, narrow infeasibility (see number_fidelity.json's "negative_zero"
// case, which deliberately uses "-0.0" rather than bare "-0"): serde_json's
// arbitrary_precision parser fast-paths a *bare integer* literal with no '.'
// or 'e' through Rust's native `str::parse::<i64/u64>`, which treats "-0" and
// "0" as equal and returns unsigned 0, discarding the sign before the
// string-retention branch is ever reached (rust/src/tests.rs asserted this
// directly against upstream serde_json 1.0.150's `parse_any_number`, not
// afdata code). Any float-shaped spelling ("-0.0", "-0e0") is unaffected —
// those always take the string-retention path. Working around the bare-
// integer case would require a hand-rolled JSON tokenizer to rewrite "-0"
// tokens outside of strings before handing text to serde_json, which is
// exactly the fragile hand-scanning this project's own conventions reject
// elsewhere; not attempted here.
#[test]
fn negative_zero_bare_integer_is_a_known_serde_json_limitation() {
    let v: serde_json::Value = serde_json::from_str("-0").expect("parses");
    assert_eq!(
        serde_json::to_string(&v).expect("serializes"),
        "0",
        "if this starts passing, serde_json fixed the upstream limitation \
         documented above -- promote \"-0\" (bare integer) back into \
         number_fidelity.json's negative_zero case"
    );
}

// Known, narrow infeasibility (see number_fidelity.json's "exponent_notation"
// case, which deliberately writes "6.022e+23" with an explicit sign rather
// than bare "6.022e23"): serde_json's arbitrary_precision scanner
// (`scan_exponent`) inserts a synthetic '+' into its retained text whenever
// the source exponent has no explicit sign, in every code path (not just a
// fast-path shortcut like the bare-integer case above) -- so this one is
// unconditional for any sign-less exponent, not just an edge case. Same
// reasoning as above for not working around it with hand-rolled scanning.
#[test]
fn signless_exponent_is_a_known_serde_json_limitation() {
    let v: serde_json::Value = serde_json::from_str("6.022e23").expect("parses");
    assert_eq!(
        serde_json::to_string(&v).expect("serializes"),
        "6.022e+23",
        "if this starts passing, serde_json fixed the upstream limitation \
         documented above -- promote \"6.022e23\" (sign-less exponent) back \
         into number_fidelity.json's exponent_notation case"
    );
}
