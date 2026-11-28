#!/usr/bin/env python3
"""The release gate over search: asks every case of the last release and of the candidate, each over
its own copy of one vault and its index, and exits 1 when the release is blocked.

    scripts/eval-search.py <cases.jsonl> --candidate target/release/brain [--last 0.2.5]

The cases are private, so this runs on the maintainer's machine before a tag, never in CI. It prints
case ids and results only, never a question or a note.

A case is one JSON object a line: id, question, expect (paths, all of which must be in the top 5),
must_not_present_as_current (paths each absent, below every expected one, or marked overtaken),
and optionally required_for (a version it must pass in) and status.
"""

import argparse
import json
import os
import pathlib
import platform
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

TOP = 5
ASKED = 10
HOME = pathlib.Path.home()


def run(binary, env, *args):
    done = subprocess.run([binary, *args], env=env, capture_output=True, text=True, timeout=3600)
    if done.returncode:
        sys.exit(f"{args[0]} failed ({done.returncode}): {done.stderr.strip()[-400:]}")
    return done.stdout


def release(version, candidate, into):
    """The released binary of `version`, downloaded and checked against the release key."""
    target = {"Darwin": "aarch64-apple-darwin", "Linux": "x86_64-unknown-linux-gnu"}[platform.system()]
    name = f"brain-{version}-{target}.tar.gz"
    archive = into / name
    url = f"https://github.com/omit-nz/brain/releases/download/v{version}/{name}"
    urllib.request.urlretrieve(url, archive)
    run(candidate, os.environ, "release-verify", str(archive))
    with tarfile.open(archive) as packed:
        packed.extractall(into, filter="data")
    return str(into / f"brain-{version}" / "brain")


def home(binary, root, vault, data, runtime):
    """A scratch brain home for `binary` over its own copy of the vault, seeded from the installed
    index so only what this binary reads differently is read again."""
    copy = root / "vault"
    shutil.copytree(vault, copy, symlinks=True, ignore=shutil.ignore_patterns(".git"))
    scratch = root / "home"
    (scratch / "config").mkdir(parents=True)
    (scratch / "data").mkdir()
    (scratch / "config/config.toml").write_text(
        f'vault = "{copy}"\nme = "{me(data)}"\n\n[usage]\nsend = false\n')
    seed = data / "index.db"
    if seed.exists():
        with sqlite3.connect(f"file:{seed}?mode=ro", uri=True) as source, \
                sqlite3.connect(scratch / "data/index.db") as copied:
            source.backup(copied)
            copied.execute("UPDATE meta SET value = ? WHERE key = 'vault'", [str(copy)])
    env = {k: v for k, v in os.environ.items() if not k.startswith(("BRAIN", "ORT_"))}
    env["BRAIN_HOME"] = str(scratch)
    if runtime:
        env["ORT_DYLIB_PATH"] = runtime
    run(binary, env, "index")
    return env


def me(data):
    config = data / "config.toml"
    lines = config.read_text().splitlines() if config.exists() else []
    return next((l.split('"')[1] for l in lines if l.startswith("me ")), "me")


def passes(case, hits):
    paths = [h["path"] for h in hits]
    if any(p not in paths[:TOP] for p in case["expect"]):
        return False
    lowest = max(paths.index(p) for p in case["expect"])
    return all(
        path not in paths or paths.index(path) > lowest or hits[paths.index(path)].get("overtaken_by")
        for path in case["must_not_present_as_current"])


def answers(binary, env, cases):
    out = {}
    for case in cases:
        if case["expect"]:
            said = run(binary, env, "search", "--json", "--limit", str(ASKED), case["question"])
            out[case["id"]] = passes(case, json.loads(said[said.index("{"):])["hits"])
    return out


def main():
    data = HOME / "Library/Application Support/nz.omit.brain"
    args = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    args.add_argument("cases", type=pathlib.Path)
    args.add_argument("--candidate", required=True, help="the binary to be released")
    args.add_argument("--last", help="the last release's version, or its binary; the latest installed if unset")
    args.add_argument("--vault", type=pathlib.Path, default=HOME / "omit-brain")
    args.add_argument("--data", type=pathlib.Path, default=data)
    args = args.parse_args()
    cases = [json.loads(line) for line in args.cases.read_text().splitlines() if line.strip()]
    candidate = str(pathlib.Path(args.candidate).resolve())
    installed = args.data / "bin/brain-current"
    last = args.last or str(installed / "brain")
    version = run(candidate, os.environ, "--version").split()[1]
    runtime = os.environ.get("ORT_DYLIB_PATH") or next(
        (str(p) for p in installed.glob("lib/*onnxruntime*")), None)
    with tempfile.TemporaryDirectory(prefix="brain-eval-") as tmp:
        root = pathlib.Path(tmp).resolve()
        if not pathlib.Path(last).exists():
            (root / "release").mkdir()
            last = release(last, candidate, root / "release")
        results = {}
        for name, binary in (("last", str(pathlib.Path(last).resolve())), ("candidate", candidate)):
            env = home(binary, root / name, args.vault, args.data, runtime)
            results[name] = answers(binary, env, cases)
    blocked = 0
    for case in cases:
        n = case["id"]
        if n not in results["candidate"]:
            status = f" ({case['status']})" if "status" in case else ""
            print(f"case {n}: not measured{status}")
            continue
        passed = results["candidate"][n]
        required = case.get("required_for") == version
        blocks = not passed and (required or results["last"][n])
        blocked += blocks
        word = "pass" if passed else "FAIL, blocks" if blocks else "fail"
        print(f"case {n}: {word} (last release: {'pass' if results['last'][n] else 'fail'}"
              f"{', required' if required else ''})")
    measured = results["candidate"]
    print(f"{version}: {sum(measured.values())} of {len(measured)} measured cases pass; "
          f"{'BLOCKED by ' + str(blocked) if blocked else 'not blocked'}")
    sys.exit(1 if blocked else 0)


if __name__ == "__main__":
    main()
