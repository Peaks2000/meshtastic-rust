#!/usr/bin/env python3
"""Fetch exact-board application images from official Meshtastic releases."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import ssl
import sys
import tempfile
import urllib.request
import zipfile

API = 'https://api.github.com/repos/meshtastic/firmware/releases'
HARDWARE = 'https://raw.githubusercontent.com/meshtastic/web-flasher/main/public/data/hardware-list.json'
MAX_ARCHIVE = 512 * 1024 * 1024
MAX_IMAGE = 16 * 1024 * 1024


def request(url):
    context = ssl.create_default_context()
    for cert in ('/etc/ssl/certs/ca-certificates.crt', '/etc/pki/tls/certs/ca-bundle.crt'):
        if Path(cert).is_file():
            context.load_verify_locations(cert)
            break
    return urllib.request.urlopen(urllib.request.Request(url, headers={
        'User-Agent': 'meshtastic-rust', 'Accept': 'application/vnd.github+json'
    }), timeout=45, context=context)


def json_url(url):
    with request(url) as response:
        data = response.read(8 * 1024 * 1024 + 1)
    if len(data) > 8 * 1024 * 1024:
        raise ValueError('Official metadata is too large')
    return json.loads(data)


def board_info(board, hardware):
    matches = {(x['platformioTarget'], x['architecture']) for x in hardware
               if x.get('hwModelSlug') == board}
    if len(matches) != 1:
        raise ValueError('Cannot identify this exact board; choose a local firmware file')
    target, architecture = matches.pop()
    if not re.fullmatch(r'[a-zA-Z0-9_.-]+', target):
        raise ValueError('Invalid official board target')
    return target, architecture.replace('-', '')


def release_info(release, board, target, architecture):
    if release.get('draft') or release.get('prerelease') or re.search(
            r'revoked|retracted|yanked', release.get('name', ''), re.I):
        raise ValueError('Release is a prerelease or has been withdrawn')
    tag = release['tag_name']
    if not re.fullmatch(r'v?\d+\.\d+\.\d+\.[a-zA-Z0-9]+', tag):
        raise ValueError('Unsupported firmware release version')
    version = tag.removeprefix('v')
    name = f'firmware-{architecture}-{version}.zip'
    assets = [x for x in release['assets'] if x['name'] == name]
    if len(assets) != 1:
        raise ValueError('No firmware archive for this board in this release')
    asset = assets[0]
    digest = asset.get('digest') or ''
    url = asset['browser_download_url']
    expected = f'https://github.com/meshtastic/firmware/releases/download/{tag}/{name}'
    if url != expected or not re.fullmatch(r'sha256:[a-f0-9]{64}', digest):
        raise ValueError('Official archive has no verifiable SHA-256 checksum')
    if not 0 < asset['size'] <= MAX_ARCHIVE:
        raise ValueError('Firmware archive exceeds the download limit')
    return dict(board=board, target=target, architecture=architecture, tag=tag,
                title=release.get('name') or tag, url=url, digest=digest[7:], size=asset['size'])


def discover(board):
    target, architecture = board_info(board, json_url(HARDWARE))
    for release in json_url(API + '?per_page=40'):
        try:
            return release_info(release, board, target, architecture)
        except ValueError:
            continue
    raise ValueError('No current verified release for this board; choose a local firmware file')


def image_from_archive(archive, info):
    version = info['tag'].removeprefix('v')
    stem = f"firmware-{info['target']}-{version}"
    extension = '.bin' if info['architecture'].startswith('esp32') else '.uf2'
    names = [stem + '-update.bin', stem + '.bin'] if extension == '.bin' else [stem + extension]
    with zipfile.ZipFile(archive) as z:
        for name in names:
            matches = [m for m in z.infolist() if Path(m.filename).name == name]
            if len(matches) > 1:
                raise ValueError('Ambiguous firmware archive')
            if matches:
                member = matches[0]
                if not 0 < member.file_size <= MAX_IMAGE:
                    raise ValueError('Firmware image exceeds the size limit')
                with z.open(member) as stream:
                    data = stream.read(MAX_IMAGE + 1)
                if len(data) != member.file_size:
                    raise ValueError('Invalid firmware image length')
                return name, data
    raise ValueError('Exact-board application image is missing from the archive')


def download(board, tag, output):
    target, architecture = board_info(board, json_url(HARDWARE))
    if not re.fullmatch(r'v?\d+\.\d+\.\d+\.[a-zA-Z0-9]+', tag):
        raise ValueError('Invalid release tag')
    # Recheck withdrawal status and digest even when an archive is cached.
    info = release_info(json_url(API + '/tags/' + tag), board, target, architecture)
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True, mode=0o700)
    archive = output / (info['digest'] + '.zip')
    cached = False
    if archive.is_file() and archive.stat().st_size == info['size']:
        with archive.open('rb') as cached_file:
            cached = hashlib.file_digest(cached_file, 'sha256').hexdigest() == info['digest']
    if not cached:
        with tempfile.TemporaryDirectory(dir=output) as temporary:
            pending = Path(temporary) / 'archive.zip'
            digest = hashlib.sha256()
            count = 0
            with pending.open('wb') as destination, request(info['url']) as response:
                while chunk := response.read(1024 * 1024):
                    count += len(chunk)
                    if count > info['size'] or count > MAX_ARCHIVE:
                        raise ValueError('Download exceeds the expected archive size')
                    digest.update(chunk)
                    destination.write(chunk)
                destination.flush()
                os.fsync(destination.fileno())
            if count != info['size'] or digest.hexdigest() != info['digest']:
                raise ValueError('Firmware archive checksum verification failed')
            os.replace(pending, archive)
    name, data = image_from_archive(archive, info)
    path = output / name
    with tempfile.TemporaryDirectory(dir=output) as temporary:
        pending = Path(temporary) / 'image'
        with pending.open('wb') as image:
            image.write(data)
            image.flush()
            os.fsync(image.fileno())
        os.replace(pending, path)
    return dict(**info, path=str(path), image_sha256=hashlib.sha256(data).hexdigest())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--board', required=True)
    parser.add_argument('--tag')
    parser.add_argument('--output')
    args = parser.parse_args()
    result = download(args.board, args.tag, args.output) if args.tag and args.output else discover(args.board)
    print(json.dumps(result))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
