use crate::model::{ChatMessage, Connection};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fs, io::Write, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: String,
    pub compact_nodes: bool,
    pub show_power: bool,
    pub show_position: bool,
    pub show_signal: bool,
    pub show_role: bool,
    pub show_hops: bool,
    pub show_last_heard: bool,
    pub relative_time: bool,
    pub notifications: bool,
    pub remember_messages: bool,
    pub retention_days: u32,
    pub auto_connect: bool,
    pub last_connection: Option<Connection>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            compact_nodes: false,
            show_power: true,
            show_position: true,
            show_signal: true,
            show_role: true,
            show_hops: true,
            show_last_heard: true,
            relative_time: true,
            notifications: false,
            remember_messages: true,
            retention_days: 30,
            auto_connect: false,
            last_connection: None,
        }
    }
}

pub fn directory() -> PathBuf {
    ProjectDirs::from("org", "MeshtasticGTK", "MeshtasticGTK")
        .map(|d| d.config_dir().to_owned())
        .unwrap_or_else(|| PathBuf::from(".meshtastic-gtk"))
}

pub fn read<T: DeserializeOwned + Default>(name: &str) -> T {
    fs::read(directory().join(name))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn write<T: Serialize>(name: &str, data: &T) -> Result<()> {
    let dir = directory();
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let temporary = dir.join(format!(".{name}.tmp"));
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(data)?)?;
    file.sync_all()?;
    fs::rename(temporary, dir.join(name)).context("Saving local app data")
}

pub fn load_history(prefs: &Preferences) -> Vec<ChatMessage> {
    if !prefs.remember_messages {
        return vec![];
    }
    let mut messages: Vec<ChatMessage> = read("messages.json");
    prune_history(&mut messages, prefs);
    messages
}
pub fn prune_history(messages: &mut Vec<ChatMessage>, prefs: &Preferences) {
    if prefs.retention_days > 0 {
        let cutoff = chrono::Utc::now().timestamp() - prefs.retention_days as i64 * 86400;
        messages.retain(|m| m.timestamp >= cutoff);
    }
    if messages.len() > 5000 {
        messages.drain(..messages.len() - 5000);
    }
}
