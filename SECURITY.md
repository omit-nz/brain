# Security

## Reporting a problem

Use GitHub's private vulnerability reporting on this repository (Security, then Report a vulnerability). Please do not open a public issue for a security problem.

## How a release is protected

The answers below follow what NIST SP 800-218 v1.1 (the Secure Software Development Framework, final, February 2022) and the CISA secure software self-attestation form ask, as read on 25 November 2026. Each says what is done today and what is not done yet.

- Integrity of every update (PS.1.1, PS.2.1). Every release archive is signed with brain's release key (Ed25519, through zipsign), and an installed brain refuses an update whose signature does not verify against the key it was built with. A build without the key refuses to update at all. `docs/release.md` covers the key, its rotation and how a release is checked before it is published.
- Signing through an established certificate authority (PS.2.1). On macOS, from v0.2.0, `brain` and its runtime library are signed with omit's Apple Developer ID and notarised, the installer `brain.pkg` is signed with its Developer ID Installer identity, notarised and stapled, so Gatekeeper checks it before it runs, and an installed brain refuses an update unless Apple's signature names the same team. Windows gets an Authenticode-signed installer in a later phase.
- Information to check a release yourself (PS.2.1). `SHA256SUMS`, `release.json` and the attestation bundle ship with every release; `brain release-verify <archive>` checks an archive's release-key signature, and `pkgutil`, `spctl` and `gh attestation verify` check the rest. `docs/release.md` ("Checking a brain release yourself") gives the commands.
- Provenance (PS.3.2). Every release asset gets a build attestation through GitHub (Sigstore, SLSA Build Level 2), made by the release workflow on a GitHub-hosted runner and logged in Sigstore's public transparency log; the bundle is published beside each release as `brain-<version>.sigstore.json`.
- Builds run on GitHub-hosted runners only; a repository rule refuses a self-hosted runner, so a public pull request never runs on omit's own machines.

## What brain sends

omit looks at usage, never at notes. brain sends one report a day of counts of its own use, signed with a key made for each install, and never note text, titles, questions or paths. `docs/usage.md` lists every field; `brain usage off` stops it.

## Not done yet

- Reproducible builds and a published SBOM (later).
- Signed installers for Windows (later).
- On Linux there is no package yet, so the first install trusts GitHub's HTTPS and the release's own `SHA256SUMS`; `docs/release.md` says exactly what that first install trusts.
