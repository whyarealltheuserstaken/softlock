//! Structured local logs. Secrets, typed characters, and window titles are
//! not fields this logger will write.

use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Id, Metadata, Subscriber};

const BLOCKED: &[&str] = &[
    "secret",
    "pin",
    "password",
    "nonce",
    "note",
    "title",
    "window",
    "hash",
    "character",
    "keystroke",
];

pub fn allowed_field(name: &str) -> bool {
    !BLOCKED
        .iter()
        .any(|blocked| name.eq_ignore_ascii_case(blocked))
}

/// Writes one structured line. Field names that could carry a secret, a
/// typed character, or a window title are dropped before the line is built.
pub fn audit(event: &'static str, fields: &[(&str, &str)]) {
    let mut line = String::from(event);
    for (name, value) in fields {
        if !allowed_field(name) || value.contains("$argon2") {
            continue;
        }
        line.push(' ');
        line.push_str(name);
        line.push('=');
        line.push_str(value);
    }
    tracing::info!(target: "service", "{line}");
}

pub fn source_name(source: softlock_core::LockSource) -> &'static str {
    match source {
        softlock_core::LockSource::Hotkey => "hotkey",
        softlock_core::LockSource::Tray => "tray",
        softlock_core::LockSource::App => "app",
        softlock_core::LockSource::LocalApi => "local-api",
    }
}

/// Test capture. Install with `tracing::subscriber::with_default`.
#[derive(Clone, Default)]
pub struct Capture {
    text: Arc<Mutex<String>>,
}

impl Capture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> String {
        self.text
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl Subscriber for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "service"
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut visit = LineVisit::default();
        event.record(&mut visit);
        let mut text = self
            .text
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        text.push_str(&visit.line);
        text.push('\n');
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Default)]
struct LineVisit {
    line: String,
}

impl Visit for LineVisit {
    fn record_str(&mut self, field: &Field, value: &str) {
        if allowed_field(field.name()) && !value.contains("$argon2") {
            if !self.line.is_empty() {
                self.line.push(' ');
            }
            self.line.push_str(value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if !allowed_field(field.name()) {
            return;
        }
        let rendered = format!("{value:?}");
        if rendered.contains("$argon2") {
            return;
        }
        if !self.line.is_empty() {
            self.line.push(' ');
        }
        self.line.push_str(&rendered);
    }
}

/// Process log. One subscriber for the service process.
pub struct FileLog {
    file: Mutex<std::fs::File>,
}

impl FileLog {
    pub fn create(path: &std::path::Path) -> std::io::Result<Self> {
        // The parent is the LocalSystem profile directory, verified before
        // this call. This function does not create that directory and does
        // not delete a file that fails the check.
        #[cfg(windows)]
        let data_dir = crate::service_data_directory();
        #[cfg(windows)]
        if path.parent() == Some(data_dir.as_path()) && crate::secrets::running_as_system() {
            let file = crate::secrets::open_secured_log(path)?;
            return Ok(Self {
                file: Mutex::new(file),
            });
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl Subscriber for FileLog {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "service"
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut visit = LineVisit::default();
        event.record(&mut visit);
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        use std::io::Write;
        let _ = writeln!(file, "{}", visit.line);
        let _ = file.flush();
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}
