#!/usr/bin/env python3
"""Exercise a packaged native release through HTTP in disposable directories."""
import argparse
from functools import partial
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def smoke(folder, target, powershell):
    with tempfile.TemporaryDirectory(prefix='astrolabe-release-') as temporary:
        root = Path(temporary)
        mirror = root / 'mirror'
        shutil.copytree(folder, mirror)
        handler = partial(QuietHandler, directory=str(mirror))
        with http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                env = dict(os.environ, ASTROLABE_DOWNLOAD_URL=f'http://127.0.0.1:{server.server_port}')
                # A build attempt must fail even on developer machines with Rust installed.
                blockers = root / 'no-rust'
                blockers.mkdir()
                for name in ['cargo', 'rustc', 'rustup']:
                    if os.name == 'nt':
                        (blockers / (name + '.cmd')).write_text('@exit /b 99\n')
                    else:
                        stub = blockers / name
                        stub.write_text('#!/bin/sh\nexit 99\n')
                        stub.chmod(0o755)
                env['PATH'] = str(blockers) + os.pathsep + env['PATH']
                install_dir = root / 'install with spaces'
                if os.name == 'nt':
                    command = [powershell, '-NoProfile', '-File', str(mirror / 'install.ps1'), '-InstallDir', str(install_dir)]
                    executable = install_dir / 'astrolabe.exe'
                else:
                    command = ['sh', str(mirror / 'install.sh'), '--dir', str(install_dir)]
                    executable = install_dir / 'astrolabe'
                subprocess.run(command, env=env, check=True)
                before = executable.read_bytes()
                subprocess.run(command, env=env, check=True)
                version = json.loads((mirror / 'release-files.json').read_text())['version']
                # A build of main names itself a nightly and its commit; a release only its version.
                expected = f'astrolabe {version}'
                if os.environ.get('ASTROLABE_CHANNEL') == 'nightly':
                    expected += f" (nightly {os.environ['ASTROLABE_COMMIT'][:7]})"
                shown = subprocess.check_output([executable, '--version'], text=True).strip()
                assert shown == expected, f'--version said {shown!r}, expected {expected!r}'
                notes_env = dict(env, ASTROLABE_LAYER_STATE=str(root / 'state'), ASTROLABE_NO_DETACHED_SYNC='1', GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM='1', GIT_AUTHOR_NAME='Release test', GIT_AUTHOR_EMAIL='test@example.invalid', GIT_COMMITTER_NAME='Release test', GIT_COMMITTER_EMAIL='test@example.invalid')
                subprocess.run([executable, 'notes', 'init', str(root / 'notes')], env=notes_env, check=True)
                assert (root / 'notes/knowledge/owner.md').exists()
                missing_env = dict(env, ASTROLABE_DOWNLOAD_URL=env['ASTROLABE_DOWNLOAD_URL'] + '/missing')
                missing = subprocess.run(command, env=missing_env, capture_output=True, text=True)
                assert missing.returncode != 0, missing.stdout
                assert executable.read_bytes() == before
                if os.name != 'nt':
                    unsupported = blockers / 'uname'
                    unsupported.write_text('#!/bin/sh\nprintf "unsupported\\n"\n')
                    unsupported.chmod(0o755)
                    refusal = subprocess.run(command, env=env, capture_output=True, text=True)
                    assert refusal.returncode != 0, refusal.stdout
                    assert executable.read_bytes() == before
                    unsupported.unlink()
                suffix = '.zip' if os.name == 'nt' else '.tar.gz'
                (mirror / f'astrolabe-{target}{suffix}').write_bytes(b'corrupted download')
                failure = subprocess.run(command, env=env, capture_output=True, text=True)
                assert failure.returncode != 0, failure.stdout
                assert 'checksum mismatch' in (failure.stdout + failure.stderr).lower(), failure.stdout + failure.stderr
                assert executable.read_bytes() == before
                print('Release smoke passed: install, repeat, notes setup without Rust, missing-download and corrupt-download refusal.')
            finally:
                server.shutdown()
                thread.join()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dir', type=Path, default=Path('target/distrib'))
    parser.add_argument('--target', required=True)
    parser.add_argument('--powershell', default='powershell', choices=['powershell', 'pwsh'])
    args = parser.parse_args()
    smoke(args.dir.resolve(), args.target, args.powershell)
