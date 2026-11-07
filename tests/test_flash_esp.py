"""Destructive-operation guards tested against a fake in-memory chip only."""
import hashlib
import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch, Mock

from esptool.bin_image import ESP32S3FirmwareImage, ImageSegment

spec = importlib.util.spec_from_file_location("updater", Path(__file__).parents[1] / "tools/flash-esp.py")
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)


def image_bytes():
    image = ESP32S3FirmwareImage()
    image.chip_id = 9
    image.segments = [ImageSegment(0x3FC88000, struct.pack("<I", 0xABCD5432) + b"\x00" * 252)]
    image.segments[0].name = ".dram0.data"
    return image.save(None)


def flash_bytes():
    data = bytearray(b"\xff" * (1024 * 1024))
    entries = b"".join(struct.pack("<HBBII16sI", 0x50AA, kind, subtype, start, size, name, 0)
                       for kind, subtype, start, size, name in [
                           (1, 2, 0x9000, 0x6000, b"nvs"),
                           (0, 0, 0x10000, 0x70000, b"app"),
                           (1, 0x82, 0x80000, 0x80000, b"filesystem")])
    entries += b"\xeb\xeb" + b"\xff" * 14 + hashlib.md5(entries).digest()
    data[0x8000:0x8000 + len(entries)] = entries
    data[0x10000] = 0xE9
    return data


class FakeEsp:
    CHIP_NAME = "ESP32-S3"
    IMAGE_CHIP_ID = 9
    secure_download_mode = False

    def __init__(self):
        self.data = flash_bytes()
        self.secure = False
        self.node = b"\x00\x00\xa1\xb2\xc3\xd4"
        self._port = Mock()
        self.hard_reset = Mock()

    def get_secure_boot_enabled(self):
        return self.secure

    def get_flash_encryption_enabled(self):
        return False

    def read_mac(self):
        return self.node

    def run_stub(self):
        return self

    def flash_md5sum(self, address, size):
        return hashlib.md5(self.data[address:address + size]).hexdigest()


class FlashGuards(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.image = Path(self.temp.name) / "firmware.bin"
        self.image.write_bytes(image_bytes())
        self.backup = Path(self.temp.name) / "original-flash.bin"
        self.esp = FakeEsp()
        self.detect = self.mock("detect_chip", return_value=self.esp)
        self.mock("detect_flash_size", return_value="1MB")
        self.read = self.mock("read_flash", side_effect=lambda *_: bytes(self.esp.data))
        self.write = self.mock("write_flash", side_effect=self.write_flash)
        self.verify = self.mock("verify_flash", side_effect=self.verify_flash)

    def mock(self, name, **kwargs):
        patcher = patch.object(updater.esptool, name, **kwargs)
        self.addCleanup(patcher.stop)
        return patcher.start()

    def write_flash(self, esp, entries, **kwargs):
        self.assertTrue(self.backup.is_file(), "Writing started before backup")
        self.assertEqual(self.backup.stat().st_size, len(self.esp.data))
        self.assertEqual(kwargs["force"], False)
        self.assertEqual(kwargs["erase_all"], False)
        self.assertEqual([address for address, _ in entries], [0x10000])
        for address, data in entries:
            esp.data[address:address + len(data)] = data

    def verify_flash(self, esp, entries, **_):
        for address, data in entries:
            if esp.data[address:address + len(data)] != data:
                raise ValueError("Readback mismatch")

    def run_update(self):
        updater.update("FAKE", "esp32s3", 0xA1B2C3D4, self.image, self.backup)

    def blocked(self):
        with self.assertRaises(Exception):
            self.run_update()
        self.write.assert_not_called()
        self.verify.assert_not_called()

    def test_success_requires_backup_and_verification_preserves_other_regions(self):
        before = bytes(self.esp.data)
        self.run_update()
        self.assertEqual(self.backup.read_bytes(), before)
        self.assertEqual(self.esp.data[:0x10000], before[:0x10000])
        self.assertEqual(self.esp.data[0x80000:], before[0x80000:])
        self.write.assert_called_once()
        self.verify.assert_called_once()
        self.esp.hard_reset.assert_called_once()
        self.esp._port.close.assert_called_once()

    def test_corrupt_digest_is_blocked_before_connecting(self):
        data = bytearray(self.image.read_bytes())
        data[-1] ^= 0xFF
        self.image.write_bytes(data)
        self.blocked()
        self.detect.assert_not_called()

    def test_wrong_chip_is_blocked(self):
        self.esp.CHIP_NAME = "ESP32-C3"
        self.blocked()

    def test_different_physical_node_is_blocked(self):
        self.esp.node = b"\x00" * 6
        self.blocked()

    def test_secure_boot_is_blocked(self):
        self.esp.secure = True
        self.blocked()

    def test_incomplete_backup_is_blocked(self):
        self.read.side_effect = lambda *_: b"incomplete"
        self.blocked()

    def test_corrupt_partition_table_is_blocked(self):
        self.esp.data[0x8000 + 4] ^= 1
        self.blocked()

    def test_backup_disk_failure_is_blocked(self):
        with patch.object(updater, "write_private", side_effect=OSError("disk full")):
            self.blocked()

    def test_readback_failure_does_not_report_success_or_reboot_partial_app(self):
        self.verify.side_effect = ValueError("Readback mismatch")
        with self.assertRaisesRegex(ValueError, "Readback mismatch"):
            self.run_update()
        self.assertTrue(self.backup.is_file())
        self.esp.hard_reset.assert_not_called()
        self.esp._port.close.assert_called_once()

    def test_oversized_image_cannot_cross_into_settings_partition(self):
        with self.assertRaisesRegex(ValueError, "too large"):
            updater.application_partition(self.esp.data, 0x70001)


if __name__ == "__main__":
    unittest.main()
