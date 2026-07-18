//! Format-independent document values: dot-path get/set/add/remove, type-directed
//! CLI-string coercion, keyed-list (slug-addressed array) helpers, and typed
//! serde adapters, backed by pluggable JSON/TOML/YAML/dotenv/INI format readers
//! and writers. Ported from the standalone `agent-first-config` crate.
//!
//! # Features
//!
//! JSON support is always compiled in — `agent_first_data` already depends on
//! `serde_json` as a core dependency.
//!
//! - **toml**: Enable TOML format support (format-preserving with toml_edit)
//! - **yaml**: Enable YAML format support with CST-backed source-preserving mutation
//! - **dotenv**: Enable source-preserving dotenv format support
//! - **ini**: Enable INI Core v1 format support
//! - **schema**: Enable the `CliSchema` trait and documentation rendering
//!
//! This module never redacts values on decode/encode/save — it returns and
//! saves raw values as-is; redaction is the caller's responsibility.

pub mod coerce;
pub mod error;
pub mod file;
pub mod keyed;
pub mod path;
pub mod traverse;
pub mod typed;
pub mod value;

pub mod format;

#[cfg(feature = "schema")]
pub mod schema;

pub use coerce::{ScalarKind, ValueType, guard_bare_overwrite, scalar_kind, value_from_type};
pub use error::{DocumentError, DocumentResult};
pub use file::{Document, DocumentFile};
pub use keyed::{KeyedList, add_keyed, remove_keyed};
pub use path::{join_path, parse_path};
pub use traverse::{get_path, get_path_ref, set_path, unset_path};
pub use typed::{from_value, to_value};
pub use value::Value;

pub use format::Format;

#[cfg(feature = "schema")]
pub use schema::{
    CliSchema, FieldDef, render_annotated_toml, render_annotated_yaml, render_doc_markdown,
};
