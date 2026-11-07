//! `astrolabe update`: the program and each app's plugin, moved together to the newest
//! release or nightly. It runs the same published installer and app commands a person
//! would, so it adds no download or install path of its own.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set by CI: the commit a build came from, and `nightly` for a build of main.
const COMMIT: Option<&str> = option_env!("ASTROLABE_COMMIT");
const CHANNEL: Option<&str> = option_env!("ASTROLABE_CHANNEL");
const REPO: &str = "https://github.com/NateChristiansen/Astrolabe";
const PLUGIN: &str = "astrolabe@astrolabe";
const MARKETPLACE: &str = "astrolabe";
/// Where the plugin's files sit in the marketplace's checkout of this repo.
const PLUGIN_DIR: &str = "plugin/astrolabe";

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

    /// Pull the newest commit at the ref the marketplace already follows.
    fn refresh(self) -> Vec<String> {
        match self {
            App::Claude => owned(&["plugin", "marketplace", "update", MARKETPLACE]),
            App::Codex => owned(&["plugin", "marketplace", "upgrade", MARKETPLACE]),
        }
    }

    /// Replace the installed copy with the marketplace's. Both apps cache the plugin by
    /// its version, which stays the same across nightlies, so their own update commands
    /// call a changed nightly current. The removal may fail when nothing is installed.
    pub fn reinstall(self) -> Vec<(Vec<String>, bool)> {
        match self {
            App::Claude => vec![
                (owned(&["plugin", "uninstall", PLUGIN]), false),
                (owned(&["plugin", "install", PLUGIN]), true),
            ],
            App::Codex => vec![
                (owned(&["plugin", "remove", PLUGIN]), false),
                (owned(&["plugin", "add", PLUGIN]), true),
            ],
        }
    }

    /// The commands that swap the registration for one at `git_ref`, as the install
    /// guide gives them. The removals may fail when nothing is registered.
    pub fn steps(self, git_ref: &str) -> Vec<(Vec<String>, bool)> {
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

fn owned(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
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
        App::Codex => codex_ref(&std::fs::read_to_string(codex_home()?.join("config.toml")).ok()?),
    }
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".codex")))
}

/// The plugin as the app installed it, then the marketplace's copy it should match, read
/// from the app's own records. None when they don't show both, which counts as stale.
fn copies(app: App) -> Option<(PathBuf, PathBuf)> {
    match app {
        App::Claude => {
            let plugins = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .or_else(|| home().map(|h| h.join(".claude")))?
                .join("plugins");
            let installed = read_json(&plugins.join("installed_plugins.json")).ok()?;
            let installed = installed["plugins"][PLUGIN]
                .as_array()?
                .iter()
                .find_map(|e| e["installPath"].as_str())?;
            let known = read_json(&plugins.join("known_marketplaces.json")).ok()?;
            let clone = known[MARKETPLACE]["installLocation"].as_str()?;
            Some((PathBuf::from(installed), Path::new(clone).join(PLUGIN_DIR)))
        }
        App::Codex => {
            // Codex keeps no install record: its cache is marketplace/plugin/version, and the
            // plugin shares the marketplace's name.
            let home = codex_home()?;
            let source = home
                .join(".tmp/marketplaces")
                .join(MARKETPLACE)
                .join(PLUGIN_DIR);
            let manifest = read_json(&source.join(".claude-plugin/plugin.json")).ok()?;
            let version = manifest["version"].as_str()?;
            let installed = home
                .join("plugins/cache")
                .join(MARKETPLACE)
                .join(MARKETPLACE)
                .join(version);
            Some((installed, source))
        }
    }
}

/// Whether two copies of the plugin differ in any file, either way. Claude Code's `.in_use`
/// bookkeeping beside the files doesn't count, and a copy that can't be read differs.
pub fn differs(installed: &Path, source: &Path) -> bool {
    match (files(installed), files(source)) {
        (Some(a), Some(b)) => a != b,
        _ => true,
    }
}

fn files(root: &Path) -> Option<BTreeMap<PathBuf, Vec<u8>>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            if dir == root && entry.file_name() == ".in_use" {
                continue;
            }
            let path = entry.path();
            if entry.file_type().ok()?.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).ok()?;
                found.insert(path.strip_prefix(root).ok()?.to_path_buf(), bytes);
            }
        }
    }
    Some(found)
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
    let seen: Vec<Seen> = App::ALL
        .iter()
        .map(|&app| {
            let cli = on_path(app.cli());
            let follows = cli.as_deref().and_then(|c| registered(app, c));
            Seen { app, cli, follows }
        })
        .collect();
    let channel = chosen.unwrap_or_else(|| match seen.iter().find_map(|s| s.follows.clone()) {
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
        .and_then(|target| run(channel, &target, &seen, &copies, &scratch, dry_run, force));
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// One app as this command found it: its command if it's on PATH, and where its
/// marketplace points (see `registered`).
struct Seen {
    app: App,
    cli: Option<PathBuf>,
    follows: Option<Option<String>>,
}

/// What an app's plugin needs.
enum Plugin {
    /// Its command isn't on PATH.
    Missing,
    /// It follows the target and its installed copy matches the marketplace's.
    Current,
    /// It follows the target, but the installed copy is behind the marketplace's.
    Reinstall,
    /// It follows another ref, or none: register it again at the target.
    Reregister,
    Failed(String),
}

/// Run each command in turn, stopping at the first required one that fails.
fn perform(app: App, cli: &Path, steps: &[(Vec<String>, bool)]) -> Result<(), String> {
    for (words, required) in steps {
        match Command::new(cli).args(words).output() {
            Ok(out) if out.status.success() || !required => {}
            Ok(out) => {
                return Err(format!(
                    "`{}` failed: {}",
                    shown(app.cli(), words),
                    last_line(&out.stderr)
                ));
            }
            Err(e) => return Err(format!("couldn't run `{}`: {e}", app.cli())),
        }
    }
    Ok(())
}

fn run(
    channel: Channel,
    target: &Target,
    seen: &[Seen],
    copies: &dyn Fn(App) -> Option<(PathBuf, PathBuf)>,
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

    // A marketplace that already follows the target is refreshed first, so a nightly
    // that changed the plugin shows before anything is called up to date. A dry run
    // compares with the marketplace's copy as it stands.
    let plans: Vec<(App, Option<&PathBuf>, Plugin)> = seen
        .iter()
        .map(|s| {
            let Some(cli) = s.cli.as_ref() else {
                return (s.app, None, Plugin::Missing);
            };
            let follows = s
                .follows
                .as_ref()
                .is_some_and(|r| r.as_deref() == Some(target.git_ref.as_str()));
            let refreshed = || match dry_run {
                true => Ok(()),
                false => perform(s.app, cli, &[(s.app.refresh(), true)]),
            };
            let plan = if force || !follows {
                Plugin::Reregister
            } else if let Err(why) = refreshed() {
                Plugin::Failed(why)
            } else if copies(s.app).is_some_and(|(installed, source)| !differs(&installed, &source))
            {
                Plugin::Current
            } else {
                Plugin::Reinstall
            };
            (s.app, Some(cli), plan)
        })
        .collect();
    let settled = plans
        .iter()
        .all(|(_, _, p)| matches!(p, Plugin::Missing | Plugin::Current));
    if !force && !dry_run && target.current && settled {
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

    for (app, cli, plan) in &plans {
        let (label, git_ref) = (app.label(), &target.git_ref);
        let refresh = shown(app.cli(), &app.refresh());
        let (steps, done) = match plan {
            Plugin::Missing => {
                report.push(format!("{label}: skipped, `{}` isn't on PATH.", app.cli()));
                continue;
            }
            Plugin::Failed(why) => {
                failed = true;
                report.push(format!("{label}: {why}"));
                continue;
            }
            Plugin::Current if dry_run => {
                report.push(format!(
                    "{label}: plugin matches its marketplace's copy; would run {refresh}, and reinstall if that changes it."
                ));
                continue;
            }
            Plugin::Current => {
                report.push(format!("{label}: plugin already matches {git_ref}."));
                continue;
            }
            Plugin::Reinstall => (app.reinstall(), "reinstalled from"),
            Plugin::Reregister => (app.steps(git_ref), "now follows"),
        };
        if dry_run {
            // A real run refreshes the marketplace before this step; a dry run hasn't.
            let first = matches!(plan, Plugin::Reinstall).then_some(refresh);
            let commands: Vec<String> = first
                .into_iter()
                .chain(steps.iter().map(|(w, _)| shown(app.cli(), w)))
                .collect();
            report.push(format!("{label}: would run {}.", commands.join("; ")));
            continue;
        }
        match cli.map_or(Ok(()), |cli| perform(*app, cli, &steps)) {
            Ok(()) => report.push(format!("{label}: plugin {done} {git_ref}.")),
            Err(why) => {
                failed = true;
                report.push(format!("{label}: {why}"));
            }
        }
    }
    if plans.iter().all(|(_, _, p)| matches!(p, Plugin::Missing)) {
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

    /// A fresh folder for one test, removed when it ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "astrolabe-update-test-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A copy of the plugin with a manifest and these hooks.
    fn plugin(at: &Path, hooks: &str) -> PathBuf {
        std::fs::create_dir_all(at.join(".claude-plugin")).unwrap();
        std::fs::create_dir_all(at.join("hooks")).unwrap();
        std::fs::write(
            at.join(".claude-plugin/plugin.json"),
            r#"{"name":"astrolabe","version":"0.5.0"}"#,
        )
        .unwrap();
        std::fs::write(at.join("hooks/hooks.json"), hooks).unwrap();
        at.to_path_buf()
    }

    const OLD_HOOKS: &str = r#"{"hooks":{"PreToolUse":[]}}"#;
    const NEW_HOOKS: &str = r#"{"hooks":{"PreToolUse":[],"UserPromptSubmit":[]}}"#;

    /// A stand-in for an app's command that only writes down each call.
    fn fake_cli(dir: &Path) -> PathBuf {
        if cfg!(windows) {
            let cli = dir.join("app.cmd");
            std::fs::write(&cli, "@echo off\r\necho %*>>\"%~dp0calls.txt\"\r\n").unwrap();
            cli
        } else {
            let cli = dir.join("app");
            std::fs::write(
                &cli,
                "#!/bin/sh\necho \"$*\" >> \"$(dirname \"$0\")/calls.txt\"\n",
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            cli
        }
    }

    fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.txt"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_owned())
            .collect()
    }

    fn following(git_ref: &str, cli: Option<PathBuf>) -> [Seen; 2] {
        [App::Claude, App::Codex].map(|app| Seen {
            app,
            cli: cli.clone(),
            follows: Some(Some(git_ref.into())),
        })
    }

    #[test]
    fn copies_differ_by_any_file_but_not_by_the_in_use_marks() {
        let dir = Scratch::new("differs");
        let source = plugin(&dir.0.join("source"), NEW_HOOKS);
        let installed = plugin(&dir.0.join("installed"), NEW_HOOKS);
        std::fs::create_dir_all(installed.join(".in_use")).unwrap();
        std::fs::write(installed.join(".in_use/1234"), "").unwrap();
        assert!(!differs(&installed, &source));

        std::fs::write(installed.join("hooks/hooks.json"), OLD_HOOKS).unwrap();
        assert!(differs(&installed, &source), "changed hooks");

        std::fs::write(installed.join("hooks/hooks.json"), NEW_HOOKS).unwrap();
        std::fs::write(installed.join("hooks/removed.json"), "{}").unwrap();
        assert!(differs(&installed, &source), "a file the source dropped");

        assert!(
            differs(&dir.0.join("missing"), &source),
            "no installed copy"
        );
    }

    #[test]
    fn with_no_app_found_only_the_program_moves() {
        let seen = following("nightly", None);
        let scratch = std::env::temp_dir();
        let report = run(
            Channel::Nightly,
            &nightly(false),
            &seen,
            &|_| None,
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
    fn a_dry_run_checks_each_copy_and_reregisters_another_ref() {
        let dir = Scratch::new("dry-run");
        let source = plugin(&dir.0.join("source"), NEW_HOOKS);
        let stale = plugin(&dir.0.join("stale"), OLD_HOOKS);
        let current = plugin(&dir.0.join("current"), NEW_HOOKS);
        let mut seen = following("nightly", Some(PathBuf::from("app")));
        seen[1].follows = Some(Some("main".into()));
        let copies = |app| {
            Some((
                if app == App::Claude { &current } else { &stale }.clone(),
                source.clone(),
            ))
        };
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &seen,
            &copies,
            &dir.0,
            true,
            false,
        )
        .unwrap();
        assert!(report.contains("Program: already this build."), "{report}");
        assert!(
            report.contains("Claude Code: plugin matches its marketplace's copy; would run claude plugin marketplace update astrolabe, and reinstall if that changes it."),
            "{report}"
        );
        assert!(
            report.contains("Codex: would run codex plugin remove"),
            "{report}"
        );
        assert!(report.contains("--ref nightly"), "{report}");

        // Both on nightly now, and Claude Code's copy is behind the marketplace's.
        let seen = following("nightly", Some(PathBuf::from("app")));
        let copies = |app| {
            Some((
                if app == App::Claude { &stale } else { &current }.clone(),
                source.clone(),
            ))
        };
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &seen,
            &copies,
            &dir.0,
            true,
            false,
        )
        .unwrap();
        assert!(
            report.contains("Claude Code: would run claude plugin marketplace update astrolabe; claude plugin uninstall astrolabe@astrolabe; claude plugin install astrolabe@astrolabe."),
            "{report}"
        );
        assert!(!report.contains("Already up to date"), "{report}");
    }

    #[test]
    fn a_stale_copy_on_the_same_nightly_is_reinstalled() {
        let dir = Scratch::new("stale");
        let cli = fake_cli(&dir.0);
        let source = plugin(&dir.0.join("source"), NEW_HOOKS);
        let stale = plugin(&dir.0.join("stale"), OLD_HOOKS);
        let current = plugin(&dir.0.join("current"), NEW_HOOKS);
        let seen = following("nightly", Some(cli));

        // The program is this nightly and both apps follow it, but Claude Code's cached
        // copy predates a new hook: the case the version-keyed caches miss.
        let copies = |app| {
            Some((
                if app == App::Claude { &stale } else { &current }.clone(),
                source.clone(),
            ))
        };
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &seen,
            &copies,
            &dir.0,
            false,
            false,
        )
        .unwrap();
        assert!(
            report.contains("Claude Code: plugin reinstalled from nightly."),
            "{report}"
        );
        assert!(
            report.contains("Codex: plugin already matches nightly."),
            "{report}"
        );
        assert_eq!(
            calls(&dir.0),
            [
                "plugin marketplace update astrolabe",
                "plugin marketplace upgrade astrolabe",
                "plugin uninstall astrolabe@astrolabe",
                "plugin install astrolabe@astrolabe",
            ]
        );

        // Once both copies match, only the marketplaces are refreshed.
        std::fs::remove_file(dir.0.join("calls.txt")).unwrap();
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &seen,
            &|_| Some((current.clone(), source.clone())),
            &dir.0,
            false,
            false,
        )
        .unwrap();
        assert!(report.ends_with("Already up to date."), "{report}");
        assert_eq!(
            calls(&dir.0),
            [
                "plugin marketplace update astrolabe",
                "plugin marketplace upgrade astrolabe",
            ]
        );
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
