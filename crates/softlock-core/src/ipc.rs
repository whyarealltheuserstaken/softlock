//! Versioned, length-prefixed JSON messages. There is no unlock request.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::name::PRODUCT_DISPLAY_NAME;
use crate::platform::{GuardHealth, OverlayHealth};
use crate::state::{FailReason, LockSource, StateName};

pub const IPC_VERSION: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RequestKind {
    Hello {
        client_version: String,
    },
    GetStatus,
    Lock {
        source: LockSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    HardLock,
    GetConfig,
    ValidateConfig {
        toml: String,
    },
    PutConfig {
        toml: String,
        base_revision: u64,
    },
    ConfirmSensitiveChange {
        token: String,
    },
    /// Wire kind is `Set` + the product display name + `Pin`.
    SetPin {
        pin: String,
    },
    /// Wire kind is `Clear` + the product display name + `Pin`.
    ClearPin,
    Subscribe,
    CheckUpdate,
    InstallUpdate {
        version: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusFallback {
    pub reason: FailReason,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ResponseKind {
    Hello {
        service_version: String,
        state: StateName,
        capabilities: Vec<String>,
    },
    Status {
        state: StateName,
        lock_source: Option<LockSource>,
        guard: GuardHealth,
        overlay: OverlayHealth,
        last_fallback: Option<StatusFallback>,
    },
    Accepted,
    AlreadyLocked,
    Config {
        toml: String,
        revision: u64,
    },
    Valid,
    Invalid {
        errors: Vec<String>,
    },
    Applied,
    NeedsConfirmation {
        fields: Vec<String>,
    },
    Conflict,
    Locked,
    Ok,
    UpToDate,
    Available {
        version: String,
        notes: String,
    },
    Installing,
    Unsupported,
    Unauthorized,
    StateChanged {
        state: StateName,
    },
    ConfigChanged {
        revision: u64,
    },
    FellBackToOsLock {
        reason: FailReason,
        at_ms: u64,
    },
    UpdateAvailable {
        version: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedRequest {
    pub id: u64,
    pub kind: RequestKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcError {
    Unsupported { id: u64 },
    Malformed,
}

pub fn request_json(id: u64, kind: &RequestKind) -> String {
    envelope(id, kind)
}

pub fn response_json(id: u64, kind: &ResponseKind) -> String {
    envelope(id, kind)
}

pub fn encode_frame(json: &str) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4 + json.len());
    let length = u32::try_from(json.len()).expect("ipc message fits in a u32");
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(json.as_bytes());
    frame
}

pub fn decode_request_frame(bytes: &[u8]) -> Result<DecodedRequest, IpcError> {
    if bytes.len() < 4 {
        return Err(IpcError::Malformed);
    }
    let length = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes")) as usize;
    if bytes.len() != 4 + length {
        return Err(IpcError::Malformed);
    }
    let json = std::str::from_utf8(&bytes[4..]).map_err(|_| IpcError::Malformed)?;
    decode_request_json(json)
}

fn decode_request_json(json: &str) -> Result<DecodedRequest, IpcError> {
    let mut value: Value = serde_json::from_str(json).map_err(|_| IpcError::Malformed)?;
    let id = value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or(IpcError::Malformed)?;
    let version = value
        .get("v")
        .and_then(Value::as_u64)
        .ok_or(IpcError::Malformed)?;
    if version != IPC_VERSION {
        return Err(IpcError::Unsupported { id });
    }
    rewrite_pin_kind(&mut value, false);
    let kind = serde_json::from_value(value).map_err(|_| IpcError::Malformed)?;
    Ok(DecodedRequest { id, kind })
}

fn envelope<T: Serialize>(id: u64, body: &T) -> String {
    let mut value = serde_json::to_value(body).expect("message serializes");
    rewrite_pin_kind(&mut value, true);
    let mut object = Map::new();
    object.insert("v".to_string(), Value::from(IPC_VERSION));
    object.insert("id".to_string(), Value::from(id));
    if let Some(fields) = value.as_object() {
        for (key, field) in fields {
            object.insert(key.clone(), field.clone());
        }
    }
    serde_json::to_string(&Value::Object(object)).expect("message is json")
}

/// The spec's pin requests spell the product name. The source keeps that
/// spelling in the name constant and rewrites it onto the wire.
fn rewrite_pin_kind(value: &mut Value, to_wire: bool) {
    let Some(kind) = value
        .get("kind")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let set_wire = format!("Set{PRODUCT_DISPLAY_NAME}Pin");
    let clear_wire = format!("Clear{PRODUCT_DISPLAY_NAME}Pin");
    let mapped = if to_wire {
        match kind.as_str() {
            "SetPin" => set_wire,
            "ClearPin" => clear_wire,
            _ => return,
        }
    } else if kind == set_wire {
        "SetPin".to_string()
    } else if kind == clear_wire {
        "ClearPin".to_string()
    } else {
        return;
    };
    if let Some(object) = value.as_object_mut() {
        object.insert("kind".to_string(), Value::String(mapped));
    }
}
