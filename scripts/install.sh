#!/usr/bin/env bash
# Install brain from a signed release.
#
# Usage: install.sh [<version>]     the newest release when no version is given
#
# Downloads the archive for this machine and SHA256SUMS over HTTPS, checks the archive against
# SHA256SUMS, unpacks it, and has the brain it holds check the archive's signature against the
# release key built into it. Only then does the release folder replace the install, and
# ~/.local/bin/brain link to it. docs/release.md says what this first install trusts.
# macOS on Apple silicon and Linux x86_64; Windows installs arrive in Phase 3.
set -euo pipefail

repo=omit-nz/brain
die() { echo "install.sh: $*" >&2; exit 1; }
fetch() { curl --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 3 -fsSL "$@"; }

case "$(uname -s) $(uname -m)" in
  "Darwin arm64") target=aarch64-apple-darwin ;;
  "Linux x86_64") target=x86_64-unknown-linux-gnu ;;
  *) die "brain publishes no release for $(uname -s) $(uname -m)" ;;
esac

version=${1:-}
# A version goes into URLs and file names, so it is checked before anything is fetched.
release_version() {
  local v=${1#v}
  case "$v" in
    *[!0-9A-Za-z.+-]* | "") die "not a release version: $1" ;;
    [0-9]*.[0-9]*.[0-9]*) echo "$v" ;;
    *) die "not a release version: $1" ;;
  esac
}
if [ -n "$version" ]; then
  version=$(release_version "$version")
else
  latest=$(fetch -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
  version=$(release_version "${latest##*/}")
fi

name="brain-$version-$target.tar.gz"
base="https://github.com/$repo/releases/download/v$version"
work=$(mktemp -d "${TMPDIR:-/tmp}/brain-install.XXXXXX")
stage=
trap 'rm -rf "$work" ${stage:+"$stage"}' EXIT

fetch -o "$work/$name" "$base/$name"
fetch -o "$work/SHA256SUMS" "$base/SHA256SUMS"
expected=$(awk -v n="$name" '$2 == n || $2 == "*" n { print $1 }' "$work/SHA256SUMS")
[ -n "$expected" ] || die "SHA256SUMS does not list $name"
if command -v sha256sum >/dev/null; then
  got=$(sha256sum "$work/$name" | cut -d' ' -f1)
else
  got=$(shasum -a 256 "$work/$name" | cut -d' ' -f1)
fi
[ "$got" = "$expected" ] || die "$name has SHA-256 $got, SHA256SUMS says $expected; nothing installed"

mkdir "$work/unpacked"
tar -xzf "$work/$name" -C "$work/unpacked"
new="$work/unpacked/brain-$version"
[ -x "$new/brain" ] || die "$name holds no brain-$version/brain"
"$new/brain" release-verify "$work/$name" >/dev/null ||
  die "$name is not signed by the brain release key; nothing installed"

# The new release is copied in beside the install first, so the swap is two renames on one
# filesystem, and a failed swap puts the old install back.
dest=$("$new/brain" install-dir)
parent=$(dirname "$dest")
mkdir -p "$parent"
stage=$(mktemp -d "$parent/.brain-new.XXXXXX")
cp -R "$new" "$stage/brain"
if [ -e "$dest" ]; then
  mv "$dest" "$stage/old"
fi
if ! mv "$stage/brain" "$dest"; then
  if [ -e "$stage/old" ] && ! mv "$stage/old" "$dest"; then
    kept=$stage
    stage=
    die "the swap failed; the old install is at $kept/old, rename it to $dest"
  fi
  die "could not move the new release into place; nothing changed"
fi

mkdir -p "$HOME/.local/bin"
ln -sfn "$dest/brain" "$HOME/.local/bin/brain"
"$dest/brain" --version
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) echo "Add $HOME/.local/bin to your PATH to run brain." ;;
esac
