#!/usr/bin/env python3
"""eval-search.py against stand-in brains: what blocks a release, and what it prints."""

import json
import pathlib
import sqlite3
import subprocess
import sys
import tempfile

SCRIPT = pathlib.Path(__file__).with_name("eval-search.py")

# A stand-in brain: `search` answers from the table written beside it.
STUB = """#!/usr/bin/env python3
import json, pathlib, sys
table = json.loads(pathlib.Path(__file__ + ".json").read_text())
if sys.argv[1] == "--version":
    print("brain " + table["version"] + " (protocol 1)")
elif sys.argv[1] == "search":
    print(json.dumps({"gap": False, "hits": table["answers"].get(sys.argv[-1], [])}))
"""


def hit(path, **extra):
    return {"path": path, **extra}


def case(n, question, expect, must_not=(), **extra):
    return {"id": n, "question": question, "expect": expect,
            "must_not_present_as_current": list(must_not), **extra}


def run(cases, last, candidate):
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        (root / "vault").mkdir()
        (root / "vault/secret-note.md").write_text("words\n")
        (root / "data").mkdir()
        with sqlite3.connect(root / "data/index.db") as db:
            db.execute("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)")
            db.execute("INSERT INTO meta VALUES ('vault', 'elsewhere')")
        (root / "cases.jsonl").write_text("".join(json.dumps(c) + "\n" for c in cases))
        bins = []
        for name, version, answers in (("last", "0.2.5", last), ("candidate", "0.2.6", candidate)):
            path = root / name
            path.write_text(STUB)
            path.chmod(0o755)
            pathlib.Path(f"{path}.json").write_text(json.dumps({"version": version, "answers": answers}))
            bins.append(str(path))
        done = subprocess.run(
            [sys.executable, str(SCRIPT), str(root / "cases.jsonl"), "--last", bins[0],
             "--candidate", bins[1], "--vault", str(root / "vault"), "--data", str(root / "data")],
            capture_output=True, text=True, timeout=120)
        return done.returncode, done.stdout + done.stderr


class Check:
    failed = 0

    def __call__(self, name, ok, said):
        print(("ok   " if ok else "FAIL ") + name)
        if not ok:
            print(said)
            Check.failed += 1


check = Check()
good = {"q": [hit("secret-note.md")]}
one = [case(1, "q", ["secret-note.md"])]

code, said = run(one, good, good)
check("a case both pass does not block", code == 0 and "case 1: pass" in said, said)
code, said = run(one, good, {})
check("a case that passed and now fails blocks", code == 1 and "case 1: FAIL, blocks" in said, said)
code, said = run(one, {}, {})
check("a case both fail does not block", code == 0 and "case 1: fail" in said, said)
code, said = run([case(1, "q", ["secret-note.md"], required_for="0.2.6")], {}, {})
check("a required case blocks whatever the last release did", code == 1, said)
code, said = run([case(1, "q", ["secret-note.md"], required_for="0.2.7")], {}, {})
check("another release's requirement does not", code == 0, said)

two = [case(1, "q", ["new.md"], ["old.md"], required_for="0.2.6")]
code, said = run(two, {}, {"q": [hit("old.md"), hit("new.md")]})
check("a must-not note above an expected one fails", code == 1, said)
code, said = run(two, {}, {"q": [hit("new.md"), hit("old.md")]})
check("below every expected one it passes", code == 0, said)
code, said = run(two, {}, {"q": [hit("old.md", overtaken_by="new.md"), hit("new.md")]})
check("marked overtaken it passes", code == 0, said)
code, said = run(two, {}, {"q": [hit(f"n{i}.md") for i in range(5)] + [hit("new.md")]})
check("an expected note sixth fails", code == 1, said)

code, said = run([case(2, "q", [], ["old.md"], status="open_in_0.2.6")], {}, {})
check("a case with nothing expected is not measured", code == 0 and
      "case 2: not measured (open_in_0.2.6)" in said, said)

code, said = run(one, good, good)
check("no note path or question is printed", "secret-note" not in said and "q\n" not in said, said)

sys.exit(1 if Check.failed else 0)
