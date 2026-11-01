//! What a launch checks before trusting that its layer is in force: the guard answers,
//! and the session's hook reported. A receipt covers session-start only; a wrong
//! pre-command key alone on Codex would still pass both.
use crate::acp::Launch;
use crate::hook::{SELF_TEST_SESSION, session_start_since};
use crate::layer::{DATA_DIR_VAR, PROGRAM, launch_env};
use serde_json::json;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Slowest seen in the probes was about 1.7 s, on small samples with no load and with a
/// cheap hook. A false failure stops work and looks like a safety fault, so this is far
/// past that. The cost of the margin: on Codex a first turn can run this long before a
/// missing receipt ends it; on Claude nothing can run, since Ready is held.
pub const RECEIPT_WAIT: Duration = Duration::from_secs(30);
/// Overrides `RECEIPT_WAIT`, in milliseconds, for a machine slower than that.
pub const RECEIPT_WAIT_VAR: &str = "ASTROLABE_RECEIPT_WAIT_MS";
const SELF_TEST_WAIT: Duration = Duration::from_secs(10);

/// Runs before a launch or resume starts anything. A real command's cwd and a write in the
/// folder belong to the doctor, which can add itself here; this only reads the path.
pub type FolderCheck = fn(&Path) -> Result<(), String>;

/// Codex on Windows, seen on one PC in three folders: under %LOCALAPPDATA% commands run in
/// system32 and writes fail silently, and the model can still report success. Cause not
/// proved; %TEMP% is presumed the same, untested. Refused because no answer from the
/// session could be trusted to show it.
pub fn refuse_unsafe_windows_folder(cwd: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    let cwd = crate::hook::comparable(cwd);
    let roots = ["LOCALAPPDATA", "TEMP", "TMP"].map(|name| {
        let value = std::env::var_os(name).unwrap_or_default();
        (name, crate::hook::comparable(Path::new(&value)))
    });
    match unsafe_folder(&cwd, &roots) {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

/// Whole path parts, ignoring case and slash style. An unset variable is no root.
pub fn unsafe_folder(cwd: &str, roots: &[(&str, String)]) -> Option<String> {
    let parts = |path: &str| -> Vec<String> {
        let path = path.trim_start_matches(r"\\?\").replace('\\', "/");
        path.to_lowercase()
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let folder = parts(cwd);
    roots.iter().find_map(|(name, root)| {
        let root = parts(root);
        (!root.is_empty() && folder.starts_with(&root)).then(|| {
            format!(
                "{cwd} is under %{name}%, where commands run in system32 and writes fail silently while the model can still report success. Use a folder outside AppData and the temp folder."
            )
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptAt {
    /// Hold `Ready` until it arrives.
    BeforeReady,
    /// The client doesn't fire it until a prompt: count from the first.
    AfterFirstPrompt,
}

#[derive(Debug, Clone)]
pub struct Protection {
    pub at: ReceiptAt,
    pub wait: Duration,
    pub reports: PathBuf,
}

impl Protection {
    pub fn new(at: ReceiptAt, data_dir: &Path) -> Self {
        let wait = std::env::var(RECEIPT_WAIT_VAR)
            .ok()
            .and_then(|ms| ms.parse().ok())
            .map_or(RECEIPT_WAIT, Duration::from_millis);
        Self {
            at,
            wait,
            reports: crate::hook::reports_dir_in(data_dir),
        }
    }

    pub(crate) fn receipt_arrived(&self, agent_session: &str, since_ms: i64) -> bool {
        session_start_since(&self.reports, agent_session, since_ms)
    }

    pub(crate) fn missing_message(&self) -> String {
        format!(
            "no session-start receipt within {:.1} s: the layer's hooks did not run, so the session would be unguarded. Check the hook configuration and that `{PROGRAM}` is on the launch's PATH. On a slow machine, raise {RECEIPT_WAIT_VAR}.",
            self.wait.as_secs_f64()
        )
    }

    /// Runs the guard the way a client will, by name on the launch's PATH. A missing
    /// binary or a wrong PATH would otherwise fail open with no sign.
    pub(crate) fn check_guard(&self, launch: &Launch, cwd: &Path) -> Result<(), String> {
        let ask = |command: &str| {
            let input = json!({
                "session_id": SELF_TEST_SESSION,
                "cwd": cwd,
                "tool_name": "Bash",
                "tool_input": { "command": command },
            });
            run_guard(launch, cwd, &input.to_string())
        };
        if !ask("sh -c 'git push --force'")?.contains("\"permissionDecision\":\"deny\"") {
            return Err("it did not deny a force push in its self-test".to_owned());
        }
        if !ask("ls")?.is_empty() {
            return Err("it did not stay silent on a plain command in its self-test".to_owned());
        }
        Ok(())
    }
}

fn run_guard(launch: &Launch, cwd: &Path, input: &str) -> Result<String, String> {
    let mut command = Command::new(PROGRAM);
    for name in &launch.env_remove {
        command.env_remove(name);
    }
    let mut child = command
        .args(["hook", "pre-command"])
        .envs(launch.env.iter().map(|(name, value)| (name, value)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("`{PROGRAM}` could not start from the launch's PATH: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let deadline = Instant::now() + SELF_TEST_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("it did not answer its self-test in time".to_owned());
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

impl Launch {
    /// A real session's launch: the program on `PATH`, the data folder named, and the
    /// checks turned on. `at` is the driver's `RECEIPT_AT`.
    pub fn protect(
        &mut self,
        at: ReceiptAt,
        program_dir: &Path,
        data_dir: &Path,
        inherited_path: Option<OsString>,
    ) -> Result<(), std::env::JoinPathsError> {
        self.env.retain(|(name, _)| name != DATA_DIR_VAR);
        self.env
            .extend(launch_env(program_dir, data_dir, inherited_path)?);
        self.protection = Some(Protection::new(at, data_dir));
        Ok(())
    }
}
