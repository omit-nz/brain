# Architecture

## Layers

Each crate depends only on the crates below it.

- `brain-platform`: everything that touches the operating system. Folders, atomic and private file writes, moves that refuse links, archiving instead of deleting, and the clock. Platform-specific code lives here and nowhere else.
- `brain-note`: one note. Frontmatter that round-trips byte for byte, the one function that decides what a note may disclose, ids, wikilinks, the vault walk and the promote gate.
- `brain-index`: the SQLite index beside each person's data, embeddings and hybrid search. The index is a cache: it can always be rebuilt from the vault.
- `brain`: the binary. The CLI, the learning-loop import, the inbox, and updates; later the daemon, the MCP server and the hooks.

`brain-repo-rules` holds no code. Its tests read the repository itself and fail when a repo rule breaks.

## Platforms

CI builds and tests every push on macOS, Linux and Windows, on GitHub-hosted runners only. A test that means something only on Unix (file modes) asserts the Unix fact under `cfg(unix)` and the Windows fact under `cfg(windows)` inside `brain-platform`'s tests; neither is skipped. `.gitattributes` fixes LF line ends everywhere, so byte-compared files mean the same on every platform. Every job stages ONNX Runtime from NuGet with `scripts/fetch-ort.sh` and sets `ORT_DYLIB_PATH`, because GitHub's copy has been gone since November 2026.
