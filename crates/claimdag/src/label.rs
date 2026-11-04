//! A warning recorded when a call runs that used to be stopped for who the
//! caller is.
//!
//! The line is `allow`, `warn`, the rule, and the detail, separated by tabs.
//! It is what a seat reads. The call still runs. `warn.log` in the graph
//! directory keeps the same line; a directory that cannot take the file still
//! gets it on stderr.

use std::io::Write;
use std::path::Path;

/// The file beside `work.bin` that keeps these lines, oldest first.
pub const FILE: &str = "warn.log";

/// One warning a seat can read.
#[must_use]
pub fn line(rule: &str, detail: &str) -> String {
    format!("allow\twarn\t{rule}\t{detail}")
}

/// Append `text` to `warn.log`. A directory that cannot take the file is
/// skipped: the caller has already shown the line where a seat reads it.
pub fn append(dir: &Path, text: &str) {
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(FILE))
    else {
        return;
    };
    let _ = writeln!(file, "{text}");
    let _ = file.sync_all();
}

/// Show `text` on stderr and append it to `warn.log`.
pub fn emit(dir: &Path, text: &str) {
    eprintln!("{text}");
    append(dir, text);
}
