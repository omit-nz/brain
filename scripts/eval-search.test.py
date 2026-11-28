#!/usr/bin/env python3
"""eval-search.py against stand-in brains: what blocks a release, what it refuses to judge, what it
writes and what it prints."""

import importlib.util
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile

SCRIPT = pathlib.Path(__file__).with_name("eval-search.py")

# A stand-in brain: `search` and `due` answer from the table written beside it, and `index` writes
# to its vault and index as a real one would.
STUB = """#!/usr/bin/env python3
import json, os, pathlib, sqlite3, sys
table = json.loads(pathlib.Path(__file__ + ".json").read_text())
if sys.argv[1] == "--version":
    print("brain " + table["version"] + " (protocol 1)")
elif sys.argv[1] == "index":
    home = pathlib.Path(os.environ["BRAIN_HOME"])
    vault = pathlib.Path((home / "config/config.toml").read_text().split('"')[1])
    with (vault / "secret-note.md").open("a") as note:
        note.write("an id\\\\n")
    with sqlite3.connect(home / "data/index.db") as db:
        if db.execute("SELECT value FROM meta WHERE key = 'vault'").fetchone() != (str(vault),):
            sys.exit("the index is of another vault")
        db.execute("INSERT INTO meta VALUES ('indexed', 'yes')")
    if (home / "data/models/embedder").is_file():
        print(table["index"])
    else:
        print("embeddings skipped, search uses words and links: no model")
elif sys.argv[1] == "search":
    if table["answers"] is None:
        sys.exit("secret-note.md could not answer " + sys.argv[-1])
    print(json.dumps({"gap": False, "hits": table["answers"].get(sys.argv[-1], [])}))
elif sys.argv[1] == "due" and sys.argv[2::2] == ["--within", "--json"]:
    if table["due"] is None:
        sys.exit("secret-note.md is due and could not be read")
    print(json.dumps(table["due"][sys.argv[3]]))
else:
    sys.exit("the stand-in was asked something else")
"""

INDEXED = "0 added, 1 changed\n1 embedded, 0 failed, 0 pending"
QUESTION = "a private question"


def hit(path, **extra):
    return {"path": path, **extra}


def case(n, expect, must_not=(), **extra):
    return {"id": n, "question": QUESTION, "expect": expect,
            "must_not_present_as_current": list(must_not), **extra}


def due_case(n, expect, must_not=(), within=7, **extra):
    call = {"tool": "due"} if within is None else {"tool": "due", "within_days": within}
    return case(n, expect, must_not, real_path={"brain_calls": [call]}, **extra)


def listed(due=(), left_open=()):
    """What a candidate's `due --json` prints; the last release printed the due rows alone."""
    return {"due": [hit(p) for p in due], "open": [hit(p) for p in left_open], "open_more": 0}


def run(cases, last, candidate, index=INDEXED, runtime=True, args=None, due=({}, {})):
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        (root / "vault").mkdir()
        for name in ("secret-note.md", "new.md", "old.md"):
            (root / "vault" / name).write_text("words\n")
        data = root / "data"
        (data / "models").mkdir(parents=True)
        (data / "models/embedder").write_text("model\n")
        if runtime:
            (data / "bin/brain-current/lib").mkdir(parents=True)
            (data / "bin/brain-current/lib/libonnxruntime.dylib").write_text("runtime\n")
        with sqlite3.connect(data / "index.db") as db:
            db.execute("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)")
            db.execute("INSERT INTO meta VALUES ('vault', 'elsewhere')")
        installed = (data / "index.db").read_bytes()
        (root / "cases.jsonl").write_text("".join(json.dumps(c) + "\n" for c in cases))
        bins = []
        for name, version, answers, dues in (("last", "0.2.5", last, due[0]),
                                             ("candidate", "0.2.6", candidate, due[1])):
            path = root / name
            path.write_text(STUB)
            path.chmod(0o755)
            pathlib.Path(f"{path}.json").write_text(
                json.dumps({"version": version, "answers": answers, "index": index, "due": dues}))
            bins.append(str(path))
        given = ["--last", bins[0]] if args is None else args
        done = subprocess.run(
            [sys.executable, str(SCRIPT), str(root / "cases.jsonl"), *given,
             "--candidate", bins[1], "--vault", str(root / "vault"), "--data", str(data)],
            capture_output=True, text=True, timeout=120,
            env={k: v for k, v in os.environ.items() if not k.startswith(("BRAIN", "ORT_"))})
        kept = (root / "vault/secret-note.md").read_text() == "words\n" and \
            (data / "index.db").read_bytes() == installed
        return done.returncode, done.stdout + done.stderr, kept


class Check:
    failed = 0

    def __call__(self, name, ok, said=""):
        print(("ok   " if ok else "FAIL ") + name)
        if not ok:
            print(said)
            Check.failed += 1


check = Check()
good = {QUESTION: [hit("secret-note.md")]}
one = [case(1, ["secret-note.md"])]

code, said, kept = run(one, good, good)
check("a case both pass does not block", code == 0 and "case 1: pass" in said, said)
check("the gate never touches the vault or index", kept, said)
check("the output names both versions", "0.2.6 against 0.2.5" in said, said)
code, said, _ = run(one, good, {})
check("a case that passed and now fails blocks", code == 1 and "case 1: FAIL, blocks" in said, said)
code, said, _ = run(one, {}, {})
check("a case both fail does not block", code == 0 and "case 1: fail" in said, said)
code, said, _ = run([case(1, ["secret-note.md"], required_for="0.2.6")], {}, {})
check("a required case blocks whatever the last release did", code == 1, said)
code, said, _ = run([case(1, ["secret-note.md"], required_for="0.2.7")], {}, {})
check("another release's requirement does not", code == 0, said)

two = [case(1, ["new.md"], ["old.md"], required_for="0.2.6")]
code, said, _ = run(two, {}, {QUESTION: [hit("old.md"), hit("new.md")]})
check("a must-not note above an expected one fails", code == 1 and "case 1: FAIL" in said, said)
code, said, _ = run(two, {}, {QUESTION: [hit("new.md"), hit("old.md")]})
check("below every expected one it passes", code == 0, said)
code, said, _ = run(two, {}, {QUESTION: [hit("old.md", overtaken_by="new.md"), hit("new.md")]})
check("marked overtaken it passes", code == 0, said)
code, said, _ = run(two, {}, {QUESTION: [hit(f"n{i}.md") for i in range(5)] + [hit("new.md")]})
check("an expected note sixth fails", code == 1 and "case 1: FAIL" in said, said)

code, said, _ = run(one + [case(2, [], ["old.md"], status="open_in_0.2.6")], good, good)
check("a case with nothing expected is not measured", code == 0 and
      "case 2: not measured (open_in_0.2.6)" in said, said)

stale = [due_case(1, [], ["old.md"], required_for="0.2.6")]
nothing = {"7": []}
code, said, _ = run(stale, good, good, due=(nothing, {"7": listed(left_open=["old.md"])}))
check("a due case fails on a note listed as left open",
      code == 1 and "case 1: FAIL, blocks (last release: pass" in said, said)
code, said, _ = run(stale, good, good, due=(nothing, {"7": listed(["new.md"], ["new.md"])}))
check("it passes when the note is not listed", code == 0 and "case 1: pass" in said, said)
others = [f"n{i}.md" for i in range(20)]
code, said, _ = run(stale, good, good, due=(nothing, {"7": listed(others[:19] + ["old.md"])}))
check("a note twentieth in the review list is seen", code == 1 and "case 1: FAIL" in said, said)
code, said, _ = run(stale, good, good, due=(nothing, {"7": listed(others + ["old.md"])}))
check("a note after the twentieth is not", code == 0, said)
code, said, _ = run(stale, good, good, due=({"7": [hit("old.md")]}, {"7": listed()}))
check("the last release's array is read too", "(last release: fail" in said, said)
wanted = [due_case(1, ["new.md"])]
for where, rows in (("left open", listed(left_open=["new.md"])), ("due", listed(["new.md"]))):
    code, said, _ = run(wanted, good, good, due=(nothing, {"7": rows}))
    check(f"an expected note listed as {where} passes", "case 1: pass" in said, said)
code, said, _ = run(wanted, good, good, due=(nothing, {"7": listed()}))
check("an expected note due does not list fails", "case 1: fail" in said, said)
code, said, _ = run([due_case(1, [], ["old.md"], within=None)], good, good,
                    due=({"14": []}, {"14": listed()}))
check("a call that named no window gets the tool's 14 days", code == 0 and "case 1: pass" in said,
      said)
code, said, _ = run(one + [due_case(2, [])], good, good)
check("a due case naming no note is not measured", "case 2: not measured" in said, said)
code, said, _ = run(stale, good, good, due=(nothing, None))
check("a failing due prints no note path", code != 0 and "due failed (1)" in said and
      "secret-note" not in said, said)

for name, cases, why in (
        ("no cases", [], "no case can be measured"),
        ("no measurable case", [case(2, [])], "no case can be measured"),
        ("two cases under one id", [case(1, ["new.md"]), case(1, ["old.md"])], "case 1: id used twice"),
        ("an expected note that is not there", [case(1, ["gone.md"])], "case 1: names a note"),
        ("a must-not note that is not there", [case(1, ["new.md"], ["gone.md"])], "case 1: names a note"),
        ("a required case expecting nothing", one + [case(2, [], required_for="0.2.6")],
         "case 2: required")):
    code, said, _ = run(cases, good, good)
    check(f"{name} is refused", code != 0 and why in said and "gone.md" not in said, said)

for name, index in (
        ("skipped embeddings", "1 added\nembeddings skipped, search uses words and links: x"),
        ("pending embeddings", "1 added\n3 embedded, 0 failed, 5 pending"),
        ("stopped embeddings", "1 added\n0 embedded, 0 failed, 0 pending\nembedding stopped: x"),
        ("a writer holding the index", "another brain command is updating it now")):
    code, said, _ = run(one, good, good, index=index)
    check(f"{name} stop the gate", code != 0 and "embeddings" in said and "case 1" not in said, said)

code, said, _ = run(one, good, good, runtime=False)
check("no ONNX Runtime stops the gate", code != 0 and "ONNX Runtime" in said, said)
code, said, _ = run(one, good, good, args=[])
check("the last release must be named", code == 2 and "--last" in said, said)

code, said, _ = run(one, good, good)
check("no note path or question is printed", "secret-note" not in said and QUESTION not in said, said)
code, said, _ = run(one, good, None)
check("a failing search prints neither", code != 0 and "search failed (1)" in said and
      "secret-note" not in said and QUESTION not in said, said)

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("eval_search", SCRIPT)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def slow(command, **_):
    raise subprocess.TimeoutExpired(command, 1)


gate.subprocess.run = slow
try:
    gate.run("brain", {}, "search", QUESTION)
    said = "returned"
except SystemExit as stopped:
    said = str(stopped)
except subprocess.TimeoutExpired as error:
    said = str(error)
check("a search that never ends prints no question", "search did not finish" in said and
      QUESTION not in said, said)

rust = SCRIPT.parent.parent.joinpath("crates/brain/src/mcp.rs").read_text()
check("the gate reads as many due rows as the tool lists",
      f"const DUE_SHOWN: usize = {gate.DUE_SHOWN};" in rust)

sys.exit(1 if Check.failed else 0)
