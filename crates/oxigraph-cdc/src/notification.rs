use crate::channel::ChangeEvent;
use oxrdf::Quad;
use serde_json::{Value as JsonValue, json};
use std::fmt::Write as _;

/// Format a single `ChangeEvent` as an AS2 JSON-LD notification.
pub fn format_notification(event: &ChangeEvent, server_url: &str) -> JsonValue {
    json!({
        "@context": [
            "https://www.w3.org/ns/activitystreams",
            "https://www.w3.org/ns/solid/notifications/v1"
        ],
        "id": format!("urn:oxigraph:changelog:{}", event.id),
        "type": activity_type(event),
        "actor": {
            "type": "Application",
            "name": "oxigraph-cloud"
        },
        "published": &event.timestamp,
        "object": server_url,
        "state": event.id.to_string(),
        "content": {
            "oxcdc:operation": &event.operation,
            "oxcdc:inserted": quads_to_nquads_string(&event.inserted),
            "oxcdc:removed": quads_to_nquads_string(&event.removed),
            "oxcdc:insertCount": event.inserted.len(),
            "oxcdc:removeCount": event.removed.len()
        }
    })
}

/// Format a batch of `ChangeEvent`s as a single AS2 notification.
///
/// # Panics
///
/// Panics if `events` is empty.
#[expect(clippy::expect_used, reason = "caller guarantees non-empty slice")]
pub fn format_batch_notification(events: &[ChangeEvent], server_url: &str) -> JsonValue {
    if events.len() == 1 {
        return format_notification(&events[0], server_url);
    }

    // Caller guarantees non-empty slice
    let last = events.last().expect("batch must not be empty");
    let batch: Vec<JsonValue> = events
        .iter()
        .map(|e| {
            json!({
                "oxcdc:id": e.id,
                "oxcdc:operation": &e.operation,
                "oxcdc:inserted": quads_to_nquads_string(&e.inserted),
                "oxcdc:removed": quads_to_nquads_string(&e.removed),
                "oxcdc:insertCount": e.inserted.len(),
                "oxcdc:removeCount": e.removed.len()
            })
        })
        .collect();

    json!({
        "@context": [
            "https://www.w3.org/ns/activitystreams",
            "https://www.w3.org/ns/solid/notifications/v1"
        ],
        "id": format!("urn:oxigraph:changelog:{}", last.id),
        "type": "Update",
        "actor": {
            "type": "Application",
            "name": "oxigraph-cloud"
        },
        "published": &last.timestamp,
        "object": server_url,
        "state": last.id.to_string(),
        "content": {
            "oxcdc:batch": batch
        }
    })
}

/// Determine AS2 activity type from event.
fn activity_type(event: &ChangeEvent) -> &'static str {
    if event.removed.is_empty() && !event.inserted.is_empty() {
        "Create"
    } else if event.inserted.is_empty() && !event.removed.is_empty() {
        "Delete"
    } else {
        "Update"
    }
}

/// Serialize quads to N-Quads string for the delta payload.
fn quads_to_nquads_string(quads: &[Quad]) -> String {
    let mut out = String::new();
    for quad in quads {
        // oxrdf::Quad implements Display, producing N-Quads format
        // Writing to String is infallible; ignore the Result
        _ = writeln!(&mut out, "{quad}");
    }
    out
}
