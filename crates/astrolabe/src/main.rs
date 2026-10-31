use std::path::{Path, PathBuf};

const USAGE: &str = "Usage: astrolabe <--version|doctor|init|serve [--db <path>]>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version"] => println!("astrolabe {}", env!("CARGO_PKG_VERSION")),
        ["doctor" | "init"] => println!("not implemented (phase 1a)"),
        ["serve"] => match astrolabe::default_db_path() {
            Some(db) => serve(&db),
            None => fail("no app-data directory found; pass --db <path>"),
        },
        ["serve", "--db", db] => serve(&PathBuf::from(db)),
        [] | ["--help" | "-h"] => println!("{USAGE}"),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn serve(db: &Path) {
    if let Err(error) = astrolabe::serve(db) {
        fail(&error.to_string());
    }
}

fn fail(message: &str) {
    eprintln!("astrolabe: {message}");
    std::process::exit(1);
}
