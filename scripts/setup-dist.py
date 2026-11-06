#!/usr/bin/env python3
"""Fetch the pinned release tool into a disposable folder, never a user's install."""
import hashlib
import os
from pathlib import Path
import platform
import sys
import tarfile
import urllib.request
import zipfile

VERSION = '0.34.0'
ASSETS = {
    ('Darwin', 'arm64'): ('aarch64-apple-darwin', 'tar.xz', 'd1eec30e63eed23e5a5bd48fe96eae314799811bffca254ec1d2264638ba1890'),
    ('Darwin', 'x86_64'): ('x86_64-apple-darwin', 'tar.xz', '6aa89bb1e7c18f51ae8b148ce2fc6ab00ac46ed8173d4f2ed9afa12101218773'),
    ('Windows', 'AMD64'): ('x86_64-pc-windows-msvc', 'zip', '7bb53fbc998321f77a3bbb044084a52747c8ed041cf6d6a604802aa57b9c80c6'),
    ('Linux', 'x86_64'): ('x86_64-unknown-linux-gnu', 'tar.xz', '7fee44ff3ff71ed0064b32d4ec62fee6b58bd37c5887b12a86aedddc5af0c7b5'),
}


def setup(destination):
    target, extension, expected = ASSETS[(platform.system(), platform.machine())]
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / f'cargo-dist-{target}.{extension}'
    url = f'https://github.com/axodotdev/cargo-dist/releases/download/v{VERSION}/{archive.name}'
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError('cargo-dist download checksum mismatch')
    archive.write_bytes(data)
    if extension == 'zip':
        with zipfile.ZipFile(archive) as package:
            package.extractall(destination)
    else:
        with tarfile.open(archive) as package:
            package.extractall(destination, filter='data')
    executable = 'dist.exe' if os.name == 'nt' else 'dist'
    binary = next(destination.rglob(executable))
    print(binary)
    if os.environ.get('GITHUB_PATH'):
        with open(os.environ['GITHUB_PATH'], 'a') as path_file:
            path_file.write(str(binary.parent) + '\n')


if __name__ == '__main__':
    setup(Path(sys.argv[1]).resolve())
