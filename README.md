# Meshtastic Rust

A native Rust and GTK4 desktop client for Meshtastic nodes.

## Features

- USB serial, Wi-Fi/TCP, and Bluetooth LE connections.
- Node list, channel messages, and direct conversations.
- Node configuration, channels, and module settings.
- App themes, notifications, and local message history.
- ESP32 firmware updates and factory reset + flash, with backups and verification.
- UF2 updates with automatic drive detection.

## Pull Requests
  Please do make pull requests if something does not work. I do not care if you use AI as long as the code is not complete rubbish.

## Build and run

Requires Rust, GTK 4.12+, and GTK development tools. Protoc is bundled.

```sh
git clone https://github.com/Peaks2000/meshtastic-rust.git
cd meshtastic-rust
```

### Linux

Install dependencies for your distribution:

```sh
# Fedora
sudo dnf install gtk4-devel dbus-devel bluez-libs-devel gcc pkgconf-pkg-config python3

# Ubuntu / Debian
sudo apt install libgtk-4-dev libdbus-1-dev build-essential pkg-config python3-venv
```

Build and launch:

```sh
cargo build --release --locked
./target/release/meshtastic-gtk
```

Optional ESP32 flashing helper:

```sh
./tools/setup-esptool.sh
```

### Windows

From an MSYS2 UCRT64 shell with Rust installed:

```sh
pacman -S mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-toolchain mingw-w64-ucrt-x86_64-pkgconf
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release --locked
./target/release/meshtastic-gtk.exe
```

Keep the MSYS2 UCRT64 `bin` directory on PATH. Standalone distribution requires
bundling the GTK runtime. Windows builds have not been validated.

## Third-party licences

This project is [GPL-3.0-only](LICENSE).

- Meshtastic protocol definitions and logos: GPL-3.0; logos are Meshtastic LLC trademarks.
- GTK4: LGPL-2.1-or-later. Rust GTK bindings: MIT.
- Espressif esptool: GPL-2.0-or-later.

See [THIRD_PARTY.md](THIRD_PARTY.md) for sources and attribution. Rust dependencies
and versions are listed in [Cargo.toml](Cargo.toml) and [Cargo.lock](Cargo.lock).
