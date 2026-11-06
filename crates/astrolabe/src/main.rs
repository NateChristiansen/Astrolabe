use std::path::PathBuf;

const USAGE: &str = "Usage: astrolabe <--version|repo add <name> [<checkout>]|hook <event> --if-active|on|off|status|activate <--effort <n> [--worker]|--status|--off>|effort <new|save> …|journal \"<sentence>\"|efforts [--repo <name>] [--all]|home [--open] [--serve [--port <n>]] [--theme auto|light|dark] [--palette ember|iris|slate]|notes <sync|status>>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version"] => println!("astrolabe {}", env!("CARGO_PKG_VERSION")),
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
        [first @ ("effort" | "journal"), rest @ ..] => {
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
        ["home", rest @ ..] => {
            let mut open = false;
            let mut serve = false;
            let mut port: Option<u16> = None;
            let mut theme = None;
            let mut palette = None;
            let mut words = rest.iter();
            while let Some(word) = words.next() {
                match *word {
                    "--open" => open = true,
                    "--serve" => serve = true,
                    "--port" => match words.next().and_then(|w| w.parse().ok()) {
                        Some(p) => port = Some(p),
                        None => fail("--port takes a number"),
                    },
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
                            None => fail("--palette takes ember, iris or slate"),
                        }
                    }
                    _ => usage(),
                }
            }
            match notes_here() {
                Ok(notes) => {
                    if (theme.is_some() || palette.is_some())
                        && let Err(why) = astrolabe::home::set_look(&notes, theme, palette)
                    {
                        fail(&why);
                    }
                    if let Some(port) = port
                        && let Err(why) = astrolabe::home::set_port(&notes, port)
                    {
                        fail(&why);
                    }
                    let port = port.unwrap_or_else(|| astrolabe::home::port(&notes));
                    if serve {
                        // The machine's one server; a second leaves, naming the first.
                        let Some(hold) = astrolabe::home::hold(&notes) else {
                            println!("{}", astrolabe::home::url(&notes));
                            eprintln!("already serving");
                            return;
                        };
                        match astrolabe::home::listen(port) {
                            Ok((listener, url)) => {
                                println!("{url}");
                                if open && let Err(why) = open_in_browser(&url) {
                                    fail(&why);
                                }
                                astrolabe::home::serve(&notes, listener, hold);
                            }
                            Err(why) => fail(&why),
                        }
                    }
                    // The file first, then the server if none is up; the address is what
                    // opens, since the file never reloads under the reader.
                    match notes.board() {
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
