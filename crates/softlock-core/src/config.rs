//! User settings. The wire format is TOML. The schema is generated from these types.
//!
//! A document that does not parse, or that names a field this version does not
//! know, is rejected. The caller keeps the last document that parsed.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::name::PRODUCT_DISPLAY_NAME;

/// Largest config document accepted from disk or from `PutConfig`.
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;

pub const CONFIG_FILE_NAME: &str = "config.toml";
pub const CONFIG_ERRORS_NAME: &str = "config.errors.toml";

const DEFAULT_HOTKEY: &str = "Ctrl+Alt+Shift+L";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    schema_version: u64,
    document: String,
    hotkey: String,
    message: String,
}

impl Config {
    pub fn initial() -> Self {
        Self::parse(&commented_default()).expect("built-in config")
    }

    pub fn schema_version(&self) -> u64 {
        self.schema_version
    }

    pub fn document(&self) -> &str {
        &self.document
    }

    pub fn hotkey(&self) -> &str {
        &self.hotkey
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn parse(toml: &str) -> Result<Self, Vec<String>> {
        if toml.trim().is_empty() {
            return Err(vec!["empty config".to_string()]);
        }
        if toml.len() > MAX_CONFIG_BYTES {
            return Err(vec![format!("config exceeds {MAX_CONFIG_BYTES} bytes")]);
        }
        if toml.contains('\0') {
            return Err(vec!["invalid config".to_string()]);
        }
        let document: UserConfig = toml::from_str(toml).map_err(field_error)?;
        let mut errors = document.check();
        if !errors.is_empty() {
            errors.sort();
            errors.dedup();
            return Err(errors);
        }
        Ok(Self {
            schema_version: u64::from(document.schema_version),
            hotkey: document.lock.hotkey.clone(),
            message: document.lock.message.clone(),
            document: toml.to_string(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigStore {
    config: Config,
    revision: u64,
}

pub enum ConfigError {
    Invalid(Vec<String>),
    Conflict,
}

impl ConfigStore {
    pub fn new() -> Self {
        Self {
            config: Config::initial(),
            revision: 1,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn document(&self) -> &str {
        self.config.document()
    }

    pub fn hotkey(&self) -> &str {
        self.config.hotkey()
    }

    pub fn message(&self) -> &str {
        self.config.message()
    }

    pub fn apply(&mut self, toml: String, base_revision: u64) -> Result<(), ConfigError> {
        if base_revision != self.revision {
            return Err(ConfigError::Conflict);
        }
        match Config::parse(&toml) {
            Ok(config) => {
                self.config = config;
                self.revision = self.revision.saturating_add(1);
                Ok(())
            }
            Err(errors) => Err(ConfigError::Invalid(errors)),
        }
    }
}

/// The published JSON Schema is generated from this type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct UserConfig {
    pub schema_version: u32,
    pub lock: LockSection,
    pub unlock: UnlockSection,
    pub keep_awake: KeepAwakeSection,
    pub pause: PauseSection,
    pub network: NetworkSection,
    pub takeover: TakeoverSection,
    pub hardening: HardeningSection,
    pub updates: UpdatesSection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct LockSection {
    pub hotkey: String,
    pub message: String,
    pub show_clock: bool,
    pub display: ScreenDisplay,
    pub dim_percent: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScreenDisplay {
    #[default]
    Opaque,
    Dimmed,
    Transparent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct UnlockSection {
    pub methods: Vec<String>,
    pub softlock_pin_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct KeepAwakeSection {
    pub enabled: bool,
    pub keep_display_on: TriState,
    pub battery_floor_percent: u8,
    pub heat_check: HeatCheck,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TriState {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HeatCheck {
    #[default]
    Auto,
    Off,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct PauseSection {
    pub enabled: bool,
    pub apps: Vec<PausedApp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PausedApp {
    pub path: String,
    pub mode: PauseMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PauseMode {
    #[default]
    Efficiency,
    Suspend,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct NetworkSection {
    pub keep_online: bool,
    pub notify_on_drop: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct TakeoverSection {
    pub win_l: bool,
    pub idle_lock: bool,
    pub idle_minutes: u16,
    pub lid_close: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct HardeningSection {
    pub disable_touch_pen: bool,
    pub disable_game_controllers: bool,
    pub uac_require_credentials: bool,
    pub suppress_notifications: bool,
    pub stop_accessibility_input: bool,
    pub power_button: PowerButton,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PowerButton {
    #[default]
    DoNothing,
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct UpdatesSection {
    pub channel: UpdateChannel,
    pub auto_check: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Dev,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyChord {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub virtual_key: u16,
    pub label: String,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            lock: LockSection::default(),
            unlock: UnlockSection::default(),
            keep_awake: KeepAwakeSection::default(),
            pause: PauseSection::default(),
            network: NetworkSection::default(),
            takeover: TakeoverSection::default(),
            hardening: HardeningSection::default(),
            updates: UpdatesSection::default(),
        }
    }
}

impl Default for LockSection {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            message: String::new(),
            show_clock: true,
            display: ScreenDisplay::Opaque,
            dim_percent: 50,
        }
    }
}

impl Default for UnlockSection {
    fn default() -> Self {
        Self {
            methods: vec!["windows_hello".to_string()],
            softlock_pin_enabled: false,
        }
    }
}

impl Default for KeepAwakeSection {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_display_on: TriState::Auto,
            battery_floor_percent: 15,
            heat_check: HeatCheck::Auto,
        }
    }
}

impl Default for NetworkSection {
    fn default() -> Self {
        Self {
            keep_online: true,
            notify_on_drop: true,
        }
    }
}

impl Default for TakeoverSection {
    fn default() -> Self {
        Self {
            win_l: false,
            idle_lock: false,
            idle_minutes: 10,
            lid_close: false,
        }
    }
}

impl Default for HardeningSection {
    fn default() -> Self {
        Self {
            disable_touch_pen: true,
            disable_game_controllers: true,
            uac_require_credentials: true,
            suppress_notifications: true,
            stop_accessibility_input: true,
            power_button: PowerButton::DoNothing,
        }
    }
}

impl Default for UpdatesSection {
    fn default() -> Self {
        Self {
            channel: UpdateChannel::Stable,
            auto_check: true,
        }
    }
}

impl UserConfig {
    fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.schema_version != 1 {
            errors.push("schema_version: unsupported".to_string());
        }
        if parse_hotkey(&self.lock.hotkey).is_none() {
            errors.push("lock.hotkey: unsupported chord".to_string());
        }
        if self.lock.message.chars().count() > 200
            || self.lock.message.chars().any(|c| c.is_control())
        {
            errors.push("lock.message: unsupported".to_string());
        }
        if self.lock.display == ScreenDisplay::Dimmed && !(10..=90).contains(&self.lock.dim_percent)
        {
            errors.push("lock.dim_percent: out of range".to_string());
        }
        if !(5..=50).contains(&self.keep_awake.battery_floor_percent) {
            errors.push("keep_awake.battery_floor_percent: out of range".to_string());
        }
        if !(1..=240).contains(&self.takeover.idle_minutes) {
            errors.push("takeover.idle_minutes: out of range".to_string());
        }
        if self.pause.apps.len() > 32 {
            errors.push("pause.apps: too many".to_string());
        }
        for (index, app) in self.pause.apps.iter().enumerate() {
            if app.path.is_empty()
                || app.path.len() > 260
                || app.path.contains('\0')
                || app.path.contains("..")
            {
                errors.push(format!("pause.apps[{index}].path: unsupported"));
            }
        }
        if self.unlock.methods.is_empty() {
            errors.push("unlock.methods: empty".to_string());
        }
        errors
    }
}

pub fn hotkey_in_document(document: &str) -> Option<HotkeyChord> {
    let config = Config::parse(document).ok()?;
    parse_hotkey(config.hotkey())
}

pub fn parse_hotkey(text: &str) -> Option<HotkeyChord> {
    if text.is_empty() || text.len() > 64 || text.chars().any(|c| c.is_control()) {
        return None;
    }
    let mut control = false;
    let mut alt = false;
    let mut shift = false;
    let mut key: Option<char> = None;
    for part in text.split('+') {
        if part.is_empty() {
            return None;
        }
        match part {
            "Ctrl" => control = true,
            "Alt" => alt = true,
            "Shift" => shift = true,
            other if key.is_none() && other.chars().count() == 1 => {
                let ch = other.chars().next()?;
                if ch.is_ascii_alphanumeric() {
                    key = Some(ch.to_ascii_uppercase());
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let key = key?;
    if !control || !alt || !shift {
        return None;
    }
    Some(HotkeyChord {
        control,
        alt,
        shift,
        virtual_key: u16::from(key as u8),
        label: format!("Ctrl+Alt+Shift+{key}"),
    })
}

pub fn published_schema_json() -> String {
    let schema = schemars::schema_for!(UserConfig);
    serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string())
}

/// `additionalProperties: false` on the root, and every table key, including
/// nested ones, is a property the schema names.
pub fn schema_accepts_document(schema_json: &str, document: &str) -> Result<(), String> {
    let schema: serde_json::Value =
        serde_json::from_str(schema_json).map_err(|_| "schema".to_string())?;
    let root = schema_object(&schema, &schema).ok_or_else(|| "schema".to_string())?;
    if root.get("additionalProperties") != Some(&serde_json::Value::Bool(false)) {
        return Err("schema".to_string());
    }
    let parsed: toml::Value = toml::from_str(document).map_err(|_| "document".to_string())?;
    let table = parsed.as_table().ok_or_else(|| "document".to_string())?;
    accept_table(&schema, root, "", table)
}

fn accept_table(
    root_schema: &serde_json::Value,
    object: &serde_json::Value,
    prefix: &str,
    table: &toml::map::Map<String, toml::Value>,
) -> Result<(), String> {
    let properties = object
        .get("properties")
        .and_then(|value| value.as_object())
        .ok_or_else(|| "schema".to_string())?;
    for (key, value) in table {
        let Some(property) = properties.get(key) else {
            let name = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            return Err(format!("unknown field: {name}"));
        };
        if let Some(nested) = value.as_table() {
            let child = schema_object(root_schema, property).ok_or_else(|| "schema".to_string())?;
            let next = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            accept_table(root_schema, child, &next, nested)?;
        }
    }
    Ok(())
}

fn schema_object<'a>(
    root: &'a serde_json::Value,
    schema: &'a serde_json::Value,
) -> Option<&'a serde_json::Value> {
    if schema.get("properties").is_some() {
        return Some(schema);
    }
    if let Some(items) = schema.get("allOf").and_then(|value| value.as_array()) {
        for item in items {
            if let Some(found) = schema_object(root, item) {
                return Some(found);
            }
        }
    }
    let reference = schema.get("$ref")?.as_str()?;
    let name = reference.rsplit('/').next()?;
    let defs = root
        .get("definitions")
        .or_else(|| root.get("$defs"))?
        .get(name)?;
    schema_object(root, defs)
}

fn field_error(error: toml::de::Error) -> Vec<String> {
    let text = error.to_string();
    if let Some(name) = quoted_field(&text, "unknown field `") {
        return vec![format!("unknown field: {name}")];
    }
    if let Some(name) = quoted_field(&text, "missing field `") {
        return vec![format!("missing field: {name}")];
    }
    vec!["invalid config".to_string()]
}

fn quoted_field<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let rest = text.split(marker).nth(1)?;
    let name = rest.split('`').next()?;
    if name.is_empty() || name.len() > 64 || name.chars().any(|c| c.is_control()) {
        None
    } else {
        Some(name)
    }
}

fn commented_default() -> String {
    format!(
        "# {name} settings. Edit this file while the computer is unlocked.\n\
         schema_version = 1\n\
         \n\
         [lock]\n\
         # Chord that locks. Ctrl, Alt, and Shift, plus one letter or digit.\n\
         hotkey = \"Ctrl+Alt+Shift+L\"\n\
         # Shown on the lock screen. Empty keeps the standard caption.\n\
         message = \"\"\n\
         show_clock = true\n\
         display = \"opaque\"\n",
        name = PRODUCT_DISPLAY_NAME,
    )
}

/// A change noticed while locked is offered after unlock. It is not applied
/// until the owner accepts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigOffer {
    effective: String,
    pending: Option<String>,
}

impl ConfigOffer {
    pub fn new(effective: &str) -> Self {
        Self {
            effective: effective.to_string(),
            pending: None,
        }
    }

    pub fn note_disk(&mut self, locked: bool, disk: &str) {
        if locked && disk != self.effective {
            self.pending = Some(disk.to_string());
        }
    }

    pub fn pending(&self) -> Option<&str> {
        self.pending.as_deref()
    }

    pub fn accept(&mut self) -> Option<String> {
        self.pending.take()
    }

    pub fn decline(&mut self) {
        self.pending = None;
    }

    pub fn replace_effective(&mut self, document: &str) {
        self.effective = document.to_string();
        if self.pending.as_deref() == Some(document) {
            self.pending = None;
        }
    }
}
