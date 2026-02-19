use super::*;
use serde_json::{json, Value};

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../spec/fixtures");

fn load_fixture(name: &str) -> Value {
    let path = format!("{}/{}", FIXTURES_DIR, name);
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
    serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("failed to parse {}: {}", path, e))
}

// ═══════════════════════════════════════════
// Fixture-driven tests (cross-language spec)
// ═══════════════════════════════════════════

#[test]
fn test_redact_fixtures() {
    let cases = load_fixture("redact.json");
    for case in cases.as_array().expect("redact.json must be an array") {
        let name = case["name"].as_str().expect("missing name");
        let mut input = case["input"].clone();
        let expected = &case["expected"];
        internal_redact_secrets(&mut input);
        assert_eq!(&input, expected, "[redact/{name}]");
    }
}

#[test]
fn test_protocol_fixtures() {
    let cases = load_fixture("protocol.json");
    for case in cases.as_array().expect("protocol.json must be an array") {
        let name = case["name"].as_str().expect("missing name");
        let typ = case["type"].as_str().expect("missing type");
        let args = &case["args"];
        let result = match typ {
            "ok" => build_json_ok(args["result"].clone(), None),
            "ok_trace" => build_json_ok(
                args["result"].clone(),
                Some(args["trace"].clone()),
            ),
            "error" => build_json_error(
                args["message"].as_str().expect("missing message"),
                None,
            ),
            "error_trace" => build_json_error(
                args["message"].as_str().expect("missing message"),
                Some(args["trace"].clone()),
            ),
            "status" => {
                let code = args["code"].as_str().expect("missing code");
                let fields = args["fields"].clone();
                build_json(code, fields, None)
            }
            other => panic!("unknown protocol type: {other}"),
        };
        if let Some(expected) = case.get("expected") {
            assert_eq!(&result, expected, "[protocol/{name}]");
        }
        if let Some(expected_contains) = case.get("expected_contains") {
            let ec = expected_contains.as_object().expect("expected_contains must be object");
            let ro = result.as_object().expect("result must be object");
            for (k, v) in ec {
                assert_eq!(ro.get(k).unwrap_or(&Value::Null), v, "[protocol/{name}] key {k}");
            }
        }
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
                    let input = arr[0].as_i64().expect("input must be i64");
                    let expected = arr[1].as_str().expect("expected must be string");
                    assert_eq!(format_bytes_human(input), expected, "[helpers/format_bytes_human({input})]");
                }
            }
            "format_with_commas" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_u64().expect("input must be u64");
                    let expected = arr[1].as_str().expect("expected must be string");
                    assert_eq!(format_with_commas(input), expected, "[helpers/format_with_commas({input})]");
                }
            }
            "extract_currency_code" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = if arr[1].is_null() { None } else { arr[1].as_str() };
                    assert_eq!(extract_currency_code(input), expected, "[helpers/extract_currency_code({input})]");
                }
            }
            "parse_size" => {
                for tc in test_cases {
                    let arr = tc.as_array().expect("case must be [input, expected]");
                    let input = arr[0].as_str().expect("input must be string");
                    let expected = if arr[1].is_null() { None } else { arr[1].as_u64() };
                    assert_eq!(parse_size(input), expected, "[helpers/parse_size({input:?})]");
                }
            }
            other => panic!("unknown helper: {other}"),
        }
    }
}

// ═══════════════════════════════════════════
// Protocol builders
// ═══════════════════════════════════════════

#[test]
fn build_ok_with_trace() {
    let v = build_json_ok(
        json!({"count": 42}),
        Some(json!({"duration_ms": 150})),
    );
    assert_eq!(v["code"], "ok");
    assert_eq!(v["result"]["count"], 42);
    assert_eq!(v["trace"]["duration_ms"], 150);
}

#[test]
fn build_ok_without_trace() {
    let v = build_json_ok(json!({"count": 42}), None);
    assert_eq!(v["code"], "ok");
    assert_eq!(v["result"]["count"], 42);
    assert!(v.get("trace").is_none() || v["trace"].is_null());
}

#[test]
fn build_error_with_trace() {
    let v = build_json_error("not found", Some(json!({"duration_ms": 5})));
    assert_eq!(v["code"], "error");
    assert_eq!(v["error"], "not found");
    assert_eq!(v["trace"]["duration_ms"], 5);
}

#[test]
fn build_error_without_trace() {
    let v = build_json_error("fail", None);
    assert_eq!(v["code"], "error");
    assert_eq!(v["error"], "fail");
    assert!(v.get("trace").is_none() || v["trace"].is_null());
}

#[test]
fn build_error_empty_message() {
    let v = build_json_error("", None);
    assert_eq!(v["code"], "error");
    assert_eq!(v["error"], "");
}

#[test]
fn build_generic_with_trace() {
    let v = build_json(
        "not_found",
        json!({"resource": "user", "id": 123}),
        Some(json!({"duration_ms": 8})),
    );
    assert_eq!(v["code"], "not_found");
    assert_eq!(v["resource"], "user");
    assert_eq!(v["id"], 123);
    assert_eq!(v["trace"]["duration_ms"], 8);
}

#[test]
fn build_generic_without_trace() {
    let v = build_json("progress", json!({"current": 3}), None);
    assert_eq!(v["code"], "progress");
    assert_eq!(v["current"], 3);
    assert!(v.get("trace").is_none() || v["trace"].is_null());
}

#[test]
fn build_generic_non_object_fields() {
    let v = build_json("test", json!("string_value"), None);
    assert_eq!(v["code"], "test");
}

#[test]
fn build_generic_code_overrides_fields() {
    let v = build_json("real", json!({"code": "fake"}), None);
    assert_eq!(v["code"], "real");
}

#[test]
fn build_generic_trace_overrides_fields_trace() {
    let v = build_json(
        "test",
        json!({"trace": "should_be_overridden", "other": 1}),
        Some(json!({"duration_ms": 5})),
    );
    assert_eq!(v["trace"]["duration_ms"], 5);
    assert_eq!(v["other"], 1);
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
    assert!(!out.contains("MB"));
}

#[test]
fn json_non_string_secret_redacted() {
    let out = output_json(&json!({"count_secret": 42}));
    assert!(out.contains("\"***\""));
    assert!(!out.contains("42"));
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
    let out = output_yaml(&json!({"created_epoch_ns": 1707868800000000000i64}));
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
    assert!(out.contains("reserve:"));
    assert!(!out.contains("reserve_btc"));
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
    assert!(out.contains("\"5.0MB\""));
}

#[test]
fn yaml_fmt_bytes_kb() {
    let out = output_yaml(&json!({"payload_bytes": 456789}));
    assert!(out.contains("\"446.1KB\""));
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
    assert!(out.contains("\"0.5 BTC\""));
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
        "code": "ok",
        "result": {"count": 3},
        "trace": {"duration_ms": 12}
    }));
    assert_eq!(out, "code=ok result.count=3 trace.duration=12ms");
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
    let out = output_plain(&json!({"created_epoch_ns": -60000000000i64}));
    assert_eq!(out, "created=1969-12-31T23:59:00.000Z");
}

#[test]
fn negative_epoch_ns_minus_one() {
    let out = output_plain(&json!({"created_epoch_ns": -1}));
    assert_eq!(out, "created=1969-12-31T23:59:59.999Z");
}

// ═══════════════════════════════════════════
// Negative bytes
// ═══════════════════════════════════════════

#[test]
fn negative_bytes_small() {
    let out = output_plain(&json!({"delta_bytes": -100}));
    assert_eq!(out, "delta=-100B");
}

#[test]
fn negative_bytes_mb() {
    let out = output_plain(&json!({"delta_bytes": -5242880}));
    assert_eq!(out, "delta=-5.0MB");
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
// internal_redact_secrets
// ═══════════════════════════════════════════

#[test]
fn redact_flat() {
    let mut v = json!({"api_key_secret": "sk-123", "name": "test"});
    internal_redact_secrets(&mut v);
    assert_eq!(v["api_key_secret"], "***");
    assert_eq!(v["name"], "test");
}

#[test]
fn redact_nested() {
    let mut v = json!({"config": {"password_secret": "real"}});
    internal_redact_secrets(&mut v);
    assert_eq!(v["config"]["password_secret"], "***");
}

#[test]
fn redact_array_traversal() {
    let mut v = json!([{"api_key_secret": "a"}, {"token_secret": "b"}]);
    internal_redact_secrets(&mut v);
    assert_eq!(v[0]["api_key_secret"], "***");
    assert_eq!(v[1]["token_secret"], "***");
}

#[test]
fn redact_non_string_redacted() {
    let mut v = json!({"count_secret": 42});
    internal_redact_secrets(&mut v);
    assert_eq!(v["count_secret"], "***");
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
    assert!(out.contains("file_size: \"5.0MB\""));
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
        "api_key=*** cache_ttl=3600s count=42 created_at=2025-02-07T00:00:00.000Z file_size=5.0MB payment=50000000msats price=$99.99 request_timeout=5.0s success_rate=95.5% user_name=alice"
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
    let startup_val = build_json(
        "startup",
        json!({
            "config": {"api_key_secret": "sk-sensitive-key", "timeout_s": 30},
            "args": {"input_path": "data.json"},
            "env": {"RUST_LOG": "info"}
        }),
        None,
    );
    let out = output_yaml(&startup_val);
    assert!(out.contains("code: \"startup\""));
    assert!(out.contains("api_key: \"***\""));
    assert!(out.contains("timeout: \"30s\""));
    assert!(out.contains("input_path: \"data.json\""));
    assert!(out.contains("RUST_LOG: \"info\""));
    assert!(!out.contains("sk-sensitive-key"));
}

#[test]
fn readme_cli_progress_plain() {
    let progress = build_json(
        "progress",
        json!({"current": 3, "total": 10, "message": "processing"}),
        Some(json!({"duration_ms": 1500})),
    );
    let out = output_plain(&progress);
    assert!(out.contains("code=progress"));
    assert!(out.contains("current=3"));
    assert!(out.contains("message=processing"));
    assert!(out.contains("total=10"));
    assert!(out.contains("trace.duration=1.5s"));
}

#[test]
fn readme_jsonl_output() {
    let result = build_json_ok(
        json!({"status": "success"}),
        Some(json!({"duration_ms": 250, "api_key_secret": "sk-123"})),
    );
    let out = output_json(&result);
    assert!(out.contains("\"code\":\"ok\""));
    assert!(out.contains("\"status\":\"success\""));
    assert!(out.contains("\"api_key_secret\":\"***\""));
    assert!(!out.contains("sk-123"));
    assert!(!out.contains('\n'));
}
