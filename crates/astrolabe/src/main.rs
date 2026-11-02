use astrolabe::launch::Machine;
use astrolabe::{Result, doctor, init};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const USAGE: &str = "Usage: astrolabe <--version|init [<path>]|init --clone <url> [<path>]|doctor <inspect|apply|install|adapters|remove|verify [<repo>]>|repo add <name> <checkout>|serve [--db <path>]|where|hook <event>|mcp>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version"] => println!("astrolabe {}", env!("CARGO_PKG_VERSION")),
        [verb @ ("init" | "doctor" | "repo"), rest @ ..] => {
            if let Err(error) = setup(verb, rest) {
                fail(&error.to_string());
            }
        }
        ["serve"] => match Machine::from_env() {
            Ok(machine) => serve(&machine.db()),
            Err(error) => fail(&error.to_string()),
        },
        ["serve", "--db", db] => serve(&PathBuf::from(db)),
        ["where"] => match Machine::from_env() {
            Ok(machine) => println!("{}", machine.data_dir.display()),
            Err(error) => fail(&error.to_string()),
        },
        ["hook", "pre-command"] => astrolabe::hook::pre_command(),
        ["hook", event] => astrolabe::hook::run(event),
        ["mcp"] => {
            if let Err(error) = astrolabe::mcp::serve() {
                fail(&error.to_string());
            }
        }
        [] | ["--help" | "-h"] => println!("{USAGE}"),
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

/// Only a typed yes. Closed input is a no.
fn ask(text: &str) -> bool {
    println!("{text}\nType yes to go ahead, anything else to skip:");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let read = std::io::stdin().lock().read_line(&mut answer);
    read.is_ok() && answer.trim() == "yes"
}

fn setup(verb: &str, rest: &[&str]) -> Result<()> {
    let machine = Machine::from_env()?;
    let registry = &machine.registry;
    let lines = match (verb, rest) {
        ("init", []) => init::create(registry, Path::new("astrolabe-agent"))?,
        ("init", ["--clone", url]) => init::clone(registry, url, None)?,
        ("init", ["--clone", url, path]) => init::clone(registry, url, Some(path))?,
        ("init", [path]) => init::create(registry, Path::new(path))?,
        ("doctor", ["inspect"]) => doctor::inspect(&machine)?,
        ("doctor", ["apply"]) => doctor::apply(&machine, &mut ask)?,
        ("doctor", ["install"]) => doctor::install(&mut ask)?,
        ("doctor", ["adapters"]) => doctor::adapters(&machine.data_dir, &mut ask)?,
        ("doctor", ["remove"]) => doctor::remove(&machine, &mut ask)?,
        ("doctor", ["verify", repo @ ..]) if repo.len() < 2 => {
            let say = &mut |line: String| println!("{line}");
            if !doctor::verify(&machine, repo.first().copied(), say)? {
                std::process::exit(1);
            }
            Vec::new()
        }
        ("repo", ["add", name, checkout]) => {
            registry.add(name, Path::new(checkout))?;
            vec![format!("{name} added")]
        }
        _ => usage(),
    };
    for line in lines {
        println!("{line}");
    }
    Ok(())
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
