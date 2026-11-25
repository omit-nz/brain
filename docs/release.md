# Releases

One release carries one version for every part of brain, signed once. This page covers what a release holds, how one is cut, checked and rolled back, what the first install trusts, and how the signing key is made and rotated.

## What a release holds

A release is the GitHub release for tag `v<version>` on `omit-nz/brain`. For each target (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`) it holds `brain-<version>-<target>.tar.gz`, with one folder inside, `brain-<version>/`, holding `brain` (or `brain.exe`) and `lib/` with ONNX Runtime 1.24.2 from NuGet (`scripts/fetch-ort.sh`). Each archive carries its zipsign signature inside it, made with the archive's file name as the signing context, so a signed archive renamed to another version or target no longer verifies.

Beside the archives: `brain-<version>.mcpb`, `SHA256SUMS` (every archive and the `.mcpb`), and `release.json`, which reads `{version, protocol: 1, targets: {<target>: {url, sha256}}}` for the archives only.

`brain-<version>.mcpb` is the Claude Desktop extension: `manifest.json` (its version filled from the tag), `icon.png`, and `server/brain`, byte for byte the `brain` of the macOS archive. Its only job is to hand over: Claude Desktop runs it as `brain mcp --door claude-desktop --hand-over`, and it starts the installed brain in its place, or exits saying to install brain first. So the extension never serves from its own, possibly older, copy, and an update to the install reaches it at once. `verify` refuses to publish unless that `brain` matches the signed archive's and the manifest's args end in `--hand-over`. It ships unsigned until the Developer ID secrets (`MCPB_CERT`, `MCPB_KEY`) exist; `mcpb sign` then signs it in the same job.

`brain --version` prints `brain <version> (protocol 1)`. The protocol changes only when two brains of different releases can no longer work together.

## The install

`scripts/install.sh` puts the release folder at `brain-current` inside brain's data folder (`brain install-dir` prints the path) and links `~/.local/bin/brain` to the binary in it. `brain update` replaces that folder whole, so the binary and the runtime library always come from one release. A brain that `install.sh` did not install, such as a `cargo build`, refuses to update itself.

`install.sh` copies the new release folder in beside the install before touching it, so replacing the install is two renames on one filesystem; if the second fails, the old install is put back. A version given to it must look like `1.2.3`, checked before anything is downloaded, and every download, redirects included, stays on HTTPS. `scripts/install.test.sh` pins each of these with a stubbed `curl` and release.

## What is trusted, and when

The first install trusts HTTPS from GitHub and `SHA256SUMS` from the same release: `install.sh` downloads both, checks the archive against `SHA256SUMS`, unpacks it, and then has the brain it just unpacked check the archive's signature against the release key compiled into that brain. A failed check installs nothing. On a first install that signature check guards against nothing the checksum does not: the binary doing the checking came from the same download, and `install.sh` runs it before it knows the signature is good, so an attacker who can replace both the archive and `SHA256SUMS` on GitHub runs code as you. The first install's trust is GitHub's HTTPS and the integrity of the release page, no more.

Every later update is checked by the brain already installed, before anything is replaced: `brain update` downloads the archive and refuses it unless its signature verifies against the key that brain was built with. A build whose key file is the all-zero placeholder ships no key, and refuses to update or to verify anything.

## How the daemon updates itself

The daemon checks for a newer release after its first pass at start, and then once a day. It installs one only through the same path as `brain update`: the archive's signature must verify against the release key built into the running brain (and, once releases are notarised, Apple's signature must name omit's Team ID), and the install folder is replaced whole. Then it prints `installed brain <version>; exiting for launchd to restart` and exits with code 75; launchd's `KeepAlive` (restart unless the exit was clean) starts the new binary. A failed check or install prints one line, is counted as an update error, and leaves the old version running until the next day's check. A brain run from anywhere but its install, such as a `cargo build`, says so at each daily check and never updates itself.

## Cutting a release

1. Set `version` under `[workspace.package]` in `Cargo.toml`; every crate takes the workspace version (`every_crate_takes_the_workspace_version`). Commit through a PR.
2. Tag the merged commit `v<version>` and push the tag.
3. `.github/workflows/release.yml` runs:
   - `build`, on macOS, Linux and Windows, checks the tag matches the workspace version, builds `brain` with `--release --locked`, and packs it with ONNX Runtime.
   - `sign`, on Linux, signs every archive with `RELEASE_SIGNING_KEY` (and `RELEASE_SIGNING_KEY_PREVIOUS` when it is set), decoded into a private temporary folder and never printed, writes `SHA256SUMS` and `release.json`, and drafts the release. It installs zipsign 0.2.2 from crates.io with `cargo install --locked`, so that crate and its lock file are trusted with the key; the checkouts in `sign` and `verify` keep no token on disk.
   - `verify`, on macOS, builds `brain` from the tag, refuses to go on if its release key is the placeholder, downloads every asset of the draft, checks `SHA256SUMS`, runs `brain release-verify` on every archive (the same check clients run, so a signature that verifies only where it was made fails here), and only then publishes the release.

A release that fails any job stays a draft. Delete the draft and the tag, fix, and tag again.

## Checking a release by hand

```bash
shasum -a 256 -c SHA256SUMS
brain release-verify brain-<version>-<target>.tar.gz   # exit 0 when signed by the release key
```

## Rolling back

`brain update` only moves to a newer version. To roll back, cut a new release from the last good commit with a higher version number, so every install moves forward onto it. To take a bad release away before many installs reach it, mark it a draft again in GitHub; installs and updates then see the release before it as the newest.

To put one machine back on an older release, run `scripts/install.sh <older version>`.

## The signing key

The key is a zipsign ed25519 key pair. `scripts/release-key.sh init` makes it once: the private key goes base64-encoded to 1Password (item `brain release signing key`, vault `Omit-Infra`) and to the repo secret `RELEASE_SIGNING_KEY`, both on stdin only, and the public key is written to `crates/brain/src/keys/release.pub` to be committed. The script never puts the private key in an argument, on stdout or on stderr, and removes its temporary folder on exit; `scripts/release-key.test.sh` pins that with stubs for `zipsign`, `op` and `gh`.

Before it makes anything, `init` refuses when the public key file is not the placeholder, when 1Password already holds an item with that title, or when the repo already has `RELEASE_SIGNING_KEY`, so a second run from another checkout cannot replace a key a build already trusts.

`scripts/release-key.sh rotate` makes a new key while keeping the old one for one release: it needs exactly one `brain release signing key` item, copies that old private key from 1Password into `RELEASE_SIGNING_KEY_PREVIOUS`, sets the new key as `RELEASE_SIGNING_KEY`, writes the new public key, renames the old 1Password item as retired, and stores the new key under the plain title. The next release is signed by both keys and its binary trusts only the new one, so installs that trust only the old key can still take it. Once that release is out, delete `RELEASE_SIGNING_KEY_PREVIOUS`. If the old key leaked, rotate anyway, but treat every install as untrusted until it has been reinstalled with `install.sh`.

### A run that stopped

Both commands store the new key first as `brain release signing key (pending)` and give it the plain title only as their last step, after the repo secrets and `release.pub` are written. While a pending item exists, every run refuses. To repair:

- If `release.pub` did not change, the run stopped before or at setting `RELEASE_SIGNING_KEY`. Delete the pending item in 1Password and run the same command again. A rotate that stopped here already copied the old key to `RELEASE_SIGNING_KEY_PREVIOUS`, and the next run copies the same old key again.
- If `release.pub` changed, every secret was set and only the renames are left. After a rotate, rename the old item to `brain release signing key (retired <date>)` if that has not happened, then rename the pending item to `brain release signing key`. After an init, rename the pending item.

`release-key.test.sh` and `install.test.sh` run on `ubuntu-24.04` in CI and on macOS by hand; they are not run under Git Bash on Windows. Running the key script needs zipsign 0.2.2 (`cargo install zipsign --version 0.2.2 --locked`), `jq`, `op` signed in to the account that holds `Omit-Infra`, and `gh` signed in with admin rights on `omit-nz/brain`.
