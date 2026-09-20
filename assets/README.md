# Assets

The mark is the letter **k** of [JetBrains Mono][jbm] — the face a developer
already has open — with the arm that belongs elsewhere **cut free and marked**.
That is the whole idea of the tool in one letter: some of what is here is not
yours.

[jbm]: https://github.com/JetBrains/JetBrainsMono

| file | what it is |
| --- | --- |
| `logo.svg` | the mark. Follows the reader's light or dark scheme |
| `logo-mono.svg` | one colour, taking `currentColor`, for anywhere that cannot carry two |
| `icon.svg` | the mark reversed out of a tile — favicon, dock, app store |
| `wordmark.svg` | `kitbag`, same letter, same arm, no cut so the name reads |
| `banner.svg` | mark + wordmark + line, for a social preview or a header |
| `*.png`, `*-dark.png` | renders of each, one per scheme |

## Why there are PNGs

GitHub strips the `<style>` block out of an SVG it serves to a README, which
leaves every CSS-filled path with **no fill at all** — an empty rectangle where
a logo should be. So markdown uses the PNGs through `<picture>`, and the SVGs
stay the real assets for anywhere CSS survives.

## Licence

JetBrains Mono is under the [SIL Open Font License][ofl], which permits using
its glyph outlines in a logo. The mark is drawn from the ExtraBold `k`; no font
file is redistributed here, only the outline of one letter.

[ofl]: https://github.com/JetBrains/JetBrainsMono/blob/master/OFL.txt

## Re-rendering

The PNGs come from the SVGs with the theme CSS flattened per scheme, rendered
square by `qlmanage` and cropped back to each artwork's own aspect:

```bash
python3 assets/render.py      # writes every *.png beside its *.svg
```
