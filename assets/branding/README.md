# Astrolabe icon

`astrolabe.svg` is the editable source for the open-sweep mark. Reuse it for the page header, browser icon, docs and future project surfaces. Keep the ring, arm, pivot cutout and crescent together; do not stretch the mark or add effects.

The standalone file uses two-color Iris and follows the viewing system's light or dark appearance. In the page, `--icon-accent` follows `--accent` and `--icon-detail` follows `--ink`. The browser icon keeps the same two colors; they update when the palette or appearance changes. Both the served pages and standalone home file embed the vector, so they need no image files beside them.

For artwork outside the site, use Iris: accent `#6b4fc2` with detail `#26232e` on light surfaces, or accent `#a98bff` with detail `#e9e6f1` on dark surfaces. Set both icon variables to the same color for single-color use. The pivot stays transparent. Keep the SVG as the source when exporting raster artwork at the size a destination requires.

Future plugin listings and app shortcuts should reuse this mark when the host supports an icon; do not invent metadata fields. The current plugin manifests do not declare icons.
