# Releases

One release carries one version for every part of brain, signed once. This page covers what a release holds, how one is cut, checked and rolled back, what the first install trusts, and how the signing key is made and rotated.

## What a release holds

A release is the GitHub release for tag `v<version>` on `omit-nz/brain`. For each target (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`) it holds `brain-<version>-<target>.tar.gz`, with one folder inside, `brain-<version>/`, holding `brain` (or `brain.exe`) and `lib/` with ONNX Runtime 1.24.2 from NuGet (`scripts/fetch-ort.sh`). Each archive carries its zipsign signature inside it, made with the archive's file name as the signing context, so a signed archive renamed to another version or target no longer verifies.

In the macOS archive, `brain` (identifier `nz.omit.brain`, hardened runtime) and `lib/libonnxruntime.dylib` are signed with omit's Developer ID and notarised.

Beside the archives: `brain.pkg`, the macOS installer (below); `brain-<version>.mcpb`; `brain-<version>.sigstore.json`, the build attestation for every archive, the `.pkg` and the `.mcpb`; `SHA256SUMS`, over the archives, the `.pkg`, the `.mcpb` and the bundle; and `release.json`, which reads `{version, protocol: 1, targets: {<target>: {url, sha256}}, pkg: {url, sha256}, attestations: {url, sha256}}`.

`brain-<version>.mcpb` is the Claude Desktop extension: `manifest.json` (its version filled from the tag), `icon.png`, and `server/brain`, byte for byte the `brain` of the macOS archive. Its only job is to hand over: Claude Desktop runs it as `brain mcp --door claude-desktop --hand-over`, and it starts the installed brain in its place, or exits saying to install brain first. So the extension never serves from its own, possibly older, copy, and an update to the install reaches it at once. `verify` refuses to publish unless `scripts/verify-mcpb.sh` passes: exactly those three files, that `brain` byte for byte the signed archive's and executable, and the manifest the committed `mcpb/manifest.json` with only its version changed. The `mcpb` tool comes from `mcpb/tool`'s committed lockfile, installed with `npm ci --ignore-scripts`. `mcpb sign` signs it with omit's Developer ID (`MCPB_CERT`, `MCPB_KEY`) in a job of its own on a fresh runner, the only one holding them; a tag without them fails.

`brain --version` prints `brain <version> (protocol 1)`. The protocol changes only when two brains of different releases can no longer work together.

## The install

`brain.pkg` on a Mac, and `scripts/install.sh` on Linux and for CI and scripted installs, put the release folder at `brain-current` inside brain's data folder (`brain install-dir` prints the path) and link `~/.local/bin/brain` to the binary in it. `brain update` replaces that folder whole, so the binary and the runtime library always come from one release. A brain run from outside the install, such as a `cargo build`, refuses to update itself.

`install.sh` copies the new release folder in beside the install before touching it, so replacing the install is two renames on one filesystem; if the second fails, the old install is put back. A version given to it must look like `1.2.3`, checked before anything is downloaded, and every download, redirects included, stays on HTTPS. `scripts/install.test.sh` pins each of these with a stubbed `curl` and release.

## What is trusted, and when

On a Mac, the first install is checked by macOS itself. A downloaded `brain.pkg` carries the quarantine flag, so Gatekeeper checks it when it is opened: signed with omit's Developer ID Installer identity, notarised by Apple, unaltered since. The notarisation ticket is stapled to the package, so the check holds offline too. Nothing from the package runs before that check passes.

On Linux, the first install trusts HTTPS from GitHub and `SHA256SUMS` from the same release: `install.sh` downloads both, checks the archive against `SHA256SUMS`, unpacks it, and then has the brain it just unpacked check the archive's signature against the release key compiled into that brain. A failed check installs nothing. On a first install that signature check guards against nothing the checksum does not: the binary doing the checking came from the same download, and `install.sh` runs it before it knows the signature is good, so an attacker who can replace both the archive and `SHA256SUMS` on GitHub runs code as you. The first install's trust is GitHub's HTTPS and the integrity of the release page, no more.

Every later update is checked by the brain already installed, before anything is replaced: `brain update` downloads the archive and refuses it unless its signature verifies against the key that brain was built with. On macOS it also refuses the release unless the unpacked `lib/libonnxruntime.dylib` meets `anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "5BCNYRT23U"` (`codesign --verify --strict -R`), a Developer ID Application signature of omit's Apple team, compiled into brain from `crates/brain/src/keys/team_id`, and the unpacked `brain` meets the same with `identifier "nz.omit.brain"` in front, so no other program the team signs passes as brain. Each must be a plain file, never a link. So the release key alone cannot put code on a Mac. A build whose key file is the all-zero placeholder ships no key, and refuses to update or to verify anything.

## How the daemon updates itself

The daemon checks for a newer release after its first pass at start, and then once a day. It installs one only through the same path as `brain update`: the archive's signature must verify against the release key built into the running brain (and, on macOS, the new `brain` and its runtime library must be signed by omit's Apple team), and the install folder is replaced whole. Then it prints `installed brain <version>; starting it`, lets go of the index and the vault, and runs the new binary in its own place with the same arguments: on macOS and Linux the same process, so launchd keeps the agent it tracks and a daemon run in a terminal stays in that terminal; on Windows the old process waits for the new one. If the new binary will not start, the daemon exits with an error, and launchd's `KeepAlive` (restart unless the exit was clean) starts it again. A failed check or install prints one line, is counted as an update error, and leaves the old version running until the next day's check. A brain run from anywhere but its install, such as a `cargo build`, says so at each daily check and never updates itself.

## Cutting a release

1. Set `version` under `[workspace.package]` in `Cargo.toml`; every crate takes the workspace version (`every_crate_takes_the_workspace_version`). Commit through a PR.
2. Tag the merged commit `v<version>` and push the tag.
3. `.github/workflows/release.yml` runs:
   - `build`, on macOS, Linux and Windows, checks the tag matches the workspace version, builds `brain` with `--release --locked`, and packs it with ONNX Runtime.
   - `macos-sign`, on a fresh macOS runner and the only job holding the Apple secrets, runs `scripts/sign-macos.sh` on the macOS build: it imports the Developer ID Application and Installer identities into a keychain of its own (removed at the end), signs the runtime library and then `brain`, checks both against the team requirement updates ask for, notarises them, packs the update archive, builds `brain.pkg` from the same folder with `scripts/build-pkg.sh`, signs it with the Installer identity, notarises and staples it. A missing secret, an identity of another team or a refused notarisation fails the tag. `scripts/sign-macos.test.sh` pins these with stub Apple tools.
   - `mcpb` packs the Claude Desktop extension from the signed macOS build, and `mcpb-sign` signs it.
   - `sign`, on Linux, signs every archive with `RELEASE_SIGNING_KEY` (and `RELEASE_SIGNING_KEY_PREVIOUS` when it is set), decoded into a private temporary folder and never printed. It installs zipsign 0.2.2 from crates.io with `cargo install --locked`, so that crate and its lock file are trusted with the key; the checkouts in `sign` and `verify` keep no token on disk.
   - `attest` records build provenance (SLSA, through `actions/attest`, in Sigstore's public log) over the final bytes of every archive, `brain.pkg` and the `.mcpb`, and keeps the bundle as `brain-<version>.sigstore.json`.
   - `draft` writes `SHA256SUMS` and `release.json` and drafts the release.
   - `verify`, on macOS, builds `brain` from the tag, refuses to go on if its release key is the placeholder, downloads every asset of the draft, checks `SHA256SUMS`, runs `brain release-verify` on every archive (the same check clients run, so a signature that verifies only where it was made fails here), checks the `.mcpb`, then checks Apple's signatures as a Mac would (`pkgutil --check-signature`, `spctl --assess --type install`, `stapler validate` on `brain.pkg`; the notarised requirement and the update check's own requirements on the macOS `brain` and runtime library, kept the same as the update check's by `the_release_checks_what_an_update_checks`) and every attestation, and only then publishes the release as a pre-release.
4. Run the release's Live check (the plan's, or at least an update from the previous release on a Mac), then mark it the latest: `gh release edit v<version> --repo omit-nz/brain --latest --prerelease=false`. Installs and updates take a pre-release already; marking it latest moves GitHub's latest-release link to it.

The Apple secrets on `omit-nz/brain` are `MACOS_APP_CERT_P12` and `MACOS_APP_CERT_PASSWORD` (the Developer ID Application identity, as a base64 `.p12`), `MACOS_INSTALLER_CERT_P12` and `MACOS_INSTALLER_CERT_PASSWORD` (the Developer ID Installer identity), `MCPB_CERT` and `MCPB_KEY` (the Application identity as PEM), and the notary API key as `NOTARY_KEY_P8`, `NOTARY_KEY_ID` and `NOTARY_ISSUER_ID`. omit sets them from its own 1Password vault. The team id is not a secret: it is `crates/brain/src/keys/team_id`.

A release that fails any job stays a draft. Delete the draft and the tag, fix, and tag again.

## Checking a brain release yourself

From the release's assets, in one folder:

```bash
shasum -a 256 -c SHA256SUMS
spctl --assess --type install -vv brain.pkg            # source=Notarized Developer ID
pkgutil --check-signature brain.pkg                    # Developer ID Installer, team 5BCNYRT23U
gh attestation verify brain.pkg -R omit-nz/brain --bundle brain-<version>.sigstore.json \
  --signer-workflow omit-nz/brain/.github/workflows/release.yml
brain release-verify brain-<version>-<target>.tar.gz   # exit 0 when signed by the release key
```

`gh attestation verify` works for every archive and the `.mcpb` the same way; with `--bundle` it needs no GitHub sign-in. With `--signer-workflow` it also checks the attestation was made by this repository's release workflow, and is logged in Sigstore's public transparency log. Inside the macOS archive, `codesign --verify --strict -R='identifier "nz.omit.brain" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "5BCNYRT23U"' brain` is the check an installed brain makes before it takes an update.

The release key's public half is `crates/brain/src/keys/release.pub`, 32 bytes, in hex `a2dd8a575985a7598b68478c36b9d67ed14649985af03dd1796579043d2dd2f6`. How it is rotated, and what happens when it leaks, is under The signing key below. Apple revokes a Developer ID certificate at omit's request, and a revoked certificate fails Gatekeeper's check of a downloaded `brain.pkg`. An installed brain's update check looks up no revocation, so a stolen Developer ID is answered by rotating the release key as well: an update needs both.

## The macOS package

`brain.pkg` installs a release into the current person's home only, with no admin password: the release folder becomes `brain-current` in brain's data folder (`~/Library/Application Support/nz.omit.brain/bin/`), and its postinstall links `~/.local/bin/brain` to the binary, the same link `scripts/install.sh` makes. It starts nothing; `brain init`, run from the installed brain, starts the daemon. `pkg/distribution.xml` and `pkg/scripts/postinstall` define it, `scripts/build-pkg.sh` builds it, and CI's `pkg` job builds it unsigned, checks its payload carries no `._` files, installs it into the runner's home as an unprivileged user and runs the installed `brain`. A repository rule keeps the package home-only (`the_pkg_installs_for_the_user_only`). Every release from v0.2.0 carries it signed, notarised and stapled, made by `macos-sign` above; there is no unsigned package on a release page.

## Rolling back

`brain update` only moves to a newer version. To roll back, cut a new release from the last good commit with a higher version number, so every install moves forward onto it. To take a bad release away before many installs reach it, mark it a draft again in GitHub; installs and updates then see the release before it as the newest.

To put one machine back on an older release, run `scripts/install.sh <older version>`.

## The signing key

The key is a zipsign ed25519 key pair. `scripts/release-key.sh init` makes it once: the private key goes base64-encoded to 1Password (item `brain release signing key`, vault `Omit-Infra`) and to the repo secret `RELEASE_SIGNING_KEY`, both on stdin only, and the public key is written to `crates/brain/src/keys/release.pub` to be committed. The script never puts the private key in an argument, on stdout or on stderr, and removes its temporary folder on exit; `scripts/release-key.test.sh` pins that with stubs for `zipsign`, `op` and `gh`.

Before it makes anything, `init` refuses when the public key file is not the placeholder, when 1Password already holds an item with that title, or when the repo already has `RELEASE_SIGNING_KEY`, so a second run from another checkout cannot replace a key a build already trusts.

`scripts/release-key.sh rotate` makes a new key while keeping the old one for one release: it needs exactly one `brain release signing key` item, copies that old private key from 1Password into `RELEASE_SIGNING_KEY_PREVIOUS`, sets the new key as `RELEASE_SIGNING_KEY`, writes the new public key, renames the old 1Password item as retired, and stores the new key under the plain title. The next release is signed by both keys and its binary trusts only the new one, so installs that trust only the old key can still take it. Once that release is out, delete `RELEASE_SIGNING_KEY_PREVIOUS`. If the old key leaked, rotate anyway. A Mac still refuses any update its Apple signature does not tie to omit's team; treat every Linux install as untrusted until it has been reinstalled with `install.sh`.

### A run that stopped

Both commands store the new key first as `brain release signing key (pending)` and give it the plain title only as their last step, after the repo secrets and `release.pub` are written. While a pending item exists, every run refuses. To repair:

- If `release.pub` did not change, the run stopped before or at setting `RELEASE_SIGNING_KEY`. Delete the pending item in 1Password and run the same command again. A rotate that stopped here already copied the old key to `RELEASE_SIGNING_KEY_PREVIOUS`, and the next run copies the same old key again.
- If `release.pub` changed, every secret was set and only the renames are left. After a rotate, rename the old item to `brain release signing key (retired <date>)` if that has not happened, then rename the pending item to `brain release signing key`. After an init, rename the pending item.

`release-key.test.sh` and `install.test.sh` run on `ubuntu-24.04` in CI and on macOS by hand; they are not run under Git Bash on Windows. Running the key script needs zipsign 0.2.2 (`cargo install zipsign --version 0.2.2 --locked`), `jq`, `op` signed in to the account that holds `Omit-Infra`, and `gh` signed in with admin rights on `omit-nz/brain`.
