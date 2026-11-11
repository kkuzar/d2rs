# Credits and licenses

d2rs is licensed under Apache-2.0 (see ../LICENSE). It contains no code, text or data copied from the projects
below; they are listed because d2rs learned from them.

| Project | License | What d2rs learned from it |
|---|---|---|
| [IgorKonovalov/diablo-2-recompilation](https://github.com/IgorKonovalov/diablo-2-recompilation) | none stated | The 1.14d reverse-engineering documentation: how the game loads and saves characters, item and property rules, the map generator (DRLG), table hashes. d2rs implements these facts in its own code. |
| [WalterCouto/D2CE](https://github.com/WalterCouto/D2CE) | GPL-3.0 | Save-format notes (`d2s_File_Format.md`), version numbers, and editor behaviour to compare against. No D2CE code or data is in d2rs. |
| [dschu012/d2s-editor](https://github.com/dschu012/d2s-editor), [dschu012/d2s](https://github.com/dschu012/d2s) | MIT, none stated | The idea of a visual inventory grid. |
| [daidodo/diablo_edit](https://github.com/daidodo/diablo_edit) | MIT | Its parse failures (golem and realm data), which showed which save fields an editor must handle. |
| [ladislav-zezula/StormLib](https://github.com/ladislav-zezula/StormLib) | MIT | The MPQ archive format: hash, encryption and file-key rules, and known test values. |
| [blacha/diablo2](https://github.com/blacha/diablo2) | MIT | MPQ format knowledge and known test values. |

The Rust crates compiled into the Windows exe and the web version keep their own licenses; their notices are in
[dependencies.json](dependencies.json).

d2rs ships no game archives, art or executables. `crates/d2s/data` holds column subsets (including item names) derived from the game's 1.14d data tables; everything else it shows is read from the user's own installation at runtime. The game's name and the
trademark notice are in the main README.
