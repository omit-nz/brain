#!/usr/bin/env python3
"""Whether it is safe to go back to a brain older than 0.2.7 after `brain import
clear-review-dates`.

0.2.7 records each copy it rewrites as accepting both the bytes written and the ones before. An
older brain reads only the first, so it takes a file still holding the old bytes for one the person
edited: it stops updating it, and drops its record when its source goes. This looks at the file of
every record carrying the old hash, and exits 1 naming each that still holds those bytes or cannot
be read. It also exits 1 when it cannot look: a data folder, record file or vault that is not
there. It changes nothing. It needs Python 3.11 or newer.

    scripts/clearing-rollback-check.py [--data <folder>] [--config <config.toml>]
"""

import argparse
import hashlib
import json
import os
import pathlib
import sys

try:
    import tomllib
except ImportError:
    sys.exit("this check needs Python 3.11 or newer; nothing was looked at")

SHOWN = 20
MEMORY = "working-memory"


def defaults():
    home = os.environ.get("BRAIN_HOME")
    if home:
        return pathlib.Path(home) / "data", pathlib.Path(home) / "config/config.toml"
    if sys.platform == "darwin":
        data = pathlib.Path.home() / "Library/Application Support/nz.omit.brain"
        return data, data / "config.toml"
    share = pathlib.Path(os.environ.get("XDG_DATA_HOME") or pathlib.Path.home() / ".local/share")
    config = pathlib.Path(os.environ.get("XDG_CONFIG_HOME") or pathlib.Path.home() / ".config")
    return share / "brain", config / "brain/config.toml"


def shown(text):
    """`text` as a terminal prints it: a file name is not brain's choosing."""
    return "".join(c if c.isprintable() else "?" for c in str(text))


def load(path, kind, missing=None):
    """The records in `path`, each an object, held as `kind`: `missing` when the file is not
    there and that is allowed."""
    try:
        held = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        if missing is not None:
            return missing
        sys.exit(f"{shown(path)} is not there: is --data brain's data folder?")
    except (OSError, ValueError) as error:
        sys.exit(f"{shown(path)} cannot be read, so nothing is known: {shown(error)}")
    records = held.values() if isinstance(held, dict) else held
    if not isinstance(held, kind) or not all(isinstance(r, dict) for r in records):
        sys.exit(f"{shown(path)} is not brain's record file, so nothing is known")
    return held


def copy_of(key):
    """Where the copy of a source's key is in the vault."""
    kind, rest = key.split("/", 1)
    return f"{MEMORY}/{rest}" if kind == "memory" else rest


def standing_for(data, vault):
    """Each vault and file a record with an old hash stands for, with the record's two hashes."""
    for key, entry in load(data / "import-ledger.json", dict).items():
        if entry.get("was"):
            root = pathlib.Path(entry.get("vault") or vault)
            yield root, copy_of(key), entry
    for record in load(data / "follow-archive.json", list, missing=[]):
        if record.get("was"):
            root = pathlib.Path(record["vault"])
            yield root, record["path"], record
            yield root, copy_of(record["key"]), record


def holds_anything(folder):
    try:
        return any(folder.iterdir())
    except OSError:
        return False


def in_the_way(path, record):
    """Why the file at `path` stops a rollback, or None. An older brain accepts only
    `dst_hash`, and reads a file that is not there the same as a newer one."""
    try:
        held = hashlib.sha256(path.read_bytes()).hexdigest()
    except (FileNotFoundError, NotADirectoryError):
        return None
    except OSError as error:
        return f"cannot be read: {error.strerror}"
    if held == record["was"] and held != record.get("dst_hash"):
        return "still holds what it did before"
    return None


def main():
    data, config = defaults()
    args = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    args.add_argument("--data", type=pathlib.Path, default=data)
    args.add_argument("--config", type=pathlib.Path, default=config)
    args = args.parse_args()
    try:
        vault = tomllib.loads(args.config.read_text(encoding="utf-8"))["vault"]
    except (OSError, ValueError, KeyError) as error:
        sys.exit(f"{shown(args.config)} does not say where the vault is: {shown(error)}")
    found, gone = {}, {}
    try:
        for root, rel, record in standing_for(args.data, vault):
            # A vault out of reach, or an empty folder where it was mounted, would read as one
            # with nothing in the way.
            if not holds_anything(root):
                gone.setdefault(str(root), set()).add(rel)
                continue
            why = in_the_way(root / rel, record)
            if why:
                found[str(root / rel)] = why
    except (KeyError, TypeError, ValueError, AttributeError) as error:
        sys.exit(f"a record is not one brain wrote, so nothing is known: {shown(repr(error))}")
    if not found and not gone:
        print("no copy is waiting on the newer brain: rolling back is safe")
        return
    print("do not roll back yet")
    if found:
        names = sorted(found)
        print(f"{len(names)} of brain's copies would read as edited by you:")
        for name in names[:SHOWN]:
            print(f"  {shown(name)}: {found[name]}")
        if len(names) > SHOWN:
            print(f"  and {len(names) - SHOWN} more")
        print(
            "\nRun `brain import clear-review-dates --imported-on <day> --apply` again, with the"
            " same days, then roll back. A file that cannot be read has to be made readable"
            " first.\nIf you put notes back from the commit made before clearing, this is why you"
            " see this: an older brain would take those restored notes for ones you edited."
            " Clearing them again before rolling back removes the restored dates."
        )
    for root, files in sorted(gone.items()):
        print(
            f"\n{shown(root)} is not there or is empty, so {len(files)} recorded copies in it could"
            " not be looked at. Put the folder back and run this again. If it is gone for good,"
            " it holds nothing an older brain could misread."
        )
    sys.exit(1)


if __name__ == "__main__":
    main()
