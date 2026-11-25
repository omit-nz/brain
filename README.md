# brain, by omit

Leave a brain at home. brain is the shared context layer of a business: the knowledge, working memory, history, email and screen context every person and every AI tool in it draws on, kept on the business's own machines. A Rust daemon and CLI with a local MCP plug into the AI the business already pays for, and sync between colleagues goes through a relay that only ever holds ciphertext. Apache-2.0. Built and cared for by [omit](https://omit.nz).

## Install

On a Mac (Apple silicon): download `brain.pkg` and open it. It installs for you alone, with no admin password, and macOS checks omit's signature before it runs. Then run `brain init --me <your name>` in your notes folder, which also starts the daemon. Releases carry `brain.pkg` once its Developer ID signing is in place; until then, use the script below.

On Linux (x86_64), for CI and for scripted installs:

```bash
curl --proto '=https' -fsSL https://raw.githubusercontent.com/omit-nz/brain/main/scripts/install.sh | bash
```

Then `brain update` moves you to each new release once its signature checks. [docs/release.md](docs/release.md) says what is trusted and when, and [SECURITY.md](SECURITY.md) answers the questions a security review asks.

## Connect your AI

- Claude Code: `brain connect claude-code`. It runs Claude Code's own `claude mcp remove` and `claude mcp add` for a user-scope server named `brain`, so it replaces any server of that name you added yourself, and it writes no file itself; start a new session afterwards.
- Claude Desktop: open `brain-<version>.mcpb` from the release page. The extension's copy of brain hands every connection to the brain you installed, so it needs the install above first.

Both reach your notes through the same five tools: search your brain, search what your team shares, capture, correct and due.
