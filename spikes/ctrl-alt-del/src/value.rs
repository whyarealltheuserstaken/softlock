use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `REG_DWORD`.
pub const REG_DWORD: u32 = 4;

/// `REG_SZ`.
pub const REG_SZ: u32 = 1;

/// A registry value exactly as it was, or the fact that it was not there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedValue {
    Absent,
    Present { reg_type: u32, data: Vec<u8> },
}

impl RecordedValue {
    pub fn dword(value: u32) -> Self {
        Self::Present {
            reg_type: REG_DWORD,
            data: value.to_le_bytes().to_vec(),
        }
    }

    pub fn reg_sz(text: &str) -> Self {
        let mut data = Vec::new();
        for unit in text.encode_utf16().chain(std::iter::once(0)) {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        Self::Present {
            reg_type: REG_SZ,
            data,
        }
    }
}

impl std::fmt::Display for RecordedValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent => formatter.write_str("absent"),
            Self::Present { reg_type, data } if *reg_type == REG_DWORD && data.len() == 4 => {
                let bytes = [data[0], data[1], data[2], data[3]];
                write!(formatter, "dword {}", u32::from_le_bytes(bytes))
            }
            Self::Present { reg_type, data } => {
                write!(formatter, "type {reg_type} {}", hex_encode(data))
            }
        }
    }
}

impl Serialize for RecordedValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Absent => serializer.serialize_str("absent"),
            Self::Present { reg_type, data } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("reg_type", reg_type)?;
                map.serialize_entry("data_hex", &hex_encode(data))?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RecordedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Absent(String),
            Present { reg_type: u32, data_hex: String },
        }

        match Raw::deserialize(deserializer)? {
            Raw::Absent(text) if text == "absent" => Ok(Self::Absent),
            Raw::Absent(text) => Err(serde::de::Error::custom(format!(
                "expected \"absent\" or a value, got {text}"
            ))),
            Raw::Present { reg_type, data_hex } => {
                let data = hex_decode(&data_hex).map_err(serde::de::Error::custom)?;
                Ok(Self::Present { reg_type, data })
            }
        }
    }
}

fn hex_encode(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("hex value has an odd length: {text}"));
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let high = hex_nibble(bytes[index])?;
        let low = hex_nibble(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(format!("hex value contains {byte:02x}")),
    }
}
