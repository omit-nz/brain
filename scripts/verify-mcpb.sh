#!/usr/bin/env bash
# Checks a brain .mcpb as a client would receive it: a zip Claude Desktop's own reader opens, signed
# (mcpb/tool/zip-check.mjs, which needs `npm ci --ignore-scripts --prefix mcpb/tool`), exactly
# the three files the release job packs, the server binary byte for byte the release's and
# executable, and the manifest the committed mcpb/manifest.json with only its version set.
# Anything a packer could slip in (a repointed command, an extra file, another argument) is refused.
#
# Usage: verify-mcpb.sh <brain-VERSION.mcpb> <the release's brain binary> <VERSION>
set -euo pipefail

mcpb=$1
binary=$2
version=$3
here=$(cd "$(dirname "$0")" && pwd)
ext=$(mktemp -d)
trap 'rm -rf "$ext"' EXIT
node "$here/../mcpb/tool/zip-check.mjs" "$mcpb"
unzip -q "$mcpb" -d "$ext"
diff <(printf '%s\n' icon.png manifest.json server/brain) <(cd "$ext" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
cmp "$binary" "$ext/server/brain"
test -x "$ext/server/brain"
diff <(jq -S --arg v "$version" '.version = $v' "$here/../mcpb/manifest.json") <(jq -S . "$ext/manifest.json")
