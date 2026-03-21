use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::prelude::*;
use pyo3::types::PyDict;
use tracing_core::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Flag to indicate whether it's safe to call into Python.
/// Set to `false` during interpreter shutdown to prevent segfaults.
static PYTHON_AVAILABLE: AtomicBool = AtomicBool::new(false);

struct PythonLoggingLayer;

impl PythonLoggingLayer {
    fn rust_level_to_python_level(level: &Level) -> u32 {
        match *level {
            Level::ERROR => 40,                // logging.ERROR
            Level::WARN => 30,                 // logging.WARNING
            Level::INFO => 20,                 // logging.INFO
            Level::DEBUG | Level::TRACE => 10, // logging.DEBUG (Python has no TRACE)
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for PythonLoggingLayer
where
    S: tracing_core::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing_core::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        let py_level = Self::rust_level_to_python_level(metadata.level());

        // Convert the Rust target format (e.g. "rattler_package_streaming::reqwest::sparse")
        // to Python format (e.g. "rattler.package_streaming.reqwest.sparse").
        let target = metadata.target().replace("::", ".");

        // Collect the message from the event fields
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let message = if visitor.message.is_empty() {
            "(no message)".to_string()
        } else {
            visitor.message
        };

        // Only call into Python if the interpreter is still available.
        // Background tokio threads may emit tracing events after Python
        // starts shutting down, which would segfault.
        if !PYTHON_AVAILABLE.load(Ordering::Relaxed) {
            return;
        }

        let fields = visitor.fields;
        Python::with_gil(|py| {
            let _ = (|| -> PyResult<()> {
                let logging = py.import("logging")?;
                let logger_name = format!("rattler.{target}");
                let logger = logging.call_method1("getLogger", (logger_name,))?;

                let kwargs = PyDict::new(py);
                if !fields.is_empty() {
                    let extra = PyDict::new(py);
                    for (k, v) in &fields {
                        extra.set_item(k, v)?;
                    }
                    kwargs.set_item("extra", extra)?;
                }
                logger.call_method("log", (py_level, &message), Some(&kwargs))?;
                Ok(())
            })();
        });
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl tracing_core::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing_core::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &tracing_core::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }
}

/// Initialize Rust tracing and bridge events to Python's `logging` module.
///
/// The `directives` argument is a tracing filter directive string, e.g. `"debug"`, `"info"`,
/// or a target-specific filter like `"rattler_solve=debug"`. If `None`, the `RUST_LOG`
/// environment variable is used, defaulting to `"off"` if unset.
///
/// This function can only be called once. Subsequent calls will raise a `RuntimeError`.
#[pyfunction]
#[pyo3(signature = (directives=None))]
pub fn setup_tracing(py: Python<'_>, directives: Option<&str>) -> PyResult<()> {
    let env_filter = match directives {
        Some(d) => EnvFilter::builder()
            .parse(d)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("{e}")))?,
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("off")),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(PythonLoggingLayer)
        .try_init()
        .map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Failed to initialize tracing (was setup_tracing already called?): {e}"
            ))
        })?;

    // Mark Python as available now that tracing is set up.
    PYTHON_AVAILABLE.store(true, Ordering::Relaxed);

    // Register an atexit handler to disable the Python bridge before
    // the interpreter finalizes. This prevents segfaults from background
    // tokio threads that emit tracing events during shutdown.
    let atexit = py.import("atexit")?;
    let shutdown_fn = wrap_pyfunction!(shutdown_tracing_bridge, py)?;
    atexit.call_method1("register", (shutdown_fn,))?;

    Ok(())
}

/// Called via `atexit` to disable the Python logging bridge before interpreter shutdown.
#[pyfunction]
fn shutdown_tracing_bridge() {
    PYTHON_AVAILABLE.store(false, Ordering::Relaxed);
}

/// Emit a test tracing event at the given level and target. For testing the Python bridge only.
///
/// `level` must be one of: "error", "warn", "info", "debug", "trace".
/// `target` must be one of: `rattler_test`, `rattler_test_a`, `rattler_test_b`.
#[pyfunction]
#[pyo3(signature = (level, message, target="rattler_test"))]
pub fn _emit_test_trace(level: &str, message: &str, target: &str) -> PyResult<()> {
    // tracing macros require compile-time target strings, so we match on known test targets.
    macro_rules! emit {
        ($target:expr) => {
            match level {
                "error" => tracing::error!(target: $target, "{}", message),
                "warn" => tracing::warn!(target: $target, "{}", message),
                "info" => tracing::info!(target: $target, "{}", message),
                "debug" => tracing::debug!(target: $target, "{}", message),
                "trace" => tracing::trace!(target: $target, "{}", message),
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid level: {level}. Must be one of: error, warn, info, debug, trace"
                    )));
                }
            }
        };
    }

    match target {
        "rattler_test" => emit!("rattler_test"),
        "rattler_test_a" => emit!("rattler_test_a"),
        "rattler_test_b" => emit!("rattler_test_b"),
        _ => {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Invalid target: {target}. Must be one of: rattler_test, rattler_test_a, rattler_test_b"
            )));
        }
    }

    Ok(())
}

/// Emit a test tracing event that includes both a message and structured fields.
///
/// `fields` is a dict of key-value pairs. Since tracing requires compile-time field names,
/// entries are sorted by key and mapped to fixed names (`a`, `b`, `c`, `d`, `e`).
/// Up to 5 fields are supported.
#[pyfunction]
#[pyo3(signature = (level, message, fields))]
pub fn _emit_test_trace_with_fields(
    level: &str,
    message: &str,
    fields: std::collections::HashMap<String, String>,
) -> PyResult<()> {
    let vals = _sorted_values(fields)?;
    _emit_with_fields(level, Some(message), &vals)
}

/// Emit a test tracing event that has only structured fields and no message.
///
/// Same field-name mapping as `_emit_test_trace_with_fields`.
#[pyfunction]
pub fn _emit_test_trace_fields_only(
    level: &str,
    fields: std::collections::HashMap<String, String>,
) -> PyResult<()> {
    let vals = _sorted_values(fields)?;
    _emit_with_fields(level, None, &vals)
}

fn _sorted_values(fields: std::collections::HashMap<String, String>) -> PyResult<Vec<String>> {
    let mut entries: Vec<_> = fields.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let vals: Vec<String> = entries.into_iter().map(|(_, v)| v).collect();
    if vals.is_empty() || vals.len() > 5 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "fields must have 1-5 entries",
        ));
    }
    Ok(vals)
}

fn _emit_with_fields(level: &str, message: Option<&str>, vals: &[String]) -> PyResult<()> {
    macro_rules! emit {
        ($level_fn:ident, msg=$msg:expr, $($field:ident = $val:expr),+) => {
            match $msg {
                Some(m) => tracing::$level_fn!(target: "rattler_test", $($field = %$val),+, "{}", m),
                None => tracing::$level_fn!(target: "rattler_test", $($field = %$val),+),
            }
        };
    }

    macro_rules! emit_for_level {
        ($($field:ident = $val:expr),+) => {
            match level {
                "error" => emit!(error, msg=message, $($field = $val),+),
                "warn" => emit!(warn, msg=message, $($field = $val),+),
                "info" => emit!(info, msg=message, $($field = $val),+),
                "debug" => emit!(debug, msg=message, $($field = $val),+),
                "trace" => emit!(trace, msg=message, $($field = $val),+),
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid level: {level}. Must be one of: error, warn, info, debug, trace"
                    )));
                }
            }
        };
    }

    match vals.len() {
        1 => emit_for_level!(a = vals[0]),
        2 => emit_for_level!(a = vals[0], b = vals[1]),
        3 => emit_for_level!(a = vals[0], b = vals[1], c = vals[2]),
        4 => emit_for_level!(a = vals[0], b = vals[1], c = vals[2], d = vals[3]),
        5 => emit_for_level!(
            a = vals[0],
            b = vals[1],
            c = vals[2],
            d = vals[3],
            e = vals[4]
        ),
        _ => unreachable!(),
    }

    Ok(())
}
