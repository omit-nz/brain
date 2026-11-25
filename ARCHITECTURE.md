# Architecture

## Layers

Each crate depends only on the crates below it.

- `brain-platform`: everything that touches the operating system. Folders, atomic and private file writes, moves that refuse links, archiving instead of deleting, and the clock. Platform-specific code lives here and nowhere else.
- `brain-note`: one note. Frontmatter that round-trips byte for byte, the one function that decides what a note may disclose, ids, wikilinks, the vault walk and the promote gate.
- `brain-index`: the SQLite index beside each person's data, embeddings and hybrid search. The index is a cache: it can always be rebuilt from the vault.
- `brain`: the binary. The CLI, the learning-loop import, the inbox, updates and the daemon; later the MCP server and the hooks.

`brain-repo-rules` holds no code. Its tests read the repository itself and fail when a repo rule breaks.

## Processes

One process writes the index: whoever holds `writer.lock` in the data folder, which the operating system frees when that process dies, however it dies. `brain daemon` holds it for as long as it runs, watches the vault, and after any change that can matter (a `.md` path, a rename or a removal, never anything inside a hidden folder) brings the index up to date, then embeds one batch at a time so a save is never stuck behind a long first embed. It writes no id into a note saved in the last five seconds, so a note being typed is left alone until its owner stops, and a full pass every ten minutes catches whatever the watcher missed. `brain index` and `brain import` take the lock when it is free and otherwise leave indexing to the daemon; the import writes each copy's id itself, so its copies are final with or without an index. The CLI, and later the MCP server and the hooks, only read the index; `brain status` says whether a daemon runs by a shared probe that never blocks a writer.

## Platforms

CI builds and tests every push on macOS, Linux and Windows, on GitHub-hosted runners only. A test that means something only on Unix (file modes) asserts the Unix fact under `cfg(unix)` and the Windows fact under `cfg(windows)` inside `brain-platform`'s tests; neither is skipped. `.gitattributes` fixes LF line ends everywhere, so byte-compared files mean the same on every platform. Every job stages ONNX Runtime from NuGet with `scripts/fetch-ort.sh` and sets `ORT_DYLIB_PATH`, because GitHub's copy has been gone since November 2026.

`brain start` and `brain stop` run the daemon as a launchd agent, so they work on macOS only in Phase 1; on Linux and Windows they say so, `brain daemon` runs by hand, and an update finds no agent to restart. Their launchd tests run on macOS only, against a `launchctl` stub named by `BRAIN_LAUNCHCTL`; `start_outside_macos_says_so` runs on the other two.
