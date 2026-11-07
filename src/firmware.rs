//! Official firmware lookup; this module never accesses the radio.
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{path::PathBuf, process::Command};

#[derive(Clone, Debug, Deserialize)]
pub struct Firmware {
    pub board: String,
    pub target: String,
    pub tag: String,
    pub title: String,
    pub size: u64,
    pub path: Option<PathBuf>,
}

pub fn fetch(board: &str, tag: Option<&str>) -> Result<Firmware> {
    let mut command = Command::new(crate::flashing::python().context("Python runtime is missing")?);
    command
        .arg("-I")
        .arg(crate::flashing::flash_helper().with_file_name("firmware-download.py"))
        .args(["--board", board]);
    if let Some(tag) = tag {
        command
            .args(["--tag", tag, "--output"])
            .arg(crate::storage::directory().join("firmware-cache"));
    }
    let output = command.output().context("Starting firmware lookup")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    serde_json::from_slice(&output.stdout).context("Reading official firmware metadata")
}
