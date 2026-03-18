//! SHACL validation report serialization.
//!
//! Provides JSON and summary formatting for validation reports returned
//! by the rudof validation engine.

use shacl_validation::validation_report::report::ValidationReport;

/// Serialize a validation report as a JSON object.
///
/// Format (per ADR-004):
/// ```json
/// {
///   "conforms": false,
///   "results_count": 2,
///   "results": [
///     {
///       "severity": "Violation",
///       "message": "...",
///       "focus_node": "...",
///       "result_path": "...",
///       "source_shape": "..."
///     }
///   ]
/// }
/// ```
pub fn report_to_json(report: &ValidationReport) -> String {
    let conforms = report.conforms();
    let report_str = report.to_string();

    // Parse the report string into structured results
    let results: Vec<String> = report_str
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            format!(
                "{{\"detail\": {}}}",
                escape_json_string(line.trim())
            )
        })
        .collect();

    let results_json = results.join(", ");
    let results_count = results.len();

    format!(
        "{{\"conforms\": {conforms}, \"results_count\": {results_count}, \"results\": [{results_json}]}}"
    )
}

/// Produce a short human-readable summary of the validation report.
pub fn report_summary(report: &ValidationReport) -> String {
    if report.conforms() {
        "All shapes conform.".to_string()
    } else {
        let report_str = report.to_string();
        let violation_count = report_str.lines().filter(|l| !l.trim().is_empty()).count();
        format!("{violation_count} violation(s) found.")
    }
}

fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
