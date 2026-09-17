//! Writes precomputed round packs for the server to serve and validate against.
//!
//!   cargo run --release --bin export_rounds -- <out-dir> [count] [first-round] [--all-months]
//!
//! Without a flag it writes the default rounds, `<out-dir>/pack-N.json`. With
//! `--all-months` it also writes each month's rounds, favouring that month's
//! seasonal theme, to `<out-dir>/month-MM/pack-N.json`. The server serves a
//! month's rounds when they are uploaded and the default ones otherwise.

use std::path::{Path, PathBuf};

fn write_packs(out: &Path, first: u64, count: u64, month: Option<u32>) -> std::io::Result<usize> {
    std::fs::create_dir_all(out)?;
    let packs = word_legend_app::export::build_packs(first, count, month);
    let mut total = 0;
    for (index, json) in &packs {
        let path = out.join(format!("pack-{index}.json"));
        std::fs::write(&path, json)?;
        total += json.len();
        println!("  {} ({:.0} KB)", path.display(), json.len() as f64 / 1024.0);
    }
    Ok(total)
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all_months = args.iter().any(|a| a == "--all-months");
    let mut positional = args.iter().filter(|a| !a.starts_with("--"));
    let out: PathBuf = positional.next().map(PathBuf::from).unwrap_or_else(|| "rounds".into());
    let count: u64 = positional.next().and_then(|a| a.parse().ok()).unwrap_or(1_000);
    let first: u64 = positional.next().and_then(|a| a.parse().ok()).unwrap_or(0);

    println!("building {count} default rounds from {first}...");
    let mut total = write_packs(&out, first, count, None)?;
    if all_months {
        for month in 1..=12 {
            let season = word_legend_app::rotation::season_for_month(month).unwrap_or("none");
            println!("building month {month:02} ({season})...");
            total += write_packs(&out.join(format!("month-{month:02}")), first, count, Some(month))?;
        }
    }

    println!("{:.1} MB total. Upload with server/upload-rounds.sh", total as f64 / 1_048_576.0);
    Ok(())
}
