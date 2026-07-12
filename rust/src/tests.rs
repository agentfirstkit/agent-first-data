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
            "RedactionTraceOnly" => RedactionPolicy::RedactionTraceOnly,
            "RedactionNone" => RedactionPolicy::RedactionNone,
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
            style: OutputStyle::Readable,
        };
        let expected = &case["expected"];

        let got = redactor.value(&case["input"]);
        assert_eq!(&got, expected, "[redaction_options/{name}] value");

        let mut input = case["input"].clone();
        output_options.redaction.redact_in_place(&mut input);
        assert_eq!(&input, expected, "[redaction_options/{name}] in-place");

        let json_out = output_json_with_options(&case["input"], &output_options);
        let parsed_json: Value = serde_json::from_str(&json_out)
            .unwrap_or_else(|e| panic!("[redaction_options/{name}] invalid json output: {e}"));
        assert_eq!(parsed_json, *expected, "[redaction_options/{name}] json");

        if let Some(expected_yaml) = case.get("expected_yaml").and_then(Value::as_str) {
            assert_eq!(
                output_yaml_with_options(&case["input"], &output_options),
                expected_yaml,
                "[redaction_options/{name}] yaml"
            );
        }
        if let Some(expected_plain) = case.get("expected_plain").and_then(Value::as_str) {
            assert_eq!(
                output_plain_with_options(&case["input"], &output_options),
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
            style: OutputStyle::Readable,
        };
        let expected = &case["expected"];
        assert_eq!(
            redactor.value(&case["input"]),
            *expected,
            "[security/{name}] redacted value"
        );

        let outputs = [
            output_json_with_options(&case["input"], &output_options),
            output_yaml_with_options(&case["input"], &output_options),
            output_plain_with_options(&case["input"], &output_options),
        ];
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
        style: OutputStyle::Readable,
    };

    for seed in 0..64 {
        let input = generated_property_value(seed);
        let json_a = output_json_with_options(&input, &options);
        let json_b = output_json_with_options(&input, &options);
        assert_eq!(json_a, json_b, "[seed {seed}] json output changed");
        assert_no_property_secret(&json_a, &format!("[seed {seed}] json"));

        let yaml_a = output_yaml_with_options(&input, &options);
        let yaml_b = output_yaml_with_options(&input, &options);
        assert_eq!(yaml_a, yaml_b, "[seed {seed}] yaml output changed");
        assert_no_property_secret(&yaml_a, &format!("[seed {seed}] yaml"));

        let plain_a = output_plain_with_options(&input, &options);
        let plain_b = output_plain_with_options(&input, &options);
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
                .expect("builder failed")
                .into_value(),
            "result_trace" => crate::protocol::json_result(args["result"].clone())
                .trace(args["trace"].clone())
                .build()
                .expect("builder failed")
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
                let message = args["message"].as_str().expect("progress message required");
                let mut builder = crate::protocol::json_progress(message);
                if let Some(fields) = args.get("fields").and_then(Value::as_object) {
                    for (k, v) in fields {
                        builder = builder.field(k, v.clone());
                    }
                }
                builder.build().expect("builder failed").into_value()
            }
            "log" => {
                let level_str = args["level"].as_str().expect("log level required");
                let level = match level_str {
                    "debug" => crate::protocol::LogLevel::Debug,
                    "info" => crate::protocol::LogLevel::Info,
                    "warn" => crate::protocol::LogLevel::Warn,
                    "error" => crate::protocol::LogLevel::Error,
                    other => panic!("unknown log level: {other}"),
                };
                let message = args["message"].as_str().expect("log message required");
                let mut builder = crate::protocol::json_log(level, message);
                if let Some(fields) = args.get("fields").and_then(Value::as_object) {
                    for (k, v) in fields {
                        builder = builder.field(k, v.clone());
                    }
                }
                builder.build().expect("builder failed").into_value()
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
            "parse_size" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = if arr[1].is_null() {
                        None
                    } else {
                        arr[1].as_u64()
                    };
                    assert_eq!(
                        parse_size(input),
                        expected,
                        "[helpers/parse_size({input:?})]"
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
        let expected_yaml = case["expected_yaml"]
            .as_str()
            .expect("expected_yaml must be string");
        let expected_plain = case["expected_plain"]
            .as_str()
            .expect("expected_plain must be string");

        let json_out = output_json(&input);
        let parsed_json: Value = serde_json::from_str(&json_out)
            .unwrap_or_else(|e| panic!("[output/{name}] invalid json output: {e}"));
        assert_eq!(parsed_json, expected_json, "[output/{name}] json mismatch");

        let yaml_out = output_yaml(&input);
        assert_eq!(yaml_out, expected_yaml, "[output/{name}] yaml mismatch");

        let plain_out = output_plain(&input);
        assert_eq!(plain_out, expected_plain, "[output/{name}] plain mismatch");
    }
}

// ═══════════════════════════════════════════
// output_json
// ═══════════════════════════════════════════

#[test]
fn json_single_line() {
    let out = output_json(&json!({"a": 1, "b": {"c": 2}}));
    assert!(!out.contains('\n'));
}

#[test]
fn json_secrets_redacted() {
    let out = output_json(&json!({"api_key_secret": "sk-123", "name": "test"}));
    assert!(out.contains("\"***\""));
    assert!(!out.contains("sk-123"));
    assert!(out.contains("\"name\""));
}

#[test]
fn json_nested_secrets_redacted() {
    let out = output_json(&json!({"config": {"password_secret": "real"}}));
    assert!(!out.contains("real"));
    assert!(out.contains("***"));
}

#[test]
fn json_original_keys_preserved() {
    let out = output_json(&json!({"duration_ms": 1280}));
    assert!(out.contains("\"duration_ms\""));
    assert!(out.contains("1280"));
    assert!(!out.contains("\"duration\":"));
}

#[test]
fn json_raw_values_not_formatted() {
    let out = output_json(&json!({"size_bytes": 5242880}));
    assert!(out.contains("5242880"));
    assert!(!out.contains("MiB"));
}

#[test]
fn json_non_string_secret_redacted() {
    let out = output_json(&json!({"count_secret": 42}));
    assert!(out.contains("\"***\""));
    assert!(!out.contains("42"));
}

#[test]
fn json_with_trace_only_redacts_trace_only() {
    let out = output_json_with_options(
        &json!({
            "code": "ok",
            "result": {"api_key_secret": "sk-live-123"},
            "trace": {"request_secret": "top-secret"}
        }),
        &RedactionPolicy::RedactionTraceOnly.into(),
    );
    assert!(out.contains("\"request_secret\":\"***\""));
    assert!(out.contains("\"api_key_secret\":\"sk-live-123\""));
}

#[test]
fn json_with_none_keeps_secret_values() {
    let out = output_json_with_options(
        &json!({"api_key_secret": "sk-live-123"}),
        &RedactionPolicy::RedactionNone.into(),
    );
    assert!(out.contains("\"api_key_secret\":\"sk-live-123\""));
    assert!(!out.contains("\"***\""));
}

#[test]
fn redaction_policy_into_redactor() {
    let redactor: Redactor = RedactionPolicy::RedactionTraceOnly.into();
    assert_eq!(
        redactor,
        Redactor::new().policy(RedactionPolicy::RedactionTraceOnly)
    );
}

#[test]
fn redaction_policy_into_output_options() {
    let options: OutputOptions = RedactionPolicy::RedactionNone.into();
    assert_eq!(
        options.redaction,
        Redactor::new().policy(RedactionPolicy::RedactionNone)
    );
    assert_eq!(options.style, OutputStyle::Readable);
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
    let out = output_json(&input);
    assert!(out.contains("<afdata:max-depth>"), "{out}");
    assert!(!out.contains("***"), "{out}");
}

#[test]
fn json_default_output_redacts_secrets() {
    let out = output_json(&json!({"api_key_secret": "sk-live-123"}));
    assert!(out.contains("\"api_key_secret\":\"***\""));
}

// ═══════════════════════════════════════════
// output_yaml: key stripping
// ═══════════════════════════════════════════

#[test]
fn yaml_starts_with_separator() {
    let out = output_yaml(&json!({"a": 1}));
    assert!(out.starts_with("---\n"));
}

#[test]
fn yaml_strip_ms() {
    let out = output_yaml(&json!({"duration_ms": 42}));
    assert!(out.contains("duration:"));
    assert!(!out.contains("duration_ms"));
}

#[test]
fn yaml_raw_keeps_suffix_keys_and_structure() {
    let options = OutputOptions {
        redaction: Redactor::new().policy(RedactionPolicy::RedactionTraceOnly),
        style: OutputStyle::Raw,
    };
    let out = output_yaml_with_options(
        &json!({
            "code": "result",
            "rows": [{"api_key_secret": "sk-live-1", "duration_ms": 42}],
            "trace": {"request_secret": "top-secret"}
        }),
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
fn yaml_with_options_defaults_to_readable_style() {
    let out = output_yaml_with_options(
        &json!({"duration_ms": 42}),
        &OutputOptions {
            redaction: Redactor::new().policy(RedactionPolicy::RedactionNone),
            style: OutputStyle::Readable,
        },
    );
    assert!(out.contains("duration: \"42ms\""));
    assert!(!out.contains("duration_ms:"));
}

#[test]
fn yaml_strip_s() {
    let out = output_yaml(&json!({"timeout_s": 30}));
    assert!(out.contains("timeout:"));
    assert!(!out.contains("timeout_s"));
}

#[test]
fn yaml_strip_ns() {
    let out = output_yaml(&json!({"gc_pause_ns": 450000}));
    assert!(out.contains("gc_pause:"));
    assert!(!out.contains("gc_pause_ns"));
}

#[test]
fn yaml_strip_us() {
    let out = output_yaml(&json!({"query_us": 830}));
    assert!(out.contains("query:"));
    assert!(!out.contains("query_us"));
}

#[test]
fn yaml_strip_bytes() {
    let out = output_yaml(&json!({"file_size_bytes": 5242880}));
    assert!(out.contains("file_size:"));
    assert!(!out.contains("file_size_bytes"));
}

#[test]
fn yaml_strip_epoch_ms() {
    let out = output_yaml(&json!({"created_at_epoch_ms": 1738886400000i64}));
    assert!(out.contains("created_at:"));
    assert!(!out.contains("created_at_epoch_ms"));
}

#[test]
fn yaml_strip_epoch_s() {
    let out = output_yaml(&json!({"cached_epoch_s": 1738886400}));
    assert!(out.contains("cached:"));
    assert!(!out.contains("cached_epoch_s"));
}

#[test]
fn yaml_strip_epoch_ns() {
    let out = output_yaml(&json!({"created_epoch_ns": "1707868800000000000"}));
    assert!(out.contains("created:"));
    assert!(!out.contains("created_epoch_ns"));
}

#[test]
fn yaml_strip_rfc3339() {
    let out = output_yaml(&json!({"expires_rfc3339": "2026-02-14T10:30:00Z"}));
    assert!(out.contains("expires:"));
    assert!(!out.contains("expires_rfc3339"));
}

#[test]
fn yaml_strip_secret() {
    let out = output_yaml(&json!({"api_key_secret": "sk-123"}));
    assert!(out.contains("api_key:"));
    assert!(!out.contains("api_key_secret"));
}

#[test]
fn yaml_strip_percent() {
    let out = output_yaml(&json!({"cpu_percent": 85}));
    assert!(out.contains("cpu:"));
    assert!(!out.contains("cpu_percent"));
}

#[test]
fn yaml_strip_msats() {
    let out = output_yaml(&json!({"balance_msats": 50000}));
    assert!(out.contains("balance:"));
    assert!(!out.contains("balance_msats"));
}

#[test]
fn yaml_strip_sats() {
    let out = output_yaml(&json!({"withdrawn_sats": 1234}));
    assert!(out.contains("withdrawn:"));
    assert!(!out.contains("withdrawn_sats"));
}

#[test]
fn yaml_strip_btc() {
    let out = output_yaml(&json!({"reserve_btc": 0.5}));
    assert!(out.contains("reserve_btc"));
}

#[test]
fn yaml_strip_usd_cents() {
    let out = output_yaml(&json!({"price_usd_cents": 999}));
    assert!(out.contains("price:"));
    assert!(!out.contains("price_usd_cents"));
}

#[test]
fn yaml_strip_eur_cents() {
    let out = output_yaml(&json!({"price_eur_cents": 850}));
    assert!(out.contains("price:"));
    assert!(!out.contains("price_eur_cents"));
}

#[test]
fn yaml_strip_jpy() {
    let out = output_yaml(&json!({"price_jpy": 1500}));
    assert!(out.contains("price:"));
    assert!(!out.contains("price_jpy"));
}

#[test]
fn yaml_strip_generic_cents() {
    let out = output_yaml(&json!({"fare_thb_cents": 15050}));
    assert!(out.contains("fare:"));
    assert!(!out.contains("fare_thb_cents"));
}

#[test]
fn yaml_strip_minutes() {
    let out = output_yaml(&json!({"timeout_minutes": 30}));
    assert!(out.contains("timeout:"));
    assert!(!out.contains("timeout_minutes"));
}

#[test]
fn yaml_strip_hours() {
    let out = output_yaml(&json!({"validity_hours": 24}));
    assert!(out.contains("validity:"));
    assert!(!out.contains("validity_hours"));
}

#[test]
fn yaml_strip_days() {
    let out = output_yaml(&json!({"cert_days": 365}));
    assert!(out.contains("cert:"));
    assert!(!out.contains("cert_days"));
}

#[test]
fn yaml_no_strip_size() {
    let out = output_yaml(&json!({"buffer_size": "10M"}));
    assert!(out.contains("buffer_size:"));
}

#[test]
fn yaml_no_strip_no_suffix() {
    let out = output_yaml(&json!({"user_id": 123, "config_path": "a.yml"}));
    assert!(out.contains("user_id:"));
    assert!(out.contains("config_path:"));
}

#[test]
fn yaml_strip_uppercase_secret() {
    let out = output_yaml(&json!({"DATABASE_URL_SECRET": "postgres://..."}));
    assert!(out.contains("DATABASE_URL:"));
    assert!(!out.contains("DATABASE_URL_SECRET"));
}

#[test]
fn yaml_strip_uppercase_s() {
    let out = output_yaml(&json!({"CACHE_TTL_S": 3600}));
    assert!(out.contains("CACHE_TTL:"));
    assert!(!out.contains("CACHE_TTL_S"));
}

// ═══════════════════════════════════════════
// Key collision detection
// ═══════════════════════════════════════════

#[test]
fn yaml_key_collision_keeps_originals() {
    let out = output_yaml(&json!({"response_ms": 150, "response_s": 1}));
    assert!(out.contains("response_ms: 150"));
    assert!(out.contains("response_s: 1"));
}

#[test]
fn plain_key_collision_keeps_originals() {
    let out = output_plain(&json!({"response_ms": 150, "response_s": 1}));
    assert!(out.contains("response_ms=150"));
    assert!(out.contains("response_s=1"));
}

#[test]
fn plain_raw_keeps_suffix_keys_and_redacts_trace() {
    let options = OutputOptions {
        redaction: Redactor::new().policy(RedactionPolicy::RedactionTraceOnly),
        style: OutputStyle::Raw,
    };
    let out = output_plain_with_options(
        &json!({
            "duration_ms": 42,
            "trace": {"request_secret": "top-secret"}
        }),
        &options,
    );
    assert!(out.contains("duration_ms=42"));
    assert!(out.contains("trace.request_secret=***"));
    assert!(!out.contains("duration=42ms"));
}

// ═══════════════════════════════════════════
// output_yaml: value formatting
// ═══════════════════════════════════════════

#[test]
fn yaml_fmt_ms_small() {
    let out = output_yaml(&json!({"latency_ms": 42}));
    assert!(out.contains("\"42ms\""));
}

#[test]
fn yaml_fmt_ms_to_seconds() {
    let out = output_yaml(&json!({"duration_ms": 1280}));
    assert!(out.contains("\"1.28s\""));
}

#[test]
fn yaml_fmt_ms_5000() {
    let out = output_yaml(&json!({"request_timeout_ms": 5000}));
    assert!(out.contains("\"5.0s\""));
}

#[test]
fn yaml_fmt_ms_1500() {
    let out = output_yaml(&json!({"duration_ms": 1500}));
    assert!(out.contains("\"1.5s\""));
}

#[test]
fn yaml_fmt_s() {
    let out = output_yaml(&json!({"cache_ttl_s": 3600}));
    assert!(out.contains("\"3600s\""));
}

#[test]
fn yaml_fmt_ns() {
    let out = output_yaml(&json!({"gc_pause_ns": 450000}));
    assert!(out.contains("\"450000ns\""));
}

#[test]
fn yaml_fmt_us() {
    let out = output_yaml(&json!({"query_us": 830}));
    assert!(out.contains("\"830\u{03bc}s\""));
}

#[test]
fn yaml_fmt_minutes() {
    let out = output_yaml(&json!({"timeout_minutes": 30}));
    assert!(out.contains("\"30 minutes\""));
}

#[test]
fn yaml_fmt_hours() {
    let out = output_yaml(&json!({"validity_hours": 24}));
    assert!(out.contains("\"24 hours\""));
}

#[test]
fn yaml_fmt_days() {
    let out = output_yaml(&json!({"cert_days": 365}));
    assert!(out.contains("\"365 days\""));
}

#[test]
fn yaml_fmt_epoch_ms() {
    let out = output_yaml(&json!({"created_at_epoch_ms": 1738886400000i64}));
    assert!(out.contains("\"2025-02-07T00:00:00.000Z\""));
}

#[test]
fn yaml_fmt_epoch_s() {
    let out = output_yaml(&json!({"cached_epoch_s": 1738886400}));
    assert!(out.contains("\"2025-02-07T00:00:00.000Z\""));
}

#[test]
fn yaml_fmt_bytes() {
    let out = output_yaml(&json!({"file_size_bytes": 5242880}));
    assert!(out.contains("\"5.0MiB\""));
}

#[test]
fn yaml_fmt_bytes_kb() {
    let out = output_yaml(&json!({"payload_bytes": 456789}));
    assert!(out.contains("\"446.1KiB\""));
}

#[test]
fn yaml_fmt_usd_cents() {
    let out = output_yaml(&json!({"price_usd_cents": 9999}));
    assert!(out.contains("\"$99.99\""));
}

#[test]
fn yaml_fmt_eur_cents() {
    let out = output_yaml(&json!({"price_eur_cents": 850}));
    assert!(out.contains("\"\u{20ac}8.50\""));
}

#[test]
fn yaml_fmt_jpy() {
    let out = output_yaml(&json!({"price_jpy": 1500}));
    assert!(out.contains("\"\u{00a5}1,500\""));
}

#[test]
fn yaml_fmt_generic_cents() {
    let out = output_yaml(&json!({"fare_thb_cents": 15050}));
    assert!(out.contains("\"150.50 THB\""));
}

#[test]
fn yaml_fmt_msats() {
    let out = output_yaml(&json!({"payment_msats": 50000000}));
    assert!(out.contains("\"50000000msats\""));
}

#[test]
fn yaml_fmt_sats() {
    let out = output_yaml(&json!({"withdrawn_sats": 1234}));
    assert!(out.contains("\"1234sats\""));
}

#[test]
fn yaml_fmt_btc() {
    let out = output_yaml(&json!({"reserve_btc": 0.5}));
    assert!(out.contains("reserve_btc: 0.5"));
}

#[test]
fn yaml_fmt_percent_int() {
    let out = output_yaml(&json!({"cpu_percent": 85}));
    assert!(out.contains("\"85%\""));
}

#[test]
fn yaml_fmt_percent_float() {
    let out = output_yaml(&json!({"success_rate_percent": 95.5}));
    assert!(out.contains("\"95.5%\""));
}

#[test]
fn yaml_fmt_secret() {
    let out = output_yaml(&json!({"api_key_secret": "sk-1234567890abcdef"}));
    assert!(out.contains("\"***\""));
    assert!(!out.contains("sk-1234567890abcdef"));
}

#[test]
fn yaml_fmt_rfc3339_passthrough() {
    let out = output_yaml(&json!({"expires_rfc3339": "2026-02-14T10:30:00Z"}));
    assert!(out.contains("\"2026-02-14T10:30:00Z\""));
}

#[test]
fn yaml_fmt_size_passthrough() {
    let out = output_yaml(&json!({"buffer_size": "10M"}));
    assert!(out.contains("\"10M\""));
}

#[test]
fn yaml_strings_always_quoted() {
    let out = output_yaml(&json!({"name": "alice"}));
    assert!(out.contains("\"alice\""));
}

#[test]
fn yaml_numbers_unquoted() {
    let out = output_yaml(&json!({"count": 42}));
    assert!(out.contains("count: 42"));
    assert!(!out.contains("\"42\""));
}

#[test]
fn yaml_nested_key_stripping() {
    let out = output_yaml(&json!({
        "config": {
            "api_key_secret": "sk-123",
            "timeout_s": 30
        }
    }));
    assert!(out.contains("config:"));
    assert!(out.contains("  api_key: \"***\""));
    assert!(out.contains("  timeout: \"30s\""));
}

// ═══════════════════════════════════════════
// output_plain: logfmt format
// ═══════════════════════════════════════════

#[test]
fn plain_single_line() {
    let out = output_plain(&json!({"a": 1, "b": 2, "c": 3}));
    assert!(!out.contains('\n'));
}

#[test]
fn plain_key_value_pair() {
    let out = output_plain(&json!({"user_id": 123}));
    assert_eq!(out, "user_id=123");
}

#[test]
fn plain_sorted_keys() {
    let out = output_plain(&json!({"z": 1, "a": 2, "m": 3}));
    assert_eq!(out, "a=2 m=3 z=1");
}

#[test]
fn plain_dot_notation_nesting() {
    let out = output_plain(&json!({"trace": {"duration_ms": 150, "source": "db"}}));
    assert!(out.contains("trace.duration=150ms"));
    assert!(out.contains("trace.source=db"));
}

#[test]
fn plain_sorted_by_dot_path() {
    let out = output_plain(&json!({
        "kind": "result",
        "result": {"count": 3},
        "trace": {"duration_ms": 12}
    }));
    assert_eq!(out, "kind=result result.count=3 trace.duration=12ms");
}

#[test]
fn plain_quoted_spaces() {
    let out = output_plain(&json!({"message": "uploading chunks"}));
    assert!(out.contains("message=\"uploading chunks\""));
}

#[test]
fn plain_arrays_comma_joined() {
    let out = output_plain(&json!({"fields": ["email", "age"]}));
    assert!(out.contains("fields=email,age"));
}

#[test]
fn plain_null_empty() {
    let out = output_plain(&json!({"RUST_LOG": null}));
    assert!(out.contains("RUST_LOG="));
}

#[test]
fn plain_key_stripping_and_formatting() {
    let out = output_plain(&json!({"duration_ms": 1280, "api_key_secret": "sk-123"}));
    assert_eq!(out, "api_key=*** duration=1.28s");
}

#[test]
fn plain_deep_nesting() {
    let out = output_plain(&json!({"a": {"b": {"c": "deep"}}}));
    assert_eq!(out, "a.b.c=deep");
}

#[test]
fn plain_secrets_redacted() {
    let out = output_plain(&json!({"api_key_secret": "real-key"}));
    assert!(out.contains("api_key=***"));
    assert!(!out.contains("real-key"));
}

#[test]
fn plain_empty_object() {
    let out = output_plain(&json!({}));
    assert_eq!(out, "");
}

#[test]
fn plain_bool_unquoted() {
    let out = output_plain(&json!({"active": true, "disabled": false}));
    assert_eq!(out, "active=true disabled=false");
}

#[test]
fn plain_nested_secrets() {
    let out = output_plain(&json!({"config": {"api_key_secret": "real", "host": "localhost"}}));
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
    let out = output_plain(&json!({"size_bytes": 1024.5}));
    assert_eq!(out, "size_bytes=1024.5");
}

#[test]
fn fallthrough_bytes_string() {
    let out = output_plain(&json!({"size_bytes": "unknown"}));
    assert_eq!(out, "size_bytes=unknown");
}

#[test]
fn fallthrough_bytes_bool() {
    let out = output_plain(&json!({"size_bytes": false}));
    assert_eq!(out, "size_bytes=false");
}

#[test]
fn fallthrough_epoch_ms_float() {
    let out = output_plain(&json!({"created_epoch_ms": 1707868800000.5}));
    assert_eq!(out, "created_epoch_ms=1707868800000.5");
}

#[test]
fn fallthrough_epoch_ms_bool() {
    let out = output_plain(&json!({"created_epoch_ms": true}));
    assert_eq!(out, "created_epoch_ms=true");
}

#[test]
fn fallthrough_epoch_ms_string() {
    let out = output_plain(&json!({"created_epoch_ms": "yesterday"}));
    assert_eq!(out, "created_epoch_ms=yesterday");
}

#[test]
fn fallthrough_ms_string() {
    let out = output_plain(&json!({"latency_ms": "fast"}));
    assert_eq!(out, "latency_ms=fast");
}

#[test]
fn fallthrough_ms_bool() {
    let out = output_plain(&json!({"latency_ms": true}));
    assert_eq!(out, "latency_ms=true");
}

#[test]
fn fallthrough_s_string() {
    let out = output_plain(&json!({"dns_ttl_s": "auto"}));
    assert_eq!(out, "dns_ttl_s=auto");
}

#[test]
fn fallthrough_usd_cents_negative() {
    let out = output_plain(&json!({"refund_usd_cents": -499}));
    assert_eq!(out, "refund_usd_cents=-499");
}

#[test]
fn fallthrough_eur_cents_negative() {
    let out = output_plain(&json!({"refund_eur_cents": -100}));
    assert_eq!(out, "refund_eur_cents=-100");
}

#[test]
fn fallthrough_jpy_negative() {
    let out = output_plain(&json!({"refund_jpy": -1500}));
    assert_eq!(out, "refund_jpy=-1500");
}

#[test]
fn fallthrough_percent_string() {
    let out = output_plain(&json!({"cpu_percent": "high"}));
    assert_eq!(out, "cpu_percent=high");
}

#[test]
fn fallthrough_percent_bool() {
    let out = output_plain(&json!({"cpu_percent": true}));
    assert_eq!(out, "cpu_percent=true");
}

#[test]
fn fallthrough_btc_string() {
    let out = output_plain(&json!({"reserve_btc": "pending"}));
    assert_eq!(out, "reserve_btc=pending");
}

#[test]
fn fallthrough_msats_string() {
    let out = output_plain(&json!({"cost_msats": "pending"}));
    assert_eq!(out, "cost_msats=pending");
}

#[test]
fn fallthrough_minutes_string() {
    let out = output_plain(&json!({"timeout_minutes": "infinite"}));
    assert_eq!(out, "timeout_minutes=infinite");
}

// ═══════════════════════════════════════════
// Case sensitivity
// Only _secret/_SECRET, NOT _Secret/_sEcReT
// ═══════════════════════════════════════════

#[test]
fn case_lowercase_secret() {
    let out = output_plain(&json!({"api_key_secret": "real"}));
    assert!(out.contains("***"));
    assert!(!out.contains("real"));
}

#[test]
fn case_uppercase_secret() {
    let out = output_plain(&json!({"DATABASE_URL_SECRET": "postgres://..."}));
    assert!(out.contains("DATABASE_URL=***"));
}

#[test]
fn case_mixed_secret_not_matched() {
    let out = output_plain(&json!({"api_key_Secret": "real"}));
    assert!(out.contains("api_key_Secret=real"));
    assert!(!out.contains("***"));
}

#[test]
fn case_uppercase_s() {
    let out = output_plain(&json!({"CACHE_TTL_S": 3600}));
    assert!(out.contains("CACHE_TTL=3600s"));
}

#[test]
fn case_mixed_ms_not_matched() {
    let out = output_plain(&json!({"latency_Ms": 42}));
    assert_eq!(out, "latency_Ms=42");
}

// ═══════════════════════════════════════════
// Negative epoch timestamps
// ═══════════════════════════════════════════

#[test]
fn negative_epoch_ms() {
    let out = output_plain(&json!({"created_epoch_ms": -1000}));
    assert_eq!(out, "created=1969-12-31T23:59:59.000Z");
}

#[test]
fn negative_epoch_s() {
    let out = output_plain(&json!({"cached_epoch_s": -60}));
    assert_eq!(out, "cached=1969-12-31T23:59:00.000Z");
}

#[test]
fn negative_epoch_ns() {
    let out = output_plain(&json!({"created_epoch_ns": "-60000000000"}));
    assert_eq!(out, "created=1969-12-31T23:59:00.000Z");
}

#[test]
fn negative_epoch_ns_minus_one() {
    let out = output_plain(&json!({"created_epoch_ns": "-1"}));
    assert_eq!(out, "created=1969-12-31T23:59:59.999Z");
}

// ═══════════════════════════════════════════
// Negative bytes
// ═══════════════════════════════════════════

#[test]
fn negative_bytes_small() {
    let out = output_plain(&json!({"delta_bytes": -100}));
    assert_eq!(out, "delta_bytes=-100");
}

#[test]
fn negative_bytes_mb() {
    let out = output_plain(&json!({"delta_bytes": -5242880}));
    assert_eq!(out, "delta_bytes=-5242880");
}

// ═══════════════════════════════════════════
// Duration _ms boundary
// ═══════════════════════════════════════════

#[test]
fn ms_boundary_999() {
    let out = output_plain(&json!({"latency_ms": 999}));
    assert_eq!(out, "latency=999ms");
}

#[test]
fn ms_boundary_1000() {
    let out = output_plain(&json!({"latency_ms": 1000}));
    assert_eq!(out, "latency=1.0s");
}

#[test]
fn ms_boundary_1001() {
    let out = output_plain(&json!({"latency_ms": 1001}));
    assert_eq!(out, "latency=1.001s");
}

#[test]
fn ms_float_small() {
    let out = output_plain(&json!({"latency_ms": 0.5}));
    assert_eq!(out, "latency=0.5ms");
}

#[test]
fn ms_zero() {
    let out = output_plain(&json!({"latency_ms": 0}));
    assert_eq!(out, "latency=0ms");
}

// ═══════════════════════════════════════════
// Zero values
// ═══════════════════════════════════════════

#[test]
fn zero_bytes() {
    let out = output_plain(&json!({"size_bytes": 0}));
    assert_eq!(out, "size=0B");
}

#[test]
fn zero_percent() {
    let out = output_plain(&json!({"cpu_percent": 0}));
    assert_eq!(out, "cpu=0%");
}

#[test]
fn zero_usd_cents() {
    let out = output_plain(&json!({"price_usd_cents": 0}));
    assert_eq!(out, "price=$0.00");
}

#[test]
fn zero_s() {
    let out = output_plain(&json!({"timeout_s": 0}));
    assert_eq!(out, "timeout=0s");
}

// ═══════════════════════════════════════════
// Suffix priority (longest match first)
// ═══════════════════════════════════════════

#[test]
fn suffix_priority_epoch_ms_over_ms() {
    let out = output_plain(&json!({"created_at_epoch_ms": 1738886400000i64}));
    assert!(out.contains("2025-02-07"));
    assert!(!out.contains("ms"));
}

#[test]
fn suffix_priority_usd_cents_over_s() {
    let out = output_plain(&json!({"price_usd_cents": 999}));
    assert_eq!(out, "price=$9.99");
}

#[test]
fn suffix_priority_msats_over_s() {
    let out = output_plain(&json!({"cost_msats": 2056}));
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
    let v = build_cli_error("missing --sql", None);
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
    let v = build_cli_error("bad flag", Some("try --help"));
    assert_eq!(v["kind"], "error");
    assert_eq!(v["error"]["hint"], "try --help");
}

#[test]
fn build_cli_error_without_hint_has_no_hint_key() {
    let v = build_cli_error("oops", None);
    assert!(v.as_value()["error"].get("hint").is_none());
}

#[test]
fn cli_output_dispatches_json() {
    let v = crate::protocol::json_result(json!({"size_bytes": 1024}))
        .build()
        .expect("builder failed");
    let out = cli_output(v.as_value(), OutputFormat::Json);
    assert!(out.contains("size_bytes")); // json: raw keys, no suffix processing
    assert!(!out.contains('\n'));
}

#[test]
fn cli_output_dispatches_yaml() {
    let v = crate::protocol::json_result(json!({"size_bytes": 1024}))
        .build()
        .expect("builder failed");
    let out = cli_output(v.as_value(), OutputFormat::Yaml);
    assert!(out.starts_with("---"));
    assert!(out.contains("size:")); // yaml: suffix stripped
}

#[test]
fn cli_output_dispatches_plain() {
    let v = crate::protocol::json_result(json!({"size_bytes": 1024}))
        .build()
        .expect("builder failed");
    let out = cli_output(v.as_value(), OutputFormat::Plain);
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
    for format in [OutputFormat::Json, OutputFormat::Plain, OutputFormat::Yaml] {
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
    let v = build_cli_version("1.2.3");
    assert_eq!(v.as_value()["kind"], "result");
    assert_eq!(v.as_value()["result"]["version"], "1.2.3");
    // 0.16 API: trace is always present
    assert!(v.as_value().get("trace").is_some());
}

#[test]
fn cli_handle_version_defaults_to_json_for_agent_cli() {
    let raw = vec!["agent-cli".to_string(), "--version".to_string()];
    let out = cli_handle_version_or_continue(
        &raw,
        "agent-cli",
        "1.2.3",
        &VersionConfig::agent_cli_default(),
    )
    .expect("valid version request")
    .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["kind"], "result");
    assert_eq!(parsed["result"]["version"], "1.2.3");
}

#[test]
fn cli_handle_version_honors_explicit_output_formats() {
    let raw = vec![
        "agent-cli".to_string(),
        "--version".to_string(),
        "--output".to_string(),
        "plain".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        "agent-cli",
        "1.2.3",
        &VersionConfig::agent_cli_default(),
    )
    .expect("valid version request")
    .expect("version should render");
    assert!(out.contains("kind=result"), "{out}");
    assert!(out.contains("result.version=1.2.3"), "{out}");
}

#[test]
fn cli_handle_version_supports_inline_output_format() {
    let raw = vec![
        "agent-cli".to_string(),
        "--output=yaml".to_string(),
        "--version".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        "agent-cli",
        "1.2.3",
        &VersionConfig::agent_cli_default(),
    )
    .expect("valid version request")
    .expect("version should render");
    assert!(out.starts_with("---\n"), "{out}");
    assert!(out.contains("kind: \"result\""), "{out}");
    assert!(out.contains("version: \"1.2.3\""), "{out}");
}

#[test]
fn cli_handle_version_supports_json_alias() {
    let raw = vec![
        "agent-cli".to_string(),
        "--version".to_string(),
        "--json".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        "agent-cli",
        "1.2.3",
        &VersionConfig::conventional_default(),
    )
    .expect("valid version request")
    .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["kind"], "result");
    assert_eq!(parsed["result"]["version"], "1.2.3");
}

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
        "agent-cli",
        "1.2.3",
        &VersionConfig::conventional_default(),
    )
    .expect_err("conflicting formats must return error");
    assert_eq!(err["kind"], "error");
    assert!(
        err["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("conflicting output formats")),
        "error should mention conflict: {}",
        err["error"]["message"]
    );
}

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
        "agent-cli",
        "1.2.3",
        &VersionConfig::agent_cli_default(),
    )
    .expect_err("invalid version output must return error");
    assert_eq!(err["kind"], "error");
    assert_eq!(err["error"]["code"], "cli_error");
    assert!(
        err["error"]["message"]
            .as_str()
            .is_some_and(|s| s.contains("xml")),
        "error should mention invalid value: {err}"
    );
}

#[test]
fn cli_handle_version_can_preserve_conventional_bare_text() {
    let raw = vec!["agent-cli".to_string(), "--version".to_string()];
    let out = cli_handle_version_or_continue(
        &raw,
        "agent-cli",
        "1.2.3",
        &VersionConfig::conventional_default(),
    )
    .expect("valid version request")
    .expect("version should render");
    assert_eq!(out, "agent-cli 1.2.3\n");
}

#[test]
fn cli_handle_version_conventional_mode_still_honors_json_output() {
    let raw = vec![
        "agent-cli".to_string(),
        "--version".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        "agent-cli",
        "1.2.3",
        &VersionConfig::conventional_default(),
    )
    .expect("valid version request")
    .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["kind"], "result");
    assert_eq!(parsed["result"]["version"], "1.2.3");
}

#[test]
fn cli_handle_version_protocol_v1_adds_code_and_trace() {
    let raw = vec![
        "agent-cli".to_string(),
        "--version".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    let out = cli_handle_version_or_continue(
        &raw,
        "agent-cli",
        "1.2.3",
        &VersionConfig::conventional_default().with_protocol_v1(),
    )
    .expect("valid version request")
    .expect("version should render");
    let parsed: Value = serde_json::from_str(out.trim()).expect("version json must parse");
    assert_eq!(parsed["kind"], "result");
    assert_eq!(parsed["result"]["code"], "version");
    assert_eq!(parsed["result"]["version"], "1.2.3");
    assert_eq!(parsed["trace"], serde_json::json!({}));
    validate_protocol_event(&parsed, true).expect("strict protocol event");
}

#[test]
fn cli_handle_version_without_version_returns_none() {
    let raw = vec!["agent-cli".to_string(), "ping".to_string()];
    assert!(
        cli_handle_version_or_continue(
            &raw,
            "agent-cli",
            "1.2.3",
            &VersionConfig::agent_cli_default(),
        )
        .expect("valid non-version request")
        .is_none()
    );
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
    let out = output_yaml(&data);
    assert!(out.starts_with("---\n"));
    assert!(out.contains("api_key: \"***\""));
    assert!(out.contains("cache_ttl: \"3600s\""));
    assert!(out.contains("count: 42"));
    assert!(out.contains("created_at: \"2025-02-07T00:00:00.000Z\""));
    assert!(out.contains("file_size: \"5.0MiB\""));
    assert!(out.contains("payment: \"50000000msats\""));
    assert!(out.contains("price: \"$99.99\""));
    assert!(out.contains("request_timeout: \"5.0s\""));
    assert!(out.contains("success_rate: \"95.5%\""));
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
    let out = output_plain(&data);
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
    let out = output_json(&data);
    assert!(out.contains("\"api_key_secret\":\"***\""));
    assert!(out.contains("\"created_at_epoch_ms\":1738886400000"));
    assert!(out.contains("\"file_size_bytes\":5242880"));
    assert!(out.contains("\"user_id\":123"));
    assert!(!out.contains("sk-1234567890abcdef"));
    assert!(!out.contains('\n'));
}

#[test]
fn readme_cli_startup_yaml() {
    let startup_val = crate::protocol::json_log(LogLevel::Info, "startup")
        .fields(json!({
            "config": {"api_key_secret": "sk-sensitive-key", "timeout_s": 30},
            "args": {"input_path": "data.json"},
            "env": {"RUST_LOG": "info"}
        }))
        .build()
        .expect("builder failed");
    let out = output_yaml(&startup_val);
    assert!(out.contains("kind: \"log\""));
    assert!(out.contains("api_key: \"***\""));
    assert!(out.contains("timeout: \"30s\""));
    assert!(out.contains("input_path: \"data.json\""));
    assert!(out.contains("RUST_LOG: \"info\""));
    assert!(!out.contains("sk-sensitive-key"));
}

#[test]
fn readme_cli_progress_plain() {
    let progress = crate::protocol::json_progress("processing")
        .field("current", json!(3))
        .field("total", json!(10))
        .trace(json!({"duration_ms": 1500}))
        .build()
        .expect("builder failed");
    let out = output_plain(&progress);
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
        .build()
        .expect("builder failed");
    let out = output_json(&result);
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
        .build()
        .expect("builder failed");
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
    let event = crate::protocol::json_progress("uploading")
        .field("current", json!(3))
        .field("total", json!(10))
        .build()
        .expect("builder failed");
    let text = serde_json::to_string(&event).expect("serialize");
    match decode_protocol_event(&text).expect("decode progress") {
        DecodedEvent::Progress(decoded) => {
            assert_eq!(decoded.message, "uploading");
            assert_eq!(decoded.fields.get("current"), Some(&json!(3)));
            assert_eq!(decoded.fields.get("total"), Some(&json!(10)));
        }
        other => panic!("expected DecodedEvent::Progress, got {other:?}"),
    }
}

#[test]
fn decode_log_event_with_extension_fields() {
    let event = crate::protocol::json_log(LogLevel::Warn, "disk low")
        .field("free_bytes", json!(1024))
        .build()
        .expect("builder failed");
    let text = serde_json::to_string(&event).expect("serialize");
    match decode_protocol_event(&text).expect("decode log") {
        DecodedEvent::Log(decoded) => {
            assert_eq!(decoded.level, LogLevel::Warn);
            assert_eq!(decoded.message, "disk low");
            assert_eq!(decoded.fields.get("free_bytes"), Some(&json!(1024)));
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
