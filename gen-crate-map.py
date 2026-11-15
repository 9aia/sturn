#!/usr/bin/env python3
"""Generate the crate-map table inside reference/crate-api.md
(from workspace Cargo.tomls — never hand-edit the marked region)."""
import glob, os, pathlib, re

# See gen-cli-docs.py — STURN_ROOT overrides the `..` worktree layout.
ROOT = pathlib.Path(
    os.environ.get("STURN_ROOT") or pathlib.Path(__file__).parent / "..")
WST = (ROOT / "Cargo.toml").read_text()

LAYER = {
    "sturn-core": "domain",
    "sturn-claim": "domain",
    "sturn-ask": "domain",
    "sturn-wal": "port",
    "sturn-bus": "port",
    "sturn-ws": "port",
    "sturn-secret": "port",
    "sturn-service": "port",
    "sturn-wal-git": "adapter",
    "sturn-bus-redb": "adapter",
    "sturn-ws-git": "adapter",
    "sturn-git": "adapter",
    "sturn-odb": "adapter",
    "sturn-keyring": "adapter",
    "sturn-dotenv": "adapter",
    "sturn-systemd": "adapter",
    "sturn-fuse": "adapter",
    "sturn-config": "adapter",
    "sturn": "engine",
    "sturn-driver": "driver",
}
ORDER = {"domain": 0, "port": 1, "adapter": 2, "engine": 3,
         "driver": 4}

# workspace members — explicit paths, `crates/*` globs, and xtask
members_block = re.search(r"members\s*=\s*\[(.*?)\]", WST, re.S).group(1)
raw = re.findall(r'"([^"]+)"', members_block)
members = []
for m in raw:
    if "*" in m:
        members += sorted(
            str(p.relative_to(ROOT))
            for p in ROOT.glob(m) if (p / "Cargo.toml").exists())
    elif (ROOT / m / "Cargo.toml").exists():
        members.append(m)

ws_desc = ""
md = re.search(r"\[workspace\.package\](.*?)(?=\n\[)", WST, re.S)
if md:
    dm = re.search(r'^description\s*=\s*"([^"]+)"', md.group(1), re.M)
    ws_desc = dm.group(1) if dm else ""

def description(toml_path):
    t = toml_path.read_text()
    m = re.search(r'^description\s*=\s*"([^"]+)"', t, re.M)
    if m:
        return m.group(1)
    inherited = bool(re.search(r'^description\.workspace\s*=', t, re.M))
    # fallback: first `//!` paragraph of the crate's lib/main —
    # beats an inherited workspace tagline for crate-specific prose
    for src in ("src/lib.rs", "src/main.rs"):
        p = toml_path.parent / src
        if p.exists():
            para = []
            for line in p.read_text().splitlines():
                st = line.strip()
                if st.startswith("//!"):
                    para.append(st.lstrip("/! ").strip())
                elif para:
                    break
            if para:
                return tidy(" ".join(para))
    if inherited:
        return ws_desc
    return "—"


def tidy(text):
    """Module docs → one table-cell purpose: drop code fences, unwrap
    `[`link`]` text, cap at ~200 chars on a sentence boundary."""
    text = text.split("```")[0]
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)   # [t](url) → t
    text = text.replace("[`", "`").replace("`]", "`")
    text = re.sub(r"\s+", " ", text).strip()
    if len(text) <= 200:
        return text
    cut = text[:200]
    dot = max(cut.rfind(". "), cut.rfind("! "), cut.rfind("? "))
    return ((cut[:dot + 1] if dot > 60 else cut.rstrip() + "…").rstrip(" ,;:"))

rows = []
for rel in members:
    t = ROOT / rel / "Cargo.toml"
    name = re.search(r'^name\s*=\s*"([^"]+)"', t.read_text(), re.M).group(1)
    desc = (description(t).replace("<", "&lt;").replace(">", "&gt;")
            .replace("{{", "&#123;&#123;").replace("}}", "&#125;&#125;"))
    rows.append((name, LAYER.get(name, "crate"), desc))
rows.sort(key=lambda r: (ORDER.get(r[1], 9), r[0]))
print(f"crate map: {len(rows)} crates")

table = "| crate | layer | purpose |\n|---|---|---|\n" + "\n".join(
    f"| `{n}` | {l} | {d} |" for n, l, d in rows)

arch = pathlib.Path("reference/crate-api.md")
s = arch.read_text()
if "<!-- crate-map -->" not in s:
    raise SystemExit("architecture.md lacks the <!-- crate-map --> marker")
s = re.sub(
    r"<!-- crate-map -->\n(?:\|[^\n]*\n?)*",
    "<!-- crate-map -->\n" + table + "\n", s, count=1)
arch.write_text(s)
print("crate-api.md crate table regenerated")
