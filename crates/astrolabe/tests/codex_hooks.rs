//! `astrolabe codex-hooks` and `astrolabe status` read Codex's stored hook trust and tell a
//! confirmed loss from trust that holds, on fixture Codex folders.
mod common;
use common::TempDir;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The fixture plugin's fingerprint by Codex's rule, from an independent implementation that
/// reproduced Codex's stored value on a real installation. Codex hashes relative paths in the
/// platform's form, so Windows, with backslashes, gets a different value.
const PRINT: &str = if cfg!(windows) {
    "sha256:0b26da3ee8daccbf7639517e2d21bd53dd5f72fa908b6db19b8ce6e96cbb58c9"
} else {
    "sha256:1af4f7e3a926392e567ca2f31bff2d59c3ca61c20568edd20e682ac51dc3eaae"
};
const OTHER: &str = "sha256:a4c0f900a52fc59e47e2d60eed1a8a8d2ea6aef05c861e39cd5013af122f15fe";
const ID: &str = "astrolabe@market";
/// The fixture's hooks, one handler each, as Codex keys them.
const EVENTS: [&str; 4] = ["session_start", "pre_tool_use", "stop", "session_end"];
const HOOKS: &str = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"a"}]}],"PreToolUse":[{"hooks":[{"type":"command","command":"b"}]}],"Stop":[{"hooks":[{"type":"command","command":"c"}]}],"SessionEnd":[{"hooks":[{"type":"command","command":"d"}]}]}}
"#;

struct Codex {
    _dir: TempDir,
    home: PathBuf,
}

impl Codex {
    /// A Codex folder with the plugin installed once and each hook in `states`.
    fn new(plugin_on: bool, states: &[&str]) -> Self {
        let dir = TempDir::new();
        let home = dir.0.join("codex");
        write_plugin(&copy_dir(&home, "0.6.0"));
        let mut config = format!("[plugins.\"{ID}\"]\nenabled = {plugin_on}\n\n[hooks.state]\n");
        for (event, state) in EVENTS.iter().zip(states) {
            config += &format!("\n[hooks.state.\"{ID}:hooks/hooks.json:{event}:0:0\"]\n{state}\n");
        }
        fs::write(home.join("config.toml"), config).unwrap();
        Codex { _dir: dir, home }
    }

    fn trusted() -> Self {
        Self::new(true, &every(&state(PRINT)))
    }

    /// Adds a state entry for a key Codex wouldn't look up for this copy.
    fn add_state(&self, key: &str, state: &str) {
        let config = self.home.join("config.toml");
        let mut text = fs::read_to_string(&config).unwrap();
        text += &format!("\n[hooks.state.\"{ID}:hooks/hooks.json:{key}\"]\n{state}\n");
        fs::write(config, text).unwrap();
    }

    fn check(&self, extra: &[&str]) -> (String, bool) {
        let out = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .arg("codex-hooks")
            .arg("--codex-home")
            .arg(&self.home)
            .args(extra)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    }
}

fn every(state: &str) -> [&str; 4] {
    [state; 4]
}

fn state(content: &str) -> String {
    format!(
        "trusted_hash = \"sha256:{}\"\ntrusted_content_hash = \"{content}\"",
        "1".repeat(64)
    )
}

fn copy_dir(home: &Path, version: &str) -> PathBuf {
    let root = home.join("plugins/cache/market/astrolabe").join(version);
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_plugin(root: &Path) {
    for (rel, text) in [
        (".claude-plugin/plugin.json", "{\"name\":\"astrolabe\"}\n"),
        ("hooks/hooks.json", HOOKS),
        ("skills/effort/SKILL.md", "skill\n"),
        ("README.md", "readme\n"),
        // Skipped by Codex's walk, so a cache here doesn't change the fingerprint.
        ("__pycache__/a.pyc", "x"),
    ] {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

fn installed(c: &Codex) -> PathBuf {
    c.home.join("plugins/cache/market/astrolabe/0.6.0")
}

#[test]
fn matching_trust_says_hooks_run() {
    // Catches a fingerprint that drifts from Codex's: every trusted machine would show a loss.
    let (text, runs) = Codex::trusted().check(&[]);
    assert!(runs, "{text}");
    assert!(text.contains("Codex runs Astrolabe's hooks here"), "{text}");
}

#[test]
fn a_changed_skill_file_is_a_confirmed_loss() {
    // The 387 case: hooks.json unchanged, one skill's text changed, Codex skips every hook.
    let c = Codex::trusted();
    fs::write(
        installed(&c).join("skills/effort/SKILL.md"),
        "skill, edited\n",
    )
    .unwrap();
    let (text, runs) = c.check(&[]);
    assert!(!runs);
    assert!(
        text.contains("Codex has stopped running Astrolabe's hooks here"),
        "{text}"
    );
}

#[test]
fn one_hook_trusted_on_an_older_copy_is_a_loss() {
    // Catches a check that stops at the first matching hook.
    let c = Codex::new(
        true,
        &[&state(PRINT), &state(OTHER), &state(PRINT), &state(PRINT)],
    );
    assert!(c.check(&[]).0.contains("stopped running"));
}

#[test]
fn hooks_never_trusted_say_so_and_ask_nothing() {
    let (text, runs) = Codex::new(true, &[]).check(&[]);
    assert!(!runs);
    assert!(
        text.contains("at least one has never been trusted"),
        "{text}"
    );
}

#[test]
fn an_entry_for_a_hook_since_removed_is_ignored() {
    // Codex never prunes old entries and reads only the installed hooks' keys, so a hook a
    // release dropped would otherwise show "lost" for good, re-trusting or not.
    let c = Codex::trusted();
    c.add_state("user_prompt_submit:0:0", &state(OTHER));
    c.add_state("pre_tool_use:0:1", &state(OTHER));
    let (text, runs) = c.check(&[]);
    assert!(runs, "{text}");
}

#[test]
fn a_new_hook_nobody_trusted_is_not_read_as_running() {
    // A release adds a hook and the owner trusts the others: Codex skips the new one.
    let p = state(PRINT);
    let c = Codex::new(true, &[&p, &p, &p]);
    let (text, runs) = c.check(&[]);
    assert!(!runs);
    assert!(
        text.contains("at least one has never been trusted"),
        "{text}"
    );
}

#[test]
fn hooks_or_the_plugin_turned_off_are_the_owner_s_choice() {
    let hook_off = format!("enabled = false\n{}", state(PRINT));
    let p = state(PRINT);
    let c = Codex::new(true, &[&p, &hook_off, &p, &p]);
    assert!(c.check(&[]).0.contains("turned off"));
    let c = Codex::new(false, &every(&p));
    assert!(c.check(&[]).0.contains("turned off"));
}

#[test]
fn a_stored_form_this_build_does_not_know_is_unverified_not_lost() {
    // A future Codex that stores its trust differently mustn't raise a false alarm.
    let odd = state("sha512:abc");
    let c = Codex::new(true, &every(&odd));
    let (text, runs) = c.check(&[]);
    assert!(!runs);
    assert!(text.contains("Can't tell"), "{text}");
}

#[test]
fn trust_from_before_content_checks_uses_codex_s_baseline() {
    // Codex pins a one-time baseline beside its config for trust given before 0.164.0.
    let legacy = format!("trusted_hash = \"sha256:{}\"", "1".repeat(64));
    let c = Codex::new(true, &every(&legacy));
    assert!(c.check(&[]).0.contains("Can't tell"), "no baseline yet");
    let config = c.home.join("config.toml");
    let mut identity = Sha256::new();
    for part in [config.as_os_str().as_encoded_bytes(), ID.as_bytes()] {
        identity.update((part.len() as u64).to_le_bytes());
        identity.update(part);
    }
    let baseline = c
        .home
        .join(".hook-trust")
        .join(format!("{:x}", identity.finalize()));
    fs::create_dir_all(baseline.parent().unwrap()).unwrap();
    fs::write(&baseline, PRINT).unwrap();
    assert!(c.check(&[]).1, "baseline matches");
    fs::write(&baseline, OTHER).unwrap();
    assert!(
        c.check(&[]).0.contains("stopped running"),
        "baseline differs"
    );
}

#[test]
fn the_copy_codex_runs_is_judged_by_its_own_version_rule() {
    // Codex runs `local` if present, else the highest version, so a stale copy left behind
    // by a failed delete mustn't hide a loss behind "can't tell" or be judged instead.
    let c = Codex::trusted();
    let base = c.home.join("plugins/cache/market/astrolabe");
    fs::rename(base.join("0.6.0"), base.join("0.10.0")).unwrap();
    for stale in ["0.9.0", "0.10.0-rc.1", "not a version"] {
        write_plugin(&copy_dir(&c.home, stale));
        fs::write(base.join(stale).join("README.md"), "stale\n").unwrap();
    }
    let (text, runs) = c.check(&[]);
    assert!(runs, "0.10.0 is the highest by semantic version: {text}");
    write_plugin(&copy_dir(&c.home, "local"));
    fs::write(base.join("local/README.md"), "local\n").unwrap();
    assert!(c.check(&[]).0.contains("stopped running"), "local wins");
}

#[test]
fn no_codex_or_no_plugin_is_not_a_lapse() {
    let dir = TempDir::new();
    let out = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .args(["codex-hooks", "--codex-home"])
        .arg(dir.0.join("none"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("isn't installed in Codex here"));
}

#[test]
fn status_says_when_codex_has_stopped_running_the_hooks() {
    // The switch alone said "on" while Codex skipped every hook.
    let c = Codex::trusted();
    fs::write(installed(&c).join("README.md"), "changed\n").unwrap();
    let state = c.home.parent().unwrap().join("state");
    fs::create_dir_all(&state).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .arg("status")
        .env("ASTROLABE_LAYER_STATE", &state)
        .env("CODEX_HOME", &c.home)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Astrolabe is off"), "{text}");
    assert!(
        text.contains("Codex has stopped running Astrolabe's hooks here"),
        "{text}"
    );
}

#[test]
fn codex_s_files_are_only_read() {
    let c = Codex::trusted();
    let before = fs::read(c.home.join("config.toml")).unwrap();
    c.check(&[]);
    assert_eq!(fs::read(c.home.join("config.toml")).unwrap(), before);
    assert!(!c.home.join(".hook-trust").exists());
}
