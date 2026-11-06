//! `astrolabe update`: the program and each app's plugin, moved together to the newest
//! release or nightly. It runs the same published installer and app commands a person
//! would, so it adds no download or install path of its own.
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set by CI: the commit a build came from, and `nightly` for a build of main.
const COMMIT: Option<&str> = option_env!("ASTROLABE_COMMIT");
const CHANNEL: Option<&str> = option_env!("ASTROLABE_CHANNEL");
const REPO: &str = "https://github.com/NateChristiansen/Astrolabe";
const PLUGIN: &str = "astrolabe@astrolabe";
const MARKETPLACE: &str = "astrolabe";

/// `astrolabe 0.5.0`, and for a nightly build `astrolabe 0.5.0 (nightly 1a2b3c4)`.
pub fn version_line() -> String {
    match (built_nightly(), COMMIT) {
        (true, Some(commit)) => format!("astrolabe {VERSION} (nightly {})", short(commit)),
        _ => format!("astrolabe {VERSION}"),
    }
}

fn built_nightly() -> bool {
    CHANNEL == Some("nightly")
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    Release,
    Nightly,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Channel::Release => "release",
            Channel::Nightly => "nightly",
        }
    }
}

/// A release tag reads `v` and a version; anything else a marketplace follows (a branch,
/// the `nightly` tag, a local folder) moves with main.
pub fn channel_of(git_ref: Option<&str>) -> Channel {
    match git_ref {
        Some(r)
            if r.strip_prefix('v').is_some_and(|v| {
                let parts: Vec<&str> = v.split('.').collect();
                parts.len() == 3
                    && parts
                        .iter()
                        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            }) =>
        {
            Channel::Release
        }
        _ => Channel::Nightly,
    }
}

/// An agent app whose plugin this command refreshes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum App {
    Claude,
    Codex,
}

impl App {
    const ALL: [App; 2] = [App::Claude, App::Codex];

    fn label(self) -> &'static str {
        match self {
            App::Claude => "Claude Code",
            App::Codex => "Codex",
        }
    }

    fn cli(self) -> &'static str {
        match self {
            App::Claude => "claude",
            App::Codex => "codex",
        }
    }

    /// The commands that swap the registration for one at `git_ref`, as the install
    /// guide gives them. The removals may fail when nothing is registered.
    pub fn steps(self, git_ref: &str) -> Vec<(Vec<String>, bool)> {
        let owned = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let url = format!("{REPO}.git");
        match self {
            App::Claude => vec![
                (owned(&["plugin", "uninstall", PLUGIN]), false),
                (
                    owned(&["plugin", "marketplace", "remove", MARKETPLACE]),
                    false,
                ),
                (
                    owned(&["plugin", "marketplace", "add", &format!("{url}#{git_ref}")]),
                    true,
                ),
                (owned(&["plugin", "install", PLUGIN]), true),
            ],
            App::Codex => vec![
                (owned(&["plugin", "remove", PLUGIN]), false),
                (
                    owned(&["plugin", "marketplace", "remove", MARKETPLACE]),
                    false,
                ),
                (
                    owned(&["plugin", "marketplace", "add", &url, "--ref", git_ref]),
                    true,
                ),
                (owned(&["plugin", "add", PLUGIN]), true),
            ],
        }
    }
}

/// Where an app's Astrolabe marketplace points now: None when it has none registered,
/// Some(None) when it follows the default branch or a folder.
fn registered(app: App, cli: &Path) -> Option<Option<String>> {
    match app {
        App::Claude => {
            let out = Command::new(cli)
                .args(["plugin", "marketplace", "list", "--json"])
                .output()
                .ok()?;
            let list: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
            let entry = list.as_array()?.iter().find(|m| m["name"] == MARKETPLACE)?;
            Some(entry["ref"].as_str().map(str::to_owned))
        }
        App::Codex => {
            let home = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| home().map(|h| h.join(".codex")))?;
            codex_ref(&std::fs::read_to_string(home.join("config.toml")).ok()?)
        }
    }
}

/// The `ref` of `[marketplaces.astrolabe]` in Codex's config, read by line: the table is
/// flat and written by Codex itself.
pub fn codex_ref(config: &str) -> Option<Option<String>> {
    let mut lines = config.lines().map(str::trim);
    lines.find(|l| *l == "[marketplaces.astrolabe]")?;
    for line in lines {
        if line.starts_with('[') {
            break;
        }
        if let Some(value) = line
            .strip_prefix("ref")
            .and_then(|rest| rest.trim_start().strip_prefix('='))
        {
            return Some(Some(value.trim().trim_matches('"').to_owned()));
        }
    }
    Some(None)
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// A command on PATH, with Windows' extensions tried in order: npm installs `codex` as a
/// `.cmd` shim, which only runs when named in full.
fn on_path(name: &str) -> Option<PathBuf> {
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        extensions.iter().find_map(|ext| {
            let path = dir.join(format!("{name}{ext}"));
            path.is_file().then_some(path)
        })
    })
}

/// What to move to: the release or nightly tag, how it reads, and whether this program
/// is already that build.
struct Target {
    git_ref: String,
    label: String,
    current: bool,
}

fn target(channel: Channel, scratch: &Path) -> Result<Target, String> {
    match channel {
        Channel::Release => {
            let path = scratch.join("latest.json");
            download(
                "https://api.github.com/repos/NateChristiansen/Astrolabe/releases/latest",
                &path,
            )
            .map_err(|_| {
                "couldn't find a published release (none may exist yet); `--channel nightly` follows main"
                    .to_owned()
            })?;
            let latest: serde_json::Value = read_json(&path)?;
            let tag = latest["tag_name"]
                .as_str()
                .ok_or("the latest release has no tag")?
                .to_owned();
            Ok(Target {
                current: !built_nightly() && tag == format!("v{VERSION}"),
                label: tag.trim_start_matches('v').to_owned(),
                git_ref: tag,
            })
        }
        Channel::Nightly => {
            let path = scratch.join("nightly.json");
            download(
                &format!("{REPO}/releases/download/nightly/nightly.json"),
                &path,
            )
            .map_err(|why| {
                    format!("couldn't read the nightly build's details ({why}); a nightly is published once main has built with this change")
                })?;
            let nightly: serde_json::Value = read_json(&path)?;
            let commit = nightly["commit"]
                .as_str()
                .ok_or("the nightly build names no commit")?;
            let version = nightly["version"].as_str().unwrap_or(VERSION);
            Ok(Target {
                current: built_nightly() && COMMIT == Some(commit),
                label: format!("{version} nightly {}", short(commit)),
                git_ref: "nightly".into(),
            })
        }
    }
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("unreadable reply: {e}"))
}

/// One file over HTTPS with the tool each system already has, the same one the install
/// guide uses.
fn download(url: &str, to: &Path) -> Result<(), String> {
    let out = if cfg!(windows) {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "$ProgressPreference='SilentlyContinue'; try {{ Invoke-WebRequest -UseBasicParsing -Uri {} -OutFile {} }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}",
                    quote(url),
                    quote(&to.to_string_lossy())
                ),
            ])
            .output()
    } else {
        Command::new("curl")
            .args(["--fail", "--silent", "--show-error", "--location", "--output"])
            .arg(to)
            .arg(url)
            .output()
    }
    .map_err(|e| format!("couldn't start the download: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(last_line(&out.stderr))
    }
}

fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("it failed without saying why")
        .to_owned()
}

/// Run the target's published installer into the folder this program runs from. The
/// installer checks the archive's hash, keeps the old files in a backup and refuses any
/// file it didn't write; its own output is shown as it runs.
fn install(git_ref: &str, scratch: &Path, folder: &Path) -> Result<(), String> {
    let name = if cfg!(windows) {
        "install.ps1"
    } else {
        "install.sh"
    };
    let script = scratch.join(name);
    download(
        &format!("{REPO}/releases/download/{git_ref}/{name}"),
        &script,
    )
    .map_err(|why| format!("couldn't download the installer: {why}"))?;
    let status = if cfg!(windows) {
        // Bypass applies to this one run of the downloaded installer, not to the machine.
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .arg("-InstallDir")
            .arg(folder)
            .status()
    } else {
        Command::new("sh")
            .arg(&script)
            .arg("--dir")
            .arg(folder)
            .status()
    }
    .map_err(|e| format!("couldn't start the installer: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the installer stopped; its message is above, and the previous program is still in place".into())
    }
}

fn shown(cli: &str, words: &[String]) -> String {
    format!("{cli} {}", words.join(" "))
}

pub fn command(args: &[&str]) -> Result<String, String> {
    let mut chosen = None;
    let mut dry_run = false;
    let mut force = false;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match *word {
            "--channel" => {
                chosen = Some(match words.next().copied() {
                    Some("release") => Channel::Release,
                    Some("nightly") => Channel::Nightly,
                    _ => return Err("--channel takes release or nightly".into()),
                })
            }
            "--dry-run" => dry_run = true,
            "--force" => force = true,
            _ => {
                return Err(
                    "usage: astrolabe update [--channel release|nightly] [--dry-run] [--force]"
                        .into(),
                );
            }
        }
    }

    // Each app is found by its command; one that isn't there is skipped, not an error.
    let apps: Vec<(App, Option<PathBuf>)> =
        App::ALL.iter().map(|&a| (a, on_path(a.cli()))).collect();
    let refs: Vec<(App, Option<Option<String>>)> = apps
        .iter()
        .map(|(app, cli)| (*app, cli.as_deref().and_then(|c| registered(*app, c))))
        .collect();
    let channel = chosen.unwrap_or_else(|| match refs.iter().find_map(|(_, r)| r.clone()) {
        Some(git_ref) => channel_of(git_ref.as_deref()),
        None if built_nightly() => Channel::Nightly,
        None => Channel::Release,
    });

    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let scratch = std::env::temp_dir().join(format!("astrolabe-update-{id}"));
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let result = target(channel, &scratch)
        .and_then(|target| run(channel, &target, &apps, &refs, &scratch, dry_run, force));
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn run(
    channel: Channel,
    target: &Target,
    apps: &[(App, Option<PathBuf>)],
    refs: &[(App, Option<Option<String>>)],
    scratch: &Path,
    dry_run: bool,
    force: bool,
) -> Result<String, String> {
    let folder = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("can't tell which folder this program runs from")?
        .to_path_buf();
    let mut report = vec![format!(
        "Channel: {}. This program: {}. Newest: {}.",
        channel.name(),
        version_line().trim_start_matches("astrolabe "),
        target.label
    )];
    let plugin_current = |app: App| {
        refs.iter().any(|(a, r)| {
            *a == app
                && r.as_ref()
                    .is_some_and(|r| r.as_deref() == Some(target.git_ref.as_str()))
        })
    };
    let found: Vec<(App, &PathBuf)> = apps
        .iter()
        .filter_map(|(a, c)| c.as_ref().map(|c| (*a, c)))
        .collect();
    if !force && target.current && found.iter().all(|(a, _)| plugin_current(*a)) {
        report.push("Already up to date.".into());
        return Ok(report.join("\n"));
    }

    let mut failed = false;
    if target.current && !force {
        report.push("Program: already this build.".into());
    } else if dry_run {
        report.push(format!(
            "Program: would run the {} installer into {}.",
            target.label,
            folder.display()
        ));
    } else {
        println!("Installing Astrolabe {}…", target.label);
        install(&target.git_ref, scratch, &folder)?;
        report.push(format!("Program: installed {}.", target.label));
    }

    for (app, cli) in apps {
        let Some(cli) = cli else {
            report.push(format!(
                "{}: skipped, `{}` isn't on PATH.",
                app.label(),
                app.cli()
            ));
            continue;
        };
        if plugin_current(*app) && !force {
            report.push(format!(
                "{}: plugin already follows {}.",
                app.label(),
                target.git_ref
            ));
            continue;
        }
        let steps = app.steps(&target.git_ref);
        if dry_run {
            let plan: Vec<String> = steps.iter().map(|(w, _)| shown(app.cli(), w)).collect();
            report.push(format!("{}: would run {}.", app.label(), plan.join("; ")));
            continue;
        }
        let mut outcome = Ok(());
        for (words, required) in &steps {
            match Command::new(cli).args(words).output() {
                Ok(out) if out.status.success() || !required => {}
                Ok(out) => {
                    outcome = Err(format!(
                        "`{}` failed: {}",
                        shown(app.cli(), words),
                        last_line(&out.stderr)
                    ));
                    break;
                }
                Err(e) => {
                    outcome = Err(format!("couldn't run `{}`: {e}", app.cli()));
                    break;
                }
            }
        }
        match outcome {
            Ok(()) => report.push(format!(
                "{}: plugin now follows {}.",
                app.label(),
                target.git_ref
            )),
            Err(why) => {
                failed = true;
                report.push(format!("{}: {why}", app.label()));
            }
        }
    }
    if found.is_empty() {
        report.push("Neither app's command was found, so only the program was updated. Install the plugin as the install guide shows once you have one.".into());
    }
    if !dry_run {
        report.push("Next: close and restart your agent apps, since a running session keeps the old plugin. In Codex, review the changed hooks under /hooks.".into());
    }
    let report = report.join("\n");
    if failed { Err(report) } else { Ok(report) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_tag_is_a_release_and_anything_else_follows_main() {
        assert_eq!(channel_of(Some("v0.5.0")), Channel::Release);
        assert_eq!(channel_of(Some("v12.0.31")), Channel::Release);
        for other in [
            Some("main"),
            Some("nightly"),
            Some("v0.5"),
            Some("v0.5.0-rc1"),
            None,
        ] {
            assert_eq!(channel_of(other), Channel::Nightly, "{other:?}");
        }
    }

    #[test]
    fn codex_config_gives_the_marketplace_ref() {
        let config = "[x]\nref = \"no\"\n[marketplaces.astrolabe]\nsource_type = \"git\"\nref = \"main\"\n[plugins.y]\n";
        assert_eq!(codex_ref(config), Some(Some("main".into())));
        assert_eq!(
            codex_ref("[marketplaces.astrolabe]\nsource = \"C:/x\"\n[next]\nref = \"v1.0.0\"\n"),
            Some(None)
        );
        assert_eq!(codex_ref("[marketplaces.other]\nref = \"main\"\n"), None);
    }

    fn nightly(current: bool) -> Target {
        Target {
            git_ref: "nightly".into(),
            label: "0.5.0 nightly 1a2b3c4".into(),
            current,
        }
    }

    #[test]
    fn with_no_app_found_only_the_program_moves() {
        let apps = [(App::Claude, None), (App::Codex, None)];
        let refs = [(App::Claude, None), (App::Codex, None)];
        let scratch = std::env::temp_dir();
        let report = run(
            Channel::Nightly,
            &nightly(false),
            &apps,
            &refs,
            &scratch,
            true,
            false,
        )
        .unwrap();
        assert!(
            report.contains("Program: would run the 0.5.0 nightly 1a2b3c4 installer"),
            "{report}"
        );
        assert!(
            report.contains("Claude Code: skipped, `claude` isn't on PATH."),
            "{report}"
        );
        assert!(
            report.contains("Codex: skipped, `codex` isn't on PATH."),
            "{report}"
        );
        assert!(report.contains("only the program was updated"), "{report}");
    }

    #[test]
    fn an_app_already_on_the_target_is_left_alone() {
        let cli = PathBuf::from("claude");
        let apps = [
            (App::Claude, Some(cli.clone())),
            (App::Codex, Some(PathBuf::from("codex"))),
        ];
        let refs = [
            (App::Claude, Some(Some("nightly".into()))),
            (App::Codex, Some(Some("main".into()))),
        ];
        let scratch = std::env::temp_dir();
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &apps,
            &refs,
            &scratch,
            true,
            false,
        )
        .unwrap();
        assert!(report.contains("Program: already this build."), "{report}");
        assert!(
            report.contains("Claude Code: plugin already follows nightly."),
            "{report}"
        );
        assert!(
            report.contains("Codex: would run codex plugin remove"),
            "{report}"
        );
        assert!(report.contains("--ref nightly"), "{report}");
        let refs = [
            (App::Claude, Some(Some("nightly".into()))),
            (App::Codex, Some(Some("nightly".into()))),
        ];
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &apps,
            &refs,
            &scratch,
            true,
            false,
        )
        .unwrap();
        assert!(report.ends_with("Already up to date."), "{report}");
    }

    #[test]
    fn each_app_reregisters_at_the_target_ref() {
        let claude = App::Claude.steps("nightly");
        assert!(
            claude[2]
                .0
                .last()
                .unwrap()
                .ends_with("Astrolabe.git#nightly")
                && claude[2].1
        );
        assert!(
            !claude[0].1 && !claude[1].1,
            "removals may find nothing to remove"
        );
        let codex = App::Codex.steps("v0.6.0");
        assert_eq!(
            codex[2].0[codex[2].0.len() - 2..],
            ["--ref".to_owned(), "v0.6.0".to_owned()]
        );
        assert_eq!(codex[3].0, ["plugin", "add", PLUGIN]);
    }
}
