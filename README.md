<p align="center"><img src="app/icons/icon.png" width="160" alt="d2rs logo"></p>

[![CI](https://github.com/kkuzar/d2rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/kkuzar/d2rs/actions/workflows/ci.yml)
[![Checks](https://img.shields.io/github/checks-status/kkuzar/d2rs/main?label=checks)](https://github.com/kkuzar/d2rs/commits/main)
[![Pages](https://github.com/kkuzar/d2rs/actions/workflows/pages.yml/badge.svg)](https://github.com/kkuzar/d2rs/actions/workflows/pages.yml)
[![Release](https://img.shields.io/github/v/release/kkuzar/d2rs)](https://github.com/kkuzar/d2rs/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/kkuzar/d2rs/total)](https://github.com/kkuzar/d2rs/releases)
[![License](https://img.shields.io/github/license/kkuzar/d2rs)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.95%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Web version](https://img.shields.io/badge/web-open%20d2rs-blue)](https://kkuzar.github.io/d2rs/app/)

# d2rs

d2rs is a save editor for Diablo II and its expansion, game version 1.14d. The game is a trademark of its publisher, Blizzard Entertainment. d2rs is not affiliated with or endorsed by the publisher. A Rust program, d2rs has a Windows app, a web version, and source builds for any operating system.

d2rs edits `.d2s` character saves and PlugY stash files (`.d2x`, `.sss`). It never corrupts a save file:

- d2rs encodes every file again in memory and compares it byte for byte with the original before it allows editing.
- d2rs validates every write against the load rules of the game.
- d2rs backs up the old files before a write.
- d2rs writes the new file to a temporary file and renames it in one atomic step.
- d2rs reads the written file back and checks it.

d2rs refuses to write while the game runs, because the game overwrites the save file at Save & Exit. The project is feature complete, and it includes item generation and the seed map. Website: <https://kkuzar.github.io/d2rs/>

## Pick your game folder: everything works much better with it

d2rs ships only the core game tables: the save format, base items, item types, skills and experience. The other tables (unique, set, magic and rare names and properties, runewords) come from **your own** 1.14d game folder, which holds the MPQ files (`patch_d2.mpq`, `d2exp.mpq`, `d2data.mpq`). Use "Game folder..." in the program (or `--game dir` / `D2RS_GAME_DIR` for the CLI).

| | Without the game folder | With the game folder |
|---|---|---|
| Open, check and save a character or stash | yes | yes |
| Edit gold, level, experience, stats, skills, quests, waypoints | yes | yes |
| Move, duplicate and delete items | yes | yes |
| Item names (unique, set, magic, rare, runeword) | base name only | full names |
| Create, reroll, import items; trait editor; legit check | no | yes |
| Item pictures and the map | no | yes |

## Get d2rs

- **Web version:** open the [web version](https://kkuzar.github.io/d2rs/app/). Nothing is installed, and no file leaves your computer. It edits save files (character, items, stash) and downloads the edited files. It has no Live tab and no Backups tab, and it cannot write into your save folder.
- **Windows app:** download `d2rs_windows_x64.exe` from [Releases](https://github.com/kkuzar/d2rs/releases) and run it. The file is portable, and it needs no installer.
- **Source code (any operating system):** install Rust with [rustup](https://rustup.rs), then do one of these procedures.

Procedure A, install with Cargo:

1. If you use Linux, install `libxkbcommon-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.
2. Run `cargo install --git https://github.com/kkuzar/d2rs --locked d2rs`.
3. Run `d2rs`. Cargo puts the program in `~/.cargo/bin`.

Procedure B, build from a clone:

1. Install Git LFS (`git lfs install`), then clone the repository. The icon images need it.
2. If you use Linux, install the packages from procedure A.
3. Run `cargo build --release -p d2rs` (or `make app`).
4. Run `target/release/d2rs` (`target/release/d2rs.exe` on Windows).

Procedure C, build the web version:

1. Run `rustup target add wasm32-unknown-unknown`.
2. Run `cargo install trunk --version 0.22.0 --locked`.
3. Run `cd app && trunk serve` (or `make start`).
4. Open `http://127.0.0.1:8080/d2rs/app/` in a browser.

## Use d2rs

### GUI

Run `d2rs`, then pick a save folder or a save file. Use "Game folder..." to select your game folder (or an extracted `data` folder). Then the program shows the item art. The tabs are:

| Tab | What you can do |
|---|---|
| Character | name, class, level, experience, status flags, gold |
| Stats | attributes, points, life, mana, stamina |
| Skills | skill levels |
| Quests | quests per act and difficulty; pending rewards are only changed after a warning |
| Waypoints | waypoints per act and difficulty |
| Items | drag and drop between the inventory, stash, cube, belt, equipment, merc, golem and PlugY stash pages; a tooltip and an editor for each item; duplicate, delete, import and export (base64) |
| Items: Create item | make any unique, set item, runeword, magic, rare, crafted, superior or low quality item from the exact 1.14d tables; values random, max, min or custom (always inside the game's range) |
| Items: item editor | seed editor (type a seed or press Reroll: the item is rebuilt the way the game rolls it, the same seed always gives the same item) and trait editor (change, add or remove single properties, quality, ilvl, ethereal, sockets); a "Legit" badge compares the item with the 1.14d tables |
| Map | the character's map seed (or a typed seed): pick difficulty, act and level, view a top-down collision map or the game-style isometric picture, export one level or every level (PNG + JSON). Every level of Acts I to V builds; level 134 (Forgotten Sands) follows a spec branch no trace has verified and is marked "unverified" |
| Live | read-only view of a running 1.14d game; experimental writes are off by default; they need the pinned 1.14d Game.exe, or a modified 1.14d with the same layout after an extra confirmation, and every value stays inside the game range |
| Backups | list backups and restore one |

### CLI

Run `d2rs cli <command>`. The commands are:

| Command | What it does |
|---|---|
| `d2rs cli info <file>` | prints a summary |
| `d2rs cli verify <files...>` | parse, self-check and validate; exit code 1 on any error |
| `d2rs cli roundtrip <files...>` | checks that each file re-encodes byte for byte |
| `d2rs cli export-json <file>` | prints the save as JSON |
| `d2rs cli set <file> gold=100000 level=50 seed=4832 seed-reuse=0:1 ...` | edits a save through validation and the safe commit; every value must be inside the game's range |
| `d2rs cli verify --legit <files...>` | lists items whose properties the game could not have rolled |
| `d2rs cli add-item <file> unique="Harlequin Crest" roll=max` | creates an item (`unique`, `set`, `runeword` with `base=`, `magic`, `rare`, `base`) and places it in the inventory, stash or cube |
| `d2rs cli reroll <file> <item index> [seed=N] [keep=kind\|quality]` | rerolls an item like the game; indexes come from `info` |
| `d2rs cli item-set <file> <item index> <stat>=<value> add=... remove=... eth=0\|1 sockets=N ilvl=N` | edits single traits of an item |
| `d2rs cli seed <N\|file.d2s>` | prints a map seed's level seed bases, Act II tombs and Act III bit |
| `d2rs cli map (--seed N\|--save f.d2s) --diff 0\|1\|2 [--level id] --out dir` | exports collision PNG, isometric PNG and JSON per level (needs the game folder: `--game dir` or `D2RS_GAME_DIR`) |

### Backups

d2rs keeps the backups in the app data folder, in `d2rs/backups/<save folder id>/<character>/`. Each save folder has one backup folder. d2rs keeps the first backup of a character forever.

### Web version limits

The web version runs in your browser and does not upload your files. It has no Live tab and no Backups tab. When you save, the browser downloads the edited files. Copy them into your save folder and replace the old files. Do this while the game is closed.

## Supported files

| File | Support |
|---|---|
| `.d2s` v96 (1.10 to 1.14d) | full editing |
| `.d2s` v92..95 (1.09) | editing; items stay read-only if their self-check fails |
| `.d2s` v87, v89 (1.07, 1.08) | experimental |
| `.d2s` v71 (1.00 to 1.06) | experimental; items are read-only raw records |
| Convert old saves to 1.14d | load the old save in 1.14d and Save & Exit: the game converts it, then edit the v96 file |
| PlugY `<char>.d2x` and `_LOD_SharedStashSave.sss` | full editing |
| D2R (v97+) | not supported |

## Game data and game ranges

- d2rs limits every editable value to the range that the 1.14d game uses (`crates/d2s/src/limits.rs`), not only to the range that the file can store.
- d2rs refuses to write a save file that holds a value that the game would overflow, reset or delete. An example is a runeword whose sockets do not match its runes.
- Item generation, the legit check and the map generator follow the decompiled 1.14d behaviour. This includes the draw order of the random number generator of the game. Maps match the reference traces exactly for the tested seeds.
- Maps and item art need your own 1.14d installation (the MPQ archives). The map generator also reads one table from `Game.exe`.
- This repository holds no game archives, art or executables. It holds only the tables in `crates/d2s/data/`.
- The tables in `crates/d2s/data/` are column subsets that come from the 1.14d data tables of the game. They include item, affix and skill names. The parser needs them for the bit widths.
- d2rs reads everything else from your installation when it runs. For item art, point the program at the game folder or at an extracted data folder.

## Build and test

The test suite is private. The maintainers run it, and it reports the `private-tests` status on each commit. See [CONTRIBUTING.md](CONTRIBUTING.md).

To make a release (maintainers only):

1. Change `version` in `Cargo.toml` through a pull request.
2. Wait until the pull request is merged and `private-tests` is green.
3. Run `gh workflow run release --ref main`.

## Project layout

| Path | Content |
|---|---|
| `crates/d2s` | Lossless `.d2s` and PlugY codec, data tables, validation. It does no file input or output. |
| `crates/d2io` | Safe commit protocol: game detection, backups, atomic write, read-back check, rename. |
| `crates/d2data` | Decoders for game assets: DC6 images, palettes, colour maps, `.tbl` strings. |
| `crates/d2map` | Pure Rust level generator for 1.14d (seed tree, preset, maze and outdoor levels, act links, room tiles, collision) and the map renderers. |
| `crates/d2mem` | Optional runtime inspector for a running 1.14d `Game.exe` (Windows, Linux with Wine, macOS best effort). |
| `crates/d2host` | One layer for files, the clock and background jobs (the standard library on desktop, memory in the browser). |
| `app` | The `d2rs` program: the desktop UI (eframe and egui), the web version and the `d2rs cli` commands. |
| `tools/gen_data.py` | Regenerates `crates/d2s/data/*.tsv` from a folder of excel `.txt` tables. |

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before you open a pull request. Follow the [code of conduct](CODE_OF_CONDUCT.md). To report a security problem, read [SECURITY.md](SECURITY.md).

## Licence

d2rs uses the Apache-2.0 licence (see [LICENSE](LICENSE)). The [licenses/](licenses/) folder holds the credits, the projects that d2rs learned from, and the notices of the Rust crates that d2rs uses.
