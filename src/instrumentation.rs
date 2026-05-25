use std::io;
use std::{fmt::Display, str::FromStr};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[derive(serde::Deserialize, Default, PartialEq, Eq, Debug, Clone)]
pub enum Instrumentation {
    Json,

    #[default]
    Tui,
}

impl FromStr for Instrumentation {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "json" => Ok(Instrumentation::Json),
            "tui" => Ok(Instrumentation::Tui),
            _ => Err(s.to_string()),
        }
    }
}

impl Display for Instrumentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Instrumentation::Json => "json",
                Instrumentation::Tui => "tui",
            }
        )
    }
}

impl Instrumentation {
    pub fn setup(&self) {
        let filter = EnvFilter::builder()
            .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
            .from_env_lossy();

        match self {
            Instrumentation::Tui => {
                let indicatif_layer = tracing_indicatif::IndicatifLayer::new();
                let writer = indicatif_layer.get_stderr_writer();
                let app_log_layer = tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .compact()
                    .with_writer(writer.clone())
                    .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                        metadata.target() != crate::SUBPROCESS_LOG_TARGET
                    }));
                let subprocess_log_layer = tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .with_level(false)
                    .compact()
                    .with_writer(writer.clone())
                    .fmt_fields(tracing_subscriber::fmt::format::debug_fn(
                        |writer, field, value| {
                            if field.name() == "stream" || field.name() == "stream_prefix" {
                                // Skip the "stream" field, as that's only useful in the JSON formatter
                                return Ok(());
                            }
                            write!(writer, "{}={:?}", field, value)
                        },
                    ))
                    .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                        metadata.target() == crate::SUBPROCESS_LOG_TARGET
                    }));
                tracing_subscriber::registry()
                    .with(filter)
                    .with(app_log_layer)
                    .with(subprocess_log_layer)
                    .with(indicatif_layer)
                    .init();
            }
            Instrumentation::Json => {
                let json = tracing_subscriber::fmt::format::json().flatten_event(true);
                let layer = tracing_subscriber::fmt::layer()
                    .with_writer(io::stderr)
                    .event_format(json)
                    .json();
                tracing_subscriber::registry()
                    .with(filter)
                    .with(layer)
                    .init();
            }
        }
    }
}
