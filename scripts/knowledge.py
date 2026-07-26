#!/usr/bin/env python3
"""ctl knowledge/memory companion — the externalized knowledge layer.

This is the workflow-side companion to ctl: ctl's Rust layer stays governance-
only and does NOT manage knowledge/memory content (it's evidence, agent-owned).
This script owns the two non-canonical knowledge carriers:

  .ctl/facts.jsonl     atomic verified facts (the project knowledge base)
  ~/.ctl/memory/*.md   global, cross-project memory

Subcommands:
  fact add       record a verified fact (generates F-NNN, stamps time/actor)
  fact list      list/search facts (by category and/or search term)
  fact promote   append a fact as a markdown block into a curated spec file
  fact summary   compact digest (counts + recent) — for hook context injection
  memory verify  scan ~/.ctl/memory/*.md for project-path pollution (advisory)

Format-compatible with the facts.jsonl ctl previously wrote, so existing facts
and the append-only semantics carry over unchanged. Pure stdlib; no deps.

Exit codes: 0 success / no findings, 1 usage or IO error. (memory verify never
fails on pollution — it's advisory.)
"""

import argparse
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path


# ── paths ────────────────────────────────────────────────────────────────────

def project_root() -> Path:
    return Path.cwd()


def facts_path() -> Path:
    return project_root() / ".ctl" / "facts.jsonl"


def memory_dir() -> Path:
    home = os.environ.get("USERPROFILE") or os.environ.get("HOME")
    return Path(home) / ".ctl" / "memory" if home else None


def actor() -> str:
    """Match ctl's actor_from_env: CTL_ACTOR if set (hooks set it to the model
    label), else 'human'."""
    return os.environ.get("CTL_ACTOR") or "human"


def now_iso8601() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ── fact store ───────────────────────────────────────────────────────────────

def read_all_facts():
    """All facts in append order. Blank lines skipped; unparseable lines carry
    the line number. Empty list when the file does not exist yet."""
    path = facts_path()
    if not path.exists():
        return []
    facts = []
    with path.open("r", encoding="utf-8") as fh:
        for i, line in enumerate(fh, start=1):
            if line.strip() == "":
                continue
            try:
                facts.append(json.loads(line))
            except json.JSONDecodeError as e:
                sys.exit(f"{path}: line {i}: parse error: {e}")
    return facts


def next_fact_id(facts):
    """F-001, F-002, ... by scanning existing ids for the highest numeric
    suffix. Starts at F-001 when empty (or no parseable ids)."""
    max_n = 0
    for f in facts:
        fid = f.get("fact_id", "")
        if fid.startswith("F-"):
            try:
                max_n = max(max_n, int(fid[2:]))
            except ValueError:
                pass
    return "F-{:03d}".format(max_n + 1)


def filter_facts(facts, category=None, search=None):
    """Category matches exactly (case-insensitive); search matches statement OR
    source (case-insensitive substring). Append order."""
    cat = category.lower() if category else None
    q = search.lower() if search else None
    out = []
    for f in facts:
        fc = (f.get("category") or "").lower()
        if cat is not None and fc != cat:
            continue
        if q is not None and q not in f.get("statement", "").lower() \
                and q not in f.get("source", "").lower():
            continue
        out.append(f)
    return out


def facts_digest(facts, recent_count=5):
    """total + per-category counts + the N most recent facts as one-liners."""
    categories = {}
    for f in facts:
        c = f.get("category") or "uncategorized"
        categories[c] = categories.get(c, 0) + 1
    recent = [
        {"fact_id": f.get("fact_id"), "statement": f.get("statement"),
         "category": f.get("category")}
        for f in reversed(facts[-recent_count:])
    ]
    return {"total": len(facts), "categories": categories, "recent": recent}


def format_fact_for_promote(fact):
    cat = fact.get("category") or "uncategorized"
    return (
        "\n### Fact {fid} (category: {cat})\n"
        "**Source**: {src}\n**Verified**: {at}\n\n{stmt}\n"
    ).format(fid=fact.get("fact_id"), cat=cat, src=fact.get("source"),
             at=fact.get("recorded_at"), stmt=fact.get("statement"))


# ── fact subcommands ─────────────────────────────────────────────────────────

def cmd_fact_add(args):
    if not args.statement or not args.source:
        sys.exit("fact add: --statement and --source are required (a fact "
                 "without provenance is an opinion, not knowledge)")
    facts = read_all_facts()
    entry = {
        "fact_id": next_fact_id(facts),
        "statement": args.statement,
        "source": args.source,
        "category": args.category,
        "recorded_at": now_iso8601(),
        "recorded_by": args.by or actor(),
    }
    facts_path().parent.mkdir(parents=True, exist_ok=True)
    with facts_path().open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(entry, ensure_ascii=False) + "\n")
    print(f"Recorded {entry['fact_id']}: {entry['statement']}")
    print(f"  source: {entry['source']}  category: {entry['category'] or 'uncategorized'}")


def cmd_fact_list(args):
    facts = read_all_facts()
    matches = filter_facts(facts, args.category, args.search)
    if not matches:
        print(f"No facts match ({len(facts)} total in knowledge base).")
        return
    for f in matches:
        cat = f.get("category") or "uncategorized"
        print(f"{f.get('fact_id')}  [{cat}]  {f.get('statement')}")
        print(f"    source: {f.get('source')}")
    print(f"\n{len(matches)} fact(s) shown of {len(facts)} total.")


def cmd_fact_promote(args):
    facts = read_all_facts()
    fact = next((f for f in facts if f.get("fact_id") == args.id), None)
    if fact is None:
        sys.exit(f"Fact '{args.id}' not found in the knowledge base")
    target = Path(args.to)
    block = format_fact_for_promote(fact)
    # Append (do not overwrite curated content). Read existing to avoid duplicate.
    existing = target.read_text(encoding="utf-8") if target.exists() else ""
    if args.id in existing:
        print(f"{args.id} already referenced in {target} — not re-appended.")
        return
    with target.open("a", encoding="utf-8") as fh:
        fh.write(block)
    print(f"Promoted {args.id} into {target}")


def cmd_fact_summary(args):
    facts = read_all_facts()
    digest = facts_digest(facts, recent_count=args.recent)
    if args.json:
        print(json.dumps(digest, ensure_ascii=False))
        return
    if digest["total"] == 0:
        print("Knowledge base: empty.")
        return
    cats = ", ".join(f"{k}: {v}" for k, v in sorted(digest["categories"].items()))
    recent = "; ".join(f"{r['fact_id']} {r['statement']}"
                       for r in digest["recent"])
    print(f"Knowledge base: {digest['total']} fact(s) [{cats}] | Recent: {recent}")


# ── memory verify ────────────────────────────────────────────────────────────
# Ported from the former cli/memory.rs (ctl memory verify, #1/S). Global memory
# is shared across every project session, so project-specific content (source
# paths, build commands, code extensions, absolute paths) pollutes all of them.

CODE_EXTS = (".rs", ".ts", ".tsx", ".js", ".jsx", ".go", ".py", ".java",
             ".rb", ".vue", ".svelte")
BUILD_CMDS = ("cargo run", "cargo build", "cargo test", "cargo bench", "cargo fmt",
              "npm run", "npm test", "npm install", "npm ci", "pnpm ", "yarn ",
              "pip install", "pytest", "jest", "go build", "go test", "dotnet ")
ABS_MARKERS = ("/home/", "/users/", "/usr/local/", "c:\\", "d:\\", "c:/", "d:/")


def pollution_signals(line):
    sigs = []
    lower = line.lower()
    if "src/" in line or "\\src\\" in line:
        sigs.append("source path")
    if any(e in line for e in CODE_EXTS):
        sigs.append("code extension")
    if any(c in lower for c in BUILD_CMDS):
        sigs.append("build/test command")
    if any(m in lower for m in ABS_MARKERS):
        sigs.append("absolute path")
    return sigs


def scan_memory_dir(d, findings, files_scanned):
    for entry in sorted(d.iterdir()):
        if entry.is_dir():
            scan_memory_dir(entry, findings, files_scanned)
        elif entry.suffix == ".md":
            files_scanned[0] += 1
            try:
                content = entry.read_text(encoding="utf-8").splitlines()
            except OSError:
                continue
            for i, line in enumerate(content, start=1):
                sigs = pollution_signals(line)
                if sigs:
                    excerpt = line.strip()
                    if len(excerpt) > 80:
                        excerpt = excerpt[:77] + "..."
                    findings.append((str(entry), i, sigs, excerpt))


def cmd_memory_verify(args):
    mdir = memory_dir()
    if mdir is None:
        msg = "No home directory (USERPROFILE/HOME) — cannot locate ~/.ctl/memory/."
        if args.json:
            print(json.dumps({"has_memory_dir": False, "findings": 0, "items": []}))
        else:
            print(msg)
        return
    if not mdir.exists():
        if args.json:
            print(json.dumps({"has_memory_dir": False, "memory_dir": str(mdir),
                              "findings": 0, "items": []}))
        else:
            print(f"No global memory directory at {mdir} — nothing to scan.")
        return
    findings = []
    scanned = [0]
    scan_memory_dir(mdir, findings, scanned)
    if args.json:
        items = [{"file": f, "line": l, "signals": s, "excerpt": e}
                 for (f, l, s, e) in findings]
        print(json.dumps({"has_memory_dir": True, "memory_dir": str(mdir),
                          "files_scanned": scanned[0], "findings": len(findings),
                          "items": items}, ensure_ascii=False))
        return
    if not findings:
        print(f"No project-path pollution detected (scanned {scanned[0]} memory file(s)).")
    else:
        print(f"POLLUTION warnings ({len(findings)} across {scanned[0]} memory file(s)):")
        for (f, l, s, e) in findings:
            print(f"  {f}:{l}  [{', '.join(s)}]  {e}")
        print("\nAdvisory only — global memory is shared across all projects; "
              "review whether these references are project-specific.")


# ── CLI ──────────────────────────────────────────────────────────────────────

def main():
    ap = argparse.ArgumentParser(
        prog="knowledge.py",
        description="ctl knowledge/memory companion (externalized from ctl Rust).")
    sub = ap.add_subparsers(dest="group", required=True)

    fact = sub.add_parser("fact", help="atomic verified facts (.ctl/facts.jsonl)")
    fs = fact.add_subparsers(dest="fact_cmd", required=True)

    p = fs.add_parser("add", help="record a verified fact")
    p.add_argument("--statement", required=True)
    p.add_argument("--source", required=True, help="where verified: file:line, command, or URL")
    p.add_argument("--category", default=None)
    p.add_argument("--by", default=None, help="recorder (default: $CTL_ACTOR or 'human')")
    p.set_defaults(func=cmd_fact_add)

    p = fs.add_parser("list", help="list/search facts")
    p.add_argument("--category", default=None)
    p.add_argument("--search", default=None)
    p.set_defaults(func=cmd_fact_list)

    p = fs.add_parser("promote", help="append a fact as markdown into a spec file")
    p.add_argument("--id", required=True)
    p.add_argument("--to", required=True)
    p.set_defaults(func=cmd_fact_promote)

    p = fs.add_parser("summary", help="compact digest (for hook context)")
    p.add_argument("--recent", type=int, default=5)
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_fact_summary)

    mem = sub.add_parser("memory", help="global memory (~/.ctl/memory)")
    ms = mem.add_subparsers(dest="mem_cmd", required=True)
    p = ms.add_parser("verify", help="scan for project-path pollution (advisory)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(func=cmd_memory_verify)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
