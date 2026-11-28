#!/usr/bin/env python3
"""clearing-rollback-check.py against stand-in records and files: what it refuses, what it passes,
and that it changes nothing."""

import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile

SCRIPT = pathlib.Path(__file__).with_name("clearing-rollback-check.py")
OLD = b"with a date\n"
NEW = b"without\n"
failed = []


def check(name, ok, said=""):
    print(("ok   " if ok else "FAIL ") + name)
    if not ok:
        print(said)
        failed.append(name)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(ledger, files, archive=(), locked=None):
    """The script over a vault holding `files`, with these records. Returns its code, output and
    whether it left every file as it was."""
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        vault, data = root / "vault", root / "data"
        vault.mkdir()
        data.mkdir()
        for rel, held in files.items():
            (vault / rel).parent.mkdir(parents=True, exist_ok=True)
            (vault / rel).write_bytes(held)
        (root / "config.toml").write_text(f'vault = "{vault}"\nme = "me"\n')
        (data / "import-ledger.json").write_text(json.dumps(ledger))
        if archive:
            records = [{**record, "vault": str(vault)} for record in archive]
            (data / "follow-archive.json").write_text(json.dumps(records))
        before = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
        if locked:
            (vault / locked).chmod(0)
        done = subprocess.run(
            [sys.executable, "-I", str(SCRIPT), "--data", str(data), "--config", str(root / "config.toml")],
            capture_output=True, text=True, timeout=60)
        if locked:
            (vault / locked).chmod(0o600)
        same = before == {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
        return done.returncode, done.stdout + done.stderr, same


MOVED_ON = {"dst_hash": digest(NEW), "was": digest(OLD)}

code, said, same = run({"vault/a.md": MOVED_ON}, {"a.md": OLD})
check("an unwritten note is refused by name", code == 1 and "a.md: still holds" in said, said)
check("the refusal says to clear again, and why a restored vault shows it",
      "clear-review-dates" in said and "put notes back" in said, said)
check("it changes nothing", same)
code, said, _ = run({"vault/a.md": MOVED_ON}, {"a.md": NEW})
check("once written it passes", code == 0 and "safe" in said, said)
code, said, _ = run({"memory/p/one.md": MOVED_ON}, {"working-memory/p/one.md": OLD})
check("a memory copy is found where brain keeps it", code == 1 and "working-memory/p/one.md" in said, said)
code, said, _ = run({"vault/a.md": MOVED_ON}, {})
check("a file that is not there passes", code == 0, said)
code, said, _ = run({"vault/a.md": {"dst_hash": digest(NEW)}}, {"a.md": OLD})
check("a record with one hash is the person's affair", code == 0, said)
code, said, _ = run({"vault/a.md": MOVED_ON}, {"a.md": b"my own words\n"})
check("a note the person edited passes", code == 0, said)

record = {"key": "memory/p/one.md", "path": "_archive/working-memory/p/one.md", **MOVED_ON}
for place in ("_archive/working-memory/p/one.md", "working-memory/p/one.md"):
    code, said, _ = run({}, {place: OLD}, archive=[record])
    check(f"an archive record's file at {place} is looked at", code == 1 and place in said, said)

if os.name == "posix" and os.geteuid() != 0:
    code, said, _ = run({"vault/a.md": MOVED_ON}, {"a.md": NEW}, locked="a.md")
    check("a file that cannot be read is refused by name", code == 1 and "a.md: cannot be read" in said, said)

many = {f"vault/n{i:02}.md": MOVED_ON for i in range(25)}
code, said, _ = run(many, {f"n{i:02}.md": OLD for i in range(25)})
check("a long list is cut and counted", code == 1 and "and 5 more" in said and "n24.md" not in said, said)

with tempfile.TemporaryDirectory() as tmp:
    root = pathlib.Path(tmp)
    (root / "config.toml").write_text('vault = "/nowhere"\n')
    (root / "import-ledger.json").write_text("not json")
    done = subprocess.run(
        [sys.executable, "-I", str(SCRIPT), "--data", str(root), "--config", str(root / "config.toml")],
        capture_output=True, text=True, timeout=60)
    check("a ledger that cannot be read is never called safe",
          done.returncode != 0 and "safe" not in done.stdout, done.stdout + done.stderr)

sys.exit(1 if failed else 0)
