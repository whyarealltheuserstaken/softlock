//! Parser for `powercfg /requests` text. No operating-system calls.

use crate::model::{PowerRequest, PowerRequestKind};

pub fn parse_powercfg_requests(text: &str) -> Vec<PowerRequest> {
    let mut requests = Vec::new();
    let mut kind: Option<PowerRequestKind> = None;
    let mut requester: Option<String> = None;
    let mut reason: Vec<String> = Vec::new();

    let flush = |kind: &mut Option<PowerRequestKind>,
                 requester: &mut Option<String>,
                 reason: &mut Vec<String>,
                 requests: &mut Vec<PowerRequest>| {
        if let (Some(kind), Some(requester)) = (kind.as_ref(), requester.take()) {
            requests.push(PowerRequest {
                kind: *kind,
                requester,
                reason: reason.join("\n"),
            });
        }
        reason.clear();
    };

    for raw in text.lines() {
        let line = raw.trim().trim_end_matches('\u{feff}');
        if line.is_empty() {
            flush(&mut kind, &mut requester, &mut reason, &mut requests);
            continue;
        }
        if let Some(parsed) = PowerRequestKind::parse(line) {
            flush(&mut kind, &mut requester, &mut reason, &mut requests);
            kind = Some(parsed);
            continue;
        }
        if line.eq_ignore_ascii_case("None.") || line.eq_ignore_ascii_case("None") {
            flush(&mut kind, &mut requester, &mut reason, &mut requests);
            continue;
        }
        if line.starts_with('[') {
            flush(&mut kind, &mut requester, &mut reason, &mut requests);
            requester = Some(line.to_string());
            continue;
        }
        if requester.is_some() {
            reason.push(line.to_string());
        }
    }
    flush(&mut kind, &mut requester, &mut reason, &mut requests);
    requests
}
