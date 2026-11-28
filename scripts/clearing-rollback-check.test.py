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


def run(ledger, files, archive=(), locked=None, data_at="data", vault_at="vault", raw_archive=None):
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
        named = root / vault_at
        (root / "config.toml").write_text(f'vault = "{named}"\nme = "me"\n', encoding="utf-8")
        (data / "import-ledger.json").write_text(json.dumps(ledger))
        if archive:
            records = [{**record, "vault": str(vault)} for record in archive]
            (data / "follow-archive.json").write_text(json.dumps(records))
        if raw_archive is not None:
            (data / "follow-archive.json").write_text(raw_archive)
        before = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
        if locked:
            (vault / locked).chmod(0)
        done = subprocess.run(
            # A read that leaves the encoding to the machine fails here, whatever the machine.
            [sys.executable, "-I", "-X", "warn_default_encoding", "-W", "error::EncodingWarning",
             str(SCRIPT), "--data", str(root / data_at), "--config", str(root / "config.toml")],
            capture_output=True, text=True, encoding="utf-8", timeout=60)
        if locked:
            (vault / locked).chmod(0o700)
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
code, said, _ = run({"vault/a.md": MOVED_ON}, {"b.md": NEW})
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

# A file that holds what brain's record says it wrote is one an older brain accepts.
code, said, _ = run({"vault/a.md": {"dst_hash": digest(OLD), "was": digest(OLD)}}, {"a.md": OLD})
check("a file the older brain accepts passes", code == 0 and "safe" in said, said)

code, said, _ = run({"vault/caf\u00e9.md": MOVED_ON}, {"caf\u00e9.md": OLD})
check("a name outside ascii is found", code == 1 and "still holds" in said, said)
code, said, _ = run({"vault/a\x1b[31m.md": MOVED_ON}, {"a\x1b[31m.md": OLD})
check("a name is never printed as the terminal would act on it", code == 1 and "\x1b" not in said, repr(said))

# Looking at nothing is never called safe.
for name, how in [
    ("a data folder that is not there", {"data_at": "nowhere"}),
    ("the folder above the data folder", {"data_at": "."}),
    ("a vault that is not there", {"vault_at": "unmounted"}),
]:
    code, said, _ = run({"vault/a.md": MOVED_ON}, {"a.md": OLD}, **how)
    check(f"{name} is refused", code != 0 and "safe" not in said and "Traceback" not in said, said)
code, said, _ = run({}, {}, raw_archive="{}")
check("an archive record file of another shape is refused",
      code != 0 and "safe" not in said and "Traceback" not in said, said)
code, said, _ = run([], {})
check("a ledger of another shape is refused", code != 0 and "safe" not in said and "Traceback" not in said, said)
code, said, _ = run({"vault/a.md": {"dst_hash": digest(NEW), "was": digest(OLD), "vault": "/nowhere"}}, {})
check("a record's own vault that is not there is refused", code != 0 and "safe" not in said, said)

code, said, _ = run({"vault/a.md": MOVED_ON}, {})
check("an empty folder where the vault was is refused",
      code == 1 and "empty" in said and "safe" not in said, said)
code, said, _ = run(
    {"vault/b.md": {**MOVED_ON, "vault": "/nowhere"}, "vault/a.md": MOVED_ON}, {"a.md": OLD})
check("a vault that is gone is named with the way out, and the rest is still looked at",
      code == 1 and "/nowhere" in said.replace("\\", "/") and "gone for good" in said
      and "a.md: still holds" in said, said)
code, said, _ = run({"vault/a.md": MOVED_ON}, {".DS_Store": b"x"})
check("a folder holding only hidden files is as good as empty",
      code == 1 and "safe" not in said, said)
record = {"key": "memory/p/one.md", "path": "_archive/working-memory/p/one.md", **MOVED_ON}
code, said, _ = run({}, {}, archive=[record])
check("files out of reach are counted as files", code == 1 and "2 recorded files" in said, said)
if os.name == "posix" and os.geteuid() != 0:
    code, said, _ = run({"vault/a.md": MOVED_ON}, {"a.md": OLD}, locked=".")
    check("a vault that cannot be read is said to be that, not gone",
          code == 1 and "cannot be read" in said and "gone for good" not in said, said)
code, said, _ = run({}, {"other.md": NEW}, archive=[{**record, "was": 3}])
check("an archive record's hash that is no text is refused",
      code != 0 and "safe" not in said and "Traceback" not in said, said)
for name, ledger in [
    *((f"a hash of {was!r}", {"vault/a.md": {"dst_hash": digest(NEW), "was": was}})
      for was in (0, False, [], "", None)),
    ("a hash that is no text", {"vault/a.md": {"dst_hash": digest(NEW), "was": 3}}),
    ("a record that is no object", {"vault/a.md": 3}),
    ("a key with no kind", {"a.md": MOVED_ON}),
]:
    code, said, _ = run(ledger, {"a.md": OLD})
    check(f"{name} is refused", code != 0 and "safe" not in said and "Traceback" not in said, said)

older = subprocess.run(
    [sys.executable, "-I", "-c",
     "import runpy, sys; sys.modules['tomllib'] = None; runpy.run_path(sys.argv[1], run_name='__main__')",
     str(SCRIPT)],
    capture_output=True, text=True, timeout=60)
check("a python without tomllib is told which it needs",
      older.returncode != 0 and "3.11" in older.stderr and "Traceback" not in older.stderr,
      older.stdout + older.stderr)

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
