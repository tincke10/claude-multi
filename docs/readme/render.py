#!/usr/bin/env python3
"""Render the README header as a terminal session SVG plus linked pills.

Offline and deterministic: stdlib only, no network, no tokens. Writes, next to
this file:
  session.svg      the terminal session (JetBrains Mono embedded)
  pills/<slug>.svg one small pill per README link

The version comes from src-tauri/Cargo.toml. Regenerate with:
  python3 docs/readme/render.py
"""
from __future__ import annotations

import base64
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
CARGO_TOML = REPO / "src-tauri" / "Cargo.toml"
FONT_FILE = HERE / "assets" / "JetBrainsMono-latin-400.woff2"

W = 808
BODY_PAD_X, BODY_PAD_Y = 26, 22
FONT_PX, LINE_H = 13, 23
CHAR_W = FONT_PX * 0.6  # JetBrains Mono advance width
COLS = int((W - 2 * BODY_PAD_X) // CHAR_W)  # characters per line
BAR_H = 38

BG_TOP, BG_BOTTOM, RULE = "#0d1014", "#0a0b0d", "#1f2227"
INK, INK_MUTED, INK_DIM = "#d4d4d2", "#8b8d8a", "#5a5c59"
LIME, BLUE = "#b6f569", "#6ab8ff"
LEVELS = ["#1c2025", "#2a3b17", "#4a6b22", "#7fb03a", "#b6f569"]

TITLE = "~/claude-multi — zsh — 92×24"

# (label, href): href is a README anchor or a repo file. The README pills must match.
PILLS = [
    ("install", "#install"),
    ("preferences", "#preferences"),
    ("real usage", "#real-usage"),
    ("tracking", "#session-tracking"),
    ("orca", "#using-with-orca"),
    ("changelog", "CHANGELOG.md"),
    ("license", "LICENSE"),
]

ABOUT = [
    "one tray app, many Claude Code accounts",
    "each account isolated in its own ~/.claude-<suffix>",
    "real 5h / 7d usage, per-account tracking, Orca integration",
]

# Illustrative only: name, 5h %, 7d %, plan. Not real accounts.
EXAMPLE_ACCOUNTS = [
    ("work", 62, 31, "max"),
    ("personal", 18, 9, "pro"),
    ("client", 91, 74, "max"),
]
CELLS, CELL, GAP = 10, 10, 3


def esc(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


def read_version(cargo_toml: Path = CARGO_TOML) -> str:
    """Version of the [package] section (first `version = "..."` before any other table)."""
    in_package = False
    for line in cargo_toml.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_package = stripped == "[package]"
            continue
        m = re.match(r'version\s*=\s*"([^"]+)"', stripped)
        if in_package and m:
            return m.group(1)
    raise ValueError(f"no [package] version in {cargo_toml}")


def slug(name: str) -> str:
    return "".join(ch if ch.isalnum() or ch in "-_." else "-" for ch in name.lower())


# ---------- SVG building blocks ----------

def _font_css(font_b64: str) -> str:
    return ("@font-face{font-family:'JBM';src:url(data:font/woff2;base64,%s) format('woff2');font-weight:400}"
            ".t{font-family:'JBM',ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:%dpx;font-variant-ligatures:none;fill:%s}"
            % (font_b64, FONT_PX, INK))


def _text(y: float, parts: list[tuple[str, str]], x: float = BODY_PAD_X) -> str:
    spans = "".join(f'<tspan fill="{color}">{esc(text)}</tspan>' for text, color in parts)
    return f'<text class="t" x="{x}" y="{y:.1f}" xml:space="preserve">{spans}</text>'


def _bar(width: int, right_label: str) -> str:
    dots = "".join(f'<circle cx="{16 + 5.5 + i * 17}" cy="{BAR_H / 2}" r="5.5" fill="{c}"/>'
                   for i, c in enumerate(("#ff5f56", "#ffbd2e", "#27c93f")))
    return (f'<rect x="0.5" y="0.5" width="{width - 1}" height="{BAR_H}" fill="#ffffff" fill-opacity="0.02"/>'
            f'<line x1="0" y1="{BAR_H + 0.5}" x2="{width}" y2="{BAR_H + 0.5}" stroke="{RULE}"/>'
            f'{dots}'
            f'<text class="t" x="{16 + 3 * 17 + 8}" y="{BAR_H / 2 + 4}" font-size="11" fill="{INK_DIM}">{esc(TITLE)}</text>'
            f'<text class="t" x="{width - 16}" y="{BAR_H / 2 + 4}" font-size="11" fill="{INK_DIM}" text-anchor="end">{esc(right_label)}</text>')


def _meter(pct: int, x: float, y: float) -> str:
    """Usage meter drawn as rect cells, colored like the contributions grid."""
    filled = round(pct / 100 * CELLS)
    out = []
    for i in range(CELLS):
        fill = LEVELS[min(4, 1 + i * 4 // CELLS)] if i < filled else LEVELS[0]
        out.append(f'<rect x="{x + i * (CELL + GAP):.1f}" y="{y:.1f}" width="{CELL}" height="{CELL}" rx="2" fill="{fill}"/>')
    return "".join(out)


# Column layout (in characters) of the example usage table.
COL_NAME, COL_5H, COL_5H_PCT, COL_7D, COL_7D_PCT, COL_PLAN = 0, 21, 38, 45, 62, 69


def _col_x(col: int) -> float:
    return BODY_PAD_X + col * CHAR_W


def _usage_row(name: str, h5: int, d7: int, plan: str, y: float) -> str:
    top = y - FONT_PX + 2
    return "".join([
        _text(y, [(f"~/.claude-{name}/", BLUE)], _col_x(COL_NAME)),
        _meter(h5, _col_x(COL_5H), top),
        _text(y, [(f"{h5:>3}%", INK)], _col_x(COL_5H_PCT)),
        _meter(d7, _col_x(COL_7D), top),
        _text(y, [(f"{d7:>3}%", INK)], _col_x(COL_7D_PCT)),
        _text(y, [(plan, INK_MUTED)], _col_x(COL_PLAN)),
    ])


# ---------- session ----------

def session_rows(version: str) -> list:
    P, C, M, D, B = LIME, INK, INK_MUTED, INK_DIM, BLUE
    pad = 24  # column where inline comments start
    rows: list = []
    rows.append([("$ ", P), ("cat about.txt", C)])
    rows += [[("# ", M), (line, C)] for line in ABOUT]
    rows.append([])
    rows.append([("$ ", P), ("cms --version", C)])
    rows.append([(f"cms {version}", C)])
    rows.append([])
    for cmd, note in (("cms work --resume", '# launch Claude Code as "work" in this folder'),
                      ("cms --auto", "# pick the account from the project (Orca: set as the Claude command)")):
        rows.append([("$ ", P), (cmd.ljust(pad - 2), C), (note, D)])
    rows.append([])
    rows.append([("$ ", P), ("ls ~/.claude-*", C), ("  # example, 5h and 7d usage", D)])
    for name, h5, d7, plan in EXAMPLE_ACCOUNTS:
        rows.append(("usage", name, h5, d7, plan))
    rows.append([])
    rows.append("cursor")
    return rows


def session_svg(version: str, font_b64: str) -> str:
    P = LIME
    body = []
    y = BAR_H + BODY_PAD_Y + FONT_PX
    for row in session_rows(version):
        if row == "cursor":
            body.append(_text(y, [("$ ", P)]))
            body.append(f'<rect x="{BODY_PAD_X + 2 * CHAR_W:.1f}" y="{y - FONT_PX + 1}" width="8" height="15" fill="{LIME}"/>')
        elif isinstance(row, tuple):
            _, name, h5, d7, plan = row
            body.append(_usage_row(name, h5, d7, plan, y))
        elif row:
            body.append(_text(y, row))
        y += LINE_H
    height = int(y - FONT_PX + BODY_PAD_Y)

    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{height}" viewBox="0 0 {W} {height}">'
            f'<style>{_font_css(font_b64)}</style>'
            f'<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{BG_TOP}"/><stop offset="1" stop-color="{BG_BOTTOM}"/></linearGradient>'
            f'<clipPath id="r"><rect width="{W}" height="{height}" rx="10"/></clipPath></defs>'
            f'<g clip-path="url(#r)"><rect width="{W}" height="{height}" fill="url(#bg)"/>{_bar(W, "v" + version)}{"".join(body)}</g>'
            f'<rect x="0.5" y="0.5" width="{W - 1}" height="{height - 1}" rx="10" fill="none" stroke="{RULE}"/>'
            f'</svg>')


def pill_svg(label: str, font_b64: str) -> str:
    fs, pad, icon, gap, h = 12, 14, 12, 6, 32
    text_w = len(label) * fs * 0.6
    w = int(pad + text_w + gap + icon + pad)
    ix, iy = w - pad - icon, (h - icon) / 2
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">'
            f'<style>{_font_css(font_b64)}</style>'
            f'<rect x="0.5" y="0.5" width="{w - 1}" height="{h - 1}" rx="16" fill="#14171b" stroke="#2c3036"/>'
            f'<text class="t" x="{pad}" y="{h / 2 + fs * 0.35:.1f}" font-size="{fs}" fill="{INK}">{esc(label)}</text>'
            f'<path d="M{ix + 2.5} {iy + 9.5} L{ix + 9.5} {iy + 2.5} M{ix + 5} {iy + 2.5} H{ix + 9.5} V{iy + 7}" fill="none" stroke="{INK_MUTED}" stroke-width="1.6"/>'
            f'</svg>')


def main() -> int:
    font_b64 = base64.b64encode(FONT_FILE.read_bytes()).decode()
    version = read_version()
    (HERE / "session.svg").write_bytes(session_svg(version, font_b64).encode("utf-8"))
    pills = HERE / "pills"
    pills.mkdir(exist_ok=True)
    for old in pills.glob("*.svg"):
        old.unlink()
    for label, _ in PILLS:
        (pills / f"{slug(label)}.svg").write_bytes(pill_svg(label, font_b64).encode("utf-8"))
    print(f"rendered session.svg (v{version}) and {len(PILLS)} pills")
    return 0


if __name__ == "__main__":
    sys.exit(main())
