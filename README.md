# Meshtastic Rust GTK client

A native GTK4 desktop app with USB serial, TCP/Wi-Fi, and Bluetooth LE transports.

The left sidebar puts **Connect**, **Nodes**, and **Messages** at the top, with
**Flashing tool** and **Settings** at the bottom. Settings opens to **Node settings**
and **App settings**.

## Run on Linux

On Fedora, install the development packages once:

```sh
sudo dnf install gtk4-devel dbus-devel bluez-libs-devel gcc pkgconf-pkg-config
```

Then run:

```sh
./launch.sh
```

`cargo build --release --locked` creates `target/release/meshtastic-gtk`.
GTK 4.12 or newer is required. Protoc is bundled as a build dependency.

## Connections

* **USB:** select the USB serial port (usually `/dev/ttyUSB0` or `/dev/ttyACM0` on
  Linux, `COM3` etc. on Windows). The default rate is 115200. Close browser flashers,
  serial monitors and other clients before connecting. On Fedora, add your account
  to `dialout` and log in again if access is denied. The client does not toggle
  reset/bootloader control lines.
* **Wi-Fi:** enter the radio's hostname or IP and TCP port (normally 4403). This is
  the node's TCP API, rather than its HTTP webpage. The radio must have network
  connectivity and its TCP API enabled. Many boards cannot use Wi-Fi and BLE
  simultaneously.
* **Bluetooth LE:** scan for the Meshtastic service, select a node, and connect.
  Pair through the OS Bluetooth settings first if the radio requires a PIN. The
  transport uses ToRadio writes, FromNum notifications, and drains FromRadio.

Only one radio is connected at a time. A configuration handshake must finish before
the UI marks the connection ready. USB/TCP itself does not add encryption.

## Node settings and Apple parity

The native forms are generated at runtime from the **official Meshtastic protobuf
descriptors and field metadata**, the same upstream definitions used by the Apple
app. The forms include every Config and ModuleConfig category, eight channels,
editable user identity, canned message text, and ringtone.

Booleans use switches, enums use choice lists, and numeric fields are validated.
Nested configuration messages have nested forms. Repeated fields use JSON arrays;
byte arrays such as channel PSKs and security keys use Base64. Secret entries are
masked and have a reveal icon. Field descriptions, units, and firmware availability
notes come from the official schema. Deprecated fields remain available for older
firmware. Newer fields may not work on older firmware or on boards without the
relevant hardware.

Values must first be read from the radio: there are no writable offline defaults.
Edits preserve fields the UI does not expose, including unknown wire fields. Saving
uses the node's current admin-session passkey, refreshes expired sessions, sends
only the selected category, waits for a radio ACK, and requests readback. A timeout
or rejection is shown as a failure, never a successful save. Configuration changes
can reboot a node or disconnect its current transport. Administration currently
targets the directly connected node; remote-node PKI administration is not included.

The desktop app settings implement theme, compact node layout, per-field node
visibility, relative timestamps, startup connection, desktop notifications, and
local message-history retention. Apple-only services (iCloud, Siri, CarPlay,
WeatherKit, iOS app icon selection) do not have desktop equivalents here.

## Firmware flashing

Select the original `.bin` or `.uf2` file from an
[official Meshtastic release](https://github.com/meshtastic/firmware/releases).
The page has no manual chip, offset, or update-drive fields.

Choose **Update** to keep settings, or **Factory reset + flash (ESP32 USB)** to
restore defaults. Both modes use the original application `.bin` file. Factory
mode is a settings reset plus application flash; it does not install a merged
factory/bootloader image. After a verified application write, it erases only the
recognized NVS, filesystem, and OTA-selection data partitions and checks the erase.
The confirmation explicitly warns that settings, channels, keys, and saved nodes
will be cleared. Missing or unknown settings partitions block factory mode before
any write. UF2 currently supports the update mode only.

For ESP32 nodes, connect over USB first. Install the pinned Espressif helper once:

```sh
./tools/setup-esptool.sh
```

The updater checks the reported board against the release filename and detects the
chip from the image. The helper validates the image checksum and SHA-256, checks the
physical chip and node MAC, and refuses secure-boot/encrypted devices. It saves and
verifies a full flash backup before checking the partition table and writing only
the application at `0x10000`. It checks readback and verifies that protected flash is unchanged before restarting
the node. In Update mode this includes all saved settings; Factory mode explicitly
allows the recognized settings partitions to be cleared. Factory/erase,
bootloader, and merged images are blocked. No full erase or force option is used.
The filename check is a compatibility check, not a cryptographic guarantee that an
image is authentic. Only use firmware from a source you trust.

For UF2 nodes, put the node in its firmware-update mode and choose the firmware
file. The matching mounted drive is detected from `INFO_UF2.TXT`. The updater
requires exactly one matching device and a readable `CURRENT.UF2` backup, checks
every UF2 block, family ID, address, and block count, and refuses protected regions
or mismatched application start addresses. Currently accepted families are
nRF52840 and RP2040, with conservative application ranges. Other layouts fail
closed. UF2 transfer completion does not prove installation; reconnect and check
the running firmware version.

Backups are stored under the app's private configuration directory in
`firmware-backups/`. A complete ESP32 backup includes configuration and keys;
keep it private. Reading a full backup can take several minutes. Do not unplug the
node or close the updater during a write. No updater can guarantee immunity to
power loss, bad firmware, failing hardware, or OS crashes. If a write fails, keep
the backup and use the board's normal bootloader recovery procedure.

Flashing safeguards are tested with simulated hardware. No firmware has been
written to a physical node as part of development validation. The real hardware
check only reads the USB Meshtastic handshake and node database.

## Windows build

The Rust sources use cross-platform `tokio-serial` and `btleplug` backends. Install
Rust's GNU toolchain and GTK4 through MSYS2 UCRT64:

```sh
pacman -S mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-toolchain mingw-w64-ucrt-x86_64-pkgconf
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release --locked
```

Build from the UCRT64 environment, with its `bin` directory on PATH and
`PKG_CONFIG_PATH` pointing to its `lib/pkgconfig`. Distribute the GTK runtime DLLs,
loaders, icons and schemas along with the executable. USB drivers are supplied by
Windows/the board vendor; Bluetooth uses Windows' BLE stack. Create `tools/esptool` as a Python virtual environment and install esptool 5.6.0
for the flashing helper. A Windows installer and Windows hardware validation are
not provided by this Linux build.

## Local data

Preferences and up to 5,000 messages are saved in the platform configuration
directory (`~/.config/meshtasticgtk` on Linux). Unix directory/file permissions are
restricted to your user. History can be disabled or cleared in App settings. Radio
configuration snapshots and admin-session keys are never written to disk. No
analytics service is used.

## Checks

```sh
cargo test --locked
MESHTASTIC_UI_SMOKE=1 ./target/debug/meshtastic-gtk
./target/debug/meshtastic-gtk --probe-usb /dev/ttyUSB0
```

The Espressif helper also has in-memory safety tests (no physical device):

```sh
tools/esptool/bin/python -m unittest discover -s tests -p 'test_flash_esp.py'
```

The smoke check exercises all GTK pages and all generated settings forms without
writing to a radio. The optional USB probe performs a real read-only handshake,
prints the node count and releases the port. Firmware writes must be tested with
an image and a board chosen explicitly by the user.

See [THIRD_PARTY.md](THIRD_PARTY.md) for upstream source versions and licenses.
