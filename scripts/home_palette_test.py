"""Contrast guard for the production dashboard's paired palette tokens."""
import re
import unittest
from pathlib import Path

PALETTES = Path(__file__).resolve().parents[1] / 'crates/astrolabe/src/home/palettes.css'
# A palette's light rule or its forced-dark rule, wherever the stylesheet puts whitespace.
RULE = re.compile(r':root\[data-palette="([a-z]+)"\](\[data-theme="dark"\])?\s*\{([^{}]+)\}')
# The dark tokens a system in dark appearance gets when the viewer hasn't forced light.
FOLLOWED = re.compile(
    r'@media\s*\(\s*prefers-color-scheme\s*:\s*dark\s*\)\s*\{\s*'
    r':root\[data-palette="([a-z]+)"\]:not\(\[data-theme="light"\]\)\s*\{([^{}]+)\}\s*\}'
)


def tokens(body):
    return dict(re.findall(r'--([a-z0-9-]+)\s*:\s*([^;]+?)\s*(?:;|$)', body.strip()))


def luminance(color):
    channels = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    linear = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in channels]
    return sum(v * w for v, w in zip(linear, (0.2126, 0.7152, 0.0722)))


def contrast(a, b):
    low, high = sorted((luminance(a), luminance(b)))
    return (high + 0.05) / (low + 0.05)


class HomePaletteTests(unittest.TestCase):
    def test_readable_text_across_all_palettes_and_appearances(self):
        families = set()
        for name, dark, body in RULE.findall(PALETTES.read_text()):
            colors = tokens(body)
            families.add((name, bool(dark)))
            for foreground in ('ink', 'mute', 'accent', 'good', 'warn', 'bad'):
                for surface in ('bg', 'card', 'side', 'sel', 'needs'):
                    with self.subTest(palette=name, dark=bool(dark), foreground=foreground, surface=surface):
                        self.assertGreaterEqual(contrast(colors[foreground], colors[surface]), 4.5)
            self.assertGreaterEqual(contrast(colors['accent-ink'], colors['accent']), 4.5)
        self.assertEqual(len(families), 10, 'Each of five palettes needs light and dark tokens')

    def test_following_the_system_gives_the_same_dark_tokens(self):
        css = PALETTES.read_text()
        forced = {name: tokens(body) for name, dark, body in RULE.findall(css) if dark}
        followed = {name: tokens(body) for name, body in FOLLOWED.findall(css)}
        self.assertEqual(len(followed), 5, 'Each palette needs a dark copy for the system setting')
        self.assertEqual(set(followed), set(forced))
        for name, values in followed.items():
            with self.subTest(palette=name):
                self.assertEqual(values, forced[name])


if __name__ == '__main__':
    unittest.main()
