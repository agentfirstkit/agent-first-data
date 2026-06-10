# Logging `secret_names` design record

Status: **completed**. All four language ports expose logging initialization paths that accept legacy secret names and apply them through the same redaction options used by normal output formatting.

## What shipped

- Rust: `afdata_tracing::init_*_with_options` and `try_init_*_with_options` accept `RedactionOptions`.
- Go: `NewAfdataHandlerWithOptions`, `Init*WithOptions`, and `Init*LevelWithOptions` accept `RedactionOptions`.
- Python: `AfdataHandler`, `init_json`, `init_plain`, and `init_yaml` accept either `redaction=` or `secret_names=`.
- TypeScript: `initJson`, `initPlain`, and `initYaml` accept `{ secretNames }`.

## Preserved invariants

- `code` is always pinned to `"log"`; event fields cannot override it.
- Required envelope fields stay `timestamp_epoch_ms`, `message`, `code`, and `level`.
- The free-form `message` string is not name-redacted; secrets belong in named `_secret` fields or configured legacy fields.
- Redaction runs before formatting through `output_*_with_options`.
- Log/protocol events continue to use stdout only; stderr is not a protocol stream.

## Redaction behavior

Configured legacy names are exact matches. When `secret_names` / `SecretNames` / `secretNames` includes `authorization`, a log field named `authorization` is redacted to `***`, and an `_url` field with a query parameter named `authorization` has that parameter value redacted. Without the option, those legacy names stay visible.

This record replaces the original implementation TODO so future changes have a concise compatibility checklist.
