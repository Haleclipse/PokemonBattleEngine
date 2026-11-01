#!/usr/bin/env bash
# The committed data is what its exporters give today: dex snapshots, formats and legality data
# are exported afresh from the pinned oracle (tools/oracle-node, `npm ci` done) into
# target/data-refresh/ and compared file by file with data/ and tests/fixtures/formats/. Any
# difference fails, listed. The dex registry is copied in first: ids are never renumbered.
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/data-refresh
rm -rf "${out:?}"
mkdir -p "$out/dex" "$out/formats" "$out/legality" "$out/fixtures"
cp data/dex/ids.json data/dex/official-index.json data/dex/names-zh.json "$out/dex/"

echo "== export"
(cd tools/oracle-node &&
  node src/export-dex.mjs "../../$out/dex" > /dev/null &&
  node src/export-formats.mjs "../../$out/formats" "../../$out/fixtures" > /dev/null &&
  node src/export-legality.mjs "../../$out/legality" "../../$out/fixtures" > /dev/null)

echo "== compare"
status=0
compare() { # <exported dir> <committed dir>: every exported file against its committed copy
  local file
  for file in "$1"/*; do
    if ! cmp -s "$file" "$2/$(basename "$file")"; then
      echo "differs: $2/$(basename "$file")"
      status=1
    fi
  done
}
compare "$out/dex" data/dex
compare "$out/formats" data/formats
compare "$out/legality" data/legality
compare "$out/fixtures" tests/fixtures/formats
if [ "$status" -eq 0 ]; then
  echo "data reproduces: $(find "$out" -type f | wc -l | tr -d ' ') files"
fi
exit "$status"
