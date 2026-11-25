# Security

## Reporting a problem

Use GitHub's private vulnerability reporting on this repository (Security, then Report a vulnerability). Please do not open a public issue for a security problem.

## How a release is protected

The answers below follow what NIST SP 800-218 v1.1 (the Secure Software Development Framework, final, February 2022) and the CISA secure software self-attestation form ask, as read on 25 November 2026. Each says what is done today and what is not done yet.

- Integrity of every update (PS.1.1, PS.2.1). Every release archive is signed with brain's release key (Ed25519, through zipsign), and an installed brain refuses an update whose signature does not verify against the key it was built with. A build without the key refuses to update at all. `docs/release.md` covers the key, its rotation and how a release is checked before it is published.
- Signing through an established certificate authority (PS.2.1). Not done yet. On macOS, `brain` and its runtime library will be signed with an Apple Developer ID and notarised, the installer `brain.pkg` signed with a Developer ID Installer identity, notarised and stapled, and updates refused unless Apple's signature names the same team. This waits on the Developer ID certificates. Windows gets an Authenticode-signed installer in a later phase.
- Information to check a release yourself (PS.2.1). `SHA256SUMS` and `release.json` ship with every release; `brain release-verify <archive>` checks an archive's signature. `docs/release.md` says how to check a release by hand.
- Provenance (PS.3.2). Not done yet: build attestations through GitHub (Sigstore, SLSA Build Level 2) are planned for the release job, with the attestation bundle published beside each release.
- Builds run on GitHub-hosted runners only; a repository rule refuses a self-hosted runner, so a public pull request never runs on omit's own machines.

## What brain sends

brain sends one report a day of counts of its own use, signed with a key made for each install, and never note text, titles, questions or paths. `docs/usage.md` lists every field; `brain usage off` stops it.

## Not done yet

- Reproducible builds and a published SBOM (later).
- On Linux there is no package yet, so the first install trusts GitHub's HTTPS and the release's own `SHA256SUMS`; `docs/release.md` says exactly what that first install trusts.
