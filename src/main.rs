use std::path::Path;
use std::process::ExitCode;

use notestrike::Notebook;
use uuid::Uuid;

const USAGE: &str = "NoteStrike local notebook\n\
  notestrike init NOTEBOOK_DIR\n\
  notestrike new NOTEBOOK_DIR TITLE BODY\n\
  notestrike edit NOTEBOOK_DIR NOTE_ID BODY\n\
  notestrike rename NOTEBOOK_DIR NOTE_ID TITLE\n\
  notestrike show NOTEBOOK_DIR NOTE_ID\n\
  notestrike list NOTEBOOK_DIR\n\
  notestrike search NOTEBOOK_DIR QUERY\n\
  notestrike backlinks NOTEBOOK_DIR NOTE_ID\n\
  notestrike warnings NOTEBOOK_DIR\n\
  notestrike repair NOTEBOOK_DIR BAD_FILENAME REPLACEMENT.md";

fn main() -> ExitCode {
    match run(&std::env::args().collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("NoteStrike: {error}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let command = args.get(1).ok_or("missing command")?.as_str();
    if command == "help" || command == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    let dir = Path::new(args.get(2).ok_or("missing notebook directory")?);
    if command == "init" {
        expect(args, 3)?;
        Notebook::create(dir)?;
        println!("Created {}", dir.display());
        return Ok(());
    }
    let mut notebook = Notebook::open(dir)?;
    if command != "warnings" {
        for warning in notebook.warnings() {
            eprintln!(
                "Warning: {}: {} (use `notestrike warnings` for repair details)",
                warning.path.display(),
                warning.reason
            );
        }
    }
    match command {
        "new" => {
            expect(args, 5)?;
            let note = notebook.add(&args[3], &args[4])?;
            println!("{}", note.id);
        }
        "edit" => {
            expect(args, 5)?;
            notebook.update(id(args, 3)?, &args[4])?;
            println!("Saved.");
        }
        "rename" => {
            expect(args, 5)?;
            notebook.rename(id(args, 3)?, &args[4])?;
            println!("Renamed.");
        }
        "show" => {
            expect(args, 4)?;
            let note = notebook.get(id(args, 3)?).ok_or("note does not exist")?;
            println!("{}\n{}\n{}", note.id, note.title, note.body);
        }
        "list" => {
            expect(args, 3)?;
            for note in notebook.list() {
                println!("{}\t{}", note.id, note.title);
            }
        }
        "search" => {
            expect(args, 4)?;
            for note in notebook.search(&args[3]) {
                println!("{}\t{}", note.id, note.title);
            }
        }
        "backlinks" => {
            expect(args, 4)?;
            for source in notebook.backlinks(id(args, 3)?) {
                if let Some(note) = notebook.get(source) {
                    println!("{}\t{}", note.id, note.title);
                }
            }
        }
        "warnings" => {
            expect(args, 3)?;
            for warning in notebook.warnings() {
                println!("{}\t{}", warning.path.display(), warning.reason);
            }
        }
        "repair" => {
            expect(args, 5)?;
            let backup = notebook.repair_from_file(&args[3], Path::new(&args[4]))?;
            println!("Repaired; original bytes preserved at {}", backup.display());
        }
        _ => return Err(format!("unknown command: {command}")),
    }
    Ok(())
}

fn expect(args: &[String], count: usize) -> Result<(), String> {
    if args.len() == count {
        Ok(())
    } else {
        Err(format!("expected {} arguments", count - 1))
    }
}

fn id(args: &[String], index: usize) -> Result<Uuid, String> {
    Uuid::parse_str(args.get(index).ok_or("missing note ID")?)
        .map_err(|e| format!("invalid note ID: {e}"))
}
