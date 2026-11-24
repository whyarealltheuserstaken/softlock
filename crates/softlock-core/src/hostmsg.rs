//! Private Lock Host channel. These kinds are not public requests.
//!
//! A frame that uses one of them on the public pipe fails public decode.
//! The nonce is compared with [`crate::state::LaunchNonce::eq_ct`] by the
//! Service. This module does not log it.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::ipc::{IPC_VERSION, IpcError, MAX_FRAME, encode_frame, message_json};
use crate::platform::{GuardHealth, OverlayHealth};
use crate::state::{LaunchNonce, Secret};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ServiceToHost {
    Welcome {
        nonce: NonceBytes,
    },
    Arm {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Which secret fields the prompt may offer. Password is the account
    /// password. PIN is this product's PIN, and only when a hash is stored.
    Methods {
        pin: bool,
        password: bool,
    },
    Disarm,
    /// Lock the signed-in session. The Service confirms it and disconnects
    /// the console itself when this does not. The host must not disconnect.
    OsLock,
    PromptResult {
        accepted: bool,
        backoff_ms: u64,
        /// This result is for the account password. The host must not apply
        /// it to the PIN backoff, and it must not swallow PIN keystrokes.
        #[serde(default)]
        password_path: bool,
        /// The password budget for this observation window is used up.
        /// The prompt says so and keeps the PIN available.
        #[serde(default)]
        password_exhausted: bool,
    },
}

#[derive(Debug)]
pub enum HostToService {
    HostHello {
        nonce: LaunchNonce,
    },
    HostReady {
        monitors: u32,
        nonce: LaunchNonce,
    },
    Heartbeat {
        seq: u64,
        guard: GuardHealth,
        overlay: OverlayHealth,
        nonce: LaunchNonce,
    },
    SubmitSecret {
        method: String,
        secret: Secret,
        nonce: LaunchNonce,
    },
    GuardDown {
        nonce: LaunchNonce,
    },
    /// Hard lock while the session is already Locked. The public pipe cannot
    /// do this. The App's public HardLock is only accepted while Unlocked.
    HardLock {
        nonce: LaunchNonce,
    },
}

impl HostToService {
    pub fn nonce(&self) -> &LaunchNonce {
        match self {
            Self::HostHello { nonce }
            | Self::HostReady { nonce, .. }
            | Self::Heartbeat { nonce, .. }
            | Self::SubmitSecret { nonce, .. }
            | Self::GuardDown { nonce }
            | Self::HardLock { nonce } => nonce,
        }
    }
}

/// Wire form of the 32-byte nonce. Debug output is redacted.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NonceBytes(String);

impl NonceBytes {
    pub fn from_nonce(nonce: &LaunchNonce) -> Self {
        let text: String = nonce.iter_hex();
        Self(text)
    }

    pub fn to_nonce(&self) -> Result<LaunchNonce, IpcError> {
        LaunchNonce::from_hex(&self.0).ok_or(IpcError::Malformed)
    }
}

impl Drop for NonceBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for NonceBytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NonceBytes([redacted])")
    }
}

impl LaunchNonce {
    pub fn iter_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for byte in self.as_bytes() {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }

    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        let raw = text.as_bytes();
        for (index, chunk) in raw.chunks(2).enumerate() {
            let hi = hex_value(chunk[0])?;
            let lo = hex_value(chunk[1])?;
            bytes[index] = (hi << 4) | lo;
        }
        Some(Self::from_bytes(bytes))
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum HostWire {
    HostHello {
        nonce: NonceBytes,
    },
    HostReady {
        monitors: u32,
        nonce: NonceBytes,
    },
    Heartbeat {
        seq: u64,
        guard: GuardHealth,
        overlay: OverlayHealth,
        nonce: NonceBytes,
    },
    SubmitSecret {
        method: String,
        secret: Secret,
        nonce: NonceBytes,
    },
    GuardDown {
        nonce: NonceBytes,
    },
    HardLock {
        nonce: NonceBytes,
    },
}

pub fn encode_service(id: u64, body: &ServiceToHost) -> Result<Vec<u8>, IpcError> {
    let mut json = message_json(id, body)?;
    let frame = encode_frame(&json)?;
    json.zeroize();
    Ok(frame)
}

fn encode_host(id: u64, body: &HostWire) -> Result<Vec<u8>, IpcError> {
    let mut json = message_json(id, body)?;
    let frame = encode_frame(&json)?;
    json.zeroize();
    Ok(frame)
}

pub fn decode_host_frame(bytes: &[u8]) -> Result<HostToService, IpcError> {
    if bytes.len() < 4 {
        return Err(IpcError::Malformed);
    }
    let length = crate::ipc::checked_body_len(bytes)?;
    if bytes.len() != 4 + length || length > MAX_FRAME {
        return Err(IpcError::Malformed);
    }
    let mut json = std::str::from_utf8(&bytes[4..])
        .map_err(|_| IpcError::Malformed)?
        .to_string();
    let parsed = decode_host_json(&json);
    json.zeroize();
    parsed
}

fn decode_host_json(json: &str) -> Result<HostToService, IpcError> {
    let mut value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| IpcError::Malformed)?;
    let Some(version) = value.get("v").and_then(|item| item.as_u64()) else {
        return Err(IpcError::Malformed);
    };
    if version != IPC_VERSION {
        return Err(IpcError::Malformed);
    }
    if let Some(object) = value.as_object_mut() {
        object.remove("v");
        object.remove("id");
    }
    let wire: HostWire = HostWire::deserialize(&value).map_err(|_| IpcError::Malformed)?;
    match wire {
        HostWire::HostHello { nonce } => Ok(HostToService::HostHello {
            nonce: nonce.to_nonce()?,
        }),
        HostWire::HostReady { monitors, nonce } => Ok(HostToService::HostReady {
            monitors,
            nonce: nonce.to_nonce()?,
        }),
        HostWire::Heartbeat {
            seq,
            guard,
            overlay,
            nonce,
        } => Ok(HostToService::Heartbeat {
            seq,
            guard,
            overlay,
            nonce: nonce.to_nonce()?,
        }),
        HostWire::SubmitSecret {
            method,
            secret,
            nonce,
        } => Ok(HostToService::SubmitSecret {
            method,
            secret,
            nonce: nonce.to_nonce()?,
        }),
        HostWire::GuardDown { nonce } => Ok(HostToService::GuardDown {
            nonce: nonce.to_nonce()?,
        }),
        HostWire::HardLock { nonce } => Ok(HostToService::HardLock {
            nonce: nonce.to_nonce()?,
        }),
    }
}

/// Builds a host report frame. The nonce hex lives only inside the frame.
pub fn host_hello_frame(id: u64, nonce: &LaunchNonce) -> Result<Vec<u8>, IpcError> {
    encode_host(
        id,
        &HostWire::HostHello {
            nonce: NonceBytes::from_nonce(nonce),
        },
    )
}

pub fn host_ready_frame(id: u64, monitors: u32, nonce: &LaunchNonce) -> Result<Vec<u8>, IpcError> {
    encode_host(
        id,
        &HostWire::HostReady {
            monitors,
            nonce: NonceBytes::from_nonce(nonce),
        },
    )
}

pub fn host_heartbeat_frame(
    id: u64,
    seq: u64,
    guard: GuardHealth,
    overlay: OverlayHealth,
    nonce: &LaunchNonce,
) -> Result<Vec<u8>, IpcError> {
    encode_host(
        id,
        &HostWire::Heartbeat {
            seq,
            guard,
            overlay,
            nonce: NonceBytes::from_nonce(nonce),
        },
    )
}

pub fn host_secret_frame(
    id: u64,
    method: &str,
    secret: &Secret,
    nonce: &LaunchNonce,
) -> Result<Vec<u8>, IpcError> {
    encode_host(
        id,
        &HostWire::SubmitSecret {
            method: method.to_string(),
            secret: Secret::new(secret.expose()),
            nonce: NonceBytes::from_nonce(nonce),
        },
    )
}

pub fn host_hard_lock_frame(id: u64, nonce: &LaunchNonce) -> Result<Vec<u8>, IpcError> {
    encode_host(
        id,
        &HostWire::HardLock {
            nonce: NonceBytes::from_nonce(nonce),
        },
    )
}

pub fn host_guard_down_frame(id: u64, nonce: &LaunchNonce) -> Result<Vec<u8>, IpcError> {
    encode_host(
        id,
        &HostWire::GuardDown {
            nonce: NonceBytes::from_nonce(nonce),
        },
    )
}

pub fn decode_service_frame(bytes: &[u8]) -> Result<ServiceToHost, IpcError> {
    if bytes.len() < 4 {
        return Err(IpcError::Malformed);
    }
    let length = crate::ipc::checked_body_len(bytes)?;
    if bytes.len() != 4 + length {
        return Err(IpcError::Malformed);
    }
    let mut json = std::str::from_utf8(&bytes[4..])
        .map_err(|_| IpcError::Malformed)?
        .to_string();
    let mut value: serde_json::Value =
        serde_json::from_str(&json).map_err(|_| IpcError::Malformed)?;
    json.zeroize();
    let Some(version) = value.get("v").and_then(|item| item.as_u64()) else {
        return Err(IpcError::Malformed);
    };
    if version != IPC_VERSION {
        return Err(IpcError::Malformed);
    }
    if let Some(object) = value.as_object_mut() {
        object.remove("v");
        object.remove("id");
    }
    ServiceToHost::deserialize(&value).map_err(|_| IpcError::Malformed)
}
