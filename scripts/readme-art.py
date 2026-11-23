#!/usr/bin/env python3
"""Writes the animated banner at the top of the README, in a dark and a
light version: docs/assets/banner-dark.svg and banner-light.svg.

GitHub shows an SVG as an image, which loads no web fonts, so the three
fonts are embedded (they are OFL-licensed, from node_modules/@fontsource).
The animation is CSS inside the SVG and stops for readers who ask for
reduced motion. Run after `pnpm install`:  python3 scripts/readme-art.py
"""

import base64
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FONTS = ROOT / "node_modules/@fontsource"
OUT = ROOT / "docs/assets"

THEMES = {
    "dark": dict(bg="#151718", panel="#1b1d1f", card="#202325", line="#2c2f32", text="#e3e4e6", text2="#b9babd",
                 text3="#939598", accent="#94bce3", accent2="#749dc4", kw="#94bce3", str="#a8d5b2", num="#e8c48c",
                 warn="#e0b46a", grid="#ffffff", grid_alpha=".035"),
    "light": dict(bg="#f2f2f3", panel="#e9e9ea", card="#f5f5f8", line="#d6d6d9", text="#1d1f20", text2="#424244",
                  text3="#66676a", accent="#416180", accent2="#5980a6", kw="#416180", str="#3d7a4f", num="#9a6418",
                  warn="#a8741e", grid="#1d1f20", grid_alpha=".05"),
}

ENGINES = ["PostgreSQL", "MySQL", "SQLite", "SQL Server", "ClickHouse", "DuckDB", "Redis", "MongoDB",
           "Qdrant", "Neo4j", "MariaDB", "CockroachDB", "Valkey"]


def font(name: str, file: str) -> str:
    data = base64.b64encode((FONTS / name / "files" / file).read_bytes()).decode()
    return f"url(data:font/woff2;base64,{data}) format('woff2')"


def chips(c: dict, x0: float) -> tuple[str, float]:
    """One run of engine chips starting at x0; returns the SVG and its width."""
    out, x = [], x0
    for name in ENGINES:
        width = 22 + len(name) * 9.4
        out.append(
            f'<g transform="translate({x:.1f} 0)"><rect width="{width:.1f}" height="34" fill="{c["card"]}" stroke="{c["line"]}"/>'
            f'<rect x="10" y="14" width="6" height="6" fill="{c["accent2"]}"/>'
            f'<text x="24" y="22.5" class="chip">{name}</text></g>'
        )
        x += width + 12
    return "".join(out), x - x0


def banner(c: dict) -> str:
    run, run_width = chips(c, 0)
    code1 = ('<tspan class="kw">SELECT</tspan> city, <tspan class="kw">count</tspan>(*) <tspan class="kw">AS</tspan> trips')
    code2 = ('<tspan class="kw">FROM</tspan> rides <tspan class="kw">GROUP BY</tspan> city <tspan class="kw">ORDER BY</tspan> 2 <tspan class="kw">DESC</tspan>;')
    rows = [("İstanbul", "48 213"), ("Ankara", "31 977"), ("İzmir", "22 508")]
    row_svg = "".join(
        f'<g class="row r{i}" transform="translate(0 {i * 30})">'
        f'<rect width="424" height="30" fill="{c["card"] if i % 2 else c["panel"]}"/>'
        f'<text x="14" y="20" class="mono">{city}</text><text x="410" y="20" class="mono num" text-anchor="end">{n}</text></g>'
        for i, (city, n) in enumerate(rows)
    )
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1280 440" width="1280" height="440" role="img" aria-label="LewIDE — the open-source desktop client for every database">
<title>LewIDE — the open-source desktop client for every database</title>
<style>
@font-face {{ font-family: 'Barlow Condensed'; font-weight: 600; src: {font("barlow-condensed", "barlow-condensed-latin-600-normal.woff2")}; }}
@font-face {{ font-family: 'Barlow'; font-weight: 400; src: {font("barlow", "barlow-latin-400-normal.woff2")}; }}
@font-face {{ font-family: 'IBM Plex Mono'; font-weight: 400; src: {font("ibm-plex-mono", "ibm-plex-mono-latin-400-normal.woff2")}; }}
.word {{ font: 600 104px 'Barlow Condensed', 'Arial Narrow', sans-serif; letter-spacing: .5px; fill: {c["text"]}; }}
.tag {{ font: 400 26px 'Barlow', 'Helvetica Neue', Arial, sans-serif; fill: {c["text"]}; }}
.sub {{ font: 400 19px 'Barlow', 'Helvetica Neue', Arial, sans-serif; fill: {c["text3"]}; }}
.mono {{ font: 400 15px 'IBM Plex Mono', Menlo, Consolas, monospace; fill: {c["text"]}; }}
.small {{ font: 400 12.5px 'IBM Plex Mono', Menlo, Consolas, monospace; fill: {c["text3"]}; }}
.chip {{ font: 600 17px 'Barlow Condensed', 'Arial Narrow', sans-serif; letter-spacing: .4px; fill: {c["text2"]}; }}
.kw {{ fill: {c["kw"]}; }}
.num {{ fill: {c["num"]}; }}
.cover {{ fill: {c["card"]}; }}
.caret {{ fill: {c["accent"]}; }}
.draw {{ stroke-dasharray: 60; stroke-dashoffset: 60; animation: draw 1.1s .2s cubic-bezier(.6,0,.2,1) forwards; }}
.drop {{ opacity: 0; transform: translateY(-14px); transform-box: fill-box; animation: drop .5s 1.1s cubic-bezier(.3,1.5,.5,1) forwards; }}
.fade {{ opacity: 0; animation: fade .8s forwards; }}
.f1 {{ animation-delay: .5s; }} .f2 {{ animation-delay: .9s; }} .f3 {{ animation-delay: 1.2s; }} .f4 {{ animation-delay: 1.5s; }}
.type1 {{ animation: type1 9s steps(31) infinite; }}
.type2 {{ animation: type2 9s steps(41) infinite; }}
.row {{ opacity: 0; animation: rows 9s infinite; }}
.r1 {{ animation-name: rows1; }} .r2 {{ animation-name: rows2; }}
.blink {{ animation: blink 1s steps(1) infinite; }}
.c1 {{ animation: c1 9s steps(1) infinite; }}
.c2 {{ animation: c2 9s steps(1) infinite; }}
.status {{ opacity: 0; animation: status 9s infinite; }}
.marquee {{ animation: marquee 38s linear infinite; }}
@keyframes draw {{ to {{ stroke-dashoffset: 0; }} }}
@keyframes drop {{ to {{ opacity: 1; transform: none; }} }}
@keyframes fade {{ to {{ opacity: 1; }} }}
@keyframes type1 {{ 0%, 8% {{ transform: translateX(0); }} 30%, 100% {{ transform: translateX(300px); }} }}
@keyframes type2 {{ 0%, 30% {{ transform: translateX(0); }} 52%, 100% {{ transform: translateX(400px); }} }}
@keyframes rows {{ 0%, 56% {{ opacity: 0; }} 60%, 92% {{ opacity: 1; }} 98%, 100% {{ opacity: 0; }} }}
@keyframes rows1 {{ 0%, 60% {{ opacity: 0; }} 64%, 92% {{ opacity: 1; }} 98%, 100% {{ opacity: 0; }} }}
@keyframes rows2 {{ 0%, 64% {{ opacity: 0; }} 68%, 92% {{ opacity: 1; }} 98%, 100% {{ opacity: 0; }} }}
@keyframes status {{ 0%, 68% {{ opacity: 0; }} 72%, 92% {{ opacity: 1; }} 98%, 100% {{ opacity: 0; }} }}
@keyframes blink {{ 50% {{ opacity: 0; }} }}
@keyframes c1 {{ 0% {{ opacity: 1; }} 30% {{ opacity: 0; }} }}
@keyframes c2 {{ 0% {{ opacity: 0; }} 30% {{ opacity: 1; }} }}
@keyframes marquee {{ to {{ transform: translateX(-{run_width + 12:.1f}px); }} }}
@media (prefers-reduced-motion: reduce) {{
  /* The final picture, still. Only animated elements are touched: the
     others are placed with transform attributes, which must stay. */
  .draw, .drop, .fade, .row, .status, .marquee, .type1, .type2, .blink, .c1, .c2 {{ animation: none !important; }}
  .drop, .fade, .row, .status {{ opacity: 1 !important; }}
  .drop {{ transform: none !important; }}
  .draw {{ stroke-dashoffset: 0 !important; }}
  .cover, .caret {{ display: none; }}
}}
</style>
<defs>
<pattern id="grid" width="32" height="32" patternUnits="userSpaceOnUse"><path d="M32 0H0V32" fill="none" stroke="{c["grid"]}" stroke-opacity="{c["grid_alpha"]}"/></pattern>
<clipPath id="strip"><rect x="0" y="372" width="1280" height="44"/></clipPath>
<clipPath id="editor"><rect x="718" y="96" width="452" height="82"/></clipPath>
</defs>
<rect width="1280" height="440" fill="{c["bg"]}"/>
<rect width="1280" height="440" fill="url(#grid)"/>
<g stroke="{c["accent2"]}" stroke-width="2" fill="none">
<path d="M24 40V24H40"/><path d="M1240 24H1256V40"/><path d="M24 418V432H38"/><path d="M1242 432H1256V418"/>
</g>

<g transform="translate(84 92) scale(3.4)" fill="none">
<path class="draw" d="M8 2v27M5 26h25" stroke="{c["text"]}" stroke-width="3.5"/>
<rect class="drop" x="14" y="12" width="8" height="8" fill="{c["accent2"]}"/>
</g>
<text class="word fade f1" x="208" y="190">Lew<tspan fill="{c["accent"]}">IDE</tspan></text>
<text class="tag fade f2" x="86" y="252">The open-source desktop client for every database.</text>
<text class="sub fade f3" x="86" y="288">SQL, documents, key-value, vectors and graphs in one interface,</text>
<text class="sub fade f3" x="86" y="314">with an AI assistant that reads your schema, never your rows.</text>
<text class="small fade f4" x="86" y="350">macOS · Windows · Linux   —   MIT / Apache-2.0</text>

<g class="fade f2">
<rect x="702" y="58" width="482" height="296" fill="{c["card"]}" stroke="{c["line"]}"/>
<rect x="702" y="58" width="482" height="32" fill="{c["panel"]}"/>
<circle cx="720" cy="74" r="5" fill="{c["line"]}"/><circle cx="737" cy="74" r="5" fill="{c["line"]}"/><circle cx="754" cy="74" r="5" fill="{c["line"]}"/>
<rect x="776" y="62" width="150" height="28" fill="{c["card"]}"/>
<rect x="776" y="88" width="150" height="2" fill="{c["accent2"]}"/>
<text x="788" y="81" class="small">rides.sql</text>
<rect x="1088" y="66" width="86" height="18" fill="none" stroke="{c["warn"]}"/>
<text x="1131" y="79" class="small" text-anchor="middle" fill="{c["warn"]}" style="fill:{c["warn"]}">PRODUCTION</text>
<text x="716" y="124" class="small">1</text><text x="716" y="154" class="small">2</text>
<g clip-path="url(#editor)">
<text x="744" y="124" class="mono">{code1}</text>
<g class="type1"><rect class="cover" x="740" y="106" width="430" height="26"/><g class="c1"><rect class="caret blink" x="740" y="108" width="2" height="22"/></g></g>
<text x="744" y="154" class="mono">{code2}</text>
<g class="type2"><rect class="cover" x="740" y="136" width="440" height="28"/><g class="c2"><rect class="caret blink" x="740" y="138" width="2" height="22"/></g></g>
</g>
<rect x="702" y="188" width="482" height="1" fill="{c["line"]}"/>
<text x="716" y="212" class="small">city</text><text x="1170" y="212" class="small" text-anchor="end">trips</text>
<g transform="translate(730 222)">{row_svg}</g>
<g class="status"><rect x="716" y="320" width="8" height="8" fill="{c["accent2"]}"/><text x="732" y="328" class="small">3 rows · 12 ms · ClickHouse</text></g>
</g>

<g clip-path="url(#strip)"><g transform="translate(0 378)"><g class="marquee">{run}<g transform="translate({run_width + 12:.1f} 0)">{chips(c, 0)[0]}</g></g></g></g>
</svg>
"""


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, colours in THEMES.items():
        path = OUT / f"banner-{name}.svg"
        path.write_text(banner(colours), encoding="utf-8")
        print(path.relative_to(ROOT), path.stat().st_size // 1024, "KB")


if __name__ == "__main__":
    main()
