//! Packs the flag drawings into the blob baked into the exe.
//!
//! Run it through the build container:
//!
//!     make flags
//!
//! They go in as gzipped SVG rather than as pictures of a chosen size, because
//! a print can be any size and any density: the program renders each flag at
//! exactly the height the caption needs. Compressed, the whole set is smaller
//! than one fixed-size raster of it would be.
//!
//! Flags are not under copyright — they are state symbols — and these drawings
//! come from a public-domain collection derived from Wikimedia Commons. The
//! credit still travels with the program, in the README and the About panel.

use std::io::Write;

use anyhow::{bail, Context, Result};

/// ```text
/// magic   b"S4PF"
/// u8      format version (2)
/// u16     flag count
///   [u8;2] ISO 3166-1 alpha-2, lower case
///   u32    gzipped SVG length
///   ..     gzipped SVG bytes
/// ```
/// All integers little-endian. Kept deliberately dull: the reader in
/// `sort4print_core::flags` is the only other thing that knows this shape.
const VERSION: u8 = 2;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [svgz_dir, out_path] = match args.as_slice() {
        [a, b] => [a.clone(), b.clone()],
        _ => bail!("usage: pack-flags <svgz dir> <out.bin>"),
    };

    let mut flags: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in std::fs::read_dir(&svgz_dir)
        .with_context(|| format!("reading {svgz_dir}"))?
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("svgz") {
            continue;
        }
        let Some(code) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if code.len() != 2 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.is_empty() {
            bail!("{} is empty", path.display());
        }
        flags.push((code.to_ascii_lowercase(), bytes));
    }

    if flags.is_empty() {
        bail!("no flags found in {svgz_dir}");
    }
    // Sorted so the blob is reproducible whatever order the directory is read in.
    flags.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = Vec::new();
    out.extend_from_slice(b"S4PF");
    out.push(VERSION);
    out.extend_from_slice(&(flags.len() as u16).to_le_bytes());
    for (code, bytes) in &flags {
        out.extend_from_slice(code.as_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }

    let mut file = std::fs::File::create(&out_path)
        .with_context(|| format!("creating {out_path}"))?;
    file.write_all(&out)?;

    let biggest = flags.iter().max_by_key(|(_, b)| b.len()).expect("not empty");
    println!(
        "packed {} flags into {out_path} ({} KiB; largest {} at {} KiB)",
        flags.len(),
        out.len() / 1024,
        biggest.0,
        biggest.1.len() / 1024
    );
    Ok(())
}
