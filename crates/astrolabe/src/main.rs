use astrolabe::launch::Machine;
use astrolabe::{Result, doctor, init};
use std::io::Write;
use std::path::{Path, PathBuf};

const USAGE: &str = "Usage: astrolabe <--version|init [<path>]|init --clone <url> [<path>]|doctor <inspect|apply|install|adapters|remove|verify [<repo>]>|repo add <name> [<checkout>]|serve [--db <path>]|where|hook <event> [--if-active]|on|off|status|activate <--effort <n>|--status|--off>|effort <new|save> …|efforts [--repo <name>] [--all]|notes <sync|status>|mcp>";

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
        ["effort", rest @ ..] => {
            // Only the hook does the work; here the body is read so the command can be told
            // apart from one the hook already answered.
            let body = read_piped_stdin();
            let (text, ok) = astrolabe::record::plain(rest, body.as_deref());
            println!("{text}");
            if !ok {
                std::process::exit(1);
            }
        }
        ["efforts", rest @ ..] => {
            let repo = rest
                .iter()
                .position(|a| *a == "--repo")
                .and_then(|i| rest.get(i + 1).copied());
            let all = rest.contains(&"--all");
            match astrolabe::record::list(repo, all) {
                Ok(text) => println!("{text}"),
                Err(why) => fail(&why),
            }
        }
        ["notes", "sync"] => match notes_here() {
            Ok(notes) => match notes.sync() {
                Ok(text) => println!("{text}"),
                Err(why) => fail(&why),
            },
            Err(why) => fail(&why),
        },
        ["notes", "status"] => match notes_here() {
            Ok(notes) => println!("{}", notes.status_text()),
            Err(why) => fail(&why),
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
        ("repo", ["add", name, checkout @ ..]) if checkout.len() < 2 => {
            use astrolabe::Error::Setup;
            astrolabe::activate::check_repo_name(name).map_err(Setup)?;
            let given = checkout.first().map(PathBuf::from);
            let folder = match &given {
                Some(checkout) => checkout.clone(),
                None => std::env::current_dir()?,
            };
            // The notes checkout is always a record on this machine: the plugin finds the
            // notes by it, so it can't be a note in them.
            let noted = match *name {
                astrolabe::registry::AGENT => None,
                _ => astrolabe::activate::add_repo_note(name, &folder).map_err(Setup)?,
            };
            if let Some(mut lines) = noted {
                // A checkout named on purpose also goes in the database the notes layer reads,
                // as it always has; a worktree a session happens to be in doesn't.
                if let Some(checkout) = &given {
                    registry.add(name, checkout)?;
                    let at = registry.checkout(name)?;
                    lines.push(format!("Also recorded {name} at {} in this machine's database, which the notes layer reads.", at.display()));
                }
                lines
            } else {
                let checkout = match given {
                    Some(checkout) => checkout,
                    None => astrolabe::activate::main_checkout(&folder).map_err(Setup)?,
                };
                registry.add(name, &checkout)?;
                let at = registry.checkout(name)?;
                // The plugin reads only its own record, so the database alone isn't a registration.
                astrolabe::activate::add_repo(name, &at).map_err(|why| {
                    Setup(format!(
                        "{name} is in the database the notes layer reads, but the plugin's record couldn't be written ({why}), so the plugin doesn't have it. Fix that and run this again; rerunning is safe"
                    ))
                })?;
                let why = match *name {
                    astrolabe::registry::AGENT => "the notes checkout is found by this record",
                    _ => {
                        "it has no remote another machine could match, so it's matched by this folder here"
                    }
                };
                vec![format!(
                    "{name} added: registered on this machine at {}, as {why}.",
                    at.display()
                )]
            }
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

fn notes_here() -> std::result::Result<astrolabe::notes::Notes, String> {
    let dir =
        astrolabe::activate::state_dir().ok_or("Astrolabe has no home folder for its state")?;
    let root = astrolabe::activate::notes_root()?;
    Ok(astrolabe::notes::Notes::new(root, dir))
}

/// What was piped in, if anything: a terminal is left alone.
fn read_piped_stdin() -> Option<String> {
    use std::io::{IsTerminal, Read};
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut text = String::new();
    stdin.lock().take(1 << 20).read_to_string(&mut text).ok()?;
    Some(text)
}
