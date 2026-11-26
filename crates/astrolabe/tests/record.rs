//! Writing notes through the hook: `astrolabe effort new | save` as a client runs them, against
//! a notes checkout with a bare origin, from one machine and from two.
mod common;

use astrolabe::activate::STATE_VAR;
use common::TempDir;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Today, as a tick stamps it after a piece.
fn today() -> String {
    astrolabe::notes::local_day_time().0
}

/// An effort file on today's month shelf, where `effort new` files it.
fn shelf(name: &str) -> String {
    let (day, _) = astrolabe::notes::local_day_time();
    format!("{}/{name}", astrolabe::notes::effort_dir(&day))
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// One machine: its own state folder and notes clone of the shared origin, and a product
/// repo `demo` whose remote the repo note names.
struct Machine {
    dir: TempDir,
    name: &'static str,
    notes: PathBuf,
    demo: PathBuf,
    /// The read the hook last handed back in place of a command, as the app would run it.
    last_read: std::sync::Mutex<String>,
    /// Whether its hooks run on the real clock rather than the steady test clock.
    real_clock: bool,
}

struct World {
    origin: TempDir,
    real_clock: bool,
    _slot: Slot,
}

/// A test's share of the places tests may hold at once. On the steady test clock a busy
/// machine changes nothing a test sees, so this only keeps the machine from thrashing: as many
/// as the cores. A test on the real clock takes every place, as load changes what it sees:
/// with eight others beside them, real-clock tests failed on a quiet PC (2026-11-25). A
/// second world on the same thread shares its first's places, so a test can't wait on itself.
struct Slot {
    places: usize,
}

/// How many places there are: `ASTROLABE_RECORD_AT_ONCE`, else the cores.
fn at_once() -> usize {
    std::env::var("ASTROLABE_RECORD_AT_ONCE")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get()))
        .max(1)
}

static TAKEN: (std::sync::Mutex<usize>, std::sync::Condvar) =
    (std::sync::Mutex::new(0), std::sync::Condvar::new());

thread_local! {
    static HELD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Slot {
    fn take(wanted: usize) -> Slot {
        let mut places = 0;
        if HELD.get() == 0 {
            let (taken, freed) = &TAKEN;
            let limit = at_once();
            places = wanted.min(limit);
            let mut n = taken.lock().unwrap_or_else(|e| e.into_inner());
            while *n + places > limit {
                n = freed.wait(n).unwrap_or_else(|e| e.into_inner());
            }
            *n += places;
        }
        HELD.set(HELD.get() + 1);
        Slot { places }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        HELD.set(HELD.get() - 1);
        if self.places > 0 {
            let (taken, freed) = &TAKEN;
            *taken.lock().unwrap_or_else(|e| e.into_inner()) -= self.places;
            freed.notify_all();
        }
    }
}

/// The world and one machine built once per run with real git and the real program, which each
/// test copies instead of building its own: setup was most of a record test's time.
struct Template {
    /// Unique to this run, so a path left unswapped in a copy is easy to find.
    root: PathBuf,
    origin: PathBuf,
    machine: PathBuf,
    /// The origin's main when the machine was copied from it.
    head: String,
}

impl Template {
    fn get() -> &'static Template {
        static TEMPLATE: std::sync::OnceLock<Template> = std::sync::OnceLock::new();
        TEMPLATE.get_or_init(Template::build)
    }

    fn build() -> Template {
        let temp = std::env::temp_dir();
        // A static is never dropped, so each run clears what earlier runs left. A run still
        // going rewrites its template's `in-use` with each world it copies, so it never
        // looks old.
        let stale = std::time::Duration::from_secs(3600);
        for entry in std::fs::read_dir(&temp).into_iter().flatten().flatten() {
            let marker = entry.path().join("in-use");
            let used = std::fs::metadata(&marker).or_else(|_| entry.metadata());
            let old = used
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > stale));
            if old
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("astrolabe-template-")
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        let root = temp.join(format!("astrolabe-template-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let origin = root.join("origin");
        let bare = origin.join("notes.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        let seed = root.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        for (file, text) in [
            ("knowledge/owner.md", "OWNER-TEXT"),
            ("knowledge/voice.md", "VOICE-TEXT"),
            ("knowledge/machines.md", "MACHINE-TEXT"),
            (
                "knowledge/repos/demo.md",
                "---\nremote: git@example.com:team/demo.git\n---\nREPO-DEMO-TEXT",
            ),
            ("efforts/.keep", ""),
        ] {
            write(&seed.join(file), text);
        }
        git(&seed, &["add", "-A"]);
        git(&seed, &["commit", "-q", "-m", "seed"]);
        git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
        git(&seed, &["push", "-q", "origin", "main"]);

        let machine = root.join("machine");
        std::fs::create_dir_all(&machine).unwrap();
        let notes = machine.join("notes");
        git(&machine, &["clone", "-q", bare.to_str().unwrap(), "notes"]);
        git(&notes, &["config", "user.email", "t@example.com"]);
        git(&notes, &["config", "user.name", "T"]);
        let demo = machine.join("demo");
        std::fs::create_dir_all(&demo).unwrap();
        git(&demo, &["init", "-q", "-b", "main"]);
        git(&demo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(
            &demo,
            &["remote", "add", "origin", "git@example.com:team/demo.git"],
        );
        write(&machine.join("plugin/hooks/hooks.json"), "hooks v1");
        write(
            &machine.join("plugin/.claude-plugin/plugin.json"),
            "manifest v1",
        );
        // Registered and switched on through the program itself, once.
        let m = Machine::at(TempDir(machine.clone()), "template");
        let notes_str = m.notes.to_str().unwrap().to_owned();
        let (out, ok) = m.run(&m.notes, &["repo", "add", "agent", &notes_str], "");
        assert!(ok, "{out}");
        assert!(
            m.run(&m.demo, &["on"], "")
                .0
                .contains("on for this machine")
        );
        // The template outlives every test, so its folder is never dropped here.
        std::mem::forget(m);
        let head = git(&bare, &["rev-parse", "main"]);
        Template {
            root,
            origin,
            machine,
            head,
        }
    }
}

/// A copy of `from` at `to`, files and folders alike.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// `path` as the system names it in full, without the `\\?\` prefix Windows adds.
fn full_name(path: &Path) -> PathBuf {
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = full.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// Rewrites each spelling of `from` in `file` to `to`: as given, with backslashes doubled as
/// JSON and git config write them, and with forward slashes as git writes a Windows path.
fn swap_path(file: &Path, from: &Path, to: &Path) {
    let mut text = std::fs::read_to_string(file).unwrap();
    let (from, to) = (from.to_str().unwrap(), to.to_str().unwrap());
    for (f, t) in [
        (from.replace('\\', "\\\\"), to.replace('\\', "\\\\")),
        (from.replace('\\', "/"), to.replace('\\', "/")),
        (from.to_owned(), to.to_owned()),
    ] {
        text = text.replace(&f, &t);
    }
    std::fs::write(file, text).unwrap();
}

impl World {
    /// A world whose hooks keep the steady test clock, which counts only what a test puts
    /// on it, so a busy machine doesn't change what they do.
    fn new() -> Self {
        Self::taking(1, false)
    }

    /// A world whose hooks run on the real clock, for a test of how the hook copes with real
    /// time passing. It runs with no other test beside it, as load changes what it sees.
    fn timed() -> Self {
        Self::taking(usize::MAX, true)
    }

    fn taking(places: usize, real_clock: bool) -> Self {
        let slot = Slot::take(places);
        let _ = std::fs::write(Template::get().root.join("in-use"), "");
        let origin = TempDir::new();
        copy_tree(
            &Template::get().origin.join("notes.git"),
            &origin.0.join("notes.git"),
        );
        Self {
            origin,
            real_clock,
            _slot: slot,
        }
    }

    fn machine(&self, name: &'static str) -> Machine {
        let template = Template::get();
        let dir = TempDir::new();
        copy_tree(&template.machine, &dir.0);
        let config = dir.0.join("notes/.git/config");
        swap_path(&config, &template.origin, &self.origin.0);
        let registered = dir.0.join("state/repos/agent.json");
        swap_path(&registered, &template.machine, &dir.0);
        // The program records a checkout by its full name, which a short temp folder name
        // (`RUNNER~1` on a Windows runner) doesn't match.
        swap_path(
            &registered,
            &full_name(&template.machine),
            &full_name(&dir.0),
        );
        for file in [config, registered] {
            let text = std::fs::read_to_string(&file).unwrap();
            let marker = template.root.file_name().unwrap().to_str().unwrap();
            assert!(!text.contains(marker), "{}: {text}", file.display());
        }
        // A clone made now would hold what other machines in this world have pushed.
        if git(&self.origin.0.join("notes.git"), &["rev-parse", "main"]) != template.head {
            git(&dir.0.join("notes"), &["pull", "-q", "--ff-only"]);
        }
        let mut machine = Machine::at(dir, name);
        machine.real_clock = self.real_clock;
        machine
    }
}

impl Machine {
    fn at(dir: TempDir, name: &'static str) -> Machine {
        Machine {
            notes: dir.0.join("notes"),
            demo: dir.0.join("demo"),
            dir,
            name,
            last_read: Default::default(),
            real_clock: false,
        }
    }
}

impl Machine {
    fn state(&self) -> PathBuf {
        self.dir.0.join("state")
    }

    /// The binary with this machine's environment, for a caller that spawns it itself.
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        self.command_from(Path::new(env!("CARGO_BIN_EXE_astrolabe")), cwd, args)
    }

    /// `command` with the program at `exe`, for a copy a test replaces.
    fn command_from(&self, exe: &Path, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .current_dir(cwd)
            .env(STATE_VAR, self.state())
            .env("CLAUDE_PLUGIN_ROOT", self.dir.0.join("plugin"))
            .env("ASTROLABE_MACHINE", self.name)
            .env("ASTROLABE_NO_DETACHED_SYNC", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.com");
        if !self.real_clock {
            cmd.env("ASTROLABE_HOOK_TEST_CLOCK", "steady");
        }
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin: &str) -> (String, bool) {
        self.run_env(cwd, args, stdin, &[])
    }

    /// `run` with extra environment, such as a hook's test stall.
    fn run_env(
        &self,
        cwd: &Path,
        args: &[&str],
        stdin: &str,
        env: &[(&str, &str)],
    ) -> (String, bool) {
        let mut command = self.command(cwd, args);
        command.envs(env.iter().copied());
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text += &String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            text += &format!("\nexit {}", output.status);
        }
        (text, output.status.success())
    }

    /// A Bash command as a client runs it: through the hook, then what the hook let through.
    fn bash(&self, session: &str, command: &str) -> (String, bool) {
        self.bash_as(session, command, None)
    }

    fn bash_as(&self, session: &str, command: &str, agent: Option<&str>) -> (String, bool) {
        self.bash_from(session, &self.demo, command, agent)
    }

    /// The same from another folder, such as the notes checkout.
    fn bash_from(
        &self,
        session: &str,
        cwd: &Path,
        command: &str,
        agent: Option<&str>,
    ) -> (String, bool) {
        let mut input = json!({"session_id": session, "cwd": cwd, "tool_name": "Bash", "tool_use_id": "call-1", "tool_input": {"command": command}});
        if let Some(agent) = agent {
            input["agent_id"] = agent.into();
        }
        self.shell(input, cwd, command, &[])
    }

    /// `bash` with extra environment for the hook.
    fn bash_env(&self, session: &str, command: &str, env: &[(&str, &str)]) -> (String, bool) {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": "Bash", "tool_use_id": "call-1", "tool_input": {"command": command}});
        self.shell(input, &self.demo, command, env)
    }

    /// The same as Codex sends it, which adds a turn id.
    fn bash_codex(&self, session: &str, command: &str) -> (String, bool) {
        let input = json!({"session_id": session, "turn_id": "t1", "cwd": self.demo, "tool_name": "shell_command", "tool_use_id": "call-1", "tool_input": {"command": command}});
        self.shell(input, &self.demo, command, &[])
    }

    fn shell(
        &self,
        input: Value,
        cwd: &Path,
        command: &str,
        env: &[(&str, &str)],
    ) -> (String, bool) {
        let (out, ok) = self.run_env(
            cwd,
            &["hook", "pre-command", "--if-active"],
            &input.to_string(),
            env,
        );
        assert!(ok, "a hook always exits 0");
        if out.trim().is_empty() {
            if !command.trim_start().starts_with("astrolabe") {
                let out = Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .current_dir(cwd)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_AUTHOR_NAME", "T")
                    .env("GIT_AUTHOR_EMAIL", "t@example.com")
                    .env("GIT_COMMITTER_NAME", "T")
                    .env("GIT_COMMITTER_EMAIL", "t@example.com")
                    .output()
                    .unwrap();
                return (
                    String::from_utf8_lossy(&out.stdout).into_owned(),
                    out.status.success(),
                );
            }
            return self.plain(command);
        }
        let answer: Value = serde_json::from_str(&out).unwrap();
        let hso = &answer["hookSpecificOutput"];
        if hso["permissionDecision"] == "deny" {
            return (
                hso["permissionDecisionReason"].as_str().unwrap().to_owned(),
                false,
            );
        }
        let command = hso["updatedInput"]["command"].as_str().unwrap();
        // The app runs a read of the ticket's text; the ticket's own `--show` says the same
        // and carries whether it worked. A refused command's read fails too.
        let failing =
            (command.strip_suffix("; false")).or_else(|| command.strip_suffix("; exit 1"));
        *self.last_read.lock().unwrap() = command.to_owned();
        let read = failing.unwrap_or(command);
        let path = (read.strip_prefix("cat '"))
            .or_else(|| read.strip_prefix("Get-Content -Raw -Encoding UTF8 -LiteralPath '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{command}"));
        let printed = std::fs::read_to_string(path).unwrap();
        // Whether it worked is in the ticket the plain command looks up; the app sees text.
        let ticket: Value =
            serde_json::from_str(&std::fs::read_to_string(path.replace(".txt", ".json")).unwrap())
                .unwrap();
        assert_eq!(
            ticket["text"].as_str().unwrap().trim_end(),
            printed.trim_end()
        );
        let ok = ticket["ok"] == true;
        // A read that fails never carries a reply that worked. Effort and notes commands
        // also fail when they didn't work; activation keeps its read's own status.
        assert!(failing.is_none() || !ok, "{command}: {printed}");
        #[cfg(unix)]
        if command.starts_with("cat '") {
            let ran = Command::new("sh").arg("-c").arg(command).output().unwrap();
            assert_eq!(ran.status.success(), failing.is_none(), "{command}");
            assert_eq!(String::from_utf8_lossy(&ran.stdout), printed);
        }
        (printed, ok)
    }

    /// The command run with no hook: the shell feeds the heredoc body on stdin.
    fn plain(&self, command: &str) -> (String, bool) {
        let (head, body) = match command.split_once("<<'ASTROLABE'\n") {
            Some((h, b)) => (
                h.trim(),
                b.trim_end()
                    .trim_end_matches("ASTROLABE")
                    .trim_end()
                    .to_owned(),
            ),
            None => (command.trim(), String::new()),
        };
        let words: Vec<String> = astrolabe_words(head);
        let args: Vec<&str> = words.iter().skip(1).map(String::as_str).collect();
        self.run(&self.demo, &args, &body)
    }

    fn session_start(&self, session: &str, source: &str) -> String {
        let input = json!({"session_id": session, "cwd": self.demo, "source": source});
        let (out, ok) = self.run(
            &self.demo,
            &["hook", "session-start", "--if-active"],
            &input.to_string(),
        );
        assert!(ok);
        let out: Value = serde_json::from_str(out.trim()).unwrap();
        out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn effort_text(&self, n: u32) -> String {
        let (_, rel) = astrolabe::notes::effort_files(&self.notes)
            .into_iter()
            .find(|(m, _)| *m == n)
            .unwrap_or_else(|| panic!("no effort {n} under {}", self.notes.display()));
        std::fs::read_to_string(self.notes.join(rel)).unwrap()
    }

    fn journal_today(&self) -> String {
        astrolabe::notes::md_files(&self.notes, "journal")
            .into_iter()
            .map(|rel| std::fs::read_to_string(self.notes.join(rel)).unwrap())
            .collect()
    }

    fn log(&self) -> String {
        git(&self.notes, &["log", "--format=%s"])
    }

    fn sync(&self) -> (String, bool) {
        self.run(&self.notes, &["notes", "sync"], "")
    }
}

/// Bare words and double-quoted values, as the shell would hand them to the program.
fn astrolabe_words(head: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut rest = head.trim();
    while !rest.is_empty() {
        if let Some(q) = rest.strip_prefix('"') {
            let end = q.find('"').unwrap();
            words.push(q[..end].to_owned());
            rest = q[end + 1..].trim_start();
        } else {
            let end = rest.find(' ').unwrap_or(rest.len());
            words.push(rest[..end].to_owned());
            rest = rest[end..].trim_start();
        }
    }
    words
}

const NEW: &str = "astrolabe effort new --title \"Smooth ink\" <<'ASTROLABE'\nsmooth the ink across frames, with café in it\nASTROLABE\n";

impl Machine {
    fn hook_json(&self, event: &str, input: Value) -> Option<Value> {
        let (out, ok) = self.run(
            &self.demo,
            &["hook", event, "--if-active"],
            &input.to_string(),
        );
        assert!(ok, "{out}");
        (!out.trim().is_empty())
            .then(|| serde_json::from_str(out.trim()).unwrap_or_else(|_| panic!("{out}")))
    }

    /// Whether a detached push was asked for since the last look, and forgets it.
    fn sync_requested(&self) -> bool {
        let marker = self.state().join("sync-requested");
        let was = marker.exists();
        let _ = std::fs::remove_file(marker);
        was
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    /// Edits one field of the sync record, keeping the rest.
    fn set_sync(&self, key: &str, value: Value) {
        let path = self.state().join("sync.json");
        let mut record: Value = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}));
        record[key] = value;
        std::fs::write(path, record.to_string()).unwrap();
    }

    fn set_last_push(&self, ago_ms: i64) {
        self.set_sync("pushed_ms", (Self::now_ms() - ago_ms).into());
    }

    /// Stamps the last ask into the future, as a clock set back would.
    fn ask_from_the_future(&self) {
        let path = self.state().join("sync-asked");
        std::fs::write(&path, "").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }

    /// Forgets the last ask, so the next eligible turn end asks again.
    fn forget_ask(&self) {
        let _ = std::fs::remove_file(self.state().join("sync-asked"));
    }

    fn stop(&self, session: &str, active: bool, codex: bool) -> Option<Value> {
        let mut input = json!({"session_id": session, "cwd": self.demo, "stop_hook_active": active, "last_assistant_message": "done"});
        if codex {
            input["turn_id"] = "t1".into();
        }
        self.hook_json("stop", input)
    }

    fn edit(&self, session: &str, tool: &str) {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": tool, "tool_input": {"file_path": "x"}, "tool_response": {}});
        assert!(self.hook_json("post-tool", input).is_none());
    }
}

/// One column of the board page, from its heading to the next column.
fn column<'a>(page: &'a str, key: &str) -> &'a str {
    let start = page
        .find(&format!("class=\"col {key}\""))
        .unwrap_or_else(|| panic!("no column {key} in the board"));
    let end = page[start + 1..]
        .find("<section")
        .map_or(page.len(), |i| start + 1 + i);
    &page[start..end]
}

/// A child killed and reaped however the test ends.
struct Kill(std::process::Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[path = "record/deadlines.rs"]
mod deadlines;
#[path = "record/finishing.rs"]
mod finishing;
#[path = "record/gates.rs"]
mod gates;
#[path = "record/handoffs.rs"]
mod handoffs;
#[path = "record/home.rs"]
mod home;
#[path = "record/initiatives.rs"]
mod initiatives;
#[path = "record/lessons.rs"]
mod lessons;
#[path = "record/queueing.rs"]
mod queueing;
#[path = "record/recovery.rs"]
mod recovery;
#[path = "record/refusals.rs"]
mod refusals;
#[path = "record/results.rs"]
mod results;
#[path = "record/saving.rs"]
mod saving;
#[path = "record/sessions.rs"]
mod sessions;
#[path = "record/synchronization.rs"]
mod synchronization;

#[path = "record/replying.rs"]
mod replying;
#[path = "record/waiting.rs"]
mod waiting;
