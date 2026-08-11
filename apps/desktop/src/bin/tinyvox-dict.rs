use std::path::PathBuf;

use tinyvox_engine::{
    dictionary::{
        Dictionary,
        EntryId,
        EntrySource,
    },
    dictionary_store::DictionaryStore,
};

fn dictionary_path() -> Result<PathBuf, String> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| {
            "APPDATA environment variable is not available"
                .to_string()
        })
        .map(|path| {
            path.join("TinyVox")
                .join("dictionary.json")
        })
}

fn usage() {
    println!("TinyVox Dictionary");
    println!();
    println!("Commands:");
    println!(
        "  add <wrong> <correct>"
    );
    println!("  list");
    println!(
        "  edit <id> <wrong> <correct>"
    );
    println!("  remove <id>");
}

fn parse_id(value: &str) -> Result<EntryId, String> {
    value
        .parse::<u64>()
        .map(EntryId)
        .map_err(|_| {
            format!(
                "invalid entry id: {value}"
            )
        })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args =
        std::env::args().skip(1);

    let command = match args.next() {
        Some(command) => command,
        None => {
            usage();
            return Ok(());
        }
    };

    let path = dictionary_path()?;
    let store = DictionaryStore::new(path);
    let mut dictionary = store.load()?;

    match command.as_str() {
        "add" => {
            let wrong = args
                .next()
                .ok_or("missing <wrong> argument")?;

            let correct = args
                .next()
                .ok_or("missing <correct> argument")?;

            if args.next().is_some() {
                return Err(
                    "too many arguments".into()
                );
            }

            let id = dictionary.add(
                &wrong,
                &correct,
                EntrySource::Manual,
            );

            store.save(&dictionary)?;

            println!(
                "✓ Added dictionary entry {}: {} → {}",
                id.0,
                wrong,
                correct
            );
        }

        "list" => {
            if args.next().is_some() {
                return Err(
                    "list takes no arguments".into()
                );
            }

            if dictionary.entries().is_empty() {
                println!(
                    "Dictionary is empty."
                );

                return Ok(());
            }

            for entry in dictionary.entries() {
                println!(
                    "{}: {} → {} | {:?} | hits: {}",
                    entry.id.0,
                    entry.wrong,
                    entry.correct,
                    entry.source,
                    entry.hit_count
                );
            }
        }

        "edit" => {
            let id = parse_id(
                &args
                    .next()
                    .ok_or("missing <id> argument")?,
            )?;

            let wrong = args
                .next()
                .ok_or("missing <wrong> argument")?;

            let correct = args
                .next()
                .ok_or("missing <correct> argument")?;

            if args.next().is_some() {
                return Err(
                    "too many arguments".into()
                );
            }

            if !dictionary.edit(
                id,
                &wrong,
                &correct,
            ) {
                return Err(
                    format!(
                        "dictionary entry {} not found",
                        id.0
                    )
                    .into(),
                );
            }

            store.save(&dictionary)?;

            println!(
                "✓ Updated dictionary entry {}: {} → {}",
                id.0,
                wrong,
                correct
            );
        }

        "remove" => {
            let id = parse_id(
                &args
                    .next()
                    .ok_or("missing <id> argument")?,
            )?;

            if args.next().is_some() {
                return Err(
                    "too many arguments".into()
                );
            }

            if !dictionary.remove(id) {
                return Err(
                    format!(
                        "dictionary entry {} not found",
                        id.0
                    )
                    .into(),
                );
            }

            store.save(&dictionary)?;

            println!(
                "✓ Removed dictionary entry {}",
                id.0
            );
        }

        _ => {
            usage();

            return Err(
                format!(
                    "unknown command: {command}"
                )
                .into(),
            );
        }
    }

    Ok(())
}