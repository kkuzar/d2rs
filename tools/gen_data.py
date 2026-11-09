"""Generate d2rs's compact classic 1.14d data tables (crates/d2s/data/*.tsv).

Usage: python3 -I tools/gen_data.py <excel-dir> [out-dir [strings-dir]]
<strings-dir> holds string.tsv, expansionstring.tsv, patchstring.tsv (the extract example writes them from the .tbl
files); display names are looked up there by the txt string keys. Skill names come from SkillDesc.txt `str name` (needs SkillDesc.txt in <excel-dir>).
<excel-dir> is the exact 1.14d data/global/excel directory of .txt tables (any file name case), extracted from
the game MPQs. A column "out=src" is read from src and written as out. Rows named "Expansion" are dropped so row
order equals the in-game record index; blank-named affix rows are kept for the same reason (saves store row
index + 1). The classic save widths of the ItemStatCost rows that D2R changed are asserted, not patched.
"""
import os, sys


def blk(n, *f):
    return [c.format(i=i) for i in range(1, n + 1) for c in f]


AFFIX = ["Name", "version", "spawnable", "rare", "level", "maxlevel", "levelreq", "classspecific", "class", "frequency",
         "group"] + blk(3, "mod{i}code", "mod{i}param", "mod{i}min", "mod{i}max") + blk(7, "itype{i}") + blk(5, "etype{i}") + ["transformcolor"]
RARE = ["name", "version"] + blk(7, "itype{i}") + blk(4, "etype{i}")

TABLES = {
    "isc": (["itemstatcost"], ["ID", "Stat", "Save Bits", "Save Add", "Save Param Bits", "ValShift", "Encode",
            "CSvBits", "CSvParam", "CSvSigned", "1.09-Save Bits", "1.09-Save Add", "descpriority", "descfunc",
            "descval", "descstrpos", "descstr2"]),
    "items": (["weapons", "armor", "misc"], ["kind", "code", "name", "type", "type2", "invwidth", "invheight",
              "invfile", "uniqueinvfile", "setinvfile", "stackable", "maxstack", "durability", "nodurability",
              "quest", "compactsave", "questdiffcheck", "gemsockets", "levelreq", "reqstr", "reqdex", "minac", "maxac", "mindam",
              "maxdam", "belt", "version", "InvTrans", "normcode", "ubercode", "ultracode", "level", "magic lvl", "auto prefix",
              "spawnable", "namestr", "2handmindam", "2handmaxdam", "minmisdam", "maxmisdam", "minstack", "spawnstack"]),
    "itemtypes": (["itemtypes"], ["Code", "ItemType", "Equiv1", "Equiv2", "BodyLoc1", "BodyLoc2", "Beltable",
                  "MaxSock1", "MaxSock25", "MaxSock40", "VarInvGfx", "InvGfx1", "InvGfx2", "InvGfx3", "InvGfx4",
                  "InvGfx5", "InvGfx6", "Class", "Throwable", "Rare", "Magic", "Normal", "StaffMods", "Quiver"]),
    "uniques": (["uniqueitems"], ["index", "code", "lvl req", "invfile", "version", "enabled", "rarity", "lvl"] + blk(12, "prop{i}", "par{i}", "min{i}", "max{i}") + ["invtransform"]),
    "setitems": (["setitems"], ["index", "set", "item", "lvl req", "invfile", "rarity", "lvl", "add func"] + blk(9, "prop{i}", "par{i}", "min{i}", "max{i}")
                 + [c.format(i=i, b=b) for i in range(1, 6) for b in "ab" for c in ("aprop{i}{b}", "apar{i}{b}", "amin{i}{b}", "amax{i}{b}")] + ["invtransform"]),
    "runes": (["runes"], ["Name", "*Rune Name=Rune Name", "complete", "Rune1", "Rune2", "Rune3", "Rune4", "Rune5", "Rune6", "server"]
              + blk(6, "itype{i}") + blk(3, "etype{i}") + blk(7, "T1Code{i}", "T1Param{i}", "T1Min{i}", "T1Max{i}")),
    "magicprefix": (["magicprefix"], AFFIX),
    "magicsuffix": (["magicsuffix"], AFFIX),
    "automagic": (["automagic"], [c for c in AFFIX if c not in ("classspecific",)]),
    "rareprefix": (["rareprefix"], RARE),
    "raresuffix": (["raresuffix"], RARE),
    "properties": (["properties"], ["code"] + blk(7, "set{i}", "val{i}", "func{i}", "stat{i}")),
    "qualityitems": (["qualityitems"], None),
    "lowqualityitems": (["lowqualityitems"], None),
    "belts": (["belts"], ["name", "numboxes"]),
    "experience": (["experience"], ["Level", "Amazon"]),
    "charstats": (["charstats"], ["class", "str", "dex", "int", "vit", "stamina", "hpadd", "LifePerLevel",
                  "ManaPerLevel", "StaminaPerLevel", "LifePerVitality", "ManaPerMagic", "StaminaPerVitality",
                  "StatPerLevel"]),
    "skills": (["skills"], ["skill", "*Id=Id", "charclass", "reqlevel", "skillkey", "itypea1"]),
    "hireling": (["hireling"], ["Hireling", "Id", "Class", "Act", "Difficulty", "Level", "NameFirst", "NameLast"]),
}
# Row counts of tables d2rs does not ship, written to counts.rs: the table index stats (limits::prop_errors) are bounded by them.
COUNTS = {"STATES": "states", "MONSTATS": "monstats", "MONTYPE": "montype"}
CLASSIC = {"36": ("8", "0"), "37": ("8", "0"), "39": ("8", "50"), "41": ("8", "50"), "43": ("8", "50"), "45": ("8", "50")}
SOURCES = sorted({f for files, _ in TABLES.values() for f in files} | {"skilldesc", "itemstatcost"} | set(COUNTS.values()))


def strings(d):
    """key -> text from the game string tables, patchstring over expansionstring over string."""
    m = {}
    for f in ("string", "expansionstring", "patchstring"):
        m.update(l.rstrip("\n").split("\t", 1) for l in open(os.path.join(d, f + ".tsv"), encoding="utf-8") if "\t" in l)
    return m


# table -> (column holding the string key, column that receives the display text)
NAMES = {"items": ("namestr", "name"), "uniques": ("index", "index"), "setitems": ("index", "index"),
         "runes": ("Name", "*Rune Name"), "magicprefix": ("Name", "Name"), "magicsuffix": ("Name", "Name"),
         "automagic": ("Name", "Name"), "rareprefix": ("name", "name"), "raresuffix": ("name", "name")}


def rows(src, f):
    path = os.path.join(src, next(n for n in os.listdir(src) if n.lower() == f + ".txt"))
    lines = [l.rstrip("\r\n").split("\t") for l in open(path, encoding="latin1") if l.strip("\r\n")]
    keep = f in ("magicprefix", "magicsuffix", "automagic", "uniqueitems", "montype")
    return lines[0], [dict(zip(lines[0], r)) for r in lines[1:] if (r[0] or keep) and r[0] != "Expansion"]


def main(src, out, strs):
    tbl = strings(strs)
    desc = {r["skilldesc"]: r["str name"] for r in rows(src, "skilldesc")[1]}
    os.makedirs(out, exist_ok=True)
    stat_id = {r["Stat"]: r["ID"] for r in rows(src, "itemstatcost")[1]}
    for name, (files, cols) in TABLES.items():
        body = []
        for f in files:
            head, rs = rows(src, f)
            cols = cols or [h for h in head if h and not h.startswith("*")]
            for r in rs:
                r["kind"] = f[0]
                for c in cols:
                    if "=" in c:
                        r[c.split("=")[0]] = r.get(c.split("=")[1], "")
                if name in NAMES and r.get(NAMES[name][0]) in tbl:
                    r[NAMES[name][1]] = tbl[r[NAMES[name][0]]].split("\\n")[-1]
                if name == "skills":
                    r["skillkey"] = r["skill"]
                if name == "skills" and r.get("skilldesc") in desc:
                    r["skill"] = tbl.get(desc[r["skilldesc"]], r["skill"])
                if name == "isc" and r["ID"] in CLASSIC:
                    assert (r["Save Bits"], r["Save Add"]) == CLASSIC[r["ID"]], r["ID"]
                if name == "properties":
                    for i in range(1, 8):
                        r[f"stat{i}"] = stat_id.get(r.get(f"stat{i}", ""), "")
                body.append("\t".join(r.get(c.split("=")[0], "").strip() for c in cols))
        cols = [c.split("=")[0] for c in cols]
        open(os.path.join(out, name + ".tsv"), "w", encoding="utf-8").write("\t".join(cols) + "\n" + "\n".join(body) + "\n")
    open(os.path.join(out, "counts.rs"), "w", encoding="utf-8").write("// 1.14d row counts (generated by tools/gen_data.py)\n" + "".join(f"pub const {k}: u32 = {len(rows(src, f)[1])};\n" for k, f in COUNTS.items()))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "crates/d2s/data", sys.argv[3] if len(sys.argv) > 3 else sys.argv[1])
