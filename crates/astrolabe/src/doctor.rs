//! Inspect writes nothing, apply and remove go through `trust`, verify proves one working
//! session per client from evidence on disk, never from what the model says.
use crate::acp::{AcpClient, CLAUDE_ADAPTER, CODEX_ADAPTER, ClientEvent, adapter_entry};
use crate::driver::codex;
use crate::error::{Error, Result};
use crate::hook::comparable;
use crate::launch::{self, DEFAULT_DIALS, Machine, Prepared, Provider, Request, Say};
use crate::model::Alert;
use crate::registry::AGENT;
use crate::trust::{self, Ask, CodexPlan};
use crate::{Engine, OsProcessProbe, SystemClock};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

pub const PIN_CODEX: &str = "0.166.0";
pub const PIN_CLAUDE: &str = "2.1.314";
type Failure = Result<Option<String>>;
const VERIFY_LIMIT: Duration = Duration::from_secs(300);

fn stdout(command: &mut Command) -> String {
    let output = command.stdin(Stdio::null()).stderr(Stdio::null()).output();
    let text = |o: std::process::Output| String::from_utf8_lossy(&o.stdout).into_owned();
    output.map_or(String::new(), text)
}

pub fn refuse(path: &Path) -> Result<()> {
    let refused = crate::protect::refuse_unsafe_windows_folder(path);
    refused.map_err(Error::UnsafeSessionFolder)
}

/// On every launch: a real command in this folder must see this folder, and its write must
/// land. Host side only, as a model turn per launch is too dear; a session's proof is verify.
pub fn folder_check(cwd: &Path) -> std::result::Result<(), String> {
    crate::protect::refuse_unsafe_windows_folder(cwd)?;
    let name = format!(".astrolabe-folder-{}", std::process::id());
    let script = format!("require('fs').writeFileSync('{name}',process.cwd())");
    stdout(Command::new("node").args(["-e", &script]).current_dir(cwd));
    let wrote = std::fs::read_to_string(cwd.join(&name));
    let _ = std::fs::remove_file(cwd.join(&name));
    let shown = cwd.display();
    let wrote = wrote.map_err(|_| format!("a command's write did not land in {shown}"))?;
    if comparable(Path::new(&wrote)) != comparable(cwd) {
        return Err(format!("a command started in {shown} ran in {wrote}"));
    }
    Ok(())
}

fn is_native(path: &Path) -> bool {
    let mut start = [0; 2];
    let file = std::fs::File::open(path);
    let read = file.and_then(|mut f| std::io::Read::read_exact(&mut f, &mut start));
    let extension = path.extension().unwrap_or_default();
    let script = ["cmd", "ps1", "js"].iter().any(|e| extension == *e);
    read.is_ok() && &start != b"#!" && !script
}

fn find_native(dir: &Path, file: &str, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path());
    let mut entries: Vec<PathBuf> = entries.collect();
    entries.sort();
    entries.iter().find_map(|path| {
        if path.is_dir() {
            return (depth > 0).then(|| find_native(path, file, depth - 1))?;
        }
        let under = path.components().any(|part| part.as_os_str() == "vendor");
        let named = path.file_name().is_some_and(|name| name == file);
        (under && named && is_native(path)).then(|| path.clone())
    })
}

/// Under the script's `@scope` on the Mac, `<shim dir>/node_modules/@openai` on Windows.
pub fn native_behind(shim: &Path, file: &str) -> Option<PathBuf> {
    let real = std::fs::canonicalize(shim).unwrap_or_else(|_| shim.to_path_buf());
    let scope = real.parent()?.parent()?.parent()?;
    let scoped = scope.file_name()?.to_string_lossy().starts_with('@');
    let installed = shim.parent()?.join("node_modules/@openai");
    if scoped && let Some(found) = find_native(scope, file, 9) {
        return Some(found);
    }
    find_native(&installed, file, 9)
}

/// Its variable always wins. Else the first on `PATH`, or the native binary behind that shim.
pub fn find_client(provider: Provider) -> Option<PathBuf> {
    let (var, name) = match provider {
        Provider::Codex => ("CODEX_PATH", "codex"),
        Provider::Claude => ("CLAUDE_CODE_EXECUTABLE", "claude"),
    };
    if let Some(path) = std::env::var_os(var) {
        return Some(PathBuf::from(path));
    }
    let names: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd"]
    } else {
        &[""]
    };
    let found = std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        let mut files = names.iter().map(|s| dir.join(format!("{name}{s}")));
        files.find(|file| file.is_file())
    })?;
    if is_native(&found) {
        return Some(std::fs::canonicalize(&found).unwrap_or(found));
    }
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    native_behind(&found, &file).or(Some(found))
}

/// One can host a TUI at another version, its children outliving quit. Only reported.
pub fn daemon_lines(processes: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for line in processes.lines().map(str::trim) {
        let daemon = line.contains("app-server daemon") || line.contains("app-server-daemon");
        if daemon && !line.contains("code-mode-host") {
            let rest = line.split("releases").nth(1).unwrap_or("");
            let rest = rest.trim_start_matches(['/', '\\']);
            let version = rest.split('-').next().unwrap_or("");
            lines.push(format!("codex daemon {version:?}, pin {PIN_CODEX}: {line}"));
        }
    }
    lines
}

fn processes() -> String {
    let list = "Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine }";
    match cfg!(windows) {
        true => stdout(Command::new("powershell").args(["-NoProfile", "-Command", list])),
        false => stdout(Command::new("ps").args(["-axww", "-o", "command="])),
    }
}

fn client_line(provider: Provider, machine: &Machine) -> String {
    let (pin, package) = match provider {
        Provider::Codex => (PIN_CODEX, CODEX_ADAPTER),
        Provider::Claude => (PIN_CLAUDE, CLAUDE_ADAPTER),
    };
    let slug = provider.slug();
    let adapter = adapter_entry(&machine.adapters, package);
    let adapter = adapter
        .err()
        .map_or(String::new(), |e| format!(", PROBLEM {e}"));
    let Some(path) = find_client(provider) else {
        return format!("{slug}: PROBLEM no client{adapter}");
    };
    // Codex unpacks helpers under a home that exists, on any run. This one doesn't.
    let mut ask = Command::new(&path);
    let version = stdout(ask.arg("--version").env("CODEX_HOME", "no-such-home"));
    let flag = |good: bool, text| if good { "" } else { text };
    let pinned = flag(version.contains(pin), ", PROBLEM mismatch");
    let shim = flag(
        is_native(&path),
        ", PROBLEM not native; set its path variable",
    );
    let (path, version) = (path.display(), version.trim());
    format!("{slug}: {path}, {version}, pin {pin}{pinned}{shim}{adapter}")
}

/// Writes nothing to a home. It can't say a hash is current: verify and launches check that.
pub fn inspect(machine: &Machine) -> Result<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    for provider in Provider::ALL {
        let on = machine.registry.provider_on(provider.slug())?;
        lines.push(format!("{} on: {on}", client_line(provider, machine)));
    }
    let exe = format!("astrolabe{}", std::env::consts::EXE_SUFFIX);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path).any(|dir| dir.join(&exe).is_file());
    let line = "astrolabe on PATH, which plain clients' hooks need";
    lines.push(format!("{line}: {found}"));
    lines.extend(daemon_lines(&processes()));
    let (config, claude) = (&machine.homes.codex_config, &machine.homes.claude_json);
    let hash = trust::file_hash(config);
    lines.push(format!("{}: {hash}", config.display()));
    let Some(agent) = machine.registry.agent_repo()? else {
        lines.push("agent repo: none here; run astrolabe init".to_owned());
        return Ok(lines);
    };
    let native = trust::native(&agent);
    let text = std::fs::read_to_string(config).unwrap_or_default();
    let (projects, hooks) = trust::scan(&text);
    let (project, key) = trust::keys(&native, cfg!(windows));
    let codex = projects.contains(&project);
    let hooks = hooks.iter().filter(|(k, _)| k.starts_with(&native)).count();
    let dialog = trust::claude_state(claude, &key);
    let state = crate::init::layer_state(&agent);
    lines.push(format!("agent repo {native}, layer real: {state:?}"));
    let line = format!("{hooks} hook lines, hashes unchecked");
    lines.push(format!("  codex project line: {codex}, {line}"));
    lines.push(format!("  claude entry: {dialog:?}"));
    Ok(lines)
}

/// The two hooks, from `hooks/list` on a home holding only the project line.
pub fn plan_from_list(response: &Value, project: &str) -> Result<CodexPlan> {
    let mut hooks = Vec::new();
    let groups = response["result"]["data"].as_array().into_iter().flatten();
    for hook in groups.filter_map(|g| g["hooks"].as_array()).flatten() {
        let key = hook["key"].as_str().unwrap_or("");
        let command = hook["command"].as_str().unwrap_or("");
        if command.starts_with("astrolabe hook ") && !key.contains("<session-flags>") {
            let hash = hook["currentHash"].as_str().unwrap_or("");
            hooks.push((key.to_owned(), hash.to_owned()));
        }
    }
    if hooks.len() != 2 {
        let read = hooks.len();
        let text = format!(
            "read {read} of the repo's 2 hooks on a temporary home: the project line didn't take, or the repo's layer isn't real"
        );
        return Err(Error::Setup(text));
    }
    let project = project.to_owned();
    Ok(CodexPlan { project, hooks })
}

/// Keys and hashes from the client on a temporary home; proves the lines work there first.
pub fn plan_codex(client: &Path, repo: &Path) -> Result<CodexPlan> {
    let native = trust::native(repo);
    let parent = Path::new(&native).parent().unwrap_or(Path::new("."));
    let home = parent.join(format!(".astrolabe-doctor-{}", std::process::id()));
    std::fs::create_dir(&home)?;
    let config = home.join("config.toml");
    let outcome = (|| {
        let (project, _) = trust::keys(&native, cfg!(windows));
        let mut bare = CodexPlan::default();
        bare.project.clone_from(&project);
        std::fs::write(&config, bare.lines().join("\n") + "\n")?;
        let plan = plan_from_list(&codex::list_in_home(client, repo, &home)?, &project)?;
        std::fs::write(&config, plan.lines().join("\n") + "\n")?;
        match codex::hooks_problem(&codex::list_in_home(client, repo, &home)?) {
            Some(why) => Err(Error::Setup(format!("on a temporary home: {why}"))),
            None => Ok(plan),
        }
    })();
    // Windows can hold the folder for seconds after the client exits; a leftover is no failure.
    let _ = (0..100).find(|_| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::remove_dir_all(&home).is_ok()
    });
    outcome
}

/// `ask` is the only way in, and shows the write. Claude's trust is the owner's dialog.
pub fn apply(machine: &Machine, ask: Ask) -> Result<Vec<String>> {
    let repo = machine.registry.checkout(AGENT)?;
    let client = find_client(Provider::Codex).ok_or(Error::Setup("no codex".to_owned()))?;
    let plan = plan_codex(&client, &repo)?;
    let written = trust::apply_codex(&machine.homes.codex_config, &plan, ask)?;
    let dialog = format!("claude: run claude in {} and say yes", repo.display());
    Ok(vec![written, dialog])
}

/// Claude's entry stays: this never writes that file, and running sessions rewrite it.
pub fn remove(machine: &Machine, ask: Ask) -> Result<Vec<String>> {
    let repo = trust::native(&machine.registry.checkout(AGENT)?);
    let (_, key) = trust::keys(&repo, cfg!(windows));
    let removed = trust::remove_codex(&machine.homes.codex_config, ask)?;
    let state = trust::claude_state(&machine.homes.claude_json, &key);
    let file = machine.homes.claude_json.display();
    let left = format!(
        "claude: entry is {state:?} and is not removed here. To remove it, delete projects[{key:?}] from {file}, with Claude closed."
    );
    Ok(vec![removed, left])
}

/// Git Bash on Windows prints `/c/x` for `C:\x`.
fn same_folder(printed: &str, cwd: &Path) -> bool {
    let printed = printed.trim();
    let drive = printed
        .strip_prefix('/')
        .and_then(|rest| rest.split_once('/'));
    let printed = match drive {
        Some((drive, rest)) if cfg!(windows) && drive.len() == 1 => format!("{drive}:/{rest}"),
        _ => printed.to_owned(),
    };
    comparable(Path::new(&printed)) == comparable(cwd)
}

/// `None` is proof: each file holds the cwd its own command printed.
pub fn judge(direct: Option<&str>, shell: Option<&str>, cwd: &Path) -> Option<String> {
    let good = |found: Option<&str>| found.is_some_and(|found| same_folder(found, cwd));
    let shown = cwd.display();
    let text = format!("wanted {shown}, direct wrote: {direct:?}, shell wrote: {shell:?}");
    (!good(direct) || !good(shell)).then_some(text)
}

/// Codex on Windows writes through PowerShell: its sandbox can't run plain `sh`.
pub fn commands(token: &str, powershell: bool) -> [String; 2] {
    let shell = if powershell {
        format!("Set-Content -NoNewline -Path {token}-sh -Value (Get-Location).Path")
    } else {
        format!("sh -c 'pwd > {token}-sh'")
    };
    [
        format!("node -e \"require('fs').writeFileSync('{token}-direct',process.cwd())\""),
        shell,
    ]
}

/// Only an exact match on the raw input the adapter sent. Titles can differ from what runs.
pub fn allowed(input: Option<&Value>, commands: &[String]) -> bool {
    let command = input.and_then(|input| input["command"].as_str());
    command.is_some_and(|command| commands.iter().any(|wanted| wanted == command))
}

fn run_session(engine: &Engine, ready: &Prepared, wanted: [String; 2], say: Say) -> Result<()> {
    let [first, second] = &wanted;
    let text = format!(
        "Use your shell tool to run these two commands, each as its own call, unchanged, with no retries or workarounds, then reply DONE whatever happens.\n1. {first}\n2. {second}"
    );
    let mut session = AcpClient::launch(engine, &ready.launch, &ready.cwd)?;
    let deadline = Instant::now() + VERIFY_LIMIT;
    while Instant::now() < deadline {
        match session.next_event(Duration::from_millis(200))? {
            Some(ClientEvent::Ready) => session.prompt(&text)?,
            Some(ClientEvent::Alert(Alert::NeedsYou(request))) => {
                let ours = allowed(session.requested_input(&request.rpc_id), &wanted);
                let kind = if ours { "allow_once" } else { "reject_once" };
                say(format!("  asked: {} -> {kind}", request.title));
                let option = request.options.iter().find(|o| o.kind == kind);
                let none = || Error::Setup(format!("{kind} was not offered"));
                session.answer(&request.id, &option.ok_or_else(none)?.option_id)?;
            }
            Some(ClientEvent::TurnEnded { .. }) => return session.shutdown(),
            Some(ClientEvent::LaunchFailed { message } | ClientEvent::PromptError { message }) => {
                return Err(Error::Setup(message));
            }
            Some(ClientEvent::Exited { code, stderr }) => {
                return Err(Error::Setup(format!("exited {code:?}: {stderr}")));
            }
            _ => {}
        }
    }
    Err(Error::Setup("no end of turn in time".to_owned()))
}

fn verify_one(machine: &Machine, provider: Provider, repo: &str, say: Say) -> Failure {
    let client = &find_client(provider).ok_or(Error::Setup("no client".to_owned()))?;
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);
    let stamp = now.map_or(0, |time| time.as_millis());
    let token = format!("astrolabe-verify-{stamp}");
    let session = &format!("verify-{}-{stamp}", provider.slug());
    let request = Request {
        repo,
        provider,
        session,
        dials: DEFAULT_DIALS,
        client,
    };
    let mut ready = launch::prepare(machine, &request, say)?;
    let env = &mut ready.launch.env;
    env.push(("ANTHROPIC_MODEL".into(), "haiku".into()));
    let engine = Engine::open(&machine.db(), SystemClock, OsProcessProbe)?;
    let powershell = cfg!(windows) && matches!(provider, Provider::Codex);
    if powershell {
        say(
            "  known limit: plain `sh` and `git commit` fail here unless escalated; check results on disk"
                .to_owned(),
        );
    }
    let ran = run_session(&engine, &ready, commands(&token, powershell), say);
    let direct = ready.cwd.join(format!("{token}-direct"));
    let shell = ready.cwd.join(format!("{token}-sh"));
    let found = std::fs::read_to_string(&direct).ok();
    let printed = std::fs::read_to_string(&shell).ok();
    let problem = judge(found.as_deref(), printed.as_deref(), &ready.cwd);
    let _ = std::fs::remove_file(&direct);
    let _ = std::fs::remove_file(&shell);
    if let Some(made) = &ready.worktree {
        say(format!("  worktree {:?}", crate::worktree::close(made)?));
    }
    Ok(match ran {
        Err(why) => Some(format!("{why}. On disk: {problem:?}")),
        Ok(()) => problem,
    })
}

pub fn verify(machine: &Machine, repo: Option<&str>, say: Say) -> Result<bool> {
    let mut passed = true;
    for provider in Provider::ALL {
        let slug = provider.slug();
        if !machine.registry.provider_on(slug)? {
            say(format!("{slug}: off, not verified"));
            continue;
        }
        let repo = repo.unwrap_or(AGENT);
        let outcome = verify_one(machine, provider, repo, say);
        let problem = outcome.unwrap_or_else(|error| Some(error.to_string()));
        passed &= problem.is_none();
        say(match problem {
            None => format!("{slug}: PASS both files hold the real cwd, receipt held"),
            Some(problem) => format!("{slug}: FAIL {problem}"),
        });
    }
    Ok(passed)
}
