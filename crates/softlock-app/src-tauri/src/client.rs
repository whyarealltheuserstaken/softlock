//! One request on the public pipe, and the Subscribe stream that follows.
//!
//! Frames are the core length-prefixed JSON. This module does not invent a
//! second protocol and it does not send an unlock request.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use serde_json::Value;
use softlock_core::{
    IpcError, LockSource, RequestKind, ResponseKind, StateName, encode_frame, message_json,
};
use zeroize::Zeroize;

use crate::actions::shows_unlocked;

#[cfg_attr(not(windows), allow(dead_code))]
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerView {
    pub service_running: bool,
    pub state: Option<StateName>,
    pub lock_source: Option<LockSource>,
}

impl OwnerView {
    pub fn unknown() -> Self {
        Self {
            service_running: false,
            state: None,
            lock_source: None,
        }
    }

    pub fn locked(&self) -> bool {
        !shows_unlocked(self.service_running, self.state)
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn exchange(
    io: &mut (impl Read + Write),
    id: u64,
    kind: &RequestKind,
) -> Result<ResponseKind, IpcError> {
    let json = message_json(id, kind)?;
    let frame = encode_frame(&json)?;
    let mut json = json;
    json.zeroize();
    io.write_all(&frame).map_err(|_| IpcError::Malformed)?;
    io.flush().map_err(|_| IpcError::Malformed)?;
    let mut frame = frame;
    frame.zeroize();
    let response = read_one(io)?;
    decode_response(&response)
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn read_one(io: &mut impl Read) -> Result<Vec<u8>, IpcError> {
    softlock_core::read_prefixed_frame(io)
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn decode_response(frame: &[u8]) -> Result<ResponseKind, IpcError> {
    if frame.len() < 4 {
        return Err(IpcError::Malformed);
    }
    let json = std::str::from_utf8(&frame[4..]).map_err(|_| IpcError::Malformed)?;
    let mut value: Value = serde_json::from_str(json).map_err(|_| IpcError::Malformed)?;
    let Value::Object(object) = &mut value else {
        return Err(IpcError::Malformed);
    };
    object.remove("v");
    object.remove("id");
    ResponseKind::deserialize(&value).map_err(|_| IpcError::Malformed)
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn view_from_status(response: &ResponseKind) -> OwnerView {
    match response {
        ResponseKind::Status {
            state, lock_source, ..
        } => OwnerView {
            service_running: true,
            state: Some(*state),
            lock_source: *lock_source,
        },
        ResponseKind::StateChanged { state } => OwnerView {
            service_running: true,
            state: Some(*state),
            lock_source: None,
        },
        _ => OwnerView::unknown(),
    }
}

/// Applies a stream event. An error or an unexpected body drops Unlocked.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn view_after(previous: &OwnerView, event: &Result<ResponseKind, IpcError>) -> OwnerView {
    match event {
        Ok(response @ ResponseKind::StateChanged { .. })
        | Ok(response @ ResponseKind::Status { .. }) => view_from_status(response),
        Ok(ResponseKind::Accepted) => previous.clone(),
        _ => OwnerView::unknown(),
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn public_pipe_name() -> String {
    #[cfg(feature = "simulated-physical")]
    if let Ok(name) = std::env::var("SOFTLOCK_PIPE") {
        if !name.is_empty() {
            return name;
        }
    }
    softlock_core::pipe_name()
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn status_text(view: &OwnerView) -> String {
    format!(
        "service={}\ntray={}\nstate={}\n",
        if view.service_running {
            "running"
        } else {
            "down"
        },
        if view.locked() { "on" } else { "off" },
        view.state
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|| "Locked".to_string())
    )
}

pub fn lock_source_name(source: LockSource) -> &'static str {
    match source {
        LockSource::Hotkey => "Hotkey",
        LockSource::Tray => "Tray",
        LockSource::App => "App",
        LockSource::LocalApi => "LocalApi",
    }
}

/// The JSON the app writes for a lock. Tests read the source field.
#[cfg(test)]
pub fn lock_json(source: LockSource) -> Result<String, IpcError> {
    message_json(1, &crate::actions::lock_request(source))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read, Write};

    use super::*;

    struct Broken;

    impl Read for Broken {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("down"))
        }
    }

    impl Write for Broken {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("down"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn tray_and_app_lock_frames_name_their_sources() {
        for source in [LockSource::Tray, LockSource::App, LockSource::Hotkey] {
            let json = lock_json(source).expect("frame");
            let name = lock_source_name(source);
            assert!(json.contains(&format!("\"source\":\"{name}\"")), "{json}");
            assert!(json.contains("\"kind\":\"Lock\""));
            assert!(json.contains("\"v\":1"));
            let frame = encode_frame(&json).unwrap();
            let decoded = softlock_core::decode_request_frame(&frame).unwrap();
            assert!(matches!(
                decoded.kind,
                RequestKind::Lock { source: found, note: None } if found == source
            ));
        }
    }

    #[test]
    fn a_broken_pipe_does_not_report_unlocked() {
        let mut broken = Broken;
        let error = exchange(&mut broken, 1, &RequestKind::GetStatus);
        assert!(error.is_err());
        let failed = view_after(
            &OwnerView {
                service_running: true,
                state: Some(StateName::Unlocked),
                lock_source: None,
            },
            &Err(IpcError::Malformed),
        );
        assert!(!failed.service_running);
        assert!(failed.locked());
        assert!(!shows_unlocked(failed.service_running, failed.state));
        let text = status_text(&failed);
        assert!(text.contains("service=down"));
        assert!(text.contains("tray=on"));
        assert!(!text.contains("tray=off"));
    }

    #[test]
    fn status_and_state_changed_round_trip() {
        let status = ResponseKind::Status {
            state: StateName::Locked,
            lock_source: Some(LockSource::Tray),
            guard: softlock_core::GuardHealth {
                hook_alive: true,
                last_physical_input_ms: None,
            },
            overlay: softlock_core::OverlayHealth {
                covered_monitors: 1,
                capture_excluded: true,
                topmost: true,
            },
            last_fallback: None,
        };
        let json = message_json(7, &status).unwrap();
        let frame = encode_frame(&json).unwrap();
        let decoded = decode_response(&frame).unwrap();
        let view = view_from_status(&decoded);
        assert!(view.service_running);
        assert_eq!(view.lock_source, Some(LockSource::Tray));
        assert!(view.locked());

        let changed = ResponseKind::StateChanged {
            state: StateName::Unlocked,
        };
        let json = message_json(7, &changed).unwrap();
        let frame = encode_frame(&json).unwrap();
        let mut cursor = Cursor::new(frame);
        let read = read_one(&mut cursor).unwrap();
        let decoded = decode_response(&read).unwrap();
        let view = view_after(&view, &Ok(decoded));
        assert!(!view.locked());
        assert!(status_text(&view).contains("tray=off"));
    }
}
