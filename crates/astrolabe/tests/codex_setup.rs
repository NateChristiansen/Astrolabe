mod common;

use common::TempDir;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use toml_edit::DocumentMut;

struct Setup {
    temp: TempDir,
    config: PathBuf,
    notes: PathBuf,
    state: PathBuf,
}
impl Setup {
    fn new() -> Self {
        let temp = TempDir::new();
        let config = temp.0.join("settings/config.toml");
        let notes = temp.0.join("my notes");
        let state = temp.0.join("state");
        fs::create_dir_all(&notes).unwrap();
        fs::create_dir_all(state.join("repos")).unwrap();
        let this = Self {
            temp,
            config,
            notes,
            state,
        };
        this.register(&this.notes);
        this
    }
    fn register(&self, notes: &std::path::Path) {
        fs::write(
            self.state.join("repos/agent.json"),
            serde_json::json!({"name":"agent","checkout":notes}).to_string(),
        )
        .unwrap();
    }
    fn write(&self, text: &str) {
        fs::create_dir_all(self.config.parent().unwrap()).unwrap();
        fs::write(&self.config, text).unwrap();
    }
    fn read(&self) -> String {
        fs::read_to_string(&self.config).unwrap()
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .args(["setup", "codex", "--config"])
            .arg(&self.config)
            .args(args)
            .env("ASTROLABE_LAYER_STATE", &self.state)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    fn rejected(&self, args: &[&str], message: &str) {
        let before = fs::read(&self.config).ok();
        let out = self.run(args);
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(fs::read(&self.config).ok(), before);
    }
    fn files(&self) -> Vec<PathBuf> {
        let mut files = fs::read_dir(self.config.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        files.sort();
        files
    }
}

#[test]
fn preview_does_not_create_config_or_ownership_and_first_apply_is_repeatable() {
    let s = Setup::new();
    let preview = s.ok(&[]);
    assert!(preview.contains("Preview only"));
    assert!(!s.config.exists());
    assert!(!s.state.join("setup-codex.json").exists());
    assert!(!s.state.join("setup-codex.lock").exists());
    s.ok(&["--apply"]);
    let config = s.read();
    let receipt = fs::read(s.state.join("setup-codex.json")).unwrap();
    let modified = fs::metadata(&s.config).unwrap().modified().unwrap();
    let files = s.files();
    let doc = config.parse::<DocumentMut>().unwrap();
    assert_eq!(doc["default_permissions"].as_str(), Some("astrolabe-notes"));
    assert_eq!(doc["approvals_reviewer"].as_str(), Some("user"));
    assert_eq!(
        doc["permissions"]["astrolabe-notes"]["extends"].as_str(),
        Some(":workspace")
    );
    assert!(doc.get("projects").is_none());
    for _ in 0..3 {
        assert!(s.ok(&["--apply"]).contains("Already configured"));
        assert_eq!(s.read(), config);
        assert_eq!(fs::read(s.state.join("setup-codex.json")).unwrap(), receipt);
        assert_eq!(
            fs::metadata(&s.config).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(s.files(), files);
    }
}

#[test]
fn preserves_comments_other_settings_and_creates_one_exact_backup() {
    let s = Setup::new();
    let old = "# keep this comment\nmodel = 'my-model' # keep inline\napprovals_reviewer = 'auto_review'\n\n[projects.'a project']\ntrust_level = 'trusted'\n\n[features]\nnetwork_proxy = true\n";
    s.write(old);
    s.ok(&["--apply"]);
    let new = s.read();
    assert!(new.contains("# keep this comment"));
    assert!(new.contains("model = 'my-model' # keep inline"));
    assert!(new.contains("approvals_reviewer = 'auto_review'"));
    let doc = new.parse::<DocumentMut>().unwrap();
    assert_eq!(
        doc["projects"]["a project"]["trust_level"].as_str(),
        Some("trusted")
    );
    assert_eq!(doc["features"]["network_proxy"].as_bool(), Some(true));
    let backups = s
        .files()
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("astrolabe-backup"))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_to_string(&backups[0]).unwrap(), old);
    s.ok(&["--apply"]);
    assert_eq!(s.files().len(), 2);
    assert_eq!(s.read(), new);
}

#[test]
fn moves_only_the_notes_root_added_by_setup() {
    let s = Setup::new();
    let extra = s.temp.0.join("shared library");
    let old = format!(
        "sandbox_mode = 'workspace-write'\n[sandbox_workspace_write]\nwritable_roots = [{}]\nnetwork_access = true\nexclude_slash_tmp = true\n",
        toml_edit::Value::from(extra.to_string_lossy().as_ref())
    );
    s.write(&old);
    s.ok(&["--apply", "--reviewer", "auto_review"]);
    let next = s.temp.0.join("moved notes");
    fs::create_dir(&next).unwrap();
    s.register(&next);
    s.ok(&["--apply"]);
    let doc = s.read().parse::<DocumentMut>().unwrap();
    let managed = &doc["permissions"]["astrolabe-notes"];
    assert!(
        managed["workspace_roots"]
            .get(astrolabe::registry::native(&s.notes))
            .is_none()
    );
    assert_eq!(
        managed["workspace_roots"][astrolabe::registry::native(&next)].as_bool(),
        Some(true)
    );
    assert_eq!(
        managed["workspace_roots"][extra.to_str().unwrap()].as_bool(),
        Some(true)
    );
    assert_eq!(managed["network"]["enabled"].as_bool(), Some(true));
    assert_eq!(managed["filesystem"][":slash_tmp"].as_str(), Some("read"));
    assert!(doc.get("sandbox_mode").is_none());
    assert!(doc.get("sandbox_workspace_write").is_none());
    assert_eq!(doc["approvals_reviewer"].as_str(), Some("auto_review"));
    assert!(s.ok(&["--apply"]).contains("Already configured"));
}

#[test]
fn an_existing_notes_root_is_not_owned_or_removed_when_notes_move() {
    let s = Setup::new();
    let notes = astrolabe::registry::native(&s.notes);
    s.write(&format!(
        "sandbox_mode = 'workspace-write'\n[sandbox_workspace_write]\nwritable_roots = [{}]\n",
        toml_edit::Value::from(notes.as_str())
    ));
    s.ok(&["--apply"]);
    let next = s.temp.0.join("new notes");
    fs::create_dir(&next).unwrap();
    s.register(&next);
    s.ok(&["--apply"]);
    let doc = s.read().parse::<DocumentMut>().unwrap();
    assert_eq!(
        doc["permissions"]["astrolabe-notes"]["workspace_roots"][notes].as_bool(),
        Some(true)
    );
}

#[test]
fn custom_workspace_restrictions_are_inherited_and_left_intact() {
    let s = Setup::new();
    s.write("default_permissions = 'careful'\n[permissions.careful]\nextends = ':workspace'\n[permissions.careful.filesystem.':workspace_roots']\n'**/*.env' = 'deny' # secret rule\n");
    s.ok(&["--apply"]);
    let text = s.read();
    assert!(text.contains("'**/*.env' = 'deny' # secret rule"));
    let doc = text.parse::<DocumentMut>().unwrap();
    assert_eq!(
        doc["permissions"]["astrolabe-notes"]["extends"].as_str(),
        Some("careful")
    );
    assert!(s.ok(&["--apply"]).contains("Already configured"));
}

#[test]
fn modified_or_unowned_profiles_are_never_overwritten() {
    let s = Setup::new();
    s.write("[permissions.astrolabe-notes]\nextends = ':workspace'\n");
    s.rejected(&["--apply"], "already exists");
    fs::remove_file(&s.config).unwrap();
    s.ok(&["--apply"]);
    let text = s
        .read()
        .replace("Notes access managed", "My edited profile");
    s.write(&text);
    s.rejected(&["--apply"], "edited outside setup");
}

#[test]
fn an_externally_changed_selection_or_legacy_override_is_not_silently_reset() {
    let s = Setup::new();
    s.ok(&["--apply"]);
    let configured = s.read();
    s.write(&configured.replace(
        "default_permissions = \"astrolabe-notes\"",
        "default_permissions = \":read-only\"",
    ));
    s.rejected(&["--apply"], "selected sandbox changed");
    s.write(&format!("sandbox_mode = 'read-only'\n{configured}"));
    s.rejected(&["--apply"], "selected sandbox changed");
}

#[test]
fn malformed_conflicting_or_unknown_settings_fail_without_writes() {
    let s = Setup::new();
    for (text, expected) in [
        ("model = [SECRET INVALID\n", "invalid TOML"),
        ("sandbox_mode = 'read-only'\n", "isn't workspace-write"),
        (
            "sandbox_mode = 'danger-full-access'\n",
            "isn't workspace-write",
        ),
        (
            "sandbox_mode = 'workspace-write'\n[sandbox_workspace_write]\nfuture_setting = true\n",
            "unsupported legacy",
        ),
        (
            "default_permissions = ':read-only'\n",
            "doesn't extend :workspace",
        ),
        (
            "default_permissions = 'loop'\n[permissions.loop]\nextends = 'loop'\n",
            "cycle",
        ),
    ] {
        s.write(text);
        s.rejected(&["--apply"], expected);
        assert!(!s.state.join("setup-codex.json").exists());
        assert_eq!(s.files().len(), 1);
    }
    let out = s.run(&[]);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("SECRET"));
}

#[test]
fn interrupted_first_apply_and_notes_update_can_be_retried() {
    let s = Setup::new();
    s.ok(&["--apply"]);
    let first = s.read();
    // The receipt reached disk but the first config replacement did not.
    fs::remove_file(&s.config).unwrap();
    s.ok(&["--apply"]);
    assert_eq!(s.read(), first);
    let next = s.temp.0.join("moved notes");
    fs::create_dir(&next).unwrap();
    s.register(&next);
    s.ok(&["--apply"]);
    let updated = s.read();
    // The receipt contains both the previous and the desired profile.
    s.write(&first);
    s.ok(&["--apply"]);
    assert_eq!(s.read(), updated);
}

#[test]
fn damaged_ownership_missing_registration_and_bad_options_fail_without_touching_config() {
    let s = Setup::new();
    s.ok(&["--apply"]);
    fs::write(s.state.join("setup-codex.json"), "bad").unwrap();
    s.rejected(&["--apply"], "ownership record is invalid");
    fs::remove_file(s.state.join("repos/agent.json")).unwrap();
    s.rejected(&["--apply"], "no notes checkout");
    s.rejected(&["--reviewer", "never"], "usage:");
    s.rejected(&["--apply", "--apply"], "usage:");
}

#[test]
fn unrelated_changes_after_setup_survive_and_reviewer_changes_are_repeatable() {
    let s = Setup::new();
    s.ok(&["--apply"]);
    s.write(&format!(
        "# unrelated addition\nmodel = 'later'\n{}",
        s.read()
    ));
    s.ok(&["--apply", "--reviewer", "auto_review"]);
    let new = s.read();
    assert!(new.contains("# unrelated addition\nmodel = 'later'"));
    assert_eq!(
        new.parse::<DocumentMut>().unwrap()["approvals_reviewer"].as_str(),
        Some("auto_review")
    );
    assert!(
        s.ok(&["--apply", "--reviewer", "auto_review"])
            .contains("Already configured")
    );
    assert_eq!(s.read(), new);
}

#[cfg(unix)]
#[test]
fn symlinks_are_refused_and_private_file_permissions_are_preserved() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let s = Setup::new();
    s.write("model = 'private'\n");
    fs::set_permissions(&s.config, fs::Permissions::from_mode(0o600)).unwrap();
    s.ok(&["--apply"]);
    assert_eq!(
        fs::metadata(&s.config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let target = s.temp.0.join("target.toml");
    fs::rename(&s.config, &target).unwrap();
    symlink(&target, &s.config).unwrap();
    s.rejected(&["--apply"], "not a regular file");
}

#[test]
fn sandbox_probe_does_not_mistake_a_missing_or_writable_git_folder_for_protection() {
    let s = Setup::new();
    let git = s.temp.0.join("git metadata");
    for expected in [
        "couldn't check Git metadata protection",
        "Git metadata was writable",
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .args(["setup", "codex", "--probe"])
            .arg(&s.notes)
            .arg(&git)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains(expected));
        assert_eq!(fs::read_dir(s.notes.join(".handoffs")).unwrap().count(), 0);
        if git.exists() {
            assert_eq!(fs::read_dir(&git).unwrap().count(), 0);
        } else {
            fs::create_dir(&git).unwrap();
        }
    }
}

#[test]
fn concurrent_setup_is_refused_without_changing_config_or_ownership() {
    let s = Setup::new();
    s.ok(&["--apply"]);
    let record = fs::read(s.state.join("setup-codex.json")).unwrap();
    let lock = fs::OpenOptions::new()
        .write(true)
        .open(s.state.join("setup-codex.lock"))
        .unwrap();
    fs4::fs_std::FileExt::lock_exclusive(&lock).unwrap();
    s.rejected(&["--apply"], "another setup is running");
    assert_eq!(fs::read(s.state.join("setup-codex.json")).unwrap(), record);
}

#[test]
fn explicit_config_and_state_do_not_require_a_user_home() {
    let s = Setup::new();
    let out = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .args(["setup", "codex", "--config"])
        .arg(&s.config)
        .env("ASTROLABE_LAYER_STATE", &s.state)
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .env_remove("CODEX_HOME")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!s.config.exists());
}
