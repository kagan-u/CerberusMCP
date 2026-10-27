//! In-memory log ring buffer + Tauri event bridge.
//!
//! A single `tracing` fmt layer writes into [`LogStore`]; every completed
//! line is kept (capped) for `get_logs` and forwarded to the frontend as a
//! `log-line` event as soon as the app handle exists.

use std::sync::{Arc, Mutex};

use tauri::Emitter;

const MAX_LINES: usize = 2000;

#[derive(Default)]
pub struct LogStore {
    app: Mutex<Option<tauri::AppHandle>>,
    lines: Mutex<Vec<String>>,
}

impl LogStore {
    /// Attach the app handle (setup time); events start flowing afterwards.
    pub fn set_app(&self, app: tauri::AppHandle) {
        *self.app.lock().expect("log app lock") = Some(app);
    }

    pub fn push(&self, line: String) {
        if line.is_empty() {
            return;
        }
        {
            let mut lines = self.lines.lock().expect("log lines lock");
            lines.push(line.clone());
            let overflow = lines.len().saturating_sub(MAX_LINES);
            if overflow > 0 {
                lines.drain(..overflow);
            }
        }
        let app = self.app.lock().expect("log app lock");
        if let Some(app) = app.as_ref() {
            let _ = app.emit("log-line", &line);
        }
    }

    /// The last `limit` buffered lines, oldest first.
    pub fn snapshot(&self, limit: usize) -> Vec<String> {
        let lines = self.lines.lock().expect("log lines lock");
        let skip = lines.len().saturating_sub(limit);
        lines.iter().skip(skip).cloned().collect()
    }

    pub fn clear(&self) {
        self.lines.lock().expect("log lines lock").clear();
    }
}

/// Writer handed to the `tracing` fmt layer; accumulates until each `\n`.
pub struct LogWriter {
    store: Arc<LogStore>,
    buf: String,
}

impl LogWriter {
    fn flush_lines(&mut self) {
        while let Some(pos) = self.buf.find('\n') {
            let mut line: String = self.buf.drain(..=pos).collect();
            while line.ends_with('\n') || line.ends_with('\r') {
                line.pop();
            }
            self.store.push(line);
        }
    }
}

impl std::io::Write for LogWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if let Ok(text) = std::str::from_utf8(data) {
            self.buf.push_str(text);
            self.flush_lines();
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_lines();
        Ok(())
    }
}

/// Install the process-wide `tracing` subscriber. `RUST_LOG` wins; the
/// default is `info` so gateway lifecycle events are visible in the GUI.
pub fn init(store: Arc<LogStore>) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || LogWriter {
            store: store.clone(),
            buf: String::new(),
        })
        .try_init();
}
