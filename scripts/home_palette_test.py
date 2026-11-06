"""Contrast guard for the production dashboard's paired palette tokens."""
import re
import unittest
from pathlib import Path


def luminance(color):
    channels = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    linear = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in channels]
    return sum(v * w for v, w in zip(linear, (0.2126, 0.7152, 0.0722)))


def contrast(a, b):
    low, high = sorted((luminance(a), luminance(b)))
    return (high + 0.05) / (low + 0.05)


class HomePaletteTests(unittest.TestCase):
    def test_readable_text_across_all_palettes_and_appearances(self):
        css = (Path(__file__).resolve().parents[1] / 'crates/astrolabe/src/home/palettes.css').read_text()
        families = set()
        for selector, body in re.findall(r'(:root\[data-palette="[a-z]+"\](?:\[data-theme="dark"\])?)\{([^}]+)\}', css):
            colors = dict(re.findall(r'--([a-z-]+):(#[a-f0-9]{6})', body))
            families.add(selector)
            for foreground in ('ink', 'mute', 'accent', 'good', 'warn', 'bad'):
                for surface in ('bg', 'card', 'side', 'sel', 'needs'):
                    with self.subTest(palette=selector, foreground=foreground, surface=surface):
                        self.assertGreaterEqual(contrast(colors[foreground], colors[surface]), 4.5)
            self.assertGreaterEqual(contrast(colors['accent-ink'], colors['accent']), 4.5)
        self.assertEqual(len(families), 10, 'Each of five palettes needs light and dark tokens')


if __name__ == '__main__':
    unittest.main()
