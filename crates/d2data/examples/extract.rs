//! Extracts files from a game install and prints their size and sha256.
//! `cargo run -p d2data --example extract -- <game dir> <out dir> <mpq path>...`
#![allow(clippy::disallowed_methods)]
use sha2::{Digest, Sha256};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let gd = d2data::GameData::from_install(&a[0]).expect("open MPQs");
    for p in &a[2..] {
        let Some(b) = gd.read(p) else { println!("{p}\tMISSING"); continue };
        std::fs::write(std::path::Path::new(&a[1]).join(p.rsplit(['\\', '/']).next().unwrap_or(p)), &b).expect("write");
        if p.ends_with(".tbl") {
            let t = d2data::Tbl::parse(&b).expect("tbl");
            let tsv: String = t.iter().map(|(k, v)| format!("{k}\t{}\n", v.replace('\t', " ").replace('\r', "").replace('\n', "\\n"))).collect();
            std::fs::write(std::path::Path::new(&a[1]).join(p.rsplit(['\\', '/']).next().unwrap_or(p).replace(".tbl", ".tsv")), tsv).expect("write");
        }
        println!("{p}\t{}\t{:x}", b.len(), Sha256::digest(&b));
    }
}
