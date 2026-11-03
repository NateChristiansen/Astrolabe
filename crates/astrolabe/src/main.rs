use astrolabe::launch::Machine;
use astrolabe::{Result, doctor, init};
use std::io::Write;
use std::path::{Path, PathBuf};

const USAGE: &str = "Usage: astrolabe <--version|init [<path>]|init --clone <url> [<path>]|doctor <inspect|apply|install|adapters|remove|verify [<repo>]>|repo add <name> <checkout>|serve [--db <path>]|where|hook <event> [--if-active]|on|off|status|activate <--effort <n>|--status|--off>|mcp>";

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
        ["hook", event, "--if-active"] => astrolabe::activate::gate(event),
        [verb @ ("on" | "off" | "status")] => {
            let (text, ok) = astrolabe::activate::machine(verb);
            println!("{text}");
            if !ok {
                std::process::exit(1);
            }
        }
        ["activate", rest @ ..] => {
            let (text, ok) = astrolabe::activate::command(rest);
            println!("{text}");
            if !ok {
                std::process::exit(1);
            }
        }
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

/// Only a yes. Closed input is a no. A no says what it read, as a pipe can add to the word.
fn ask(text: &str) -> bool {
    println!("{text}\nType yes to go ahead, anything else to skip:");
    let _ = std::io::stdout().flush();
    let answer = astrolabe::trust::read_answer(&mut std::io::stdin().lock()).unwrap_or_default();
    let yes = astrolabe::trust::says_yes(&answer);
    if !yes {
        let shown = answer.iter().take(16).flat_map(|b| b.escape_ascii());
        let shown: String = shown.map(char::from).collect();
        eprintln!("astrolabe: read {} bytes: \"{shown}\"", answer.len());
    }
    yes
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
        ("doctor", ["adapters"]) => doctor::adapters(&machine.adapters, &mut ask)?,
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
            // The plugin reads only its own record, so the database alone isn't a registration.
            astrolabe::activate::add_repo(name, &registry.checkout(name)?).map_err(|why| {
                astrolabe::Error::Setup(format!(
                    "{name} is in the database the notes layer reads, but the plugin's record couldn't be written ({why}), so the plugin doesn't have it. Fix that and run this again; rerunning is safe"
                ))
            })?;
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
