#!/usr/bin/env python3
"""Wrap pinned cargo-dist installers with staging, mandatory hashes and migration.

Run after all target archives are collected and dist build --artifacts=global.
The public installers are install.sh/install.ps1; unwrapped installers are not shipped.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "aarch64-apple-darwin": ".tar.gz",
    "x86_64-apple-darwin": ".tar.gz",
    "x86_64-pc-windows-msvc": ".zip",
    "x86_64-unknown-linux-gnu": ".tar.gz",
}


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError("cargo-dist installer changed; review the wrapper anchor: " + old)
    return text.replace(old, new)


def shell_installer(generated, hashes):
    cases = "\n".join(f'        {name}) expected={digest} ;;' for name, digest in hashes.items())
    verify = '''        local expected actual
        case "$_artifact_name" in
CASES
        *) err "this archive is not part of the reviewed release" ;;
        esac
        if command -v sha256sum >/dev/null 2>&1; then
            actual=$(sha256sum "$_file" | awk '{print $1}')
        elif command -v shasum >/dev/null 2>&1; then
            actual=$(shasum -a 256 "$_file" | awk '{print $1}')
        else
            err "SHA-256 verification requires sha256sum or shasum"
        fi
        [ "$actual" = "$expected" ] || err "archive checksum mismatch; nothing was installed"
'''.replace("CASES", cases)
    anchor = '''        if [ -n "${_checksum_style:-}" ]; then
            verify_checksum "$_file" "$_checksum_style" "$_checksum_value"
        else
            say "no checksums to verify" 1>&2
        fi
'''
    generated = replace_once(generated, anchor, verify)
    generated = replace_once(generated, 'say "everything\'s installed!"', 'say "Download verified and staged."')
    # The generated installer executes only in a subshell and only writes into staging.
    return '''#!/usr/bin/env sh
# Download a verified release, then install it without changing notes or app settings.
set -eu
case "$(uname -s):$(uname -m)" in
  Darwin:arm64|Darwin:x86_64|Linux:x86_64) ;;
  *) echo "This installer supports Apple silicon/Intel macOS and x64 Linux; use PowerShell on Windows." >&2; exit 1 ;;
esac
astrolabe_stage=$(mktemp -d)
trap 'rm -rf "$astrolabe_stage"' EXIT HUP INT TERM
export ASTROLABE_UNMANAGED_INSTALL="$astrolabe_stage"
(
set --
''' + generated + '''
)
"$astrolabe_stage/astrolabe" install "$@"
'''


def powershell_installer(generated, hashes):
    cases = "\n".join(f"    '{name}' = '{digest}'" for name, digest in hashes.items())
    verify = '''  $expectedHashes = @{
CASES
  }
  if (!$expectedHashes.ContainsKey($artifact_name)) {
    throw 'This archive is not part of the reviewed release'
  }
  $actual = (Get-FileHash -LiteralPath $dir_path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $expectedHashes[$artifact_name]) {
    throw 'Archive checksum mismatch; nothing was installed'
  }
'''.replace("CASES", cases)
    anchor = '  Invoke-DownloadFile -client $wc -url $url -path $dir_path\n'
    generated = replace_once(generated, anchor, anchor + verify)
    generated = replace_once(generated, '  $arch = Get-TargetTriple $platforms\n  if (-not $platforms.ContainsKey($arch)) {', '''  $arch = Get-TargetTriple $platforms
  if ($arch -notin @('x86_64-pc-windows-gnu', 'x86_64-pc-windows-msvc')) {
    throw 'This release supports x64 Windows only; ARM emulation has not been validated'
  }
  if (-not $platforms.ContainsKey($arch)) {''')
    generated = replace_once(generated, '  exit 1\n', '  throw\n')
    generated = replace_once(generated, 'Write-Information "everything\'s installed!"', 'Write-Information "Download verified and staged."')
    return '''# Download a verified release, then install it without changing notes or app settings.
param([string]$InstallDir)
$ErrorActionPreference = 'Stop'
$stage = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
$previousStage = $env:ASTROLABE_UNMANAGED_INSTALL
try {
  New-Item -ItemType Directory -Path $stage | Out-Null
  $env:ASTROLABE_UNMANAGED_INSTALL = $stage
  & {
''' + generated + '''
  }
  $installArgs = @('install')
  if ($InstallDir) { $installArgs += @('--dir', $InstallDir) }
  & (Join-Path $stage 'astrolabe.exe') @installArgs
  if ($LASTEXITCODE -ne 0) { throw 'Astrolabe installation failed; see the message above' }
} finally {
  $env:ASTROLABE_UNMANAGED_INSTALL = $previousStage
  if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
'''


def package(folder, targets):
    version = json.loads((ROOT / 'plugin/astrolabe/.claude-plugin/plugin.json').read_text())['version']
    cargo = (ROOT / 'Cargo.toml').read_text()
    if not re.search(r'^version = "' + re.escape(version) + r'"$', cargo, re.M):
        raise ValueError('crate and plugin versions differ')
    hashes = {}
    for target in targets:
        name = f'astrolabe-{target}{TARGETS[target]}'
        hashes[name] = hashlib.sha256((folder / name).read_bytes()).hexdigest()
    shell = (folder / 'astrolabe-installer.sh').read_text()
    powershell = (folder / 'astrolabe-installer.ps1').read_text()
    if f'/releases/download/v{version}' not in shell or f'/releases/download/v{version}' not in powershell:
        raise ValueError('generated installers do not match the release version')
    shell = shell_installer(shell, hashes)
    powershell = powershell_installer(powershell, hashes)
    (folder / 'install.sh').write_text(shell, newline='\n')
    (folder / 'install.ps1').write_text(powershell, newline='\n')
    plugin = ROOT / 'plugin/astrolabe'
    with zipfile.ZipFile(folder / 'astrolabe-plugin.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(plugin.rglob('*')):
            if path.is_file():
                archive.write(path, path.relative_to(plugin).as_posix())
        archive.write(ROOT / 'LICENSE', 'LICENSE')
    names = [*hashes, 'install.sh', 'install.ps1', 'astrolabe-plugin.zip']
    checksums = ''.join(f'{hashlib.sha256((folder / name).read_bytes()).hexdigest()}  {name}\n' for name in names)
    (folder / 'SHA256SUMS').write_text(checksums, newline='\n')
    (folder / 'release-files.json').write_text(json.dumps({'version': version, 'files': [*names, 'SHA256SUMS']}, indent=2) + '\n')
    print(f'Prepared Astrolabe {version}: {len(targets)} targets. No release was published.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dir', type=Path, default=ROOT / 'target/distrib')
    parser.add_argument('--target', action='append', choices=TARGETS, help='Local preview only; release preparation requires every target')
    args = parser.parse_args()
    package(args.dir, args.target or TARGETS)
