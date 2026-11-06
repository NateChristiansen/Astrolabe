import importlib.util
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

    def test_missing_target_cannot_be_packaged_as_a_complete_release(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileNotFoundError):
                package_release.package(Path(directory), package_release.TARGETS)

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
