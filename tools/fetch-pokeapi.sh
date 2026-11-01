#!/usr/bin/env bash
# Sparse-checkout PokéAPI's CSV source data (data/v2/csv) at a pinned commit into tools/pokeapi.
# The checkout is git-ignored; what the repository commits is the derived
# data/dex/official-index.json (see tools/build-official-index.py), whose provenance records
# the commit. Bump POKEAPI_COMMIT deliberately, together with regenerating the index.
set -euo pipefail
cd "$(dirname "$0")/.."

PIN="${POKEAPI_COMMIT:-6bbd96bb8ef7356963b833074ca25672d23e41c4}"
DIR=tools/pokeapi

if [ ! -d "$DIR/.git" ]; then
  git clone --quiet --filter=blob:none --sparse https://github.com/PokeAPI/pokeapi.git "$DIR"
fi
git -C "$DIR" sparse-checkout set data/v2/csv >/dev/null
if ! git -C "$DIR" cat-file -e "$PIN^{commit}" 2>/dev/null; then
  git -C "$DIR" fetch --quiet origin "$PIN"
fi
git -C "$DIR" checkout --quiet "$PIN"
echo "$DIR at $(git -C "$DIR" rev-parse HEAD) ($(git -C "$DIR" log -1 --format=%cs))"
