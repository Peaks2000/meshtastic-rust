#!/usr/bin/env python3
"""One-session, update-only Espressif helper. Never erase all flash or force checks."""
import argparse
import hashlib
import os
from pathlib import Path
import struct
import sys

import esptool
from esptool.bin_image import LoadFirmwareImage
from esptool.util import flash_size_bytes

APP_OFFSET = 0x10000


def checked_image(chip, data):
    image = LoadFirmwareImage(chip, data)
    image.verify()
    if image.checksum != image.calculate_checksum():
        raise ValueError("Firmware checksum is invalid")
    if not image.append_digest or image.stored_digest != image.calc_digest:
        raise ValueError("Firmware SHA-256 is missing or invalid")
    if not image.segments or image.segments[0].data[:4] != struct.pack("<I", 0xABCD5432):
        raise ValueError("This is not an application update image")
    return image


def application_partition(backup, image_size):
    table = backup[0x8000:0x9000]
    if len(table) != 0x1000:
        raise ValueError("Incomplete flash backup")
    partitions = []
    footer_seen = False
    for pos in range(0, len(table), 32):
        entry = table[pos:pos + 32]
        magic = struct.unpack_from("<H", entry)[0]
        if magic == 0xEBEB:
            if entry[2:16] != b"\xff" * 14 or entry[16:] != hashlib.md5(table[:pos]).digest():
                raise ValueError("Partition table checksum failed")
            footer_seen = True
            break
        if magic != 0x50AA:
            raise ValueError("Unrecognized partition layout; nothing was written")
        _, kind, subtype, offset, size, label, flags = struct.unpack("<HBBII16sI", entry)
        if offset < 0x9000 or size == 0 or offset % 4096 or size % 4096 or offset + size > len(backup):
            raise ValueError("Invalid partition boundaries")
        partitions.append((offset, offset + size, kind, subtype, flags))
    if not footer_seen:
        raise ValueError("Cannot verify the partition table; nothing was written")
    previous = 0
    for offset, end, *_ in sorted(partitions):
        if offset < previous:
            raise ValueError("Overlapping partitions")
        previous = end
    apps = [part for part in partitions if part[0] == APP_OFFSET and part[2] == 0]
    if len(apps) != 1 or apps[0][4] != 0:
        raise ValueError("No supported unencrypted application partition at 0x10000")
    app = apps[0]
    if (image_size + 4095) // 4096 * 4096 > app[1] - app[0]:
        raise ValueError("Firmware is too large for the node's application partition")
    return app[0], app[1]


def write_private(path, data):
    with open(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())


def update(port, chip, node, image_path, backup_path):
    # Parse and validate the pinned image before opening the serial device.
    data = Path(image_path).read_bytes()
    image = checked_image(chip, data)
    esp = None
    write_started = False
    reset_done = False
    try:
        esp = esptool.detect_chip(port=port, baud=115200, connect_mode="default-reset")
        actual_chip = esp.CHIP_NAME.lower().replace("-", "")
        if actual_chip != chip or image.chip_id != esp.IMAGE_CHIP_ID:
            raise ValueError("The physical chip does not match the firmware")
        if esp.secure_download_mode or esp.get_secure_boot_enabled() or esp.get_flash_encryption_enabled():
            raise ValueError("Secure boot/encrypted flash detected; update blocked")
        mac = bytes(esp.read_mac())
        if int.from_bytes(mac[-4:], "big") != node:
            raise ValueError("The USB node identity changed; update blocked")
        esp = esp.run_stub()
        size = flash_size_bytes(esptool.detect_flash_size(esp))
        if size is None or size < 1024 * 1024 or size > 32 * 1024 * 1024:
            raise ValueError("Cannot safely determine flash capacity")
        print("Backing up the node before writing…", flush=True)
        # Stay at the conservative rate rather than assuming a fast USB bridge.
        backup = esptool.read_flash(esp, 0, size)
        if backup is None or len(backup) != size:
            raise ValueError("Flash backup is incomplete; nothing was written")
        if esp.flash_md5sum(0, size).lower() != hashlib.md5(backup).hexdigest():
            raise ValueError("Backup verification failed; nothing was written")
        write_private(backup_path, backup)
        print(f"Backup saved: {backup_path}", flush=True)
        app_start, app_end = application_partition(backup, len(data))
        if backup[app_start:app_start + 1] != b"\xe9":
            raise ValueError("No existing application found; this tool only performs updates")
        # No factory image, bootloader write, eFuse change, force flag, or full erase.
        print("Updating application…", flush=True)
        write_started = True
        esptool.write_flash(esp, [(app_start, data)], flash_size="keep", flash_mode="keep",
                            flash_freq="keep", erase_all=False, force=False, compress=True)
        esptool.verify_flash(esp, [(app_start, data)], flash_size="keep", flash_mode="keep", flash_freq="keep")
        # Also verify the bootloader, partition table, and all data outside the app.
        for start, end in [(0, app_start), (app_end, size)]:
            if end > start and esp.flash_md5sum(start, end - start).lower() != hashlib.md5(backup[start:end]).hexdigest():
                raise ValueError("Protected-region verification failed. Keep the backup for recovery")
        print("Firmware verified. Bootloader and settings regions are unchanged.", flush=True)
        esp.hard_reset()
        reset_done = True
    finally:
        if esp is not None:
            # A failed preflight returns to the original application. An interrupted
            # write stays in the ROM loader so a partial app is not presented as success.
            if not write_started and not reset_done:
                try:
                    esp.hard_reset()
                except Exception:
                    pass
            esp._port.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", required=True)
    parser.add_argument("--chip", required=True)
    parser.add_argument("--node", type=int, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--backup", type=Path, required=True)
    args = parser.parse_args()
    try:
        update(args.port, args.chip, args.node, args.image, args.backup)
    except Exception as error:
        print(f"Update stopped: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
