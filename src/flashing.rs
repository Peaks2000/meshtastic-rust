//! Guarded firmware flashing. Images are pinned in memory before confirmation.
use crate::{model::Event, storage};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, mpsc::Sender},
};

const MAX_IMAGE: u64 = 16 * 1024 * 1024;
const UF2_MAGIC: [u8; 8] = [0x55, 0x46, 0x32, 0x0a, 0x57, 0x51, 0x5d, 0x9e];

#[derive(Clone, Debug)]
pub struct UsbTarget {
    pub board: String,
    pub node: u32,
    pub port: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlashMode {
    Update,
    Factory,
}

#[derive(Clone)]
pub enum FlashPlan {
    Esp {
        mode: FlashMode,
        target: UsbTarget,
        chip: String,
        name: String,
        image: Arc<[u8]>,
    },
    Uf2 {
        volume: PathBuf,
        identity: String,
        name: String,
        image: Arc<[u8]>,
        family: u32,
    },
}
impl FlashPlan {
    pub fn is_factory(&self) -> bool {
        matches!(
            self,
            Self::Esp {
                mode: FlashMode::Factory,
                ..
            }
        )
    }
    pub fn summary(&self) -> String {
        match self {
            Self::Esp { target, name, .. } => format!(
                "{name}\n{} · {}\nA backup will be saved before updating.",
                target.board, target.port
            ),
            Self::Uf2 { name, volume, .. } => format!(
                "{name}\n{}\nThe installed application will be backed up first.",
                volume.display()
            ),
        }
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn firmware_board(name: &str) -> Result<String> {
    let lower = name.to_ascii_lowercase();
    if ["factory", "erase", "bootloader", "merged"]
        .iter()
        .any(|part| lower.contains(part))
    {
        bail!("Choose an application update. Factory, erase, and bootloader images are blocked.");
    }
    let rest = lower
        .strip_prefix("firmware-")
        .context("Choose the original firmware-BOARD-VERSION file from a Meshtastic release")?;
    let version = rest
        .char_indices()
        .find_map(|(i, c)| {
            if c == '-'
                && rest.as_bytes().get(i + 1).is_some_and(u8::is_ascii_digit)
                && rest[i + 1..].contains('.')
            {
                Some(i)
            } else {
                None
            }
        })
        .context(
            "Cannot identify the board in this filename. Keep the original release filename",
        )?;
    Ok(normalize(&rest[..version]))
}

pub fn prepare(
    firmware: &Path,
    target: Option<UsbTarget>,
    mounts: &[PathBuf],
    mode: FlashMode,
) -> Result<FlashPlan> {
    let metadata = fs::metadata(firmware).context("Choose a readable firmware file")?;
    if !metadata.is_file() || metadata.len() < 24 || metadata.len() > MAX_IMAGE {
        bail!("Invalid firmware file size");
    }
    let image: Arc<[u8]> = fs::read(firmware)?.into();
    let name = firmware
        .file_name()
        .and_then(|v| v.to_str())
        .context("Invalid firmware filename")?
        .to_string();
    let board = firmware_board(&name)?;
    match firmware
        .extension()
        .and_then(|v| v.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("bin") => {
            let target =
                target.context("Connect to the node over USB first so its board can be checked")?;
            if normalize(&target.board) != board {
                bail!(
                    "Firmware does not match the connected {} node",
                    target.board
                );
            }
            let chip = esp_chip(&image)?.to_string();
            python().context("Install the flashing helper using tools/setup-esptool.sh")?;
            Ok(FlashPlan::Esp {
                mode,
                target,
                chip,
                name,
                image,
            })
        }
        Some("uf2") => {
            if mode == FlashMode::Factory {
                bail!(
                    "Factory reset + flash is supported for ESP32 USB nodes. Use Update for UF2 firmware."
                );
            }
            let info = validate_uf2(&image)?;
            let mut candidates = vec![];
            for volume in mounts.iter().collect::<BTreeSet<_>>() {
                if let Ok(identity) = fs::read_to_string(volume.join("INFO_UF2.TXT"))
                    && volume_matches(&identity, &board, info.family)
                {
                    candidates.push((volume.to_path_buf(), identity));
                }
            }
            if candidates.is_empty() {
                bail!(
                    "No matching update drive found. Put the node in firmware-update mode (usually double-press Reset), then try again."
                );
            }
            if candidates.len() > 1 {
                bail!(
                    "More than one matching update drive found. Unplug the other nodes and try again."
                );
            }
            let (volume, identity) = candidates.remove(0);
            let current = fs::read(volume.join("CURRENT.UF2")).context("Cannot back up the installed application from this update drive; nothing was written")?;
            validate_current_uf2(&current, info.family, info.start)?;
            Ok(FlashPlan::Uf2 {
                volume,
                identity,
                name,
                image,
                family: info.family,
            })
        }
        _ => bail!("Choose a .bin or .uf2 firmware file"),
    }
}

fn esp_chip(image: &[u8]) -> Result<&'static str> {
    if image.len() < 24 || image[0] != 0xe9 || image[1] == 0 || image[1] > 16 {
        bail!("Invalid ESP32 application image");
    }
    match u16::from_le_bytes([image[12], image[13]]) {
        0 => Ok("esp32"),
        2 => Ok("esp32s2"),
        9 => Ok("esp32s3"),
        5 => Ok("esp32c3"),
        13 => Ok("esp32c6"),
        16 => Ok("esp32h2"),
        _ => bail!("Unsupported ESP32 image chip"),
    }
}

struct Uf2Info {
    family: u32,
    start: u32,
}
fn word(block: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(block[offset..offset + 4].try_into().unwrap())
}

/// Every block must be present exactly once and entirely inside an application range.
fn validate_uf2(image: &[u8]) -> Result<Uf2Info> {
    if image.is_empty() || !image.len().is_multiple_of(512) || image.len() as u64 > MAX_IMAGE {
        bail!("Incomplete UF2 image");
    }
    let count = image.len() / 512;
    let mut numbers = BTreeSet::new();
    let mut ranges = BTreeSet::new();
    let mut family = None;
    let mut start = u32::MAX;
    for block in image.as_chunks::<512>().0 {
        if block[..8] != UF2_MAGIC || word(block, 508) != 0x0ab16f30 {
            bail!("Invalid UF2 block marker");
        }
        // Only standard main-flash blocks with an explicit family ID are accepted.
        if word(block, 8) != 0x2000 {
            bail!("Unsupported UF2 flags; special/erase images are blocked");
        }
        let address = word(block, 12);
        let size = word(block, 16);
        let number = word(block, 20);
        let total = word(block, 24);
        let id = word(block, 28);
        if total as usize != count || number as usize >= count || !numbers.insert(number) {
            bail!("Missing or duplicate UF2 blocks");
        }
        if size == 0 || size > 476 || !size.is_multiple_of(4) || !address.is_multiple_of(4) {
            bail!("Invalid UF2 payload size or alignment");
        }
        if family.is_some_and(|previous| previous != id) {
            bail!("Mixed UF2 chip families");
        }
        family = Some(id);
        let end = address.checked_add(size).context("UF2 address overflow")?;
        let safe = match id {
            0xada52840 => address >= 0x26000 && end <= 0xea000,
            // Conservative application-only limit: leave upper flash/configuration alone.
            0xe48bff56 => address >= 0x10000000 && end <= 0x10100000,
            _ => false,
        };
        if !safe {
            bail!("UF2 targets an unsupported chip or a protected bootloader/settings region");
        }
        if !ranges.insert((address, end)) {
            bail!("Overlapping UF2 blocks");
        }
        start = start.min(address);
    }
    let mut previous_end = 0;
    for (address, end) in ranges {
        if address < previous_end {
            bail!("Overlapping UF2 blocks");
        }
        previous_end = end;
    }
    Ok(Uf2Info {
        family: family.unwrap(),
        start,
    })
}
fn validate_current_uf2(image: &[u8], family: u32, start: u32) -> Result<()> {
    let current =
        validate_uf2(image).context("Installed firmware could not be validated for backup")?;
    if current.family != family || current.start != start {
        bail!("The update does not match the installed application's chip or start address");
    }
    Ok(())
}
fn volume_matches(identity: &str, board: &str, family: u32) -> bool {
    let chip_matches = match family {
        0xada52840 => identity.to_lowercase().contains("nrf52840"),
        0xe48bff56 => identity.to_lowercase().contains("rp2040"),
        _ => false,
    };
    chip_matches
        && identity
            .lines()
            .filter(|line| line.starts_with("Board-ID:") || line.starts_with("Model:"))
            .filter_map(|line| line.split_once(':').map(|(_, value)| normalize(value)))
            .any(|value| {
                value == board
                    || value.strip_prefix("nrf52840") == Some(board)
                    || value.strip_prefix("rp2040") == Some(board)
            })
}

pub fn python() -> Option<PathBuf> {
    if let Some(root) = installed_helpers() {
        let path = root.join("python/bin/python3");
        if path.is_file() {
            return Some(path);
        }
    }
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/esptool");
    for relative in ["bin/python", "Scripts/python.exe"] {
        let path = base.join(relative);
        if path.is_file() {
            return Some(path);
        }
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        for name in if cfg!(windows) {
            &["python.exe", "python3.exe"][..]
        } else {
            &["python3"][..]
        } {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn installed_helpers() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let root = executable
        .parent()?
        .parent()?
        .join("libexec/meshtastic-rust");
    root.join("flash-esp.py").is_file().then_some(root)
}

fn flash_helper() -> PathBuf {
    installed_helpers()
        .map(|root| root.join("flash-esp.py"))
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/flash-esp.py"))
}

pub fn check_runtime() -> Result<String> {
    if !flash_helper().is_file() {
        bail!("Flashing helper script is missing");
    }
    let output = Command::new(python().context("Flashing Python runtime is missing")?)
        .args(["-I", "-c", "import esptool; print(esptool.__version__)"])
        .output()
        .context("Starting flashing runtime")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}
fn backup_directory() -> Result<PathBuf> {
    let root = storage::directory().join("firmware-backups");
    fs::create_dir_all(&root)?;
    let path = root.join(format!(
        "{}-{:08x}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        rand::random::<u32>()
    ));
    fs::create_dir(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

pub fn start(plan: FlashPlan, events: Sender<Event>) {
    std::thread::spawn(move || {
        let result = execute(&plan, &events);
        if let Err(error) = &result {
            let _ = events.send(Event::Firmware(format!("Error: {error:#}")));
        }
        let _ = events.send(Event::FlashFinished(result.is_ok()));
    });
}
fn execute(plan: &FlashPlan, events: &Sender<Event>) -> Result<()> {
    let directory = backup_directory()?;
    match plan {
        FlashPlan::Esp {
            mode,
            target,
            chip,
            name,
            image,
        } => {
            let staged = directory.join(name);
            private_write(&staged, image)?;
            // One connection performs identity, security, backup, partition, write and readback checks.
            let mut command = Command::new(python().context("Python/esptool is unavailable")?);
            if *mode == FlashMode::Factory {
                command.arg("-u").arg(flash_helper()).arg("--factory-reset");
            } else {
                command.arg("-u").arg(flash_helper());
            }
            let mut child = command
                .args([
                    "--port",
                    &target.port,
                    "--chip",
                    chip,
                    "--node",
                    &target.node.to_string(),
                    "--image",
                ])
                .arg(&staged)
                .arg("--backup")
                .arg(directory.join("original-flash.bin"))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context("Starting the firmware updater")?;
            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            let out = events.clone();
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let _ = out.send(Event::Firmware(line));
                }
            });
            let err = events.clone();
            let err_reader = std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let _ = err.send(Event::Firmware(line));
                }
            });
            let status = child.wait()?;
            let _ = reader.join();
            let _ = err_reader.join();
            if !status.success() {
                bail!(
                    "Update stopped. Check the log; any completed backup is in {}",
                    directory.display()
                );
            }
        }
        FlashPlan::Uf2 {
            volume,
            identity,
            name,
            image,
            family,
        } => {
            let info = validate_uf2(image)?;
            let board = firmware_board(name)?;
            let now = fs::read_to_string(volume.join("INFO_UF2.TXT"))
                .context("The update drive disconnected; nothing was written")?;
            if &now != identity || !volume_matches(&now, &board, *family) {
                bail!("The update drive identity changed; nothing was written");
            }
            let current = fs::read(volume.join("CURRENT.UF2"))
                .context("Cannot back up the installed application; nothing was written")?;
            validate_current_uf2(&current, *family, info.start)?;
            private_write(&directory.join("original-application.uf2"), &current)?;
            private_write(&directory.join(name), image)?;
            let _ = events.send(Event::Firmware(format!(
                "Backup saved to {}",
                directory.display()
            )));
            // Recheck identity after reading the backup, immediately before opening the target.
            if fs::read_to_string(volume.join("INFO_UF2.TXT"))? != *identity {
                bail!("Update drive changed; nothing was written");
            }
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(volume.join("firmware.uf2"))
                .context("Cannot open the update drive")?;
            output
                .write_all(image)
                .context("Firmware transfer interrupted; keep the node in update mode and retry")?;
            output
                .sync_all()
                .context("Could not confirm the firmware transfer was flushed")?;
            let _ = events.send(Event::Firmware(
                "Firmware transferred. Reconnect to confirm the installed version.".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn block(number: u32, total: u32, address: u32) -> Vec<u8> {
        let mut block = vec![0u8; 512];
        block[..8].copy_from_slice(&UF2_MAGIC);
        for (offset, value) in [
            (8, 0x2000),
            (12, address),
            (16, 256),
            (20, number),
            (24, total),
            (28, 0xada52840),
            (508, 0x0ab16f30),
        ] {
            block[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        block
    }
    #[test]
    fn uf2_rejects_incomplete_duplicate_and_overlapping_blocks() {
        let good = [block(0, 2, 0x26000), block(1, 2, 0x26100)].concat();
        assert!(validate_uf2(&good).is_ok());
        assert!(validate_uf2(&good[..512]).is_err());
        assert!(validate_uf2(&[block(0, 2, 0x26000), block(0, 2, 0x26100)].concat()).is_err());
        assert!(validate_uf2(&[block(0, 2, 0x26000), block(1, 2, 0x26080)].concat()).is_err());
    }
    #[test]
    fn uf2_cannot_touch_bootloader_softdevice_or_settings() {
        for address in [0, 0x1000, 0x25000, 0xe9f80, 0xea000, 0xf4000] {
            assert!(
                validate_uf2(&block(0, 1, address)).is_err(),
                "Allowed {address:x}"
            );
        }
        let mut bad = block(0, 1, 0x26000);
        bad[511] = 0;
        assert!(validate_uf2(&bad).is_err());
        let mut erase = block(0, 1, 0x26000);
        erase[28..32].copy_from_slice(&0x4d455348u32.to_le_bytes());
        assert!(validate_uf2(&erase).is_err());
    }
    #[test]
    fn board_identity_must_match_and_special_images_are_blocked() {
        assert_eq!(
            firmware_board("firmware-heltec-v3-2.7.27.abc-update.bin").unwrap(),
            "heltecv3"
        );
        assert!(firmware_board("firmware-heltec-v3-2.7.27.factory.bin").is_err());
        assert!(firmware_board("meshtastic_factory_erase.uf2").is_err());
        assert!(!volume_matches(
            "Board-ID: nRF52840-RAK4631",
            "heltecv3",
            0xada52840
        ));
        assert!(volume_matches(
            "Board-ID: nRF52840-RAK4631",
            "rak4631",
            0xada52840
        ));
        assert!(!volume_matches(
            "Board-ID: nRF52840-RAK4631Pro",
            "rak4631",
            0xada52840
        ));
        assert!(!volume_matches(
            "Board-ID: nRF52840-RAK4631",
            "rak4631",
            0xe48bff56
        ));
    }
}
