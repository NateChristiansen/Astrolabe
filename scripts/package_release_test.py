import importlib.util
import os
import subprocess
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('package_release', Path(__file__).with_name('package-release.py'))
package_release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package_release)


class ReleaseTests(unittest.TestCase):
    def test_changed_generator_is_refused_instead_of_silently_skipping_verification(self):
        with self.assertRaises(ValueError):
            package_release.shell_installer('changed installer', {})
        with self.assertRaises(ValueError):
            package_release.powershell_installer('changed installer', {})

    @unittest.skipIf(os.name == 'nt', 'POSIX shell staging wrapper')
    def test_staging_suppresses_only_the_temporary_path_warning(self):
        # The relevant cargo-dist hooks plus a staged stand-in. The real receipt
        # refusal must still reach the caller and keep its failing exit status.
        generated = r'''say() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
verify() {
        if [ -n "${_checksum_style:-}" ]; then
            verify_checksum "$_file" "$_checksum_style" "$_checksum_value"
        else
            say "no checksums to verify" 1>&2
        fi
}
check_for_shadowed_bins() { printf ' astrolabe'; }
finish() {
    say "everything's installed!"
    _shadowed_bins="$(check_for_shadowed_bins "$_install_dir" "$_bins")"
    if [ -n "$_shadowed_bins" ]; then
        warn "The following commands are shadowed by other commands in your PATH:$_shadowed_bins"
    fi
}
_install_dir=$ASTROLABE_UNMANAGED_INSTALL
_bins=astrolabe
cat > "$_install_dir/astrolabe" <<'STAGED'
#!/usr/bin/env sh
printf '%s\n' "$@"
printf 'real installation diagnostic\n' >&2
exit "$FIXTURE_INSTALL_EXIT"
STAGED
chmod +x "$_install_dir/astrolabe"
finish
'''
        wrapped = package_release.shell_installer(generated, {})
        with tempfile.TemporaryDirectory() as directory:
            script = Path(directory) / 'install.sh'
            script.write_text(wrapped)
            for exit_code in [0, 17]:
                result = subprocess.run(['sh', str(script), '--dir', 'a folder with spaces'],
                                        env=dict(os.environ, FIXTURE_INSTALL_EXIT=str(exit_code)),
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, exit_code, result.stdout + result.stderr)
                self.assertIn('Download verified and staged.', result.stdout)
                self.assertIn('install\n--dir\na folder with spaces\n', result.stdout)
                self.assertIn('real installation diagnostic', result.stderr)
                self.assertNotIn('shadowed', result.stdout + result.stderr)
        with self.assertRaises(ValueError):
            package_release.shell_installer(generated.replace('check_for_shadowed_bins "$_install_dir"', 'changed_check "$_install_dir"'), {})

    def test_missing_target_cannot_be_packaged_as_a_complete_release(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileNotFoundError):
                package_release.package(Path(directory), package_release.TARGETS)

    def test_nightly_without_its_commit_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            version = package_release.json.loads((package_release.ROOT / 'plugin/astrolabe/.claude-plugin/plugin.json').read_text())['version']
            for target, suffix in package_release.TARGETS.items():
                (folder / f'astrolabe-{target}{suffix}').write_bytes(b'archive')
            for name in ['astrolabe-installer.sh', 'astrolabe-installer.ps1']:
                (folder / name).write_text(f'/releases/download/v{version}/')
            for commit in [None, 'abc1234']:
                with self.assertRaises(ValueError):
                    package_release.package(folder, package_release.TARGETS, 'nightly', commit)

    def test_duplicate_anchor_is_refused(self):
        with self.assertRaises(ValueError):
            package_release.replace_once('a a', 'a', 'b')

    def test_marketplaces_resolve_the_same_versioned_plugin(self):
        import json
        root = package_release.ROOT
        for location, relative in [('', './plugin/astrolabe'), ('plugin/', './astrolabe')]:
            for manifest in ['.agents/plugins/marketplace.json', '.claude-plugin/marketplace.json']:
                entry = json.loads((root / location / manifest).read_text())['plugins'][0]
                source = entry['source']
                if isinstance(source, dict):
                    source = source['path']
                self.assertEqual(source, relative)
                self.assertTrue((root / location / source / 'hooks/hooks.json').is_file())


if __name__ == '__main__':
    unittest.main()
