# brain, by omit

Leave a brain at home. brain is the shared context layer of a business: the knowledge, working memory, history, email and screen context every person and every AI tool in it draws on, kept on the business's own machines. A Rust daemon and CLI with a local MCP plug into the AI the business already pays for, and sync between colleagues goes through a relay that only ever holds ciphertext. Apache-2.0. Built and cared for by [omit](https://omit.nz).

## Install

On macOS (Apple silicon) or Linux (x86_64):

```bash
curl --proto '=https' -fsSL https://raw.githubusercontent.com/omit-nz/brain/main/scripts/install.sh | bash
```

Then `brain update` moves you to each new release once its signature checks. [docs/release.md](docs/release.md) says what is trusted and when.
