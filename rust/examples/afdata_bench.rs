use agent_first_data::{
    OutputOptions, OutputStyle, output_json, output_plain, output_yaml, redacted_value,
};
use serde_json::{Value, json};
use std::hint::black_box;
use std::io::{self, Write};
use std::time::Instant;

#[derive(Clone, Copy)]
struct InputSize {
    name: &'static str,
    rows: usize,
    iterations: u64,
}

const INPUT_SIZES: &[InputSize] = &[
    InputSize {
        name: "small",
        rows: 8,
        iterations: 20_000,
    },
    InputSize {
        name: "medium",
        rows: 128,
        iterations: 2_000,
    },
    InputSize {
        name: "large",
        rows: 2_048,
        iterations: 100,
    },
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "{}",
        json!({
            "kind": "log",
            "log": {
                "benchmark": "afdata_formatters",
                "note": "Dependency-free baseline. Use for regression/profiling before choosing optimizations."
            }
        })
    )?;

    for size in INPUT_SIZES {
        let input = build_input(*size);
        run_case(&mut stdout, "json", *size, &input, bench_json)?;
        run_case(&mut stdout, "yaml", *size, &input, bench_yaml)?;
        run_case(&mut stdout, "plain", *size, &input, bench_plain)?;
        run_case(&mut stdout, "redaction", *size, &input, bench_redaction)?;
    }

    Ok(())
}

fn run_case(
    stdout: &mut impl Write,
    operation: &str,
    size: InputSize,
    input: &Value,
    mut bench: impl FnMut(&Value) -> usize,
) -> io::Result<()> {
    let started = Instant::now();
    let mut observed_bytes = 0usize;
    for _ in 0..size.iterations {
        observed_bytes = observed_bytes.wrapping_add(black_box(bench(input)));
    }
    let elapsed = started.elapsed();
    let elapsed_ns = elapsed.as_nanos();
    let ns_per_iter = elapsed_ns / u128::from(size.iterations);
    writeln!(
        stdout,
        "{}",
        json!({
            "kind": "result",
            "result": {
                "operation": operation,
                "input_scale": size.name,
                "rows": size.rows,
                "iterations": size.iterations,
                "elapsed_ns": elapsed_ns,
                "ns_per_iter": ns_per_iter,
                "observed_bytes": observed_bytes
            }
        })
    )
}

fn bench_json(value: &Value) -> usize {
    output_json(black_box(value)).len()
}

fn bench_yaml(value: &Value) -> usize {
    output_yaml(black_box(value)).len()
}

fn bench_plain(value: &Value) -> usize {
    output_plain(black_box(value)).len()
}

fn bench_redaction(value: &Value) -> usize {
    let redacted = redacted_value(black_box(value));
    value_node_count(black_box(&redacted))
}

fn build_input(size: InputSize) -> Value {
    let rows = (0..size.rows)
        .map(|idx| {
            json!({
                "id": idx,
                "request_duration_ms": 25 + (idx % 9) * 125,
                "payload_size_bytes": 512 + idx * 257,
                "created_at_epoch_ms": 1_738_886_400_000i64 + (idx as i64 * 1_000),
                "price_usd_cents": 1_299 + idx,
                "success_rate_percent": 97.5,
                "api_key_secret": format!("bench-secret-{idx}"),
                "callback_url": format!("https://user:bench-secret-{idx}@example.test/callback?trace={idx}&token_secret=bench-secret-{idx}"),
                "tags": ["alpha", "beta", "gamma"],
                "metadata": {
                    "attempt_count": idx % 5,
                    "cache_ttl_s": 60,
                    "nested_secret": {
                        "raw": format!("nested-bench-secret-{idx}")
                    }
                }
            })
        })
        .collect::<Vec<_>>();

    let output_options = OutputOptions {
        redaction: Default::default(),
        style: OutputStyle::Readable,
    };

    json!({
        "dataset": size.name,
        "rows": rows,
        "output_options_debug": format!("{output_options:?}"),
        "root_secret": "bench-root-secret"
    })
}

fn value_node_count(value: &Value) -> usize {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 1,
        Value::Array(values) => 1 + values.iter().map(value_node_count).sum::<usize>(),
        Value::Object(map) => 1 + map.values().map(value_node_count).sum::<usize>(),
    }
}
