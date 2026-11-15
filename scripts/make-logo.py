#!/usr/bin/env python3
"""Generates the ratclone logo files.

The mark is the mouse of book/src/images/logo.svg (the app icon, drawn by
hand): a mouse in front of a cloud, its tail curling into a sync arrow. This
script reuses its drawing without the dark tile and adds the wordmark, drawn
from a 5x7 pixel font as rectangles so the SVGs need no font. Colours come
from the tokyo-night theme.

Writes book/src/images/logo-{mark,wordmark,wordmark-light}.svg and
book/theme/favicon.{svg,png} (the PNG needs rsvg-convert).

    python3 scripts/make-logo.py
"""
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "book" / "src" / "images"
THEME = ROOT / "book" / "theme"
ICON = OUT / "logo.svg"

BG = "#1a1b26"      # bg
FG = "#c0caf5"      # fg
BLUE = "#7aa2f7"    # accent

NAME = "ratclone"
SPLIT = 3           # "rat" in the text colour, "clone" in blue

# 5x7 glyphs, '#' = pixel. Every glyph is 5 wide so the spacing is even.
GLYPHS = {
    "r": [".....", ".....", "#.##.", "##..#", "#....", "#....", "#...."],
    "a": [".....", ".....", ".###.", "....#", ".####", "#...#", ".####"],
    "t": [".#...", ".#...", "####.", ".#...", ".#...", ".#..#", "..##."],
    "c": [".....", ".....", ".###.", "#....", "#....", "#...#", ".###."],
    "l": [".##..", "..#..", "..#..", "..#..", "..#..", "..#..", ".###."],
    "o": [".....", ".....", ".###.", "#...#", "#...#", "#...#", ".###."],
    "n": [".....", ".....", "#.##.", "##..#", "#...#", "#...#", "#...#"],
    "e": [".....", ".....", ".###.", "#...#", "#####", "#....", ".###."],
}

# The part of the icon's 512-unit grid the mouse and its cloud occupy.
MOUSE_BOX = (56, 64, 404, 336)  # x, y, width, height


def mouse_drawing() -> str:
    """The icon's drawing without the svg element and the background tile."""
    svg = ICON.read_text()
    inner = svg[svg.index(">", svg.index("<svg")) + 1 : svg.rindex("</svg>")]
    # The first element is the tile: a rect as large as the icon.
    inner = re.sub(r'\s*<rect width="512" height="512"[^>]*/>', "", inner, count=1)
    return inner.strip("\n")


def mark(x: float, y: float, height: float) -> tuple[str, float]:
    """The mouse scaled to `height`, top-left at (x, y). Returns (svg, width)."""
    bx, by, bw, bh = MOUSE_BOX
    s = height / bh
    body = "\n".join("    " + line.strip() for line in mouse_drawing().splitlines() if line.strip())
    return (
        f'  <g transform="translate({x:.2f},{y:.2f}) scale({s:.5f}) translate({-bx},{-by})">\n{body}\n  </g>',
        bw * s,
    )


def wordmark(x: float, y: float, px: float, text_colour: str) -> tuple[str, float]:
    """'ratclone' as pixel rectangles. Returns (svg, width)."""
    groups: dict[str, list[str]] = {}
    cursor = x
    for idx, ch in enumerate(NAME):
        colour = text_colour if idx < SPLIT else BLUE
        for j, row in enumerate(GLYPHS[ch]):
            for i, cell in enumerate(row):
                if cell == "#":
                    groups.setdefault(colour, []).append(
                        f'<rect x="{cursor + i * px:.2f}" y="{y + j * px:.2f}" '
                        f'width="{px:.2f}" height="{px:.2f}"/>'
                    )
        cursor += 6 * px
    svg = "\n".join(
        f'  <g fill="{c}" shape-rendering="crispEdges">\n    ' + "\n    ".join(rects) + "\n  </g>"
        for c, rects in groups.items()
    )
    return svg, cursor - x - px


def write(path: Path, width: float, height: float, body: str, background: bool) -> None:
    bg = (
        f'  <rect width="{width:.2f}" height="{height:.2f}" rx="{round(height * 0.22, 2)}" fill="{BG}"/>\n'
        if background
        else ""
    )
    path.write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="{height:.0f}" '
        f'viewBox="0 0 {width:.2f} {height:.2f}" role="img" aria-label="{NAME}">\n'
        f"  <title>{NAME}</title>\n{bg}{body}\n</svg>\n"
    )


def lockup(text_colour: str) -> tuple[str, float, float]:
    px, height, mark_height, gap = 9, 120, 92, 26
    icon, icon_width = mark(24, (height - mark_height) / 2, mark_height)
    text, text_width = wordmark(0, 0, px, text_colour)
    tx, ty = 24 + icon_width + gap, (height - 7 * px) / 2
    body = icon + f'\n  <g transform="translate({tx:.2f},{ty:.2f})">\n{text}\n  </g>'
    return body, tx + text_width + 32, height


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    THEME.mkdir(parents=True, exist_ok=True)
    # Mark only, transparent, for small sizes.
    icon, w = mark(0, 0, 64)
    write(OUT / "logo-mark.svg", w, 64, icon, background=False)
    # Wordmark for dark backgrounds (README banner).
    body, w, h = lockup(FG)
    write(OUT / "logo-wordmark.svg", w, h, body, background=True)
    # Same wordmark, transparent with dark text, for light pages.
    body, w, h = lockup(BG)
    write(OUT / "logo-wordmark-light.svg", w, h, body, background=False)
    # Favicon for the book: the app icon itself, so it reads on light and dark tabs.
    shutil.copy(ICON, THEME / "favicon.svg")
    if shutil.which("rsvg-convert"):
        subprocess.run(
            ["rsvg-convert", "-w", "64", "-h", "64", str(THEME / "favicon.svg"),
             "-o", str(THEME / "favicon.png")],
            check=True,
        )


if __name__ == "__main__":
    main()
