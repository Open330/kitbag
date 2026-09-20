#!/usr/bin/env python3
"""Render every asset to PNG, once per colour scheme.

GitHub strips the `<style>` block out of an SVG it serves to a README, which
would leave every CSS-filled path with no fill at all. So the markdown uses flat
PNGs and the SVGs stay the real assets. `qlmanage` only makes square thumbnails,
so each render is cropped back to its artwork's own aspect afterwards.

    python3 assets/render.py
"""

import pathlib
import re
import subprocess

ASSETS = pathlib.Path(__file__).resolve().parent
SIZE = 1200

# Artwork, and the aspect its viewBox declares.
ARTWORK = {
    "logo": (64, 64),
    "icon": (64, 64),
    "wordmark": (230, 60),
    "banner": (880, 170),
}

# The theme CSS, resolved per scheme.
SCHEMES = {
    "": {  # light
        "ink": "#16181d",
        "part": "#6366f1",
        "tile": "#16181d",
        "letter": "#f4f4f7",
        "tag": "#16181d",
    },
    "-dark": {
        "ink": "#e6edf3",
        "part": "#818cf8",
        "tile": "#e6edf3",
        "letter": "#0e1014",
        "tag": "#e6edf3",
    },
}

TAG_FONT = 'font-family="ui-sans-serif, -apple-system, system-ui, sans-serif" font-size="19"'


def flatten(path: pathlib.Path, colours: dict[str, str]) -> str:
    svg = re.sub(r"<style>.*?</style>", "", path.read_text(), flags=re.S)
    for name, colour in colours.items():
        extra = f' opacity="0.62" {TAG_FONT}' if name == "tag" else ""
        svg = svg.replace(f'class="{name}"', f'fill="{colour}"{extra}')
    return svg


def main() -> None:
    out = pathlib.Path("/tmp/kitbag-render")
    out.mkdir(exist_ok=True)

    for name, (w, h) in ARTWORK.items():
        for suffix, colours in SCHEMES.items():
            svg = flatten(ASSETS / f"{name}.svg", colours)
            # Only the root element's size, never an inner rect's.
            svg = re.sub(
                r'(<svg[^>]*?)width="[\d.]+" height="[\d.]+"',
                lambda m: m.group(1) + f'width="{SIZE}"',
                svg,
                count=1,
            )
            flat = out / f"{name}{suffix}.svg"
            flat.write_text(svg)

            subprocess.run(
                ["qlmanage", "-t", "-s", str(SIZE), "-o", str(out), str(flat)],
                capture_output=True,
                check=False,
            )
            square = out / f"{name}{suffix}.svg.png"
            if not square.exists():
                print(f"could not render {name}{suffix}")
                continue

            height = round(SIZE * h / w)
            subprocess.run(
                ["sips", "-c", str(height), str(SIZE), str(square),
                 "--out", str(ASSETS / f"{name}{suffix}.png")],
                capture_output=True,
                check=False,
            )
            print(f"{name}{suffix}  {SIZE}x{height}")


if __name__ == "__main__":
    main()
