#!/usr/bin/env python3
"""Copies Clip Studio into ClipCut's interface as the Graphics tab (apps/clipcut/ui/graphics).

    python3 packaging/clipcut/sync-graphics.py [path/to/clip-stu]

Clip Studio (https://github.com/portdel18/clip-stu) loads its libraries from jsDelivr; ClipCut allows no
outside scripts, so the copy loads the same pinned versions from `graphics/lib/`, taken from Clip Studio's
own node_modules (run `npm ci` there first). Each library's version must match the one Clip Studio's
index.html pins, or nothing is copied. Data it fetches (map tiles, fonts, us-atlas, pdf.js fonts) still
comes from the internet. `graphics/clip-studio.json` records the commit and the libraries.
"""

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / "apps" / "clipcut" / "ui" / "graphics"

# CDN path in index.html → files to copy from node_modules/<package>
LIBS = {
    "maplibre-gl": ["dist/maplibre-gl.js", "dist/maplibre-gl.css", "LICENSE.txt"],
    "mp4-muxer": ["build/mp4-muxer.js", "LICENSE"],
    "fflate": ["umd/index.js", "LICENSE"],
    # loaded by scroll.js the first time a PDF is opened (CS.lib('pdfjs'))
    "pdfjs-dist": ["build/pdf.min.js", "build/pdf.worker.min.js", "LICENSE"],
    # loaded by cutout.js the first time a background is removed (CS.lib('onnxruntime-web'))
    "onnxruntime-web": ["dist/ort.webgpu.min.js", "dist/ort-wasm-simd-threaded.jsep.mjs", "dist/ort-wasm-simd-threaded.jsep.wasm", "package.json"],
}
# pinned in a script rather than in index.html
SCRIPT_PINS = {
    "pdfjs-dist": ("js/scroll.js", r"pdfjs-dist@([\d.]+)"),
    "onnxruntime-web": ("js/cutout.js", r"onnxruntime-web@([\d.]+)"),
}
# packages that ship no licence file of their own: their licence, written next to them
LICENSES = {
    "onnxruntime-web": (
        "MIT License\n\nCopyright (c) Microsoft Corporation\n\n"
        "Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated "
        "documentation files (the \"Software\"), to deal in the Software without restriction, including without limitation "
        "the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to "
        "permit persons to whom the Software is furnished to do so, subject to the following conditions:\n\n"
        "The above copyright notice and this permission notice shall be included in all copies or substantial portions of "
        "the Software.\n\n"
        "THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO "
        "THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE "
        "AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, "
        "TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE "
        "SOFTWARE.\n"
    ),
}


def fail(msg: str) -> None:
    print(f"sync-graphics: {msg}", file=sys.stderr)
    sys.exit(1)


def main() -> None:
    src = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT.parent / "clip-stu").resolve()
    html_path = src / "index.html"
    if not html_path.exists():
        fail(f"{src} is not a Clip Studio checkout (no index.html)")
    html = html_path.read_text(encoding="utf-8")
    nm = src / "node_modules"

    versions = {}
    for pkg in LIBS:
        if pkg in SCRIPT_PINS:
            file, pat = SCRIPT_PINS[pkg]
            m = re.search(pat, (src / file).read_text(encoding="utf-8"))
        else:
            m = re.search(rf"cdn\.jsdelivr\.net/npm/{re.escape(pkg)}@([\d.]+)/", html)
        if not m:
            fail(f"Clip Studio no longer pins {pkg}: update LIBS in this script")
        pin = m.group(1)
        meta = nm / pkg / "package.json"
        if not meta.exists():
            fail(f"{meta} is missing: run `npm ci` in {src}")
        have = json.loads(meta.read_text(encoding="utf-8"))["version"]
        if have != pin:
            fail(f"{pkg}: Clip Studio pins {pin} but node_modules has {have}: run `npm ci` in {src}")
        versions[pkg] = pin

    # every CDN script and stylesheet in index.html becomes its local copy
    def local(m: re.Match) -> str:
        pkg, path = m.group(1), m.group(3)
        if pkg not in LIBS or not any(f == path for f in LIBS[pkg]):
            fail(f"index.html loads {pkg}/{path}, which this script doesn't copy: add it to LIBS")
        return f"lib/{pkg}/{path}"

    out = re.sub(r"https://cdn\.jsdelivr\.net/npm/([\w.-]+)@([\d.]+)/([\w./-]+)", local, html)
    if "cdn.jsdelivr.net" in out:
        fail("index.html still loads something from jsDelivr")
    core = '<script src="js/core.js"></script>'
    if core not in out:
        fail("index.html no longer loads js/core.js")
    out = out.replace(core, '<script src="libs.js"></script>\n' + core, 1)

    if DEST.exists():
        shutil.rmtree(DEST)
    DEST.mkdir(parents=True)
    (DEST / "index.html").write_text(out, encoding="utf-8")
    for d in ("css", "js"):
        shutil.copytree(src / d, DEST / d)
    shutil.copy2(src / "LICENSE", DEST / "LICENSE")
    for pkg, files in LIBS.items():
        for f in files:
            to = DEST / "lib" / pkg / f
            to.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(nm / pkg / f, to)
        if pkg in LICENSES:
            (DEST / "lib" / pkg / "LICENSE").write_text(LICENSES[pkg], encoding="utf-8")
    (DEST / "libs.js").write_text(
        "/* Written by packaging/clipcut/sync-graphics.py: Clip Studio's libraries come from this folder,\n"
        "   because ClipCut allows no outside scripts (see CS.lib in js/core.js). */\n"
        "window.CLIP_STUDIO_LIBS = { pdfjs: 'lib/pdfjs-dist/', 'onnxruntime-web': 'lib/onnxruntime-web/dist/' };\n",
        encoding="utf-8",
    )
    commit = subprocess.run(["git", "-C", str(src), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    (DEST / "clip-studio.json").write_text(
        json.dumps({"source": "https://github.com/portdel18/clip-stu", "commit": commit, "libraries": versions}, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"sync-graphics: Clip Studio {commit[:8]} → {DEST.relative_to(ROOT)} ({', '.join(f'{k} {v}' for k, v in versions.items())})")


if __name__ == "__main__":
    main()
