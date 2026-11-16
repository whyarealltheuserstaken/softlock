//! Versioned, length-prefixed JSON messages. There is no unlock request.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use zeroize::Zeroize;

use crate::platform::{GuardHealth, OverlayHealth};
use crate::state::{Fallback, LockSource, Secret, StateName};

pub const IPC_VERSION: u64 = 1;

/// Largest accepted frame body. A longer length prefix is rejected before the
/// body is copied or parsed. 64 KiB is enough for config documents and status.
pub const MAX_FRAME: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
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
        token: Secret,
    },
    /// Canonical wire kind `SetPin`. A product rename must not change it.
    SetPin {
        pin: Secret,
    },
    /// Canonical wire kind `ClearPin`. A product rename must not change it.
    ClearPin,
    Subscribe,
    CheckUpdate,
    InstallUpdate {
        version: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
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
        last_fallback: Option<Fallback>,
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
        #[serde(flatten)]
        fallback: Fallback,
    },
    UpdateAvailable {
        version: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct DecodedRequest {
    pub id: u64,
    pub kind: RequestKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcError {
    Unsupported { id: u64 },
    Malformed,
}

pub fn message_json(id: u64, body: &impl Serialize) -> Result<String, IpcError> {
    let value = serde_json::to_value(body).map_err(|_| IpcError::Malformed)?;
    // Match the serialized value itself. Replacing it with Null first drops the
    // original on this path and leaves only the placeholder to wipe.
    let fields = match value {
        Value::Object(fields) => fields,
        mut original => {
            zeroize_json(&mut original);
            return Err(IpcError::Malformed);
        }
    };
    let mut object = Map::new();
    object.insert("v".to_string(), Value::from(IPC_VERSION));
    object.insert("id".to_string(), Value::from(id));
    for (key, field) in fields {
        object.insert(key, field);
    }
    let json = serde_json::to_string(&object).map_err(|_| IpcError::Malformed)?;
    zeroize_map(&mut object);
    if json.len() > MAX_FRAME {
        let mut json = json;
        json.zeroize();
        return Err(IpcError::Malformed);
    }
    Ok(json)
}

/// The pipe reader calls this on the 4-byte length prefix before it allocates
/// the body. A length above [`MAX_FRAME`] is rejected here, with no buffer yet.
pub fn checked_body_len(prefix: &[u8]) -> Result<usize, IpcError> {
    if prefix.len() < 4 {
        return Err(IpcError::Malformed);
    }
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&prefix[..4]);
    let length = u32::from_le_bytes(bytes) as usize;
    if length > MAX_FRAME {
        return Err(IpcError::Malformed);
    }
    Ok(length)
}

pub fn encode_frame(json: &str) -> Result<Vec<u8>, IpcError> {
    if json.len() > MAX_FRAME {
        return Err(IpcError::Malformed);
    }
    let length = u32::try_from(json.len()).map_err(|_| IpcError::Malformed)?;
    let mut frame = Vec::with_capacity(4 + json.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(json.as_bytes());
    Ok(frame)
}

pub fn decode_request_frame(bytes: &[u8]) -> Result<DecodedRequest, IpcError> {
    if bytes.len() < 4 {
        return Err(IpcError::Malformed);
    }
    let length = checked_body_len(bytes)?;
    if bytes.len() != 4 + length {
        return Err(IpcError::Malformed);
    }
    let json = std::str::from_utf8(&bytes[4..]).map_err(|_| IpcError::Malformed)?;
    decode_request_json(json)
}

fn decode_request_json(json: &str) -> Result<DecodedRequest, IpcError> {
    let mut value: Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(_) => return Err(IpcError::Malformed),
    };
    let Value::Object(object) = &mut value else {
        zeroize_json(&mut value);
        return Err(IpcError::Malformed);
    };
    let Some(id) = object.get("id").and_then(Value::as_u64) else {
        zeroize_json(&mut value);
        return Err(IpcError::Malformed);
    };
    let Some(version) = object.get("v").and_then(Value::as_u64) else {
        zeroize_json(&mut value);
        return Err(IpcError::Malformed);
    };
    if version != IPC_VERSION {
        zeroize_json(&mut value);
        return Err(IpcError::Unsupported { id });
    }
    let keys: Vec<String> = object
        .keys()
        .filter(|key| *key != "v" && *key != "id")
        .cloned()
        .collect();
    // Already matched as an object above, so the move cannot fail.
    let Value::Object(mut object) = value else {
        unreachable!("value was matched as an object");
    };
    object.remove("v");
    object.remove("id");
    // `from_value` consumes the object, so a failure drops a PIN still sitting
    // in it. Deserialize from a reference and wipe that object on both paths.
    let mut payload = Value::Object(object);
    let parsed = RequestKind::deserialize(&payload);
    zeroize_json(&mut payload);
    let kind = match parsed {
        Ok(kind) => kind,
        Err(_) => return Err(IpcError::Malformed),
    };
    if !same_fields(&keys, &kind)? {
        return Err(IpcError::Malformed);
    }
    Ok(DecodedRequest { id, kind })
}

/// Unit variants ignore unknown fields even with `deny_unknown_fields`.
/// Compare the decoded message with the fields it actually owns. The
/// re-serialized value is wiped so a PIN in it does not outlive the check.
fn same_fields(keys: &[String], kind: &RequestKind) -> Result<bool, IpcError> {
    let mut canonical = serde_json::to_value(kind).map_err(|_| IpcError::Malformed)?;
    let matches = match canonical.as_object() {
        Some(object) => {
            keys.len() == object.len() && keys.iter().all(|key| object.contains_key(key))
        }
        None => {
            zeroize_json(&mut canonical);
            return Err(IpcError::Malformed);
        }
    };
    zeroize_json(&mut canonical);
    Ok(matches)
}

fn zeroize_json(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => {
            for item in items {
                zeroize_json(item);
            }
        }
        Value::Object(map) => zeroize_map(map),
        _ => {}
    }
}

fn zeroize_map(map: &mut Map<String, Value>) {
    for value in map.values_mut() {
        zeroize_json(value);
    }
}
