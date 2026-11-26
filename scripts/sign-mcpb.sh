#!/usr/bin/env bash
# Signs a .mcpb so that every zip reader still opens it. `mcpb sign` appends its signature block
# after the zip's end record without declaring it, and a strict reader (yauzl, which Claude
# Desktop uses) refuses a zip with undeclared bytes at its end. The block's length is fixed for a
# given certificate and key, so a trial signature measures it; the end record then declares a
# comment of exactly that length, and the real signature, made over the zip as declared, becomes
# that comment. `mcpb verify` still checks it: the signature covers every byte before the block.
#
# Usage: scripts/sign-mcpb.sh <file.mcpb> <cert.pem> <key.pem>
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 <file.mcpb> <cert.pem> <key.pem>" >&2
  exit 2
fi
mcpb=$1
cert=$2
key=$3
tool="$(cd "$(dirname "$0")/.." && pwd)/mcpb/tool/node_modules/.bin/mcpb"
work=$(mktemp -d "${TMPDIR:-/tmp}/sign-mcpb.XXXXXX")
trap 'rm -rf "$work"' EXIT

size() { wc -c <"$1" | tr -d ' '; }

# A zip with no comment ends with its 22-byte end record; its last two bytes are the comment length.
unsigned=$(size "$mcpb")
eocd=$((unsigned - 22))
if [ "$(od -An -tx1 -j "$eocd" -N4 "$mcpb" | tr -d ' \n')" != 504b0506 ]; then
  echo "$mcpb does not end with a comment-free zip end record" >&2
  exit 1
fi

cp "$mcpb" "$work/trial.mcpb"
"$tool" sign "$work/trial.mcpb" -c "$cert" -k "$key" >/dev/null
block=$(($(size "$work/trial.mcpb") - unsigned))
test "$block" -gt 0 && test "$block" -le 65535

printf '%b' "\\x$(printf %02x $((block & 255)))\\x$(printf %02x $((block >> 8)))" |
  dd of="$mcpb" bs=1 seek=$((eocd + 20)) conv=notrunc 2>/dev/null
"$tool" sign "$mcpb" -c "$cert" -k "$key" >/dev/null
if [ $(($(size "$mcpb") - unsigned)) -ne "$block" ]; then
  echo "the signature changed length between the trial and the real one" >&2
  exit 1
fi
