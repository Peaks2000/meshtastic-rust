import copy
import hashlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('firmware_download', Path(__file__).parents[1] / 'tools/firmware-download.py')
f = importlib.util.module_from_spec(spec)
spec.loader.exec_module(f)


class FirmwareTests(unittest.TestCase):
    def setUp(self):
        self.hardware = [dict(hwModelSlug='HELTEC_V3', platformioTarget='heltec-v3', architecture='esp32-s3')]
        self.release = dict(tag_name='v2.7.26.54e0d8d', name='Meshtastic Beta', draft=False, prerelease=False, assets=[dict(
            name='firmware-esp32s3-2.7.26.54e0d8d.zip', size=100, digest='sha256:' + 'a' * 64,
            browser_download_url='https://github.com/meshtastic/firmware/releases/download/v2.7.26.54e0d8d/firmware-esp32s3-2.7.26.54e0d8d.zip')])
        self.info = f.release_info(self.release, 'HELTEC_V3', 'heltec-v3', 'esp32s3')
        self.name = 'firmware-heltec-v3-2.7.26.54e0d8d-update.bin'

    def archive(self, files):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w') as z:
            for name, data in files:
                z.writestr(name, data)
        buffer.seek(0)
        return buffer

    def test_board_exact_match_and_architecture(self):
        self.assertEqual(f.board_info('HELTEC_V3', self.hardware), ('heltec-v3', 'esp32s3'))
        with self.assertRaises(ValueError):
            f.board_info('HELTEC', self.hardware)
        with self.assertRaises(ValueError):
            f.board_info('HELTEC_V3', self.hardware + [dict(self.hardware[0], platformioTarget='other')])

    def test_reject_withdrawn_and_prerelease(self):
        for values in [dict(name='Beta (Revoked)'), dict(prerelease=True), dict(draft=True), dict(name='Retracted release')]:
            with self.subTest(values=values), self.assertRaises(ValueError):
                f.release_info(dict(self.release, **values), 'HELTEC_V3', 'heltec-v3', 'esp32s3')

    def test_reject_unverified_or_wrong_origin(self):
        for values in [dict(digest=None), dict(browser_download_url='https://example.com/image'), dict(size=f.MAX_ARCHIVE + 1)]:
            release = copy.deepcopy(self.release)
            release['assets'][0].update(values)
            with self.subTest(values=values), self.assertRaises(ValueError):
                f.release_info(release, 'HELTEC_V3', 'heltec-v3', 'esp32s3')

    def test_select_update_only_for_exact_board(self):
        archive = self.archive([(self.name, b'update'), (self.name.replace('-update', ''), b'combined'), ('firmware-other-2.7.26.54e0d8d-update.bin', b'wrong')])
        self.assertEqual(f.image_from_archive(archive, self.info), (self.name, b'update'))

    def test_wrong_board_and_variants_rejected(self):
        for name in [self.name.replace('heltec-v3', 'heltec-v3-tft'), 'bootloader.bin']:
            with self.assertRaises(ValueError):
                f.image_from_archive(self.archive([(name, b'wrong')]), self.info)

    def test_ambiguous_member_rejected(self):
        with self.assertRaises(ValueError):
            f.image_from_archive(self.archive([('a/' + self.name, b'a'), ('b/' + self.name, b'b')]), self.info)

    def test_archive_paths_never_extracted(self):
        self.assertEqual(f.image_from_archive(self.archive([('../../' + self.name, b'app')]), self.info), (self.name, b'app'))

    def test_download_checks_digest_and_rechecks_release(self):
        payload = self.archive([(self.name, b'app')]).getvalue()
        self.release['assets'][0].update(size=len(payload), digest='sha256:' + hashlib.sha256(payload).hexdigest())
        with tempfile.TemporaryDirectory() as directory, patch.object(f, 'json_url', side_effect=[self.hardware, self.release]), patch.object(f, 'request', return_value=io.BytesIO(payload)):
            result = f.download('HELTEC_V3', self.release['tag_name'], directory)
            self.assertEqual(Path(result['path']).read_bytes(), b'app')
        self.release['assets'][0]['digest'] = 'sha256:' + '0' * 64
        with tempfile.TemporaryDirectory() as directory, patch.object(f, 'json_url', side_effect=[self.hardware, self.release]), patch.object(f, 'request', return_value=io.BytesIO(payload)):
            with self.assertRaisesRegex(ValueError, 'checksum'):
                f.download('HELTEC_V3', self.release['tag_name'], directory)

    def test_withdrawn_release_blocks_cached_download(self):
        self.release['name'] = 'Firmware (Revoked)'
        with tempfile.TemporaryDirectory() as directory, patch.object(f, 'json_url', side_effect=[self.hardware, self.release]), patch.object(f, 'request') as request:
            with self.assertRaisesRegex(ValueError, 'withdrawn'):
                f.download('HELTEC_V3', self.release['tag_name'], directory)
            request.assert_not_called()

    def test_failed_download_leaves_no_partial_file(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(f, 'json_url', side_effect=[self.hardware, self.release]), patch.object(f, 'request', return_value=io.BytesIO(b'wrong')):
            with self.assertRaises(ValueError):
                f.download('HELTEC_V3', self.release['tag_name'], directory)
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_archive_size_limit(self):
        with patch.object(f, 'MAX_IMAGE', 2), self.assertRaisesRegex(ValueError, 'size limit'):
            f.image_from_archive(self.archive([(self.name, b'large')]), self.info)

    def test_uf2_exact_board(self):
        info = dict(self.info, architecture='nrf52840', target='rak4631')
        name = 'firmware-rak4631-2.7.26.54e0d8d.uf2'
        self.assertEqual(f.image_from_archive(self.archive([(name, b'uf2')]), info), (name, b'uf2'))


if __name__ == '__main__':
    unittest.main()
