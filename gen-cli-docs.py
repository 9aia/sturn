#!/usr/bin/env python3
"""Generate docs/reference/cli.md by walking `<cli> --help` recursively.

clap is the source of truth — the script shells out to the real binary
(`cargo build` once, then direct invocations) so the reference can
never drift from `--help`. Never hand-edit cli.md.

Per-repo knobs live in the PROG/ARGS block below.
"""
import os, pathlib, re, subprocess, sys

ROOT = pathlib.Path(
    os.environ.get("STURN_ROOT") or pathlib.Path(__file__).parent / "..")

# How to reach the CLI. `cargo build` runs once up front; afterwards the
# debug binary is invoked directly (fast — one process per help node).
PKG = "sturn"          # cargo package to build
BIN = "sturn"   # binary under target/debug
# argv prefix before the engine's own argv (`run` = engine passthrough)
PREFIX = []
OUT = pathlib.Path(__file__).parent / "reference" / "cli.md"

TARGET = ROOT / "target" / "debug"


def build():
    subprocess.run(["cargo", "build", "-q", "-p", PKG], cwd=ROOT, check=True)


def help_text(path):
    argv = [str(TARGET / BIN), *PREFIX, *path, "--help"]
    r = subprocess.run(argv, cwd=ROOT, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"help failed for {' '.join(path) or '<root>'}:\n{r.stderr}")
    return r.stdout


SECTION_RE = re.compile(r"^([A-Z][A-Za-z ]*):\s*$")
ENTRY_RE = re.compile(r"^(\s+)(\S.*)$")


def split_desc(head):
    """`-h, --help  Print help` → (sig, desc). Descriptions align after a
    2+-space gap; flag specs never contain one."""
    parts = re.split(r"\s{2,}", head.strip())
    if len(parts) > 1:
        return parts[0].strip(), " ".join(parts[1:]).strip()
    return head.strip(), ""


def parse_help(text):
    """→ {usage, about, sections: {name: [(sig, desc), ...]}}"""
    lines = text.splitlines()
    usage, about, sections = "", [], {}
    cur, entries = None, []

    def flush():
        nonlocal entries
        if cur:
            sections[cur] = entries
        entries = []

    for i, line in enumerate(lines):
        m = re.match(r"^Usage: (.*)$", line)
        if m and not usage:
            usage = m.group(1).strip()
            continue
        s = SECTION_RE.match(line)
        if s:
            flush()
            cur = s.group(1)
            continue
        if cur is None:
            if line.strip() and not line.startswith(" "):
                about.append(line.strip())
            elif line.strip():
                about.append(line.strip())
            continue
        # inside a section: blank line ends nothing (entries may span),
        # but a line indented <=2 spaces with no leading dash inside
        # Options means a new entry only for Commands/Arguments.
        stripped = line.strip()
        if not stripped:
            continue
        indent = len(line) - len(line.lstrip())
        is_entry = False
        if cur == "Commands":
            is_entry = indent <= 4 and not stripped.startswith("-")
        elif cur in ("Options", "Arguments"):
            is_entry = stripped.startswith("-") or (
                cur == "Arguments" and (stripped.startswith("<")
                                        or stripped.startswith("[")))
        if is_entry:
            sig, desc = split_desc(stripped)
            # continuation lines: deeper-indented prose after the head
            j = i + 1
            cont = []
            while j < len(lines):
                nl = lines[j]
                ns = nl.strip()
                if not ns:
                    break
                nind = len(nl) - len(nl.lstrip())
                nentry = False
                if cur == "Commands":
                    nentry = nind <= 4 and not ns.startswith("-")
                elif cur in ("Options", "Arguments"):
                    nentry = ns.startswith("-") or (
                        cur == "Arguments" and (ns.startswith("<")
                                                or ns.startswith("[")))
                if nentry or nind <= indent:
                    break
                cont.append(ns)
                j += 1
            if cont:
                desc = (desc + " " if desc else "") + " ".join(cont)
            entries.append((sig, desc))
    flush()
    return usage, " ".join(about).strip(), sections


def esc(text):
    """Escape < > outside inline-code spans so Vue doesn't see elements."""
    parts = text.split("`")
    for i in range(0, len(parts), 2):
        parts[i] = parts[i].replace("<", "&lt;").replace(">", "&gt;")
    return "`".join(parts)


def anchor(path):
    # `## `lasius code-workspace generate`` → #lasius-code-workspace-generate
    return "-".join(path.lower().replace("`", "").split())


def prog_name(usage):
    m = re.match(r"([\w-]+)", usage)
    return m.group(1) if m else "cli"


def main():
    build()
    text = help_text([])
    usage, about, sections = parse_help(text)
    name = prog_name(usage)

    # walk subcommands
    nodes = []  # (path, usage, about, sections)
    seen = set()

    def walk(path):
        key = tuple(path)
        if key in seen:
            return
        seen.add(key)
        t = help_text(path)
        u, a, secs = parse_help(t)
        nodes.append((path, u, a, secs))
        for sig, _desc in secs.get("Commands", []):
            sub = sig.split()[0] if sig.split() else sig
            if sub in ("help",):
                continue
            # clap may render `name <ARGS>` — first token is the verb
            walk(path + [sub])

    walk([])

    out = [f"# Command-Line Help for `{name}`", "",
           f"This document contains the help content for the `{name}` "
           "command-line program.", ""]
    if about:
        out += [esc(about), ""]
    out += ["**Command Overview:**", ""]
    for path, _u, _a, _s in nodes:
        label = " ".join([name, *path]) or name
        out.append(f"* [`{label}`↴](#{anchor(label)})")
    out.append("")

    for path, u, a, secs in nodes:
        label = " ".join([name, *path])
        out += [f"## `{label}`", ""]
        if a:
            out += [esc(a), ""]
        out += [f"**Usage:** `{u}`", ""]
        for sec_name in ("Arguments", "Commands", "Options"):
            entries = secs.get(sec_name)
            if not entries:
                continue
            out += [f"###### **{sec_name}:**", ""]
            for sig, desc in entries:
                if sec_name == "Commands":
                    sub = sig.split()[0]
                    if sub == "help":
                        continue
                    d = esc(desc) if desc else ""
                    out.append(f"* `{sub}`" + (f" — {d}" if d else ""))
                else:
                    d = esc(desc) if desc else ""
                    out.append(f"* `{sig}`" + (f" — {d}" if d else ""))
            out.append("")

    OUT.write_text("\n".join(out))
    print(f"cli.md: {len(nodes)} commands")


if __name__ == "__main__":
    main()
