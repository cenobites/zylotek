//! Logging and tracing for every Zylotek binary.
//!
//! One call to [`init`] installs a `tracing` subscriber. Events from `tracing` and from the `log`
//! macros the other crates use both end up in it, and spans (such as the per-request span of
//! `tower_http::trace::TraceLayer`) are included in each line.
//!
//! * Servers use [`LoggingConfig::server`]: one JSON object per line on stdout.
//! * The CLI uses [`LoggingConfig::cli`]: readable text on stderr, so stdout stays for output.
//! * `RUST_LOG` replaces the configured filter, and `ZYLOTEK_LOG_FORMAT` (`json` or `pretty`)
//!   replaces the format, when [`LoggingConfig::with_env_overrides`] is applied.

use std::{io, str::FromStr};

use tracing::Subscriber;
use tracing_subscriber::{
    EnvFilter, Layer, Registry,
    fmt::{self, MakeWriter},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

/// Replaces the filter in [`LoggingConfig`], like `RUST_LOG=zylotek_api=debug,info`.
pub const FILTER_ENV_VAR: &str = "RUST_LOG";

/// Replaces the format in [`LoggingConfig`]: `json` or `pretty`.
pub const FORMAT_ENV_VAR: &str = "ZYLOTEK_LOG_FORMAT";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// One JSON object per line, with the current span and its parents.
    Json,
    /// Human-readable text.
    Pretty,
}

impl FromStr for LogFormat {
    type Err = LoggingError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "pretty" => Ok(Self::Pretty),
            _ => Err(LoggingError::UnknownFormat(text.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOutput {
    Stdout,
    Stderr,
}

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("invalid log filter {filter:?}: {reason}")]
    InvalidFilter { filter: String, reason: String },

    #[error("unknown log format {0:?}; expected `json` or `pretty`")]
    UnknownFormat(String),

    #[error("logging is already initialised in this process")]
    AlreadyInitialised,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingConfig {
    /// Which targets log at which level, in `RUST_LOG` syntax.
    pub filter: String,
    pub format: LogFormat,
    pub output: LogOutput,
}

impl LoggingConfig {
    /// For the long-running services: JSON on stdout, `info` and above.
    pub fn server() -> Self {
        Self {
            filter: "info".to_owned(),
            format: LogFormat::Json,
            output: LogOutput::Stdout,
        }
    }

    /// For the command-line tool: text on stderr, `info` and above.
    pub fn cli() -> Self {
        Self {
            filter: "info".to_owned(),
            format: LogFormat::Pretty,
            output: LogOutput::Stderr,
        }
    }

    /// Lets `ZYLOTEK_LOG_FORMAT` choose the format. An unrecognised value is an error rather
    /// than being ignored, so a typo does not silently change the log format.
    pub fn with_env_overrides(self) -> Result<Self, LoggingError> {
        self.with_format_override(std::env::var(FORMAT_ENV_VAR).ok().as_deref())
    }

    fn with_format_override(mut self, format: Option<&str>) -> Result<Self, LoggingError> {
        if let Some(format) = format.filter(|format| !format.trim().is_empty()) {
            self.format = format.parse()?;
        }
        Ok(self)
    }
}

/// [`init`] after applying [`LoggingConfig::with_env_overrides`]: what a binary calls.
pub fn init_with_env(config: LoggingConfig) -> Result<(), LoggingError> {
    init(&config.with_env_overrides()?)
}

/// Installs the process-wide subscriber (and the `log` bridge) described by `config`.
///
/// Call it once, early, from the binary. A second call returns
/// [`LoggingError::AlreadyInitialised`].
pub fn init(config: &LoggingConfig) -> Result<(), LoggingError> {
    let rust_log = std::env::var(FILTER_ENV_VAR).ok();
    let filter = filter(rust_log.as_deref(), &config.filter)?;

    let result = match config.output {
        LogOutput::Stdout => subscriber(config.format, filter, io::stdout).try_init(),
        LogOutput::Stderr => subscriber(config.format, filter, io::stderr).try_init(),
    };
    result.map_err(|_| LoggingError::AlreadyInitialised)
}

/// `rust_log` (the `RUST_LOG` value) when it is set and valid, otherwise `default`.
fn filter(rust_log: Option<&str>, default: &str) -> Result<EnvFilter, LoggingError> {
    rust_log
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| EnvFilter::try_new(value).ok())
        .map_or_else(
            || {
                EnvFilter::try_new(default).map_err(|error| LoggingError::InvalidFilter {
                    filter: default.to_owned(),
                    reason: error.to_string(),
                })
            },
            Ok,
        )
}

/// The subscriber `init` installs, writing to `writer`. Separate from `init` so it can be
/// tested without touching process-wide state.
fn subscriber<W>(format: LogFormat, filter: EnvFilter, writer: W) -> impl Subscriber + Send + Sync
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = match format {
        LogFormat::Json => fmt::layer().json().with_writer(writer).boxed(),
        LogFormat::Pretty => fmt::layer().with_writer(writer).boxed(),
    };
    Registry::default().with(layer).with(filter)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A writer that keeps what is written to it.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture(format: LogFormat, filter_text: &str, emit: impl FnOnce()) -> String {
        let capture = Capture::default();
        let filter = filter(None, filter_text).unwrap();
        let subscriber = subscriber(format, filter, capture.clone());
        tracing::subscriber::with_default(subscriber, emit);
        capture.text()
    }

    #[test]
    fn the_capture_writer_keeps_what_it_is_given() {
        let mut writer = Capture::default();

        io::Write::write_all(&mut writer, b"kept").unwrap();
        io::Write::flush(&mut writer).unwrap();

        assert_eq!(writer.text(), "kept");
    }

    #[test]
    fn servers_log_json_to_stdout_and_the_cli_text_to_stderr() {
        let server = LoggingConfig::server();
        assert_eq!(server.format, LogFormat::Json);
        assert_eq!(server.output, LogOutput::Stdout);
        assert_eq!(server.filter, "info");

        let cli = LoggingConfig::cli();
        assert_eq!(cli.format, LogFormat::Pretty);
        assert_eq!(cli.output, LogOutput::Stderr);
        assert_eq!(cli.filter, "info");
    }

    #[test]
    fn json_lines_carry_the_message_fields_and_enclosing_spans() {
        let output = capture(LogFormat::Json, "info", || {
            let span = tracing::info_span!("request", id = 7);
            let _entered = span.enter();
            tracing::info!(count = 3, "catalogue synced");
        });

        let line: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(line["level"], "INFO");
        assert_eq!(line["fields"]["message"], "catalogue synced");
        assert_eq!(line["fields"]["count"], 3);
        assert_eq!(line["span"]["name"], "request");
        assert_eq!(line["span"]["id"], 7);
    }

    #[test]
    fn pretty_output_is_plain_text() {
        let output = capture(LogFormat::Pretty, "info", || {
            tracing::warn!("disk almost full");
        });

        assert!(output.contains("WARN"), "{output}");
        assert!(output.contains("disk almost full"), "{output}");
        assert!(!output.trim_start().starts_with('{'), "{output}");
    }

    #[test]
    fn the_filter_drops_events_below_its_level_and_keeps_the_rest() {
        let output = capture(LogFormat::Json, "warn", || {
            tracing::info!("routine");
            tracing::error!("broken");
        });

        assert!(!output.contains("routine"), "{output}");
        assert!(output.contains("broken"), "{output}");
    }

    #[test]
    fn the_filter_can_target_one_crate() {
        let output = capture(
            LogFormat::Json,
            "error,zylotek_infrastructure=debug",
            || {
                tracing::debug!("from this crate");
            },
        );

        assert!(output.contains("from this crate"), "{output}");
    }

    #[test]
    fn rust_log_replaces_the_configured_filter_when_it_is_valid() {
        let from_env = filter(Some("debug"), "error").unwrap();
        assert_eq!(from_env.to_string(), "debug");
    }

    #[test]
    fn an_empty_or_invalid_rust_log_falls_back_to_the_configured_filter() {
        for rust_log in [None, Some(""), Some("  "), Some("=[bad")] {
            let used = filter(rust_log, "warn").unwrap();
            assert_eq!(used.to_string(), "warn", "{rust_log:?}");
        }
    }

    #[test]
    fn an_invalid_configured_filter_is_reported() {
        let error = filter(None, "zylotek=[bad").unwrap_err();
        assert!(matches!(error, LoggingError::InvalidFilter { .. }));
        assert!(error.to_string().contains("zylotek=[bad"));
    }

    #[test]
    fn formats_parse_case_insensitively_and_reject_anything_else() {
        assert_eq!("json".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!(" Pretty ".parse::<LogFormat>().unwrap(), LogFormat::Pretty);
        let error = "xml".parse::<LogFormat>().unwrap_err();
        assert!(matches!(error, LoggingError::UnknownFormat(_)));
        assert!(error.to_string().contains("xml"));
    }

    #[test]
    fn the_format_override_changes_only_the_format() {
        let base = LoggingConfig::server();

        let pretty = base.clone().with_format_override(Some("pretty")).unwrap();
        assert_eq!(pretty.format, LogFormat::Pretty);
        assert_eq!(pretty.output, base.output);
        assert_eq!(pretty.filter, base.filter);

        for unset in [None, Some(""), Some("  ")] {
            assert_eq!(base.clone().with_format_override(unset).unwrap(), base);
        }
        assert!(base.with_format_override(Some("xml")).is_err());
    }

    #[test]
    fn the_environment_override_leaves_the_config_alone_when_the_variable_is_absent() {
        // Reads the real environment; the expectation is built from the same variable.
        let cli = LoggingConfig::cli();
        let from_env = std::env::var(FORMAT_ENV_VAR).ok();

        assert_eq!(
            cli.clone().with_env_overrides().ok(),
            cli.with_format_override(from_env.as_deref()).ok()
        );
    }

    #[test]
    fn init_installs_once_and_bridges_the_log_crate() {
        let config = LoggingConfig {
            filter: "off".to_owned(),
            ..LoggingConfig::cli()
        };

        init_with_env(config.clone()).unwrap();
        assert!(matches!(
            init(&config).unwrap_err(),
            LoggingError::AlreadyInitialised
        ));
        let to_stdout = LoggingConfig {
            output: LogOutput::Stdout,
            ..config.clone()
        };
        assert!(matches!(
            init(&to_stdout).unwrap_err(),
            LoggingError::AlreadyInitialised
        ));

        // The `log` macros are routed to tracing; with the filter `off` this is a no-op, but it
        // must not panic or recurse.
        log::info!("through the bridge");
        tracing::info!("through tracing");
    }
}
