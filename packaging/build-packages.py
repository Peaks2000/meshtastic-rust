#!/usr/bin/env python3
"""Create Linux amd64/x86_64 packages from a baseline-built binary and portable Python."""
import argparse
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

PROJECT = Path(__file__).resolve().parents[1]


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def ignore_runtime(directory, names):
    patterns = ['__pycache__', '*.pyc', 'test', 'tests', 'idlelib', 'tkinter', 'turtle*',
                'config-*', 'pkgconfig', 'libtcl*', 'tcl[0-9]*', 'itcl[0-9]*', 'thread[0-9]*',
                '_tkinter*', 'pip', 'pip-*.dist-info', 'ensurepip']
    return [name for name in names if any(fnmatch.fnmatch(name, pattern) for pattern in patterns)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--python-runtime', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=PROJECT / 'dist')
    args = parser.parse_args()
    version = tomllib.loads((PROJECT / 'Cargo.toml').read_text())['package']['version']
    args.output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='packages-', dir=PROJECT / 'target/package-work'))
    root = work / 'root'
    binary = root / 'usr/bin/meshtastic-rust'
    binary.parent.mkdir(parents=True)
    shutil.copy2(args.binary, binary)
    binary.chmod(0o755)
    helpers = root / 'usr/libexec/meshtastic-rust'
    helpers.mkdir(parents=True)
    for name in ('flash-esp.py', 'firmware-download.py'):
        shutil.copy2(PROJECT / 'tools' / name, helpers / name)
    runtime = helpers / 'python'
    (runtime / 'bin').mkdir(parents=True)
    shutil.copy2(args.python_runtime / 'bin/python3.12', runtime / 'bin/python3.12')
    (runtime / 'bin/python3').symlink_to('python3.12')
    shutil.copytree(args.python_runtime / 'lib', runtime / 'lib', symlinks=True, ignore=ignore_runtime)
    for source, destination in [
        ('packaging/meshtastic-rust.desktop', 'usr/share/applications/meshtastic-rust.desktop'),
        ('assets/io.github.meshtasticgtk.Client.svg', 'usr/share/icons/hicolor/scalable/apps/io.github.meshtasticgtk.Client.svg'),
        ('LICENSE', 'usr/share/licenses/meshtastic-rust/LICENSE'),
        ('THIRD_PARTY.md', 'usr/share/doc/meshtastic-rust/THIRD_PARTY.md'),
        ('packaging/python-runtime.json', 'usr/share/doc/meshtastic-rust/python-runtime.json'),
        ('packaging/flasher-requirements.txt', 'usr/share/doc/meshtastic-rust/flasher-requirements.txt'),
    ]:
        target = root / destination
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(PROJECT / source, target)
    copyright_file = root / 'usr/share/doc/meshtastic-rust/copyright'
    copyright_file.write_text((PROJECT / 'LICENSE').read_text() + '\n' + (PROJECT / 'THIRD_PARTY.md').read_text())
    # No build-machine paths or pip entry points are needed at runtime.
    run(binary, '--version')
    run(binary, '--check-flasher')
    control = root / 'DEBIAN'
    control.mkdir()
    size = sum(path.stat().st_size for path in root.rglob('*') if path.is_file() and not path.is_symlink()) // 1024
    (control / 'control').write_text(f'''Package: meshtastic-rust
Version: {version}
Section: net
Priority: optional
Architecture: amd64
Maintainer: Peaks2000 <117590961+Peaks2000@users.noreply.github.com>
Homepage: https://github.com/Peaks2000/meshtastic-rust
Installed-Size: {size}
Depends: libc6 (>= 2.39), libgtk-4-1 (>= 4.12), libglib2.0-0t64 (>= 2.54), libgdk-pixbuf-2.0-0, libdbus-1-3, libgcc-s1, librsvg2-common
Description: Native GTK4 Meshtastic desktop client
 USB, Wi-Fi and Bluetooth connections, messaging, node settings and firmware tools.
 Includes a private Python runtime for the firmware helper.
''')
    for script in ['postinst', 'postrm']:
        path = control / script
        path.write_text('#!/bin/sh\nset -e\nif command -v gtk-update-icon-cache >/dev/null 2>&1; then gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true; fi\nif command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database -q /usr/share/applications || true; fi\nexit 0\n')
        path.chmod(0o755)
    files = sorted(path for path in root.rglob('*') if path.is_file() and control not in path.parents and not path.is_symlink())
    (control / 'md5sums').write_text(''.join(f'{hashlib.md5(path.read_bytes()).hexdigest()}  {path.relative_to(root)}\n' for path in files))
    deb = args.output / f'meshtastic-rust_{version}_amd64.deb'
    run('dpkg-deb', '--build', '--root-owner-group', '-Zxz', root, deb)
    # RPM's generated shell scripts require a build path without spaces.
    rpm_root = Path(tempfile.mkdtemp(prefix='meshtastic-rust-rpm-'))
    spec = work / 'meshtastic-rust.spec'
    # Do not expose private interpreter modules as system-wide Python ABI provides/requires.
    spec.write_text(f'''%global debug_package %{{nil}}
%global _build_id_links none
%global __requires_exclude_from ^/usr/libexec/meshtastic-rust/.*$
%global __provides_exclude_from ^/usr/libexec/meshtastic-rust/.*$
%global __os_install_post %{{nil}}
Name: meshtastic-rust
Version: {version}
Release: 1
Summary: Native GTK4 Meshtastic desktop client
License: GPL-3.0-only AND GPL-2.0-or-later AND Python-2.0
URL: https://github.com/Peaks2000/meshtastic-rust
BuildArch: x86_64
Requires: gtk4 >= 4.12, glib2 >= 2.54, dbus-libs, librsvg2
%description
USB, Wi-Fi and Bluetooth Meshtastic client with messaging, settings and firmware tools.
Includes a private Python runtime for the firmware helper.
%prep
%build
%install
mkdir -p "%{{buildroot}}"
cp -a "{root}/usr" "%{{buildroot}}/usr"
%files
/usr/bin/meshtastic-rust
/usr/libexec/meshtastic-rust
/usr/share/applications/meshtastic-rust.desktop
/usr/share/icons/hicolor/scalable/apps/io.github.meshtasticgtk.Client.svg
%license /usr/share/licenses/meshtastic-rust/LICENSE
%doc /usr/share/doc/meshtastic-rust
''')
    run('rpmbuild', '-bb', '--define', f'_topdir {rpm_root}', spec)
    rpm = args.output / f'meshtastic-rust-{version}-1.x86_64.rpm'
    shutil.copy2(next((rpm_root / 'RPMS/x86_64').glob('*.rpm')), rpm)
    checksums = args.output / 'SHA256SUMS'
    checksums.write_text(''.join(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n' for path in [rpm, deb]))
    (args.output / 'build-info.json').write_text(json.dumps({'version': version, 'staging_root': str(root), 'rpm': str(rpm), 'deb': str(deb)}, indent=2) + '\n')
    print(f'Built {rpm.name} and {deb.name}\nStaging root: {root}')


if __name__ == '__main__':
    main()
