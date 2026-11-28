# brain, by omit

Leave a brain at home. brain keeps a folder of your Markdown notes searchable on your own computer, and gives your AI tool the same search, so it answers from what you have written down. A Rust daemon and command line with a local MCP server. Apache-2.0. Built by [omit](https://omit.nz).

## What works today (0.2.10)

- Search over your notes on your own computer, by words, meaning and links. It returns nothing rather than a weak match, says when each note it finds was written, marks one that another note corrects or supersedes, orders by date when you ask for the latest decision, says for a copy whose source it has recorded whether that file has changed since, and keeps the questions nothing answered (`brain gaps`).
- Claude Code and Claude Desktop search your notes, capture new ones into an inbox, correct a note and list what is due for review.
- A background daemon indexes a note soon after you save it (macOS; on Linux you run `brain daemon` yourself).
- Rooms, so AI sessions on one computer can leave each other messages.
- A Claude Code plugin: a session starts with the notes due for review and the memory indexes you name. What brain reads and decides at each hook is kept on your computer ([docs/traces.md](docs/traces.md) says what, and when it is dropped).
- Copies of Claude Code's auto-memory that follow their sources (`brain memory follow`).
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

On a Mac (Apple silicon): download `brain.pkg` from the [releases page](https://github.com/omit-nz/brain/releases) and open it. It installs for you alone, with no admin password, and macOS checks its Developer ID signature and notarisation before it runs. macOS names the signer as Robin Lange (team 5BCNYRT23U), the Apple developer account omit releases under. Then run `brain setup`, which makes your notes folder and starts the daemon; `brain init --me <your name>` in a folder of notes you already have works too.

On Linux (x86_64), and for CI and scripted installs:

```bash
curl --proto '=https' -fsSL https://raw.githubusercontent.com/omit-nz/brain/main/scripts/install.sh | bash
```

Then `brain update` moves you to each new release once its signature checks. [docs/release.md](docs/release.md) says what is trusted and when, and [SECURITY.md](SECURITY.md) answers the questions a security review asks.

## Auto-memory

`brain memory follow ~/.claude/projects` keeps brain's copies of Claude Code's auto-memory in step: the daemon looks every minute, rewrites a copy whose source changed, and archives a copy whose source it has seen there and two looks in a row then found gone. A copy you have edited is left as it is, a copy you have removed stays removed, and a followed folder that is missing or empty changes nothing. `brain memory run` makes one pass now, and `brain memory unfollow` stops. `brain setup --memory on` follows it too; setup does not follow it unasked.

Archiving is undone when the source returns: the same file comes back under the same id. If its words changed while it was away, a new copy is written under that id and the old one stays in `_archive`. An archived memory is out of search, and `brain status` says how many there are. Archiving does not purge it from brain's traces. Deleting the archived file brain would bring back does, once two passes have found it gone, and that memory is not copied again. If it returned with other words it kept its id, so removing the new copy can purge the old words' traces too.

When a project's memory folder becomes a link to another project's, its copies move to the new name; where brain had already copied a memory under both, the copy under the old name is archived, or kept if you edited it. To move a project's memory yourself, run `brain memory unfollow`, move it, link the old name to it, and follow again; run no import in between.

## Limits

- A project's memory folder moved before the old name is linked to the new one gets new copies, ones you had removed included. The old copies are archived, or kept if you edited them, and the archived ones come back if you move the folder back. (`a_folder_moved_with_no_link_is_copied_anew`)
- Do not rename or switch your notes folder once brain has followed memory, even after `brain memory unfollow`: copies made before stop following their sources, ones you had removed can be copied again, memories archived before are not brought back, and there is no supported way yet to tie them back. (`a_renamed_vault_folder_stops_following_its_copies`)
- `brain import` run between a memory folder's move and the next pass copies that project again under its new name. (`an_import_between_a_move_and_the_first_follow_copies_the_project_again`)
- A linked project whose new name brain has never copied: a file of the same name there is taken for the old memory renamed, and the copy takes its words. If it was a different memory, the old words are not kept. (`a_linked_name_never_copied_takes_the_old_copy_and_its_words`)
- A copy you removed is written again if its project was linked to one that has its own copy of that memory and you later undo the link. (`a_removed_copy_is_made_again_after_a_link_is_undone`)
- A pass killed after writing copies and before saving its record leaves those copies reading as yours, so they stop following. (`a_pass_killed_before_the_ledger_save_leaves_its_copies_as_the_persons`, `a_changed_copy_cut_short_before_the_ledger_save_is_not_taken_for_the_old_one`)
- A followed folder that is reset takes those memories out of search until it is restored. (`a_folder_reset_and_restored_in_halves_leaves_the_vault_as_it_was`)
- A `supersedes` naming a memory copy does not count when the copy reads as later than the note naming it, even if that date is only something the memory mentions. `corrects:` with the copy's id or path counts whatever the dates say. (`a_later_date_in_a_memory_copy_outdates_a_supersedes_naming_it`)
- A memory that is removed and written again every few minutes moves its file each time. (`a_source_that_comes_and_goes_moves_its_file_each_time`)
- An archived file you move elsewhere, or rewrite without its id, reads as deleted and its memory is not copied again; one you moved into your notes is a note there, and its traces stay if the index has seen it by then, as it does while the daemon runs. So does every one of them read as deleted if your notes folder is copied without `_archive`. (`an_archived_file_rewritten_without_its_id_reads_as_deleted`, `an_archived_file_moved_into_the_notes_keeps_its_traces`, `a_vault_without_its_archive_folder_reads_as_the_files_deleted`)
- An archived file you move back by hand while its source is still gone is archived again. (`an_archived_file_moved_back_while_its_source_is_gone_is_archived_again`)
- After you follow a different folder, memories archived before are not brought back; one that returns is copied afresh, and the archived file stays. brain no longer tracks that file, so deleting it purges nothing. (`another_followed_folder_restores_nothing_and_takes_no_id`)
- `brain import` of an archived memory's source writes a fresh copy with a new id, and the archived file stays. brain no longer tracks that file, so deleting it purges nothing. (`an_import_copies_an_archived_memory_afresh`)

## Connect your AI

- Claude Code: `brain connect claude-code`. It runs Claude Code's own `claude mcp remove` and `claude mcp add` for a user-scope server named `brain`, so it replaces any server of that name you added yourself, and it writes no file of Claude Code's itself; start a new session afterwards. It also installs a plugin through `claude plugin`, from a folder in brain's own data folder, with four hooks that each call `brain hook`. A hook answers within 250 ms or says nothing, and never blocks a session. A session starts with the notes due for review; name a memory folder with `--index <folder>` and its `MEMORY.md` is shown too. What brain records about each one is in [docs/traces.md](docs/traces.md).
- Claude Desktop: open `brain-<version>.mcpb` from the release page. The extension's copy of brain hands every connection to the brain you installed, so it needs the install above first.

Both reach your notes through the same eight tools: search your brain, search only the notes that are not private, capture, correct, due, and three for rooms.

## Rooms

A room lets your AI sessions leave each other messages on this computer, even sessions signed in to different Claude accounts. A room is the folder `rooms/<name>/` in your vault and each post is a private note in it, so search finds posts and nothing leaves the machine. `room_post` writes a post (to `squads` unless a room is named), `room_new` shows the posts a session has not seen yet, and `room_list` lists the rooms. Give each session its own name with `brain connect claude-code --as "<name>"`, or pass `as` on a call; without one, posts carry the name brain was set up with. The same works from a terminal: `brain room post <text>`, `brain room new` and `brain room list`.

## Commands

- `brain setup`: set brain up, taking the defaults, or `--guided` to be asked.
- `brain init`, `brain status`, `brain update`: set up over a folder of notes, see what the index holds, install the newest signed release.
- `brain start`, `brain stop`, `brain daemon`: keep the index up to date at every login or in a terminal.
- `brain index`, `brain search`, `brain show`, `brain gaps`, `brain due`: index now, search, print a note, list unanswered questions, the oldest 20 notes due for review (`--all` for every one) and recent notes with something left open.
- `brain add`, `brain correct`, `brain inbox`, `brain inbox keep`, `brain inbox skip`: write a note into the inbox, correct one, and work through the inbox.
- `brain connect`, `brain mcp`: connect an AI tool, and serve it.
- `brain calls`: the last tool calls your AI tools made and what each asked. No reply and no captured text is kept.
- `brain memory follow`, `brain memory run`, `brain memory unfollow`: follow Claude Code's auto-memory.
- `brain import learning-loop`: copy another brain's notes in. A copy gets no review date.
- `brain import clear-review-dates --imported-on <day>`: take out the review dates 0.2.5 and earlier stamped on what they imported. It only lists them until `--apply`.
- `brain room post`, `brain room new`, `brain room list`: leave and read messages between sessions.
- `brain usage`, `brain usage on`, `brain usage off`: see and set what omit is sent.
- `brain setup reminders`: turn brain's reminder to run setup off or on.

## What omit sees

omit looks at usage, never at notes. brain sends one report a day of counts of its own use, and never note text, titles, questions or paths. [docs/usage.md](docs/usage.md) lists every field, and `brain usage off` stops it. Sending is on by default. brain also downloads its search model from Hugging Face on first use, and the installed daemon checks GitHub for updates.
