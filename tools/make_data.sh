#!/bin/sh
# Builds the game tables that the code includes (crates/d2s/data/*.tsv and counts.rs). They are not in git.
# Your own copy of the game: sh tools/make_data.sh "<game folder>"  (needs your MPQ files; see README.md)
# Private copy instead (CI, private test suite): D2RS_DATA_SRC=<folder with the generated files>
set -eu
cd "$(dirname "$0")/.."
out=crates/d2s/data
if [ -n "${D2RS_DATA_SRC:-}" ]; then
	cp "$D2RS_DATA_SRC"/*.tsv "$D2RS_DATA_SRC"/counts.rs "$out"/
	echo "copied the tables from $D2RS_DATA_SRC"
	exit 0
fi
game=${1:?usage: sh tools/make_data.sh "<game folder with your MPQ files>"}
work=target/gamedata
mkdir -p "$work"
files=$(python3 -I -c 'import sys; sys.path.insert(0, "tools"); import gen_data; print(" ".join(gen_data.SOURCES))')
set -- "$game" "$work"
for f in $files; do set -- "$@" "data\\global\\excel\\$f.txt"; done
set -- "$@" 'data\local\lng\eng\string.tbl' 'data\local\lng\eng\expansionstring.tbl' 'data\local\lng\eng\patchstring.tbl'
cargo run -q -p d2data --example extract -- "$@" | tee "$work/extract.txt"
if grep -q MISSING "$work/extract.txt"; then echo "some game files are missing in that folder (see $work/extract.txt)" >&2; exit 1; fi
python3 -I tools/gen_data.py "$work" "$out" "$work"
echo "tables written to $out"
