#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyte>=0.8"]
# ///
"""Record the presentation GIF shown at the top of the README.

The real binary runs in demo mode inside a pseudo-terminal; the script types
the keys of each scene, reads the screen back through a terminal emulator
(pyte), draws every frame as a slide (title, the keys just pressed and the
terminal window) with headless Chrome, and joins the frames into a GIF with
ffmpeg. Nothing touches rclone or your accounts, and HOME/XDG point at a
temporary folder, so your config, transfer history and caches are left alone.

    cargo build --release
    uv run scripts/presentation.py            # writes book/src/images/demo.gif
    uv run scripts/presentation.py --frames   # also keep the PNG frames

Needs: uv (installs pyte), google-chrome or chromium, ffmpeg, gifsicle
(optional, shrinks the GIF), a monospace font with box-drawing and braille
glyphs (JetBrains Mono / DejaVu Sans Mono).
"""

from __future__ import annotations

import concurrent.futures
import fcntl
import hashlib
import html
import os
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from dataclasses import dataclass, field
from pathlib import Path

import pyte

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "ratclone"
OUT = ROOT / "book" / "src" / "images" / "demo.gif"
WORDMARK = ROOT / "book" / "src" / "images" / "logo-wordmark.svg"
COLS, ROWS = 160, 42
CANVAS = (1280, 800)
THEME = "tokyo-night"

MONO = "'JetBrainsMono Nerd Font Mono', 'JetBrains Mono', 'DejaVu Sans Mono', monospace"
SANS = "'Inter', 'Noto Sans', 'DejaVu Sans', system-ui, sans-serif"

# ── the script ────────────────────────────────────────────────────────────────

KEYS = {
    "Enter": b"\r",
    "Esc": b"\x1b",
    "Space": b" ",
    "Tab": b"\t",
    "BS": b"\x7f",
    "C-p": b"\x10",
    "C-n": b"\x0e",
    "S-Tab": b"\x1b[Z",
    "Down": b"\x1b[B",
    "Up": b"\x1b[A",
    "Right": b"\x1b[C",
    "Left": b"\x1b[D",
    "F5": b"\x1b[15~",
}
# How a key is drawn on its keycap.
CAP = {"Enter": "Enter", "Esc": "Esc", "Space": "Space", "Tab": "Tab", "BS": "⌫", "C-p": "Ctrl-p",
       "C-n": "Ctrl-n", "S-Tab": "Shift-Tab", "Down": "↓", "Up": "↑", "Right": "→", "Left": "←", "F5": "F5"}


@dataclass
class Step:
    keys: str = ""  # "<Enter>", "j", "<Space>sr": tokens in <>, other characters typed one by one
    hold: float = 1.0  # seconds this frame stays on screen
    caps: list[str] | None = None  # keycaps to show; default: derived from keys
    wait: float = 0.35  # real time to let the app react before reading the screen
    record: bool = True  # False: send the keys but keep no frame


@dataclass
class Scene:
    tag: str
    title: str
    subtitle: str  # may contain <kbd>…</kbd>
    steps: list[Step] = field(default_factory=list)


def key(keys: str, hold: float = 1.0, **kw) -> Step:
    return Step(keys=keys, hold=hold, **kw)


def typed(text: str, hold: float = 0.9, per: float = 0.09) -> list[Step]:
    """Type `text` one character per frame; the last frame is held."""
    steps = [Step(keys=ch, hold=per, caps=[], wait=0.12) for ch in text]
    steps[-1].hold = hold
    return steps


SCENES = [
    Scene(
        "Files",
        "Every account, one keyboard",
        "The local disk and every rclone account, browsed with Vim keys.",
        [
            key("", 1.8, caps=[]),
            key("<Tab>", 0.5),
            key("j", 0.6),
            key("<Enter>", 1.0),
            key("j", 0.35),
            key("j", 0.35),
            key("j", 0.6),
            key("<Enter>", 0.8),
            key("G", 0.6),
            key("<Enter>", 1.4),
        ],
    ),
    Scene(
        "Two panes",
        "Any two places, side by side",
        "<kbd>|</kbd> opens a second pane; <kbd>Ctrl-n</kbd> switches its account without leaving it.",
        [
            key("|", 1.0),
            key("<C-n>", 0.6),
            key("<C-n>", 1.4),
        ],
    ),
    Scene(
        "Copy",
        "Copies that keep going",
        "Mark with <kbd>Space</kbd>, send with <kbd>></kbd>. Every copy is an rclone job: browse on, or even quit.",
        [
            key("<S-Tab>", 0.6),
            key("jj", 0.4),
            key("<Space>", 0.4),
            key("<Space>", 0.4),
            key("<Space>", 0.8),
            key(">", 1.4, wait=0.6),
            key("", 1.6, caps=[], wait=2.0),
        ],
    ),
    Scene(
        "Conflicts",
        "Nothing is overwritten by surprise",
        "When a name is taken, choose: replace, skip or keep both, once or for all of them.",
        [
            key("kkk", 0.3),
            key("<Space>", 0.3),
            key("<Space>", 0.3),
            key("<Space>", 0.5),
            key(">", 2.2),
            key("a", 1.2),
            key("n", 2.0),
        ],
    ),
    Scene(
        "Transfers",
        "Every transfer, and its rclone command",
        "Progress per file, speed, and the exact <kbd>rclone</kbd> command behind each copy.",
        [
            key("T", 2.4),
            key("j", 2.0),
            key("T", 0.6),
        ],
    ),
    Scene(
        "Space",
        "Where did the space go?",
        "Folders ranked by size, the file types inside, and the largest files.",
        [
            key("U", 1.0),
            key("<Enter>", 0.6, wait=2.5),
            key("", 1.6, caps=[]),
            key("j", 0.8),
            key("l", 1.8),
            key("q", 0.6),
        ],
    ),
    Scene(
        "Accounts",
        "Add accounts from the app",
        "Pick a type, fill in the form, and log in, with a browser or over SSH.",
        [
            key("<S-Tab>", 0.4),
            key("a", 1.0),
            *typed("drive", 1.6),
            key("<Enter>", 2.2),
            key("<Esc>", 0.3),
            key("<Esc>", 0.6),
        ],
    ),
    Scene(
        "Themes",
        "Make it yours",
        "Eleven built-in themes, your own themes, and a settings screen (<kbd>,</kbd>) that restyles live.",
        [
            key(",", 1.4),
            key("<Right>", 1.2),
            key("<Right>", 1.2),
            key("<Right>", 1.2),
            key("<Right>", 1.6),
            key("<Esc>", 0.8),
        ],
    ),
]

# ── terminal driving ──────────────────────────────────────────────────────────

Cell = tuple[str, str, str, bool]  # char, fg, bg, bold
Screen = list[list[Cell]]


class Terminal:
    def __init__(self, home: Path):
        (home / "config" / "ratclone").mkdir(parents=True)
        (home / "config" / "ratclone" / "config.toml").write_text(f'theme = "{THEME}"\n')
        (home / "run").mkdir(mode=0o700)
        # crossterm drops every colour when NO_COLOR is set (some agent and CI
        # shells export it): the GIF must never inherit it.
        env = {k: v for k, v in os.environ.items() if k != "NO_COLOR"}
        env.update(
            TERM="xterm-256color",
            COLORTERM="truecolor",
            HOME=str(home),
            XDG_CONFIG_HOME=str(home / "config"),
            XDG_STATE_HOME=str(home / "state"),
            XDG_DATA_HOME=str(home / "data"),
            XDG_CACHE_HOME=str(home / "cache"),
            XDG_RUNTIME_DIR=str(home / "run"),
        )
        args = [str(BIN), "--demo"]
        self.pid, self.fd = pty.fork()
        if self.pid == 0:  # child
            os.execve(args[0], args, env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        self.pump(1.5)

    def pump(self, seconds: float) -> None:
        end = time.monotonic() + seconds
        while (left := end - time.monotonic()) > 0:
            ready, _, _ = select.select([self.fd], [], [], min(left, 0.05))
            if not ready:
                continue
            try:
                data = os.read(self.fd, 1 << 16)
            except OSError:
                return
            self.stream.feed(data)
            # crossterm asks for the cursor position at start-up.
            if b"\x1b[6n" in data:
                os.write(self.fd, b"\x1b[1;1R")

    def send(self, token: str) -> None:
        data = KEYS[token[1:-1]] if token.startswith("<") and len(token) > 1 else token.encode()
        os.write(self.fd, data)

    def snapshot(self) -> Screen:
        rows = []
        for y in range(ROWS):
            line = self.screen.buffer[y]
            row = []
            for x in range(COLS):
                c = line[x]
                fg, bg = c.fg, c.bg
                if c.reverse:
                    fg, bg = bg, fg
                row.append((c.data, color(fg), color(bg), bool(c.bold)))
            rows.append(row)
        return rows

    def close(self) -> None:
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except OSError:
            pass


def color(value: str) -> str:
    if value == "default":
        return ""
    if re.fullmatch(r"[0-9a-fA-F]{6}", value):
        return "#" + value.lower()
    return value  # a named ANSI colour; CSS understands most of them


def tokens(keys: str) -> list[str]:
    return re.findall(r"<[^>]+>|.", keys, re.S)


def caps_for(keys: str) -> list[str]:
    out = []
    for t in tokens(keys):
        out.append(CAP.get(t[1:-1], t[1:-1]) if t.startswith("<") and len(t) > 1 else t)
    return out


@dataclass
class Frame:
    scene: int  # -1: title card, -2: closing card
    caps: list[str]
    screen: Screen | None
    hold: float


def record() -> list[Frame]:
    frames: list[Frame] = []
    with tempfile.TemporaryDirectory(prefix="ratclone-presentation-") as home:
        term = Terminal(Path(home))
        try:
            for index, scene in enumerate(SCENES):
                print(f"  recording {scene.tag}…", file=sys.stderr)
                caps: list[str] = []
                for step in scene.steps:
                    for t in tokens(step.keys):
                        term.send(t)
                        term.pump(0.04)
                    term.pump(step.wait)
                    if not step.record:
                        continue
                    if step.caps is not None:
                        caps = step.caps
                    elif step.keys:
                        caps = caps_for(step.keys)
                    frames.append(Frame(index, list(caps), term.snapshot(), step.hold))
        finally:
            term.close()
    return frames


# ── drawing ───────────────────────────────────────────────────────────────────


def glyph(ch: str, wide: bool) -> str:
    if ch == "▄":
        return '<i class="lo"> </i>'
    if ch == "▀":
        return '<i class="hi"> </i>'
    cls = ["w"] if wide else []
    if 0x2500 <= ord(ch) <= 0x257F:
        cls.append("b")
    attr = f' class="{" ".join(cls)}"' if cls else ""
    return f"<i{attr}>{html.escape(ch)}</i>"


def screen_html(screen: Screen) -> tuple[str, str]:
    """Return (rows html, the theme background)."""
    counts: dict[str, int] = {}
    for row in screen:
        for _, _, bg, _ in row:
            if bg:
                counts[bg] = counts.get(bg, 0) + 1
    theme_bg = max(counts, key=counts.get) if counts else "#1a1b26"
    out = []
    for row in screen:
        parts = []
        run_style = None
        run: list[str] = []
        x = 0
        while x < len(row):
            ch, fg, bg, bold = row[x]
            wide = x + 1 < len(row) and row[x + 1][0] == ""
            style = (fg, bg, bold)
            piece = html.escape(ch) if ch and ord(ch[0]) < 128 else glyph(ch or " ", wide)
            if style != run_style and run:
                parts.append(span(run_style, run))
                run = []
            run_style = style
            run.append(piece)
            x += 2 if wide else 1
        if run:
            parts.append(span(run_style, run))
        out.append(f'<div class="r">{"".join(parts)}</div>')
    return "".join(out), theme_bg


def span(style, run: list[str]) -> str:
    fg, bg, bold = style
    css = []
    if fg:
        css.append(f"color:{fg}")
    if bg:
        css.append(f"background:{bg}")
    if bold:
        css.append("font-weight:700")
    return f'<span style="{";".join(css)}">{"".join(run)}</span>'


STYLE = f"""
  * {{ box-sizing: border-box; }}
  html, body {{ margin: 0; width: {CANVAS[0]}px; height: {CANVAS[1]}px; overflow: hidden; }}
  body {{ background: radial-gradient(1200px 700px at 30% 0%, #1f2335 0%, #16161e 60%, #111118 100%);
         font-family: {SANS}; color: #c0caf5; }}
  kbd {{ font-family: {MONO}; font-size: 0.92em; color: #c0caf5; background: #292e42;
         border: 1px solid #3b4261; border-radius: 4px; padding: 0 5px; }}
  .head {{ position: absolute; left: 52px; right: 52px; top: 26px; height: 92px; }}
  .tag {{ font: 600 11px/1 {MONO}; letter-spacing: 0.18em; text-transform: uppercase; color: #7aa2f7; }}
  .tag::before {{ content: ""; display: inline-block; width: 18px; height: 1px; background: #7aa2f7;
                  vertical-align: middle; margin-right: 8px; }}
  .tag b {{ color: #565f89; font-weight: 600; margin-left: 10px; }}
  h1 {{ margin: 12px 0 6px; font-size: 26px; font-weight: 700; color: #e6e9f8; letter-spacing: -0.01em; }}
  p.sub {{ margin: 0; font-size: 14px; color: #8b93b8; }}
  .caps {{ position: absolute; right: 0; top: 18px; display: flex; gap: 8px; }}
  .cap {{ min-width: 34px; height: 34px; padding: 0 10px; display: flex; align-items: center;
          justify-content: center; font: 600 14px/1 {MONO}; color: #e6e9f8; background: #24283b;
          border: 1px solid #3b4261; border-bottom-width: 3px; border-radius: 7px; }}
  .win {{ position: absolute; left: 52px; top: 132px; border-radius: 10px; overflow: hidden;
          box-shadow: 0 22px 60px rgba(0,0,0,.55), 0 0 0 1px rgba(122,162,247,.14); }}
  .bar {{ height: 26px; display: flex; align-items: center; padding: 0 12px; gap: 7px;
          background: #101016; font: 11px/1 {MONO}; color: #565f89; position: relative; }}
  .bar i {{ width: 11px; height: 11px; border-radius: 50%; display: inline-block; }}
  .bar span {{ position: absolute; left: 0; right: 0; text-align: center; }}
  .term {{ padding: 8px 12px 10px; font: 12px/14px {MONO}; font-variant-ligatures: none; color: #c0caf5; }}
  .r {{ height: 14px; white-space: pre; }}
  .r span {{ display: inline-block; height: 14px; vertical-align: top; }}
  .r i {{ display: inline-block; width: 1ch; font-style: normal; text-align: center; overflow: visible; }}
  .r i.w {{ width: 2ch; }}
  .r i.b {{ transform: scaleY(1.2); }}
  .r i.lo {{ background: linear-gradient(transparent 50%, currentColor 50%); height: 14px; vertical-align: top; }}
  .r i.hi {{ background: linear-gradient(currentColor 50%, transparent 50%); height: 14px; vertical-align: top; }}
  .card {{ position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center;
           justify-content: center; text-align: center; }}
  .card .tagline {{ margin-top: 26px; font-size: 22px; color: #a9b1d6; }}
  .card .tagline b {{ color: #e6e9f8; }}
  .card .small {{ margin-top: 12px; font-size: 14px; color: #565f89; }}
  .code {{ margin-top: 34px; text-align: left; background: #13131a; border: 1px solid #292e42;
           border-radius: 10px; padding: 18px 26px; font: 14px/1.75 {MONO}; color: #c0caf5; }}
  .code .c {{ color: #565f89; }}
  .code .p {{ color: #9ece6a; }}
  .links {{ margin-top: 30px; font: 14px/1 {MONO}; color: #7aa2f7; }}
  .links span {{ color: #3b4261; margin: 0 14px; }}
  .install {{ display: grid; grid-template-columns: auto auto; column-gap: 22px; margin-top: 28px;
              font-size: 13px; line-height: 1.85; }}
  .install .os {{ color: #7aa2f7; text-align: right; }}
  .install .head2 {{ grid-column: 1 / -1; }}
  .fine {{ margin-top: 14px; font-size: 12px; color: #565f89; }}
"""


def page(body: str) -> str:
    return f'<!doctype html><meta charset="utf-8"><style>{STYLE}</style>{body}'


def scene_page(frame: Frame) -> str:
    scene = SCENES[frame.scene]
    rows, bg = screen_html(frame.screen)
    caps = "".join(f'<div class="cap">{html.escape(c)}</div>' for c in frame.caps)
    return page(
        f"""
<div class="head">
  <div class="tag">{html.escape(scene.tag)}<b>{frame.scene + 1:02d} / {len(SCENES):02d}</b></div>
  <h1>{html.escape(scene.title)}</h1>
  <p class="sub">{scene.subtitle}</p>
  <div class="caps">{caps}</div>
</div>
<div class="win" style="background:{bg}">
  <div class="bar"><i style="background:#f7768e"></i><i style="background:#e0af68"></i><i style="background:#9ece6a"></i>
    <span>ratclone --demo</span></div>
  <div class="term">{rows}</div>
</div>"""
    )


def wordmark(width: int) -> str:
    svg = WORDMARK.read_text()
    svg = re.sub(r'width="[\d.]+" height="[\d.]+"', f'width="{width}"', svg, count=1)
    return svg


def title_page() -> str:
    return page(
        f"""
<div class="card">
  {wordmark(560)}
  <div class="tagline">Your <b>rclone</b> accounts, in the terminal.</div>
  <div class="small">vim keys · two panes · transfers that outlive the app</div>
</div>"""
    )


INSTALL = [
    ("Arch Linux", "paru -S ratclone-bin"),
    ("Debian, Ubuntu", "sudo apt install ./ratclone_*_amd64.deb"),
    ("Fedora, RHEL", "sudo dnf install ./ratclone-*.x86_64.rpm"),
    ("openSUSE", "sudo zypper install --allow-unsigned-rpm ./ratclone-*.x86_64.rpm"),
    ("Any Linux", "tar xf ratclone-*.tar.gz  # one static binary"),
    ("With Rust", "cargo install --locked ratclone"),
]


def closing_page() -> str:
    rows = "".join(
        f'<div class="os">{html.escape(os_name)}</div>'
        f'<div><span class="p">$</span> {html.escape(cmd)}</div>'
        for os_name, cmd in INSTALL
    )
    return page(
        f"""
<div class="card">
  {wordmark(380)}
  <div class="tagline">A keyboard-driven terminal UI for rclone. Free and open source.</div>
  <div class="code install">
    <div class="c head2"># packages for x86_64 and aarch64 on the GitHub releases page</div>
    {rows}
    <div class="c head2"># then try it on example accounts: nothing touches yours</div>
    <div class="os"></div><div><span class="p">$</span> ratclone --demo</div>
  </div>
  <div class="links">github.com/SyscallBrain/ratclone<span>·</span>syscallbrain.github.io/ratclone</div>
  <div class="fine">MIT licence · Linux · needs rclone v1.68 or newer</div>
</div>"""
    )


def chrome() -> str:
    for name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
        if shutil.which(name):
            return name
    sys.exit("headless Chrome/Chromium not found")


def shoot(browser: str, html_text: str, out: Path) -> None:
    src = out.with_suffix(".html")
    src.write_text(html_text)
    profile = out.with_suffix(".profile")
    subprocess.run(
        [
            browser,
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--force-device-scale-factor=1",
            f"--user-data-dir={profile}",
            f"--window-size={CANVAS[0]},{CANVAS[1]}",
            f"--screenshot={out}",
            src.as_uri(),
        ],
        check=True,
        capture_output=True,
    )
    shutil.rmtree(profile, ignore_errors=True)


# ── assembly ──────────────────────────────────────────────────────────────────


def main() -> None:
    keep = "--frames" in sys.argv
    if not BIN.exists():
        sys.exit(f"{BIN} not found: run `cargo build --release` first")
    for tool in ("ffmpeg",):
        if not shutil.which(tool):
            sys.exit(f"{tool} not found")
    browser = chrome()

    print("recording the demo…", file=sys.stderr)
    frames = record()
    timeline: list[tuple[str, float]] = [("title", 3.0)]
    pages: dict[str, str] = {"title": title_page(), "closing": closing_page()}
    for frame in frames:
        text = scene_page(frame)
        digest = hashlib.sha1(text.encode()).hexdigest()[:16]
        pages.setdefault(digest, text)
        # Merge identical consecutive frames.
        if timeline and timeline[-1][0] == digest:
            timeline[-1] = (digest, timeline[-1][1] + frame.hold)
        else:
            timeline.append((digest, frame.hold))
    timeline.append(("closing", 7.0))

    work = Path(tempfile.mkdtemp(prefix="ratclone-frames-"))
    print(f"drawing {len(pages)} frames in {work}…", file=sys.stderr)
    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
        list(pool.map(lambda kv: shoot(browser, kv[1], work / f"{kv[0]}.png"), pages.items()))

    concat = work / "frames.txt"
    lines = []
    for name, hold in timeline:
        lines.append(f"file '{work / (name + '.png')}'\nduration {hold:.3f}")
    lines.append(f"file '{work / (timeline[-1][0] + '.png')}'")  # ffmpeg ignores the last duration
    concat.write_text("\n".join(lines) + "\n")
    total = sum(h for _, h in timeline)
    print(f"encoding {total:.1f} s of GIF…", file=sys.stderr)

    raw = work / "demo.gif"
    subprocess.run(
        [
            "ffmpeg", "-v", "error", "-y", "-f", "concat", "-safe", "0", "-i", str(concat),
            "-vf", "fps=10,split[a][b];[a]palettegen=max_colors=256:stats_mode=full[p];"
                   "[b][p]paletteuse=dither=none:diff_mode=rectangle",
            "-loop", "0", str(raw),
        ],
        check=True,
    )
    OUT.parent.mkdir(parents=True, exist_ok=True)
    if shutil.which("gifsicle"):
        subprocess.run(["gifsicle", "-O3", "--no-comments", str(raw), "-o", str(OUT)], check=True)
    else:
        shutil.copy(raw, OUT)
    size = OUT.stat().st_size / 1e6
    print(f"{OUT.relative_to(ROOT)}  {total:.1f} s  {size:.1f} MB", file=sys.stderr)
    if keep:
        print(f"frames kept in {work}", file=sys.stderr)
    else:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
