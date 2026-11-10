use std::path::PathBuf;

const USAGE: &str = "Usage: astrolabe <--version|install [--dir <folder>] [--force]|update [--channel release|nightly] [--dry-run] [--force]|setup codex [--apply] [--check] [--reviewer user|auto_review] [--config <file>]|repo add <name> [<checkout>]|hook <event> --if-active|on|off|status|activate <--effort <n> [--worker]|--initiative <slug or title>|--status|--off>|allow --once \"<exact command>\" --said \"<the owner's words>\"|effort <new|save|read> …|initiative new …|journal \"<sentence>\"|lesson close lessons/<name>.md --as \"<disposition>\"|efforts [--repo <name>] [--all]|home [--open] [--serve [--port <n>]|--preview [--port <n>]] [--theme auto|light|dark] [--palette iris|ember|slate|moss|graphite]|notes <init <folder>|sync|status>>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version"] => println!("{}", astrolabe::update::version_line()),
        ["update", rest @ ..] => match astrolabe::update::command(rest) {
            Ok(text) => println!("{text}"),
            Err(why) => fail(&why),
        },
        ["install", rest @ ..] => match astrolabe::install::command(rest) {
            Ok(text) => println!("{text}"),
            Err(why) => fail(&why),
        },
        ["setup", "codex", rest @ ..] => match astrolabe::codex_setup::command(rest) {
            Ok(text) => println!("{text}"),
            Err(why) => fail(&why),
        },
        ["repo", "add", name, checkout @ ..] if checkout.len() < 2 => {
            if let Err(why) = add_repo(name, checkout.first().copied()) {
                fail(&why);
            }
        }
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
        ["lesson", rest @ ..] => {
            let (text, ok) = astrolabe::record::lesson::plain(rest);
            println!("{text}");
            if !ok {
                std::process::exit(1);
            }
        }
        ["allow", ..] => fail(
            "No approval recorded: astrolabe allow must run alone through the plugin's pre-command hook.",
        ),
        [first @ ("effort" | "journal" | "initiative"), rest @ ..] => {
            // Only the hook does the work; here the body is read so the command can be told
            // apart from one the hook already answered.
            let body = read_piped_stdin();
            let (text, ok) = astrolabe::record::plain(first, rest, body.as_deref());
            println!("{text}");
            if !ok {
                std::process::exit(1);
            }
        }
        ["efforts", rest @ ..] => {
            // Each flag by name: one that's mistyped, or a --repo with no name, was once
            // ignored and the whole list given as if it had been asked for.
            let (mut repo, mut all) = (None, false);
            let mut words = rest.iter();
            while let Some(word) = words.next() {
                match *word {
                    "--all" => all = true,
                    // The list is JSON with or without it.
                    "--json" => {}
                    "--repo" => {
                        let name = words
                            .next()
                            .filter(|n| !n.is_empty() && !n.starts_with("--"));
                        match name {
                            Some(name) => repo = Some(*name),
                            None => fail("--repo takes a repo's name"),
                        }
                    }
                    other => fail(&format!(
                        "efforts takes --repo <name>, --all and --json, not {other:?}"
                    )),
                }
            }
            match astrolabe::record::list(repo, all) {
                Ok(text) => println!("{text}"),
                Err(why) => fail(&why),
            }
        }
        ["home", rest @ ..] => {
            let mut open = false;
            let mut serve = false;
            let mut preview = false;
            let mut port: Option<u16> = None;
            let mut theme = None;
            let mut palette = None;
            let mut handover: Option<u16> = None;
            let mut words = rest.iter();
            while let Some(word) = words.next() {
                match *word {
                    "--open" => open = true,
                    "--serve" => serve = true,
                    "--preview" => preview = true,
                    "--port" => match words.next().and_then(|w| w.parse().ok()) {
                        Some(p) => port = Some(p),
                        None => fail("--port takes a number"),
                    },
                    // Not for people: a server whose program was replaced starts its
                    // successor with this and the port it was asked for.
                    w if w == astrolabe::home::HANDOVER_FLAG => {
                        match words.next().and_then(|w| w.parse().ok()) {
                            Some(p) => handover = Some(p),
                            None => fail("--handover takes a number"),
                        }
                    }
                    "--theme" => {
                        match words.next().and_then(|w| astrolabe::home::Theme::parse(w)) {
                            Some(t) => theme = Some(t),
                            None => fail("--theme takes auto, light or dark"),
                        }
                    }
                    "--palette" => {
                        match words
                            .next()
                            .and_then(|w| astrolabe::home::Palette::parse(w))
                        {
                            Some(p) => palette = Some(p),
                            None => fail("--palette takes iris, ember, slate, moss or graphite"),
                        }
                    }
                    _ => usage(),
                }
            }
            if preview && (serve || handover.is_some()) {
                fail("--preview runs its own server; leave out --serve");
            }
            match notes_here() {
                // Beside the machine's server, for trying a build: nothing it's given is
                // saved, and a port of 0 by default, so it never takes the machine's.
                Ok(notes) if preview => {
                    let mut look = astrolabe::home::look(&notes);
                    look.theme = theme.unwrap_or(look.theme);
                    look.palette = palette.unwrap_or(look.palette);
                    match astrolabe::home::listen(port.unwrap_or(0)) {
                        Ok((listener, url)) => {
                            println!("{url}");
                            if open && let Err(why) = open_in_browser(&url) {
                                fail(&why);
                            }
                            astrolabe::home::preview(&notes, listener, look);
                        }
                        Err(why) => fail(&why),
                    }
                }
                Ok(notes) => {
                    if (theme.is_some() || palette.is_some())
                        && let Err(why) = astrolabe::home::set_look(&notes, theme, palette)
                    {
                        fail(&why);
                    }
                    // A server taking over keeps the machine's setting as it is: the port
                    // it's given is where the last one listened, maybe one picked for 0.
                    if handover.is_none()
                        && let Some(port) = port
                        && let Err(why) = astrolabe::home::set_port(&notes, port)
                    {
                        fail(&why);
                    }
                    let port = port.unwrap_or_else(|| astrolabe::home::port(&notes));
                    if serve {
                        // The machine's one server; a second leaves, naming the first. One
                        // taking over waits for the last to exit and free the lock and port.
                        let deadline = std::time::Instant::now()
                            + handover.map_or(std::time::Duration::ZERO, |_| {
                                astrolabe::home::HANDOVER_WAIT
                            });
                        let bound = loop {
                            let held = astrolabe::home::hold(&notes)
                                .map(|hold| (hold, astrolabe::home::listen(port)));
                            match held {
                                Some((hold, Ok(listening))) => break Ok((hold, listening)),
                                _ if std::time::Instant::now() < deadline => {
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                }
                                Some((_, Err(why))) => break Err(Some(why)),
                                None => break Err(None),
                            }
                        };
                        match bound {
                            Ok((hold, (listener, url))) => {
                                println!("{url}");
                                if open && let Err(why) = open_in_browser(&url) {
                                    fail(&why);
                                }
                                let requested = handover.unwrap_or(port);
                                astrolabe::home::serve(&notes, listener, hold, requested);
                            }
                            Err(None) => {
                                println!("{}", astrolabe::home::url(&notes));
                                eprintln!("already serving");
                                return;
                            }
                            Err(Some(why)) => fail(&why),
                        }
                    }
                    // The file first, then the server if none is up; the address is what
                    // opens, since the file never reloads under the reader.
                    match astrolabe::application::board(&notes) {
                        Ok(path) => println!("{}", path.display()),
                        Err(why) => fail(&why),
                    }
                    if let Some(url) = astrolabe::home::ensure_serving(&notes) {
                        println!("{url}");
                        if open && let Err(why) = open_in_browser(&url) {
                            fail(&why);
                        }
                    }
                }
                Err(why) => fail(&why),
            }
        }
        ["notes", "init", folder] => match astrolabe::seed::init(std::path::Path::new(folder)) {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            Err(why) => fail(&why),
        },
        ["notes", "sync"] => match notes_here() {
            Ok(notes) => match astrolabe::application::sync(&notes) {
                Ok(text) => println!("{text}"),
                Err(why) => fail(&why),
            },
            Err(why) => fail(&why),
        },
        ["notes", "status"] => match notes_here() {
            Ok(notes) => println!("{}", notes.status_text()),
            Err(why) => fail(&why),
        },
        [] | ["--help" | "-h"] => println!("{USAGE}"),
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

fn add_repo(name: &str, checkout: Option<&str>) -> std::result::Result<(), String> {
    astrolabe::activate::check_repo_name(name)?;
    let folder = match checkout {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    let noted = match name {
        astrolabe::registry::AGENT => None,
        _ => astrolabe::activate::add_repo_note(name, &folder)?,
    };
    if let Some(lines) = noted {
        for line in lines {
            println!("{line}");
        }
    } else {
        let main = astrolabe::activate::main_checkout(&folder)?;
        let checkout = match checkout {
            Some(_) => PathBuf::from(astrolabe::registry::native(&folder)),
            None => main,
        };
        astrolabe::activate::add_repo(name, &checkout).map_err(|why| {
            format!("the plugin's repo record couldn't be written ({why}), so the plugin doesn't have it. Fix that and run this again; rerunning is safe")
        })?;
        let why = match name {
            astrolabe::registry::AGENT => "the notes checkout is found by this record",
            _ => {
                "it has no remote another machine could match, so it's matched by this folder here"
            }
        };
        println!(
            "{name} added: registered on this machine at {}, as {why}.",
            checkout.display()
        );
    }
    Ok(())
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

/// The default browser, as the desktop opens a file.
fn open_in_browser(path: &str) -> std::result::Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", "start", ""]).arg(path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    let status = cmd
        .status()
        .map_err(|e| format!("could not open a browser: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the browser launcher exited with {status}"))
    }
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
