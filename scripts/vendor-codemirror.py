#!/usr/bin/env python3
"""Re-vendor the CodeMirror module graph from esm.sh into assets/vendor/codemirror.

See assets/vendor/codemirror/README.md for why this exists and how it works.
Run from the repo root: `python3 scripts/vendor-codemirror.py`. Bump the
`ENTRYPOINTS` version pins below first, then diff the output before
replacing assets/vendor/codemirror/ wholesale.
"""
import re
import urllib.request
from pathlib import Path

BASE = "https://esm.sh"
OUT = Path(__file__).resolve().parent.parent / "assets" / "vendor" / "codemirror"

ENTRYPOINTS = {
    "view": "/@codemirror/view@6",
    "state": "/@codemirror/state@6",
    "commands": "/@codemirror/commands@6",
    "lang-markdown": "/@codemirror/lang-markdown@6",
    "theme-one-dark": "/@codemirror/theme-one-dark@6",
}

# Matches static import/export-from declarations anywhere in minified source,
# without relying on line or semicolon boundaries (ASI means statements can
# abut a preceding `}` with no `;`).
DECL_RE = re.compile(
    r'''(?<![.\w$])(?:import|export)\s*'''
    r'''(?:\*\s*as\s*[\w$]+\s*from\s*|\{[^}]*\}\s*from\s*|\*\s*from\s*|[\w$]+\s*from\s*)?'''
    r'''["'](/[^"']+)["']'''
)

resolved = {}  # requested path -> local filename
fetched = {}   # final resolved esm.sh path -> local filename (dedup point)
order = []     # [fname, final_path, body] in fetch order


def local_name(path):
    name = path.lstrip('/').replace('/', '__')
    # Sanitize esm.sh's version-range query strings (e.g. "^6.7.0?target=es2022")
    # since "?" etc. would otherwise be parsed as a URL query delimiter when
    # used as a relative import specifier.
    name = re.sub(r'[^A-Za-z0-9._-]', '_', name)
    if not (name.endswith('.mjs') or name.endswith('.js')):
        name += '.mjs'
    return name


def fetch(path):
    if path in resolved:
        return resolved[path]
    req = urllib.request.Request(BASE + path, headers={"User-Agent": "curl/8"})
    with urllib.request.urlopen(req, timeout=20) as resp:
        final_url = resp.geturl()
        body = resp.read().decode('utf-8')
    final_path = final_url[len(BASE):]
    if final_path in fetched:
        resolved[path] = fetched[final_path]
        return fetched[final_path]
    fname = local_name(final_path)
    fetched[final_path] = fname
    resolved[path] = fname
    entry = [fname, final_path, body]
    order.append(entry)
    # Longest-first so a shorter dep path can't accidentally be a substring
    # replaced inside a longer one first.
    deps = sorted(set(DECL_RE.findall(body)), key=len, reverse=True)
    for d in deps:
        dep_fname = fetch(d)
        entry[2] = entry[2].replace(f'"{d}"', f'"./{dep_fname}"').replace(f"'{d}'", f"'./{dep_fname}'")
    return fname


def main():
    entry_files = {name: fetch(path) for name, path in ENTRYPOINTS.items()}

    OUT.mkdir(parents=True, exist_ok=True)
    for fname, _fpath, body in order:
        (OUT / fname).write_text(body)

    print("ENTRYPOINTS (update these filenames in assets/editor.js):")
    for name, fname in entry_files.items():
        print(f"  {name} -> {fname}")
    print(f"\nTotal files written to {OUT}: {len(order)}")

    print("\nUnresolved absolute imports remaining (should be empty):")
    bad = False
    for fname, _fpath, body in order:
        for m in DECL_RE.finditer(body):
            if not m.group(1).startswith('./'):
                bad = True
                print(f"  {fname}: ...{body[max(0, m.start() - 20):m.end()]}...")
    if not bad:
        print("  (none)")


if __name__ == "__main__":
    main()
