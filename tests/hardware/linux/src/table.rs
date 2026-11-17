//! Printed table for the shared hardware-suite format.

use crate::report::SuiteResult;

pub fn print_table(result: &SuiteResult) {
    println!();
    println!("{:<28} {:<6} {:>6}  NOTE", "ID", "STATUS", "MS");
    println!("{}", "-".repeat(72));
    for row in &result.tests {
        let note = row
            .skip_reason
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(row.detail.as_str());
        let note = note.replace('\n', " ");
        println!(
            "{:<28} {:<6} {:>6}  {note}",
            row.id, row.status, row.duration_ms
        );
    }
    println!("{}", "-".repeat(72));
    println!(
        "pass={} fail={} skip={}  {}s  state={}  restored={}",
        result.summary.pass,
        result.summary.fail,
        result.summary.skip,
        result.duration_ms / 1000,
        result.state,
        result.restored
    );
}
