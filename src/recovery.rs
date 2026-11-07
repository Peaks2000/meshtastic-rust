use crate::flashing;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub enum Device {
    Esp {
        port: String,
        chip: String,
        node: u32,
    },
    Uf2 {
        volume: PathBuf,
        label: String,
    },
    Other {
        label: String,
    },
}
impl Device {
    pub fn label(&self) -> String {
        match self {
            Self::Esp { port, chip, .. } => {
                format!("{} ROM recovery · {port}", chip.to_uppercase())
            }
            Self::Uf2 { label, .. } => format!("UF2 recovery · {label}"),
            Self::Other { label } => label.clone(),
        }
    }
}

#[derive(Deserialize)]
struct Probe {
    chip: String,
    node: u32,
    recoverable: bool,
}

fn parse_probe(output: &[u8], port: &str) -> Result<Device> {
    let output = String::from_utf8_lossy(output);
    let json = output
        .lines()
        .find_map(|line| line.strip_prefix("MESHTASTIC_RECOVERY_JSON:"))
        .context("No ROM bootloader response")?;
    let probe: Probe = serde_json::from_str(json)?;
    if !probe.recoverable {
        return Ok(Device::Other {
            label: format!("{} · {port} · security-locked recovery", probe.chip),
        });
    }
    if ![
        "esp32", "esp32s2", "esp32s3", "esp32c3", "esp32c6", "esp32h2",
    ]
    .contains(&probe.chip.as_str())
    {
        bail!("Unsupported recovery chip");
    }
    Ok(Device::Esp {
        port: port.into(),
        chip: probe.chip,
        node: probe.node,
    })
}

fn probe(port: &str) -> Result<Device> {
    let mut child = Command::new(flashing::python().context("Flashing runtime not installed")?)
        .args(["-I", "-u"])
        .arg(flashing::flash_helper())
        .args(["--probe-recovery", "--port", port])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Recovery probe timed out");
        }
        thread::sleep(Duration::from_millis(40));
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("Not a responding ESP32 ROM bootloader");
    }
    parse_probe(&output.stdout, port)
}

pub fn scan(mounts: Vec<PathBuf>, skip_port: Option<String>) -> Result<Vec<Device>> {
    let mut devices = vec![];
    for volume in mounts {
        if let Ok(info) = fs::read_to_string(volume.join("INFO_UF2.TXT")) {
            let label = info
                .lines()
                .find_map(|line| {
                    line.strip_prefix("Board-ID:")
                        .or_else(|| line.strip_prefix("Model:"))
                })
                .unwrap_or("UF2 device")
                .trim()
                .to_string();
            devices.push(Device::Uf2 { volume, label });
        }
    }
    for port in serialport::available_ports()? {
        if skip_port.as_deref() == Some(&port.port_name) {
            continue;
        }
        let serialport::SerialPortType::UsbPort(info) = port.port_type else {
            continue;
        };
        let description = info.product.unwrap_or_default();
        let lower = description.to_lowercase();
        if lower.contains("dfu")
            || lower.contains("bootloader")
            || (info.vid == 0x239a && info.pid == 0x0029)
        {
            devices.push(Device::Other {
                label: format!(
                    "DFU interface · {description} · {} (use UF2 mode)",
                    port.port_name
                ),
            });
        }
        if [0x303a, 0x10c4, 0x1a86, 0x0403].contains(&info.vid)
            && let Ok(device) = probe(&port.port_name)
        {
            devices.push(device);
        }
    }
    // Standard USB DFU interfaces need not expose a serial port. Identify them on Linux,
    // but never pretend an unknown DFU protocol is safe to write using esptool.
    #[cfg(target_os = "linux")]
    if let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") {
        for entry in entries.flatten() {
            let path = entry.path();
            if fs::read_to_string(path.join("bInterfaceClass")).is_ok_and(|s| s.trim() == "fe")
                && fs::read_to_string(path.join("bInterfaceSubClass"))
                    .is_ok_and(|s| s.trim() == "01")
            {
                let id = entry.file_name().to_string_lossy().to_string();
                let parent = id.split(':').next().unwrap_or(&id);
                let product = fs::read_to_string(
                    PathBuf::from("/sys/bus/usb/devices")
                        .join(parent)
                        .join("product"),
                )
                .unwrap_or_else(|_| parent.into());
                devices.push(Device::Other {
                    label: format!("USB DFU · {} (protocol unsupported)", product.trim()),
                });
            }
        }
    }
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_rom_identity_is_required() {
        assert!(parse_probe(b"booting normally", "/dev/test").is_err());
        let device = parse_probe(b"log\nMESHTASTIC_RECOVERY_JSON:{\"chip\":\"esp32s3\",\"node\":123,\"recoverable\":true}\n", "/dev/test").unwrap();
        assert!(matches!(device, Device::Esp { node: 123, .. }));
        let blocked = parse_probe(
            b"MESHTASTIC_RECOVERY_JSON:{\"chip\":\"esp32s3\",\"node\":0,\"recoverable\":false}",
            "/dev/test",
        )
        .unwrap();
        assert!(matches!(blocked, Device::Other { .. }));
    }
}
