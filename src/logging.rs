use anyhow::Context;
use flexi_logger::{AdaptiveFormat, DeferredNow, LogTarget, Logger};
use std::io;

pub(crate) fn init() -> Result<(), anyhow::Error> {
    Logger::with_env_or_str("info")
        .check_parser_error()
        .context("RUST_LOG string formatted incorrectly")?
        .log_target(LogTarget::StdErr)
        .adaptive_format_for_stderr(AdaptiveFormat::Custom(log_format, log_format_with_color))
        .use_buffering(false)
        .start()?;
    Ok(())
}

fn log_format(
    w: &mut dyn io::Write,
    now: &mut DeferredNow,
    record: &log::Record<'_>,
) -> Result<(), io::Error> {
    let level = record.level();
    write!(
        w,
        "[{}] {} {}",
        now.now().format("%Y-%m-%d %H:%M:%S%.6f %:z"),
        level,
        record.args(),
    )?;
    format_kv_pairs(w, record);
    Ok(())
}

fn log_format_with_color(
    w: &mut dyn io::Write,
    now: &mut DeferredNow,
    record: &log::Record<'_>,
) -> Result<(), io::Error> {
    let level = record.level();
    write!(
        w,
        "[{}] {} {}",
        now.now().format("%Y-%m-%d %H:%M:%S%.6f %:z"),
        flexi_logger::style(level, level),
        record.args(),
    )?;
    format_kv_pairs(w, record);
    Ok(())
}

// Stolen from https://github.com/lrlna/femme/blob/94e5aa88cf13bf3dac5a56f51e6114aeec93928e/src/pretty.rs#L41:
fn format_kv_pairs<'b>(mut out: &mut dyn io::Write, record: &log::Record) {
    struct Visitor<'a> {
        stdout: &'a mut dyn io::Write,
    }

    impl<'kvs, 'a, 'b> log::kv::Visitor<'kvs> for Visitor<'a> {
        fn visit_pair(
            &mut self,
            key: log::kv::Key<'kvs>,
            val: log::kv::Value<'kvs>,
        ) -> Result<(), log::kv::Error> {
            write!(self.stdout, " {}={}", key, val)?;
            Ok(())
        }
    }

    let mut visitor = Visitor { stdout: &mut out };
    record.key_values().visit(&mut visitor).unwrap();
}
