#!/bin/sh
# Public-repo hygiene: no test code or test data (the suite is private), and the game is named only once, in README.md.
set -eu
cd "$(dirname "$0")/.."
git rev-parse --is-inside-work-tree >/dev/null
g() { git grep "$@" || [ $? = 1 ]; }   # exit 1 = no match (fine); anything else is a real git failure
bad=$( { g -nE '#\[ *([a-z_]+::)*(test|bench|ignore) *\]|cfg(_attr)?\(([^]]*[^a-z_])?test[^a-z_]|dev[-_]dependencies' -- '*.rs' '*.toml'
         git ls-files | grep -iE '(^|/)(tests|test_assets|benches)/|\.(d2s|d2i|d2x|sss|key|ma[0-9]|map|bak|org|mpq|dc6|dcc|ds1|dt1|cof|exe|dll|tbl|pl2|dat)$' || [ $? = 1 ]
         g -niE 'Diablo|Lord of Destruction|Blizzard' -- ':!README.md' ':!licenses/README.md' ':!crates/d2s/data' ':!tools/check_public.sh' \
           ':!docs' ':!goal_log.md' ':!opus_5_*.md' ':!fable51_*.md' ':!falbe51_plan.md' | grep -vE '// real (folder|process) name' || [ $? = 1 ]
         g -nE '/Users/|/home/[a-z]' -- ':!tools/check_public.sh' ':!docs' ':!goal_log.md' ':!opus_5_*.md' ':!fable51_*.md' ':!falbe51_plan.md' || [ $? = 1 ]
         [ "$(grep -o 'Diablo' README.md | wc -l | tr -d ' ')" = 1 ] || echo "README.md: name the game exactly once"; } )
[ -z "$bad" ] || { printf 'public hygiene check failed:\n%s\n' "$bad"; exit 1; }
