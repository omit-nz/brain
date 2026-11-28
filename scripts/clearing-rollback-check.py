#!/usr/bin/env python3
"""Whether it is safe to go back to a brain older than 0.2.7 after `brain import
clear-review-dates`.

0.2.7 records each copy it rewrites as accepting both the bytes written and the ones before. An
older brain reads only the first, so it takes a file still holding the old bytes for one the person
edited: it stops updating it, and drops its record when its source goes. This looks at the file of
every record carrying the old hash, and exits 1 naming each that still holds those bytes or cannot
be read. It changes nothing.

    scripts/clearing-rollback-check.py [--data <folder>] [--config <config.toml>]
"""

import argparse
import hashlib
import json
import os
import pathlib
import sys
import tomllib

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


def load(path, missing):
    try:
        return json.loads(path.read_text())
    except FileNotFoundError:
        return missing
    except (OSError, ValueError) as error:
        sys.exit(f"{path} cannot be read, so nothing is known: {error}")


def copy_of(key):
    """Where the copy of a source's key is in the vault."""
    kind, rest = key.split("/", 1)
    return f"{MEMORY}/{rest}" if kind == "memory" else rest


def standing_for(data, vault):
    """Each file a record with an old hash stands for, with that hash."""
    for key, entry in load(data / "import-ledger.json", {}).items():
        if entry.get("was"):
            yield pathlib.Path(entry.get("vault") or vault) / copy_of(key), entry["was"]
    for record in load(data / "follow-archive.json", []):
        if record.get("was"):
            root = pathlib.Path(record["vault"])
            yield root / record["path"], record["was"]
            yield root / copy_of(record["key"]), record["was"]


def in_the_way(path, was):
    """Why the file at `path` stops a rollback, or None. A file that is not there reads the same
    to every brain."""
    try:
        held = path.read_bytes()
    except (FileNotFoundError, NotADirectoryError):
        return None
    except OSError as error:
        return f"cannot be read: {error.strerror}"
    return "still holds what it did before" if hashlib.sha256(held).hexdigest() == was else None


def main():
    data, config = defaults()
    args = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    args.add_argument("--data", type=pathlib.Path, default=data)
    args.add_argument("--config", type=pathlib.Path, default=config)
    args = args.parse_args()
    try:
        vault = tomllib.loads(args.config.read_text())["vault"]
    except (OSError, ValueError, KeyError) as error:
        sys.exit(f"{args.config} does not say where the vault is: {error}")
    found = {}
    for path, was in standing_for(args.data, vault):
        why = in_the_way(path, was)
        if why:
            found[str(path)] = why
    if not found:
        print("no copy is waiting on the newer brain: rolling back is safe")
        return
    names = sorted(found)
    print(f"do not roll back yet: {len(names)} of brain's copies would read as edited by you")
    for name in names[:SHOWN]:
        print(f"  {name}: {found[name]}")
    if len(names) > SHOWN:
        print(f"  and {len(names) - SHOWN} more")
    print(
        "\nRun `brain import clear-review-dates --imported-on <day> --apply` again, with the same"
        " days, then roll back. A file that cannot be read has to be made readable first.\n"
        "If you put notes back from the commit made before clearing, this is why you see this: an"
        " older brain would take those restored notes for ones you edited. Clearing them again"
        " before rolling back removes the restored dates."
    )
    sys.exit(1)


if __name__ == "__main__":
    main()
