use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Output format for CLI and pipe/MCP modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    /// Structure-preserving YAML (same semantics as [`OutputFormat::Json`]).
    Yaml,
    Plain,
}

impl OutputFormat {
    /// Return the canonical CLI/config spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Plain => "plain",
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "json" => Ok(Self::Json),
            "yaml" => Ok(Self::Yaml),
            "plain" => Ok(Self::Plain),
            _ => Err("invalid output format: expected json, yaml, or plain".to_string()),
        }
    }
}

/// Where a CLI emitter sends its events.
#[cfg(feature = "cli")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputTo {
    /// Finite one-shot: `result` → stdout, `error`/`progress`/`log` → stderr.
    Split,
    /// Event stream: every event onto stdout.
    Stdout,
    /// Event stream: every event onto stderr.
    Stderr,
}

#[cfg(feature = "cli")]
impl OutputTo {
    /// Return the canonical CLI/config spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }

    /// Parse an `--output-to` value.
    pub fn parse(value: &str) -> Result<Self, String> {
        value.parse()
    }
}

#[cfg(feature = "cli")]
impl fmt::Display for OutputTo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(feature = "cli")]
impl FromStr for OutputTo {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "split" => Ok(Self::Split),
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            _ => Err("unsupported --output-to: expected split, stdout, or stderr".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutputFormat;
    #[cfg(feature = "cli")]
    use super::OutputTo;

    #[test]
    fn output_types_round_trip_text_and_serde() {
        for format in [OutputFormat::Json, OutputFormat::Yaml, OutputFormat::Plain] {
            assert_eq!(format.to_string().parse(), Ok(format));
            assert_eq!(
                serde_json::from_str::<OutputFormat>(
                    &serde_json::to_string(&format).unwrap_or_default()
                )
                .ok(),
                Some(format)
            );
        }
        #[cfg(feature = "cli")]
        {
            for destination in [OutputTo::Split, OutputTo::Stdout, OutputTo::Stderr] {
                assert_eq!(destination.to_string().parse(), Ok(destination));
                assert_eq!(
                    serde_json::from_str::<OutputTo>(
                        &serde_json::to_string(&destination).unwrap_or_default()
                    )
                    .ok(),
                    Some(destination)
                );
            }
        }

        let format_canary = "canary-output-format-secret";
        let format_error = format_canary.parse::<OutputFormat>().unwrap_err();
        assert!(!format_error.contains(format_canary));
        assert!(format_error.contains("json"));

        #[cfg(feature = "cli")]
        {
            let destination_canary = "canary-output-to-secret";
            let destination_error = destination_canary.parse::<OutputTo>().unwrap_err();
            assert!(!destination_error.contains(destination_canary));
            assert!(destination_error.contains("split"));
        }
    }
}
