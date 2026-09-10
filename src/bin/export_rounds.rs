//! Writes precomputed round packs for the server to serve and validate against.
//!
//!   cargo run --release --bin export_rounds -- <out-dir> [count] [first-round]

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let out: PathBuf = args.next().unwrap_or_else(|| "rounds".into()).into();
    let count: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1_000);
    let first: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);

    std::fs::create_dir_all(&out)?;
    println!("building {count} rounds from {first}...");

    let packs = word_legend_app::export::build_packs(first, count);
    let mut total = 0;
    for (index, json) in &packs {
        let path = out.join(format!("pack-{index}.json"));
        std::fs::write(&path, json)?;
        total += json.len();
        println!("  {} ({:.0} KB)", path.display(), json.len() as f64 / 1024.0);
    }

    println!(
        "{} packs, {:.1} MB total. Upload with server/upload-rounds.sh",
        packs.len(),
        total as f64 / 1_048_576.0
    );
    Ok(())
}
