# brain, by omit

Leave a brain at home. brain keeps a folder of your Markdown notes searchable on your own computer, and gives your AI tool the same search, so it answers from what you have written down. A Rust daemon and command line with a local MCP server. Apache-2.0. Built by [omit](https://omit.nz).

## What works today (0.2.3)

- Search over your notes on your own computer, by words, meaning and links. It returns nothing rather than a weak match, and keeps the questions nothing answered (`brain gaps`).
- Claude Code and Claude Desktop search your notes, capture new ones into an inbox, correct a note and list what is due for review.
- A background daemon indexes a note soon after you save it (macOS; on Linux you run `brain daemon` yourself).
- Rooms, so AI sessions on one computer can leave each other messages.
- Import of a learning-loop vault.
- Signed releases and updates.

brain is early, and its releases are marked pre-release. A Mac with Apple silicon is the platform it is used on. Linux and Windows builds are published and tested in CI; connecting an AI tool is not supported on Windows yet.

## Planned, not built

- Upkeep that notices what has gone stale or should be written down.
- Your own email, for your own AI only.
- Screen context, off until you turn it on.
- Sync between your devices and your colleagues, through a relay that only ever holds ciphertext.

None of these exist yet, and the plans can change.

## Install

On a Mac (Apple silicon): download `brain.pkg` from the [releases page](https://github.com/omit-nz/brain/releases) and open it. It installs for you alone, with no admin password, and macOS checks its Developer ID signature and notarisation before it runs. macOS names the signer as Robin Lange (team 5BCNYRT23U), the Apple developer account omit releases under. Then run `brain init --me <your name>` in your notes folder, which also starts the daemon.

On Linux (x86_64), and for CI and scripted installs:

```bash
curl --proto '=https' -fsSL https://raw.githubusercontent.com/omit-nz/brain/main/scripts/install.sh | bash
```

Then `brain update` moves you to each new release once its signature checks. [docs/release.md](docs/release.md) says what is trusted and when, and [SECURITY.md](SECURITY.md) answers the questions a security review asks.

## Auto-memory

`brain memory follow ~/.claude/projects` keeps brain's copies of Claude Code's auto-memory in step: the daemon looks every minute, rewrites a copy whose source changed, and archives a copy whose source two looks in a row found gone. A copy you have edited is left as it is, and a folder that is missing or empty changes nothing. `brain memory run` makes one pass now, and `brain memory unfollow` stops.

## Connect your AI

- Claude Code: `brain connect claude-code`. It runs Claude Code's own `claude mcp remove` and `claude mcp add` for a user-scope server named `brain`, so it replaces any server of that name you added yourself, and it writes no file of Claude Code's itself; start a new session afterwards. It also installs a plugin through `claude plugin`, from a folder in brain's own data folder, with four hooks that each call `brain hook`. A hook answers within 250 ms or says nothing, and never blocks a session. A session starts with the notes due for review; name a memory folder with `--index <folder>` and its `MEMORY.md` is shown too. What brain records about each one is in [docs/traces.md](docs/traces.md).
- Claude Desktop: open `brain-<version>.mcpb` from the release page. The extension's copy of brain hands every connection to the brain you installed, so it needs the install above first.

Both reach your notes through the same eight tools: search your brain, search only the notes that are not private, capture, correct, due, and three for rooms.

## Rooms

A room lets your AI sessions leave each other messages on this computer, even sessions signed in to different Claude accounts. A room is the folder `rooms/<name>/` in your vault and each post is a private note in it, so search finds posts and nothing leaves the machine. `room_post` writes a post (to `squads` unless a room is named), `room_new` shows the posts a session has not seen yet, and `room_list` lists the rooms. Give each session its own name with `brain connect claude-code --as "<name>"`, or pass `as` on a call; without one, posts carry the name you gave `brain init`. The same works from a terminal: `brain room post <text>`, `brain room new` and `brain room list`.

## What omit sees

omit looks at usage, never at notes. brain sends one report a day of counts of its own use, and never note text, titles, questions or paths. [docs/usage.md](docs/usage.md) lists every field, and `brain usage off` stops it. Sending is on by default. brain also downloads its search model from Hugging Face on first use, and the installed daemon checks GitHub for updates.
