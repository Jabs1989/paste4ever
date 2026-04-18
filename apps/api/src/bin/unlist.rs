//! `unlist` — hide one or more pastes from the public wall.
//!
//! The paste bytes live on Autonomi forever and stay retrievable by URL;
//! this only flips `listed` from 1 → 0 in the local SQLite index so the
//! /recent endpoint stops returning them. Reversible — run `relist` (or
//! just UPDATE listed=1) to bring them back.
//!
//! Usage:
//!   cargo run --release --bin unlist -- <id-prefix> [<id-prefix> ...]
//!
//! Id-prefix is matched with a trailing `%` so you can pass the short
//! form shown on the wall (e.g. `3930a695`) without copying the full
//! 64-char hash.

use rusqlite::Connection;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: unlist <id-prefix> [<id-prefix> ...]");
        std::process::exit(2);
    }

    let db_path = std::env::var("PASTES_DB").unwrap_or_else(|_| "pastes.db".to_string());
    let conn = Connection::open(&db_path)?;
    println!("Opened {}", db_path);

    for prefix in &args {
        let pattern = format!("{}%", prefix);
        let changed = conn.execute(
            "UPDATE pastes SET listed = 0 WHERE id LIKE ?1 AND listed = 1",
            [&pattern],
        )?;
        if changed == 0 {
            println!("  ({}): no matching listed rows", prefix);
        } else {
            println!("  ({}): hid {} paste(s) from the wall", prefix, changed);
        }
    }

    Ok(())
}
