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

    /// Add the marketplace from `source`, at `git_ref` when it has one.
    fn register(self, source: &str, git_ref: Option<&str>) -> Vec<String> {
        let mut words = owned(&["plugin", "marketplace", "add"]);
        match (self, git_ref) {
            (App::Claude, Some(git_ref)) => words.push(format!("{source}#{git_ref}")),
            (App::Codex, Some(git_ref)) => words.extend(owned(&[source, "--ref", git_ref])),
            (_, None) => words.push(source.to_owned()),
        }
        words
    }

    /// Install the plugin from the registered marketplace.
    fn install(self) -> Vec<String> {
        match self {
            App::Claude => owned(&["plugin", "install", PLUGIN]),
            App::Codex => owned(&["plugin", "add", PLUGIN]),
        }
    }

    /// Replace the installed copy with the marketplace's. Both apps cache the plugin by
    /// its version, which stays the same across nightlies, so their own update commands
    /// call a changed nightly current. Codex's add replaces an installed copy. Claude
    /// Code's install and update both call one current, so its copy is uninstalled first,
    /// keeping its saved data; that removal may fail when nothing is installed.
    pub fn reinstall(self) -> Vec<(Vec<String>, bool)> {
        match self {
            App::Claude => vec![
                (
                    owned(&["plugin", "uninstall", PLUGIN, "--keep-data"]),
                    false,
                ),
                (self.install(), true),
            ],
            App::Codex => vec![(self.install(), true)],
        }
    }

    /// What has to go before a marketplace can be added at another ref, kept as narrow
    /// as each app allows. Claude Code's marketplace removal uninstalls the plugin and
    /// deletes its saved data, and without a scope it does so in every scope: the plugin
    /// goes first with its data kept, then only the user declaration. Codex keeps the
    /// plugin's entry and installed copy when its marketplace goes. Each may fail when
    /// there is nothing to remove.
    fn removals(self) -> Vec<Vec<String>> {
        match self {
            App::Claude => vec![
                owned(&["plugin", "uninstall", PLUGIN, "--keep-data"]),
                owned(&[
                    "plugin",
                    "marketplace",
                    "remove",
                    MARKETPLACE,
                    "--scope",
                    "user",
                ]),
            ],
            App::Codex => vec![owned(&["plugin", "marketplace", "remove", MARKETPLACE])],
        }
    }

    /// The commands that swap the registration for one from `source`, both of which
    /// must succeed after the removals.
    fn swap(self, source: &str, git_ref: Option<&str>) -> Vec<(Vec<String>, bool)> {
        let removals = self.removals().into_iter().map(|words| (words, false));
        let adds = [self.register(source, git_ref), self.install()];
        removals
            .chain(adds.into_iter().map(|words| (words, true)))
            .collect()
    }

    /// The commands that swap the registration for the hosted one at `git_ref`.
    pub fn steps(self, git_ref: &str) -> Vec<(Vec<String>, bool)> {
        self.swap(&format!("{REPO}.git"), Some(git_ref))
    }
}

fn owned(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

/// Where an app's Astrolabe marketplace points now: None when it has none registered,
/// Some(None) when it follows the default branch or a folder. With it, what the
/// marketplace was added from, when the app's records show it.
type Registration = Option<(Option<String>, Option<String>)>;

/// An app's registration, or why its own record of it can't be read.
fn registered(app: App, cli: &Path) -> Result<Registration, String> {
    match app {
        App::Claude => Ok((|| {
            let out = Command::new(cli)
                .args(["plugin", "marketplace", "list", "--json"])
                .output()
                .ok()?;
            let list: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
            let entry = list.as_array()?.iter().find(|m| m["name"] == MARKETPLACE)?;
            // A Git URL, a GitHub `owner/repo` or a folder, as `marketplace add` takes them.
            let source = ["url", "repo", "path"]
                .iter()
                .find_map(|key| entry[*key].as_str());
            Some((
                entry["ref"].as_str().map(str::to_owned),
                source.map(str::to_owned),
            ))
        })()),
        App::Codex => {
            let Some(path) = codex_home().map(|home| home.join("config.toml")) else {
                return Ok(None);
            };
            match std::fs::read_to_string(&path) {
                Ok(config) => codex_registration(&config, &path),
                // Codex with no config has nothing registered.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(unread(&path, &e.to_string())),
            }
        }
    }
}

/// Codex's registration from its config at `path`, or why that file can't be trusted. A
/// config that isn't TOML is not an empty one: swapping a registration nobody could
/// read would remove it with nothing to put back.
fn codex_registration(config: &str, path: &Path) -> Result<Registration, String> {
    if let Err(e) = config.parse::<toml_edit::DocumentMut>() {
        let why = e.to_string();
        return Err(unread(
            path,
            why.lines().next().unwrap_or("it is not valid TOML"),
        ));
    }
    Ok(codex_ref(config).map(|git_ref| (git_ref, codex_source(config))))
}

fn unread(path: &Path, why: &str) -> String {
    format!(
        "{} couldn't be read ({why}), so its plugin wasn't touched. Fix that file, then run `astrolabe update` again.",
        path.display()
    )
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

/// The `ref` of `[marketplaces.astrolabe]` in Codex's config: None when the table isn't
/// there or the file isn't TOML, Some(None) when the table has no ref.
pub fn codex_ref(config: &str) -> Option<Option<String>> {
    codex_marketplace(config, "ref")
}

/// The `source` of that table: a URL or a folder, as `marketplace add` takes it.
fn codex_source(config: &str) -> Option<String> {
    codex_marketplace(config, "source")?
}

/// One string of `[marketplaces.astrolabe]`, read as TOML: a person may have edited the
/// file, so a comment, a quoted name or either kind of string has to read correctly.
fn codex_marketplace(config: &str, key: &str) -> Option<Option<String>> {
    let config = config.parse::<toml_edit::DocumentMut>().ok()?;
    let table = config.get("marketplaces")?.get(MARKETPLACE)?;
    table.as_table_like()?;
    Some(table.get(key).and_then(|v| v.as_str()).map(str::to_owned))
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

/// Run the target's published installer into the verified installation folder. The
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
    let before = crate::install::program_digest(folder);
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
        Err(format!(
            "the installer stopped; its message is above, and {}",
            crate::install::left_after_failure(folder, before.as_deref())
        ))
    }
}

/// Whether the plugin's repository answers and has `git_ref`, asked before a registration
/// is removed to make room for one at that ref.
fn reachable(git_ref: &str) -> Result<(), String> {
    let out = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["ls-remote", "--exit-code", &format!("{REPO}.git"), git_ref])
        .output()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(()),
        Some(2) => Err(format!("{REPO} has no {git_ref}")),
        _ => Err(last_line(&out.stderr)),
    }
}

/// A command as a person would paste it, into PowerShell on Windows and a POSIX shell
/// elsewhere, as the install guide has them. A word the shell would split or expand,
/// such as a folder with a space, is quoted; the rest print bare.
fn shown(cli: &str, words: &[String]) -> String {
    let powershell = cfg!(windows);
    let bare = |word: &str| {
        let plain = |c: char| {
            c.is_ascii_alphanumeric() || "_@+=:./#-".contains(c) || (powershell && c == '\\')
        };
        !word.is_empty() && !word.starts_with(['#', '@']) && word.chars().all(plain)
    };
    let words: Vec<String> = words
        .iter()
        .map(|word| match bare(word) {
            true => word.clone(),
            false => crate::activate::quoted(word, powershell),
        })
        .collect();
    format!("{cli} {}", words.join(" "))
}

/// A receipted custom install wins. Build outputs fall back only to the normal install,
/// not to an arbitrary executable found on PATH.
fn update_program(running: &Path, default: Option<&Path>) -> Result<PathBuf, String> {
    let folder = running
        .parent()
        .ok_or("can't tell which folder this program runs from")?;
    if let Some(program) = crate::install::installed_program(folder)? {
        return Ok(program);
    }
    if let Some(default) = default
        && let Some(program) = crate::install::installed_program(default)?
    {
        return Ok(program);
    }
    Err(format!(
        "{} is not a receipted installation, and no verified user-local installation was found. Install Astrolabe with the published installer or `astrolabe install` before updating; build outputs are left alone.",
        running.display()
    ))
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

    let running = std::env::current_exe().map_err(|e| e.to_string())?;
    let default = crate::install::default_directory().ok();
    let installed = update_program(&running, default.as_deref())?;
    let running = std::fs::canonicalize(&running).map_err(|e| e.to_string())?;
    if installed != running {
        // The installed copy knows its own channel and exact build identity. In
        // particular, a current development build must not skip an older install.
        println!("Using installed Astrolabe at {}.", installed.display());
        let status = Command::new(&installed)
            .arg("update")
            .args(args)
            .status()
            .map_err(|e| format!("couldn't run installed Astrolabe: {e}"))?;
        return if status.success() {
            Ok(String::new())
        } else {
            Err("the installed copy's update failed; its message is above".into())
        };
    }

    // Each app is found by its command; one that isn't there is skipped, not an error.
    let seen: Vec<Seen> = App::ALL
        .iter()
        .map(|&app| {
            let cli = on_path(app.cli());
            let (registration, unreadable) = match cli.as_deref().map(|c| registered(app, c)) {
                Some(Ok(registration)) => (registration, None),
                Some(Err(why)) => (None, Some(why)),
                None => (None, None),
            };
            let (follows, source) = registration.unzip();
            Seen {
                app,
                cli,
                follows,
                source: source.flatten(),
                unreadable,
            }
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
    let result = target(channel, &scratch).and_then(|target| {
        run(
            channel, &target, &seen, &copies, &reachable, &scratch, dry_run, force,
        )
    });
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// One app as this command found it: its command if it's on PATH, and where its
/// marketplace points (see `registered`).
struct Seen {
    app: App,
    cli: Option<PathBuf>,
    follows: Option<Option<String>>,
    /// What that marketplace was added from (a URL, `owner/repo` or a folder), when the
    /// app's records show it: what a failed ref change adds back.
    source: Option<String>,
    /// Why the app's own record of its marketplace couldn't be read, when it couldn't:
    /// its plugin is left alone, since nothing says what is there to put back.
    unreadable: Option<String>,
}

/// Said with recovery commands that start with removals whose outcome isn't known.
const UNSURE: &str =
    "A removal at the start may report that there is nothing to remove; carry on if it does.";

/// An app's commands in order, each with whether it must succeed.
type Steps = Vec<(Vec<String>, bool)>;

/// What an app's plugin needs.
enum Plugin {
    /// Its command isn't on PATH.
    Missing,
    /// It follows the target and its installed copy matches the marketplace's.
    Current,
    /// It follows the target, but the installed copy is behind the marketplace's.
    Reinstall,
    /// It follows another ref, or none: register it again at the target. With it, the
    /// commands that put back the registration it has now, when the app's records give
    /// its source, and how that registration reads.
    Reregister(Option<(Steps, String)>),
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

#[allow(clippy::too_many_arguments)]
fn run(
    channel: Channel,
    target: &Target,
    seen: &[Seen],
    copies: &dyn Fn(App) -> Option<(PathBuf, PathBuf)>,
    preflight: &dyn Fn(&str) -> Result<(), String>,
    scratch: &Path,
    dry_run: bool,
    force: bool,
) -> Result<String, String> {
    let folder = std::env::current_exe()
        .and_then(std::fs::canonicalize)
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
            if let Some(why) = &s.unreadable {
                return (s.app, Some(cli), Plugin::Failed(why.clone()));
            }
            let follows = s
                .follows
                .as_ref()
                .is_some_and(|r| r.as_deref() == Some(target.git_ref.as_str()));
            let refreshed = || match dry_run {
                true => Ok(()),
                false => perform(s.app, cli, &[(s.app.refresh(), true)]),
            };
            let plan = if !follows {
                let old = s.follows.as_ref().zip(s.source.as_ref());
                Plugin::Reregister(old.map(|(git_ref, source)| {
                    (
                        s.app.swap(source, git_ref.as_deref()),
                        git_ref.as_ref().unwrap_or(source).clone(),
                    )
                }))
            } else if let Err(why) = refreshed() {
                Plugin::Failed(why)
            } else if !force
                && copies(s.app).is_some_and(|(installed, source)| !differs(&installed, &source))
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

    let mut checked = None;
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
            Plugin::Reregister(_) => (app.steps(git_ref), "now follows"),
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
        let Some(cli) = cli else { continue };
        // Nothing is removed for a ref that can't be fetched. Asked once for both apps.
        if matches!(plan, Plugin::Reregister(_))
            && let Err(why) = checked.get_or_insert_with(|| preflight(git_ref))
        {
            failed = true;
            report.push(format!(
                "{label}: nothing was changed, because {git_ref} couldn't be confirmed before removing the current registration: {why}. Run `astrolabe update` again once it can be fetched."
            ));
            continue;
        }
        let commands = |steps: &[(Vec<String>, bool)], all: bool| {
            let shown: Vec<String> = steps
                .iter()
                .filter(|(_, required)| all || *required)
                .map(|(words, _)| format!("`{}`", shown(app.cli(), words)))
                .collect();
            shown.join(", then ")
        };
        let Err(why) = perform(*app, cli, &steps) else {
            // An app can report success for a copy it left alone, so look again.
            let stale = matches!(plan, Plugin::Reinstall)
                && !copies(*app).is_some_and(|(installed, source)| !differs(&installed, &source));
            if stale {
                failed = true;
                report.push(format!(
                    "{label}: the reinstall ran, but the installed copy still doesn't match its marketplace's copy, so it was left as it was. To finish, run {} and read what they print.",
                    commands(&steps, true)
                ));
            } else {
                report.push(format!("{label}: plugin {done} {git_ref}."));
            }
            continue;
        };
        failed = true;
        let retry = commands(&steps, false);
        report.push(match plan {
            Plugin::Reregister(Some((back, old))) => {
                // The old marketplace first, then its plugin, so a failure shows which
                // registration the app is left with.
                let (marketplace, plugin) = back.split_at(back.len() - 1);
                let registered = perform(*app, cli, marketplace);
                match registered.clone().and_then(|()| perform(*app, cli, plugin)) {
                    Ok(()) => format!(
                        "{label}: {why}. The previous registration was put back, so the plugin is still installed from {old}. To move it to {git_ref} yourself, run {}.",
                        commands(&steps, true)
                    ),
                    // Neither app adds another ref over a registration, so with the old
                    // marketplace back the way to the target starts with the removals.
                    Err(again) if registered.is_ok() => format!(
                        "{label}: {why}. The previous registration couldn't be put back ({again}), so {label} may have no Astrolabe plugin, hooks or guard until one of these succeeds. For {git_ref}, run {}. For {old}, run {}.",
                        commands(&steps, true),
                        commands(plugin, true)
                    ),
                    // The old marketplace wasn't added, which a removal that didn't take
                    // would also cause. What is registered is in doubt, so each way starts
                    // with the removals.
                    Err(again) => format!(
                        "{label}: {why}. The previous registration couldn't be put back ({again}), so {label} may have no Astrolabe plugin, hooks or guard until one of these succeeds. For {git_ref}, run {}. For {old}, run {}. {UNSURE}",
                        commands(&steps, true),
                        commands(back, true)
                    ),
                }
            }
            Plugin::Reregister(None) => format!(
                "{label}: {why}. {label} may have no Astrolabe plugin, hooks or guard until this succeeds: run {}. {UNSURE}",
                commands(&steps, true)
            ),
            // Only Claude Code's reinstall removes the installed copy first.
            _ if steps.len() > 1 => format!(
                "{label}: {why}. Its copy was uninstalled first, so {label} has no Astrolabe hooks or guard until this succeeds: run {retry}."
            ),
            _ => format!(
                "{label}: {why}. The installed copy was left as it was; run {retry} to try again."
            ),
        });
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
        let config = "[x]\nref = \"no\"\n[marketplaces.astrolabe]\nsource_type = \"git\"\nsource = \"https://example.invalid/a.git\"\nref = \"main\"\n[plugins.y]\n";
        assert_eq!(codex_ref(config), Some(Some("main".into())));
        assert_eq!(
            codex_ref("[marketplaces.astrolabe]\nsource = \"C:/x\"\n[next]\nref = \"v1.0.0\"\n"),
            Some(None)
        );
        assert_eq!(codex_ref("[marketplaces.other]\nref = \"main\"\n"), None);
        assert_eq!(
            codex_source(config).as_deref(),
            Some("https://example.invalid/a.git")
        );
        assert_eq!(
            codex_source("[marketplaces.astrolabe]\nsource = \"C:\\\\x\\\\plugin\"\n").as_deref(),
            Some("C:\\x\\plugin")
        );
        assert_eq!(
            codex_source("[marketplaces.astrolabe]\nsource = 'C:\\x'\n[next]\n").as_deref(),
            Some("C:\\x")
        );
        assert_eq!(
            codex_source("[marketplaces.astrolabe]\n[next]\nsource = \"x\"\n"),
            None
        );

        // A config edited by hand: comments after the values, and a quoted table name.
        let edited = "[marketplaces.\"astrolabe\"]\nsource = 'C:\\repos\\Astrolabe' # local checkout\nref = \"v1\" # pinned\n";
        assert_eq!(
            codex_source(edited).as_deref(),
            Some("C:\\repos\\Astrolabe")
        );
        assert_eq!(codex_ref(edited), Some(Some("v1".into())));
    }

    #[test]
    fn update_routes_only_to_intact_installations() {
        let root =
            std::env::temp_dir().join(format!("astrolabe-update-route-{}", std::process::id()));
        std::fs::create_dir_all(root.join("build")).unwrap();
        let name = if cfg!(windows) {
            "astrolabe.exe"
        } else {
            "astrolabe"
        };
        let build = root.join("build").join(name);
        std::fs::write(&build, "development build").unwrap();
        let default = root.join("user local");
        assert!(
            update_program(&build, Some(&default))
                .unwrap_err()
                .contains("no verified")
        );
        assert!(!default.exists(), "routing does not create an installation");
        crate::install::install(&build, &default).unwrap();
        let expected = std::fs::canonicalize(&default).unwrap().join(name);
        assert_eq!(update_program(&build, Some(&default)).unwrap(), expected);
        assert_eq!(update_program(&expected, None).unwrap(), expected);
        let custom = root.join("custom install");
        crate::install::install(&build, &custom).unwrap();
        let custom_program = std::fs::canonicalize(&custom).unwrap().join(name);
        assert_eq!(
            update_program(&custom_program, Some(&default)).unwrap(),
            custom_program
        );

        // A damaged custom install must not silently redirect to the healthy default.
        std::fs::write(&custom_program, "changed outside installer").unwrap();
        assert!(
            update_program(&custom_program, Some(&default))
                .unwrap_err()
                .contains("changed outside")
        );
        std::fs::remove_file(&custom_program).unwrap();
        assert!(
            update_program(&custom_program, Some(&default))
                .unwrap_err()
                .contains("program is missing")
        );
        std::fs::write(custom.join(".astrolabe-install.json"), "invalid").unwrap();
        assert!(
            update_program(&custom_program, Some(&default))
                .unwrap_err()
                .contains("invalid installation receipt")
        );
        std::fs::write(custom.join(".astrolabe-install-pending"), "recovery").unwrap();
        assert!(
            update_program(&custom_program, Some(&default))
                .unwrap_err()
                .contains("interrupted")
        );

        #[cfg(unix)]
        {
            std::fs::remove_file(&expected).unwrap();
            std::os::unix::fs::symlink(&build, &expected).unwrap();
            assert!(
                update_program(&build, Some(&default))
                    .unwrap_err()
                    .contains("not a regular file")
            );
            std::fs::remove_file(&expected).unwrap();
            std::fs::copy(&build, &expected).unwrap();
            std::fs::remove_file(default.join(".astrolabe-install.json")).unwrap();
            std::os::unix::fs::symlink(
                custom.join(".astrolabe-install.json"),
                default.join(".astrolabe-install.json"),
            )
            .unwrap();
            assert!(
                update_program(&build, Some(&default))
                    .unwrap_err()
                    .contains("not a regular file")
            );
        }
        assert_eq!(
            std::fs::read_to_string(&build).unwrap(),
            "development build"
        );
        std::fs::remove_dir_all(root).unwrap();
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

    /// A stand-in for an app's command that writes down each call, and fails the calls
    /// listed in `fail.txt` beside it, one per line.
    fn fake_cli(dir: &Path) -> PathBuf {
        if cfg!(windows) {
            let cli = dir.join("app.cmd");
            std::fs::write(
                &cli,
                "@echo off\r\necho %*>>\"%~dp0calls.txt\"\r\nif not exist \"%~dp0fail.txt\" exit /b 0\r\nfindstr /x /l /c:\"%*\" \"%~dp0fail.txt\" >nul\r\nif errorlevel 1 exit /b 0\r\necho boom 1>&2\r\nexit /b 1\r\n",
            )
            .unwrap();
            cli
        } else {
            // The script is written under another name and copied into place by `cp`. If this
            // process wrote the file it runs, another test thread could fork while it was
            // open for writing, and Linux refuses to run a file a child still holds open
            // that way ("text file busy", rust-lang/rust#114554). `cp` has exited before the
            // run, so nothing holds it.
            let cli = dir.join("app");
            let source = dir.join("app.source");
            std::fs::write(
                &source,
                "#!/bin/sh\ndir=$(dirname \"$0\")\necho \"$*\" >> \"$dir/calls.txt\"\nif [ -f \"$dir/fail.txt\" ] && grep -qxF -- \"$*\" \"$dir/fail.txt\"; then\n  echo boom >&2\n  exit 1\nfi\n",
            )
            .unwrap();
            let copied = Command::new("cp").arg(&source).arg(&cli).status().unwrap();
            assert!(copied.success(), "cp {source:?} {cli:?}: {copied}");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            cli
        }
    }

    /// The installed copy as a real app would leave it: `current` once the app's install
    /// command has been called, `stale` until then.
    fn installed(dir: &Path, app: App, stale: &Path, current: &Path) -> PathBuf {
        match calls(dir).contains(&app.install().join(" ")) {
            true => current.to_path_buf(),
            false => stale.to_path_buf(),
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
            source: Some(format!("{REPO}.git")),
            unreadable: None,
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
            &|_| Ok(()),
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
            &|_| Ok(()),
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
            report.contains("Codex: would run codex plugin marketplace remove astrolabe; codex plugin marketplace add"),
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
            &|_| Ok(()),
            &dir.0,
            true,
            false,
        )
        .unwrap();
        assert!(
            report.contains("Claude Code: would run claude plugin marketplace update astrolabe; claude plugin uninstall astrolabe@astrolabe --keep-data; claude plugin install astrolabe@astrolabe."),
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
                match app {
                    App::Claude => installed(&dir.0, app, &stale, &current),
                    App::Codex => current.clone(),
                },
                source.clone(),
            ))
        };
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &seen,
            &copies,
            &|_| Ok(()),
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
                "plugin uninstall astrolabe@astrolabe --keep-data",
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
            &|_| Ok(()),
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
    fn a_codex_config_that_cannot_be_read_is_an_error_not_an_empty_registration() {
        let path = Path::new("/somewhere/.codex/config.toml");
        let config = "[marketplaces.astrolabe]\nsource = \"https://example.invalid/a.git\"\nref = \"main\"\n";
        assert_eq!(
            codex_registration(config, path),
            Ok(Some((
                Some("main".into()),
                Some("https://example.invalid/a.git".into())
            )))
        );
        assert_eq!(codex_registration("[plugins.other]\n", path), Ok(None));
        let why =
            codex_registration("[marketplaces.astrolabe\nref = \"main\"\n", path).unwrap_err();
        assert!(why.contains(&path.display().to_string()), "{why}");
        assert!(
            why.contains("couldn't be read") && why.contains("its plugin wasn't touched"),
            "{why}"
        );
        assert!(!why.contains('\n'), "one line: {why}");
    }

    #[test]
    fn an_app_whose_registration_cannot_be_read_is_left_alone() {
        let claude = Scratch::new("unreadable-claude");
        let codex = Scratch::new("unreadable-codex");
        let source = plugin(&claude.0.join("source"), NEW_HOOKS);
        let current = plugin(&claude.0.join("current"), NEW_HOOKS);
        let why = "/somewhere/config.toml couldn't be read, so its plugin wasn't touched.";
        let seen = [
            Seen {
                app: App::Claude,
                cli: Some(fake_cli(&claude.0)),
                follows: Some(Some("nightly".into())),
                source: Some(format!("{REPO}.git")),
                unreadable: None,
            },
            Seen {
                app: App::Codex,
                cli: Some(fake_cli(&codex.0)),
                follows: None,
                source: None,
                unreadable: Some(why.into()),
            },
        ];
        let copies = |_| Some((current.clone(), source.clone()));
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &seen,
            &copies,
            &|_| Ok(()),
            &claude.0,
            false,
            false,
        )
        .unwrap_err();
        assert!(report.contains(&format!("Codex: {why}")), "{report}");
        assert_eq!(calls(&codex.0), [""; 0], "no Codex command runs");
        // The other app is handled as usual.
        assert_eq!(calls(&claude.0), ["plugin marketplace update astrolabe"]);
        assert!(
            report.contains("Claude Code: plugin already matches nightly."),
            "{report}"
        );

        // The program still moves: a dry run, since a real one would install it.
        let report = run(
            Channel::Nightly,
            &nightly(false),
            &seen,
            &copies,
            &|_| Ok(()),
            &claude.0,
            true,
            false,
        )
        .unwrap_err();
        assert!(
            report.contains("Program: would run the 0.5.0 nightly 1a2b3c4 installer"),
            "{report}"
        );
        assert!(report.contains(&format!("Codex: {why}")), "{report}");
        assert!(!report.contains("Codex: would run"), "{report}");
        assert_eq!(calls(&codex.0), [""; 0]);
    }

    #[test]
    fn a_printed_command_quotes_an_argument_the_shell_would_split() {
        let folder = "/Users/me/Plugin Sources/Astrolabe";
        for app in App::ALL {
            assert_eq!(
                shown(app.cli(), &app.register(folder, None)),
                format!(
                    "{} plugin marketplace add '/Users/me/Plugin Sources/Astrolabe'",
                    app.cli()
                )
            );
        }
        // The quote inside a quoted word is spelled for the shell the command is pasted into.
        let quote = if cfg!(windows) { "''" } else { "'\\''" };
        assert_eq!(
            shown("codex", &App::Codex.register("/Users/O'Brien/x y", None)),
            format!("codex plugin marketplace add '/Users/O{quote}Brien/x y'")
        );
        // Words that need no quoting print as they always did.
        assert_eq!(
            shown("claude", &App::Claude.steps("nightly")[2].0),
            "claude plugin marketplace add https://github.com/NateChristiansen/Astrolabe.git#nightly"
        );
        assert_eq!(
            shown(
                "codex",
                &App::Codex.register("C:\\repos\\Astrolabe", Some("v0.6.0"))
            ),
            if cfg!(windows) {
                "codex plugin marketplace add C:\\repos\\Astrolabe --ref v0.6.0"
            } else {
                "codex plugin marketplace add 'C:\\repos\\Astrolabe' --ref v0.6.0"
            }
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
        // The narrowest removal: the plugin's data and other scopes' declarations stay.
        assert_eq!(claude[0].0.last().unwrap(), "--keep-data");
        assert_eq!(claude[1].0[claude[1].0.len() - 2..], ["--scope", "user"]);
        let codex = App::Codex.steps("v0.6.0");
        assert_eq!(codex[0].0, ["plugin", "marketplace", "remove", MARKETPLACE]);
        assert!(!codex[0].1, "the removal may find nothing to remove");
        assert_eq!(
            codex[1].0[codex[1].0.len() - 2..],
            ["--ref".to_owned(), "v0.6.0".to_owned()]
        );
        assert_eq!(codex[2].0, ["plugin", "add", PLUGIN]);
    }

    /// One app found, registered from the hosted repository at `git_ref`.
    fn only(app: App, cli: PathBuf, git_ref: &str) -> [Seen; 1] {
        [Seen {
            app,
            cli: Some(cli),
            follows: Some(Some(git_ref.into())),
            source: Some(format!("{REPO}.git")),
            unreadable: None,
        }]
    }

    fn fail(dir: &Path, calls: &[&[String]]) {
        // Windows' findstr matches a whole line only when it ends as Windows' lines do.
        let end = if cfg!(windows) { "\r\n" } else { "\n" };
        let lines: Vec<String> = calls.iter().map(|words| words.join(" ") + end).collect();
        std::fs::write(dir.join("fail.txt"), lines.concat()).unwrap();
    }

    #[test]
    fn a_same_ref_update_removes_no_registration_even_when_forced() {
        let dir = Scratch::new("same-ref");
        let cli = fake_cli(&dir.0);
        let source = plugin(&dir.0.join("source"), NEW_HOOKS);
        let stale = plugin(&dir.0.join("stale"), OLD_HOOKS);
        let current = plugin(&dir.0.join("current"), NEW_HOOKS);
        let seen = following("nightly", Some(cli));
        let unasked = |_: &str| Err("the same ref needs no check".to_owned());
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &seen,
            &|app| Some((installed(&dir.0, app, &stale, &current), source.clone())),
            &unasked,
            &dir.0,
            false,
            false,
        )
        .unwrap();
        // Codex replaces a stale copy on add, so nothing of its is removed at all.
        assert_eq!(
            calls(&dir.0),
            [
                "plugin marketplace update astrolabe",
                "plugin marketplace upgrade astrolabe",
                "plugin uninstall astrolabe@astrolabe --keep-data",
                "plugin install astrolabe@astrolabe",
                "plugin add astrolabe@astrolabe",
            ]
        );
        assert!(
            report.contains("Codex: plugin reinstalled from nightly."),
            "{report}"
        );

        // Forced, with every copy current: a dry run, since a forced run also installs
        // the program.
        let report = run(
            Channel::Nightly,
            &nightly(true),
            &seen,
            &|_| Some((current.clone(), source.clone())),
            &unasked,
            &dir.0,
            true,
            true,
        )
        .unwrap();
        assert!(!report.contains("marketplace remove"), "{report}");
        assert!(!report.contains("codex plugin remove"), "{report}");
        assert!(
            report.contains("Claude Code: would run claude plugin marketplace update astrolabe; claude plugin uninstall astrolabe@astrolabe --keep-data; claude plugin install astrolabe@astrolabe."),
            "{report}"
        );
        assert!(
            report.contains("Codex: would run codex plugin marketplace upgrade astrolabe; codex plugin add astrolabe@astrolabe."),
            "{report}"
        );
    }

    #[test]
    fn a_ref_change_checks_the_target_before_removing_anything() {
        for app in App::ALL {
            let dir = Scratch::new(&format!("preflight-{}", app.cli()));
            let seen = only(app, fake_cli(&dir.0), "main");
            let report = run(
                Channel::Nightly,
                &nightly(true),
                &seen,
                &|_| None,
                &|_| Err("could not resolve host".into()),
                &dir.0,
                false,
                false,
            )
            .unwrap_err();
            assert_eq!(calls(&dir.0), [""; 0], "{report}");
            assert!(report.contains("nothing was changed"), "{report}");
            assert!(report.contains("could not resolve host"), "{report}");
        }
    }

    #[test]
    fn a_failure_at_any_step_of_a_ref_change_leaves_a_registration_or_recovery_steps() {
        for app in App::ALL {
            let steps = app.steps("nightly");
            let old = app.register(&format!("{REPO}.git"), Some("main"));
            for (at, (words, required)) in steps.iter().enumerate() {
                let dir = Scratch::new(&format!("inject-{}-{at}", app.cli()));
                let seen = only(app, fake_cli(&dir.0), "main");
                fail(&dir.0, &[words]);
                let result = run(
                    Channel::Nightly,
                    &nightly(true),
                    &seen,
                    &|_| None,
                    &|_| Ok(()),
                    &dir.0,
                    false,
                    false,
                );
                let calls = calls(&dir.0);
                if !required {
                    // A removal that finds nothing to remove doesn't stop the move.
                    let report = result.unwrap();
                    assert!(report.contains("plugin now follows nightly."), "{report}");
                    continue;
                }
                let report = result.unwrap_err();
                assert!(
                    calls.contains(&old.join(" ")),
                    "the old registration is added back: {calls:?}"
                );
                // The exact commands for the target are printed either way.
                for (words, required) in &steps {
                    if *required {
                        assert!(report.contains(&shown(app.cli(), words)), "{report}");
                    }
                }
                if *words == app.install() {
                    // The same command installs from the old registration, so it fails too,
                    // after the old marketplace went back in. Neither app adds another ref
                    // over a registration, so the way to the target starts with the removal.
                    assert!(report.contains("couldn't be put back"), "{report}");
                    assert!(
                        report.contains("may have no Astrolabe plugin, hooks or guard"),
                        "{report}"
                    );
                    let all: Vec<String> = steps
                        .iter()
                        .map(|(words, _)| format!("`{}`", shown(app.cli(), words)))
                        .collect();
                    assert!(
                        report.contains(&format!("For nightly, run {}.", all.join(", then "))),
                        "{report}"
                    );
                    assert!(
                        report.contains(&format!(
                            "For main, run `{}`.",
                            shown(app.cli(), &app.install())
                        )),
                        "{report}"
                    );
                } else {
                    assert!(
                        report.contains("The previous registration was put back"),
                        "{report}"
                    );
                    assert_eq!(calls.last().unwrap(), &app.install().join(" "));
                }
            }

            // The old marketplace can't be added back, in two ways: the target's add
            // failed, or the target went in, its install failed, and the removal that
            // should have cleared it for the old one failed too. What is registered is in
            // doubt either way, so both sets of commands start with the removal.
            let add = &steps[steps.len() - 2].0;
            let removal = &steps[steps.len() - 3].0;
            let install = app.install();
            let back = app.swap(&format!("{REPO}.git"), Some("main"));
            let all = |steps: &[(Vec<String>, bool)]| {
                let shown: Vec<String> = steps
                    .iter()
                    .map(|(words, _)| format!("`{}`", shown(app.cli(), words)))
                    .collect();
                shown.join(", then ")
            };
            let cases: [(&str, Vec<&[String]>); 2] = [
                ("add", vec![add, &old]),
                ("removal", vec![&install, removal, &old]),
            ];
            for (name, failing) in cases {
                let dir = Scratch::new(&format!("inject-{}-{name}", app.cli()));
                let seen = only(app, fake_cli(&dir.0), "main");
                fail(&dir.0, &failing);
                let report = run(
                    Channel::Nightly,
                    &nightly(true),
                    &seen,
                    &|_| None,
                    &|_| Ok(()),
                    &dir.0,
                    false,
                    false,
                )
                .unwrap_err();
                assert!(report.contains("couldn't be put back"), "{name}: {report}");
                assert!(
                    report.contains(&format!("For nightly, run {}.", all(&steps))),
                    "{name}: {report}"
                );
                assert!(
                    report.contains(&format!("For main, run {}.", all(&back))),
                    "{name}: {report}"
                );
                assert!(
                    report.contains("may report that there is nothing to remove"),
                    "{name}: {report}"
                );
            }
        }
    }

    #[test]
    fn a_reinstall_that_leaves_the_copy_stale_fails_and_says_how_to_finish() {
        let source = Scratch::new("unchanged-source");
        let new = plugin(&source.0.join("source"), NEW_HOOKS);
        let stale = plugin(&source.0.join("stale"), OLD_HOOKS);
        for app in App::ALL {
            let dir = Scratch::new(&format!("unchanged-{}", app.cli()));
            let seen = only(app, fake_cli(&dir.0), "nightly");
            // Every command succeeds, as when an app calls a stale copy already installed.
            let report = run(
                Channel::Nightly,
                &nightly(true),
                &seen,
                &|_| Some((stale.clone(), new.clone())),
                &|_| Ok(()),
                &dir.0,
                false,
                false,
            )
            .unwrap_err();
            assert!(!report.contains("plugin reinstalled from"), "{report}");
            assert!(
                report.contains("still doesn't match its marketplace's copy"),
                "{report}"
            );
            for (words, _) in app.reinstall() {
                assert!(report.contains(&shown(app.cli(), &words)), "{report}");
            }
        }
    }

    #[test]
    fn a_failed_reinstall_says_what_is_left_and_how_to_finish() {
        let source = Scratch::new("reinstall-source");
        let new = plugin(&source.0.join("source"), NEW_HOOKS);
        let stale = plugin(&source.0.join("stale"), OLD_HOOKS);
        for app in App::ALL {
            let dir = Scratch::new(&format!("reinstall-{}", app.cli()));
            let seen = only(app, fake_cli(&dir.0), "nightly");
            fail(&dir.0, &[&app.install()]);
            let report = run(
                Channel::Nightly,
                &nightly(true),
                &seen,
                &|_| Some((stale.clone(), new.clone())),
                &|_| Ok(()),
                &dir.0,
                false,
                false,
            )
            .unwrap_err();
            let retry = format!("run `{}`", shown(app.cli(), &app.install()));
            assert!(report.contains(&retry), "{report}");
            let left = match app {
                App::Claude => "has no Astrolabe hooks or guard until",
                App::Codex => "The installed copy was left as it was",
            };
            assert!(report.contains(left), "{report}");
        }
    }
}
