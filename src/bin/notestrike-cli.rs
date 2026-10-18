//! NoteStrike command-line helper (no GUI).

use std::path::Path;
use std::process::ExitCode;

use notestrike::document::{self, LoadOptions};
use notestrike::legacy::{self, LegacyItem};

const USAGE: &str = "NoteStrike command-line helper\n\
  notestrike-cli inspect FILE                     Show detected encoding, line endings, and problems\n\
  notestrike-cli list-notebook NOTEBOOK_DIR       List notes of a NoteStrike 0.1 notebook (read-only)\n\
  notestrike-cli import-notebook NOTEBOOK_DIR DEST_DIR\n\
                                                  Copy every note into DEST_DIR as a plain .md file.\n\
                                                  The notebook itself is never modified.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("notestrike-cli: {error}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let command = args.get(1).map(String::as_str).ok_or("missing command")?;
    match (command, args.len()) {
        ("help" | "--help" | "-h", _) => println!("{USAGE}"),
        ("inspect", 3) => {
            let loaded = document::load(
                Path::new(&args[2]),
                LoadOptions {
                    allow_large: true,
                    forced_encoding: None,
                },
            )
            .map_err(|e| e.to_string())?;
            println!("encoding\t{}", loaded.encoding.id());
            println!("bom\t{}", loaded.utf16_bom);
            println!("eol\t{}", loaded.eol.short());
            println!(
                "eol_counts\tcrlf={} lf={} cr={}",
                loaded.eol_counts.crlf, loaded.eol_counts.lf, loaded.eol_counts.cr
            );
            println!("invalid_sequences\t{}", loaded.invalid_sequences);
            if let Some(offset) = loaded.first_invalid_offset {
                println!("first_invalid_offset\t{offset}");
            }
            println!("contains_nul\t{}", loaded.contains_nul);
            println!("read_only\t{}", loaded.read_only);
            println!("bytes\t{}", loaded.snapshot.len);
            println!("sha256\t{}", loaded.snapshot.hash_hex());
        }
        ("list-notebook", 3) => {
            for item in legacy::scan(Path::new(&args[2]))? {
                match item {
                    LegacyItem::Note(note) => {
                        println!("{}\t{}\t{}", note.id, note.title, note.path.display());
                    }
                    LegacyItem::Unparsed { path, reason } => {
                        println!("-\t(unparsed: {reason})\t{}", path.display());
                    }
                }
            }
        }
        ("import-notebook", 4) => {
            let report = legacy::import(Path::new(&args[2]), Path::new(&args[3]))?;
            for (from, to) in &report.converted {
                println!("converted\t{}\t{}", from.display(), to.display());
            }
            for (from, to, reason) in &report.copied_verbatim {
                println!("verbatim\t{}\t{}\t{reason}", from.display(), to.display());
            }
            println!(
                "Imported {} note(s); the notebook was not modified.",
                report.total()
            );
        }
        _ => return Err(format!("unknown command or wrong arguments: {command}")),
    }
    Ok(())
}
