//! Opt-in host configuration. Never called by a hook or the executable installer.
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, value};

const PROFILE: &str = "astrolabe-notes";
const USAGE: &str = "usage: astrolabe setup codex [--apply] [--check] [--reviewer user|auto_review] [--config <file>]";

#[derive(Clone, Deserialize, Serialize)]
struct Managed {
    digest: String,
    notes: String,
    inserted: bool,
}

#[derive(Deserialize, Serialize)]
struct Receipt {
    format: u8,
    config: String,
    current: Managed,
    // Written before config replacement: either version is recoverable after interruption.
    previous: Option<Managed>,
}

struct Plan {
    original: Option<String>,
    updated: String,
    receipt: Receipt,
    summary: String,
}

pub fn command(args: &[&str]) -> Result<String, String> {
    // The probe is a child of `codex sandbox`, not an agent or a project command.
    if let ["--probe", notes, git] = args {
        return probe(Path::new(notes), (!git.is_empty()).then(|| Path::new(git)));
    }
    let mut apply = false;
    let mut check = false;
    let mut reviewer = None;
    let mut config = None;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match *word {
            "--apply" if !apply => apply = true,
            "--check" if !check => check = true,
            "--reviewer" if reviewer.is_none() => match words.next().copied() {
                Some(v @ ("user" | "auto_review")) => reviewer = Some(v),
                _ => return Err(USAGE.into()),
            },
            "--config" if config.is_none() => {
                config = Some(PathBuf::from(words.next().ok_or(USAGE)?));
            }
            _ => return Err(USAGE.into()),
        }
    }
    let config = match config {
        Some(path) => path,
        None => config_path()?,
    };
    let config = std::path::absolute(config).map_err(|e| e.to_string())?;
    let notes = crate::activate::notes_root()?;
    let notes = notes
        .canonicalize()
        .map_err(|e| format!("notes folder: {e}"))?;
    if !notes.is_dir() {
        return Err("the registered notes checkout is not a directory".into());
    }
    let state = crate::activate::state_dir().ok_or("Astrolabe has no state folder")?;
    let receipt_path = state.join("setup-codex.json");
    if apply {
        fs::create_dir_all(&state).map_err(|e| e.to_string())?;
    }
    // Preview and check are read-only apart from the check's removable probe files.
    let _lock = if apply {
        let path = state.join("setup-codex.lock");
        regular(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|e| format!("setup lock: {e}"))?;
        if !FileExt::try_lock_exclusive(&file).map_err(|e| format!("setup lock: {e}"))? {
            return Err("another setup is running; retry when it finishes".into());
        }
        Some(file)
    } else {
        None
    };
    let old_receipt = read_receipt(&receipt_path)?;
    let plan = plan(&config, &notes, reviewer, old_receipt.as_ref())?;
    let changed = plan.original.as_deref() != Some(plan.updated.as_str());
    let mut output = plan.summary.clone();
    if changed && apply {
        let backup = apply_plan(&config, &receipt_path, &plan)?;
        output.push_str("\nSaved the setup.");
        if let Some(backup) = backup {
            output.push_str(&format!(" Backup: {}", backup.display()));
        }
    } else if changed {
        output.push_str(
            "\nPreview only; nothing written. Run again with --apply to save these changes.",
        );
    } else {
        output.push_str("\nAlready configured; nothing changed.");
    }
    if check {
        if changed && !apply {
            return Err(format!(
                "{output}\nApply the preview before checking the saved setup."
            ));
        }
        let checked =
            check_saved(&config, &notes).map_err(|why| format!("{output}\nCheck failed: {why}"))?;
        output.push_str(&format!("\n{checked}"));
    }
    output.push_str("\nStart a fresh session. In the desktop app select Custom (config.toml); an existing chat or an explicit sandbox override may keep different permissions. Project and managed settings can override these defaults.");
    Ok(output)
}

fn config_path() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(dir).join("config.toml"));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or("no home folder; pass --config <file>")?;
    Ok(PathBuf::from(home).join(".codex/config.toml"))
}

fn regular(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(format!(
            "{} is not a regular file; nothing changed",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    regular(path)?;
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

fn read_receipt(path: &Path) -> Result<Option<Receipt>, String> {
    read_optional(path)?
        .map(|s| {
            serde_json::from_str::<Receipt>(&s)
                .map_err(|_| "the setup ownership record is invalid; nothing changed".into())
        })
        .transpose()
}

fn parse(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>().map_err(|e| {
        format!(
            "invalid TOML at byte {}; nothing changed",
            e.span().map_or(0, |s| s.start)
        )
    })
}

fn digest(item: &Item) -> String {
    let text = item
        .as_table()
        .map(|t| t.clone().into_inline_table().to_string())
        .unwrap_or_else(|| item.to_string());
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn table<'a>(parent: &'a mut Table, name: &str) -> Result<&'a mut Table, String> {
    if !parent.contains_key(name) {
        let mut t = Table::new();
        t.set_implicit(true);
        parent.insert(name, Item::Table(t));
    }
    parent[name]
        .as_table_mut()
        .ok_or_else(|| format!("{name} must be a TOML table; nothing changed"))
}

fn workspace_parent(doc: &DocumentMut, name: &str, seen: &mut Vec<String>) -> Result<(), String> {
    if name == ":workspace" {
        return Ok(());
    }
    if seen.iter().any(|n| n == name) {
        return Err("permission profile inheritance has a cycle; nothing changed".into());
    }
    seen.push(name.into());
    let parent = doc.get("permissions").and_then(|p| p.get(name)).and_then(|p| p.get("extends")).and_then(Item::as_str)
        .ok_or("the existing permission profile doesn't extend :workspace; configure it manually so its restrictions are preserved")?;
    workspace_parent(doc, parent, seen)
}

fn plan(
    config: &Path,
    notes: &Path,
    reviewer: Option<&str>,
    receipt: Option<&Receipt>,
) -> Result<Plan, String> {
    let original = read_optional(config)?;
    let mut doc = parse(original.as_deref().unwrap_or_default())?;
    let config_name = destination_name(config)?;
    let notes_name = crate::registry::native(notes);
    let existing = doc.get("permissions").and_then(|p| p.get(PROFILE));
    let owned = match (receipt, existing) {
        (Some(r), item) => {
            if r.format != 1 || r.config != config_name {
                return Err(
                    "the setup record belongs to another configuration; nothing changed".into(),
                );
            }
            match item {
                Some(item) if digest(item) == r.current.digest => Some(r.current.clone()),
                Some(item) if r.previous.as_ref().is_some_and(|p| digest(item) == p.digest) => r.previous.clone(),
                None if r.previous.is_none() => None,
                _ => return Err("the managed permission profile was edited outside setup; nothing changed. Review it before rerunning setup".into()),
            }
        }
        (None, Some(_)) => {
            return Err(format!(
                "permissions.{PROFILE} already exists without an Astrolabe ownership record; nothing changed"
            ));
        }
        (None, None) => None,
    };
    if owned.is_some()
        && (doc.get("default_permissions").and_then(Item::as_str) != Some(PROFILE)
            || doc.contains_key("sandbox_mode")
            || doc.contains_key("sandbox_workspace_write"))
    {
        return Err("the selected sandbox changed outside setup; review its restrictions before rerunning. Nothing changed".into());
    }
    let reviewer = reviewer
        .or_else(|| doc.get("approvals_reviewer").and_then(Item::as_str))
        .unwrap_or("user");
    if !["user", "auto_review"].contains(&reviewer) {
        return Err(
            "the approval reviewer isn't supported; choose --reviewer user or auto_review".into(),
        );
    }
    let reviewer = reviewer.to_owned();
    let mut managed = if owned.is_some() {
        existing
            .and_then(Item::as_table)
            .cloned()
            .ok_or("the managed profile must be a table")?
    } else {
        let mode = doc
            .get("sandbox_mode")
            .map(|i| i.as_str().ok_or("sandbox_mode must be a string"))
            .transpose()?;
        if mode.is_some_and(|m| m != "workspace-write") {
            return Err("the existing sandbox isn't workspace-write; choose its permissions manually before setup. Nothing changed".into());
        }
        let parent = if mode.is_some() {
            ":workspace"
        } else {
            doc.get("default_permissions")
                .map(|i| i.as_str().ok_or("default_permissions must be a string"))
                .transpose()?
                .unwrap_or(":workspace")
        };
        workspace_parent(&doc, parent, &mut vec![])?;
        let mut managed = Table::new();
        managed.insert(
            "description",
            value("Notes access managed by astrolabe setup codex"),
        );
        managed.insert("extends", value(parent));
        if let Some(legacy) = doc.get("sandbox_workspace_write") {
            if mode.is_none() && doc.contains_key("default_permissions") {
                return Err("legacy and named permissions are mixed; resolve that configuration before setup. Nothing changed".into());
            }
            let legacy = legacy
                .as_table()
                .ok_or("sandbox_workspace_write must be a table")?;
            for (key, item) in legacy.iter() {
                match key {
                    "writable_roots" => {
                        let roots = item.as_array().ok_or("writable_roots must be an array")?;
                        for root in roots.iter() {
                            let root = root.as_str().ok_or("writable_roots must contain paths")?;
                            table(&mut managed, "workspace_roots")?.insert(root, value(true));
                        }
                    }
                    "network_access" => {
                        table(&mut managed, "network")?.insert(
                            "enabled",
                            value(
                                item.as_bool()
                                    .ok_or("network_access must be true or false")?,
                            ),
                        );
                    }
                    "exclude_tmpdir_env_var" | "exclude_slash_tmp" => {
                        if item
                            .as_bool()
                            .ok_or("temporary-folder exclusions must be true or false")?
                        {
                            table(&mut managed, "filesystem")?.insert(
                                if key == "exclude_slash_tmp" {
                                    ":slash_tmp"
                                } else {
                                    ":tmpdir"
                                },
                                value("read"),
                            );
                        }
                    }
                    _ => {
                        return Err(format!(
                            "unsupported legacy sandbox setting {key}; nothing changed"
                        ));
                    }
                }
            }
        }
        doc.remove("sandbox_mode");
        doc.remove("sandbox_workspace_write");
        managed
    };
    let roots = table(&mut managed, "workspace_roots")?;
    if let Some(old) = owned.as_ref()
        && old.inserted
        && old.notes != notes_name
    {
        roots.remove(&old.notes);
    }
    let inserted = if let Some(old) = owned.as_ref().filter(|o| o.notes == notes_name) {
        old.inserted
    } else {
        !roots
            .get(&notes_name)
            .is_some_and(|v| v.as_bool() == Some(true))
    };
    roots.insert(&notes_name, value(true));
    let managed_item = Item::Table(managed);
    table(doc.as_table_mut(), "permissions")?.insert(PROFILE, managed_item);
    for (key, desired) in [
        ("default_permissions", PROFILE),
        ("approval_policy", "on-request"),
        ("approvals_reviewer", &reviewer),
    ] {
        if doc.get(key).and_then(Item::as_str) != Some(desired) {
            doc[key] = value(desired);
        }
    }
    let updated = doc.to_string();
    // Fingerprint all nested profile values, including the owned notes path.
    let saved = parse(&updated)?;
    let managed = &saved["permissions"][PROFILE];
    let next = Managed {
        digest: digest(managed),
        notes: notes_name.clone(),
        inserted,
    };
    let receipt = Receipt {
        format: 1,
        config: config_name,
        current: next,
        previous: owned,
    };
    let mut profile_preview = DocumentMut::new();
    table(profile_preview.as_table_mut(), "permissions")?.insert(PROFILE, managed.clone());
    let migration = if doc_contains_legacy(original.as_deref().unwrap_or_default())? {
        "\nMigrate sandbox_mode and sandbox_workspace_write into the managed profile."
    } else {
        ""
    };
    let summary = format!(
        "Config: {}\nNotes: {notes_name}\nSet default_permissions = {PROFILE:?}, approval_policy = \"on-request\", approvals_reviewer = {reviewer:?}.\nManaged permission profile (existing workspace restrictions are retained):\n{}",
        config.display(),
        profile_preview
    );
    Ok(Plan {
        original,
        updated,
        receipt,
        summary: format!("{summary}{migration}"),
    })
}

fn unique(path: &Path, kind: &str) -> PathBuf {
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).expect("system randomness unavailable");
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    path.with_file_name(format!(
        "{}.{kind}-{suffix}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}

fn create(path: &Path, bytes: &[u8], permissions: Option<fs::Permissions>) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    let result = (|| {
        if let Some(p) = permissions {
            file.set_permissions(p).map_err(|e| e.to_string())?;
        }
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())
    })();
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn replace(path: &Path, bytes: &[u8], permissions: Option<fs::Permissions>) -> Result<(), String> {
    let staged = unique(path, "astrolabe-stage");
    create(&staged, bytes, permissions)?;
    let result = fs::rename(&staged, path).map_err(|e| e.to_string());
    if result.is_err() {
        let _ = fs::remove_file(staged);
    }
    result
}

fn apply_plan(config: &Path, receipt: &Path, plan: &Plan) -> Result<Option<PathBuf>, String> {
    let parent = config.parent().ok_or("config needs a parent folder")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if read_optional(config)? != plan.original {
        return Err("configuration changed during setup; retry. Nothing replaced".into());
    }
    let permissions = fs::metadata(config).ok().map(|m| m.permissions());
    if permissions.as_ref().is_some_and(fs::Permissions::readonly) {
        return Err("configuration is read-only; nothing changed".into());
    }
    let backup = if let Some(original) = &plan.original {
        let backup = unique(config, "astrolabe-backup");
        create(&backup, original.as_bytes(), permissions.clone())?;
        Some(backup)
    } else {
        None
    };
    // Record both acceptable versions first so an interrupted config write can be retried.
    let json = serde_json::to_vec_pretty(&plan.receipt).map_err(|e| e.to_string())?;
    replace(receipt, &json, None)?;
    if read_optional(config)? != plan.original {
        return Err("configuration changed during setup; retry. Nothing replaced".into());
    }
    replace(config, plan.updated.as_bytes(), permissions)?;
    Ok(backup)
}

fn marker(dir: &Path) -> Result<PathBuf, String> {
    let path = unique(&dir.join("astrolabe-setup-check"), "probe");
    create(&path, b"Astrolabe permission check\n", None)?;
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

fn probe(notes: &Path, git: Option<&Path>) -> Result<String, String> {
    fs::create_dir_all(notes.join(".handoffs")).map_err(|e| format!("handoff folder: {e}"))?;
    marker(&notes.join(".handoffs"))?;
    let mut text = "Handoff write passed; the probe file was removed.".to_owned();
    if let Some(git) = git {
        let path = unique(&git.join("astrolabe-setup-check"), "probe");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                drop(file);
                fs::remove_file(&path).map_err(|e| format!("remove Git probe: {e}"))?;
                return Err(
                    "Git metadata was writable; the profile did not retain the expected protection"
                        .into(),
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                text.push_str(" Git metadata stayed protected.")
            }
            Err(e) => return Err(format!("couldn't check Git metadata protection: {e}")),
        }
    } else {
        text.push_str(" No project Git metadata was checked; run --check from a project checkout.");
    }
    Ok(text)
}

fn check_saved(config: &Path, notes: &Path) -> Result<String, String> {
    // Pass only the settings needed by this named profile. --config can point at a staged
    // setup for verification without replacing the owner's real configuration or trust.
    let doc = parse(&read_optional(config)?.ok_or("no saved configuration")?)?;
    let permissions = doc
        .get("permissions")
        .ok_or("no saved permission profiles")?;
    // The profile is checked through Codex's real config parser and native sandbox.
    let inline = table_inline(permissions)?;
    let git = std::process::Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let output = std::process::Command::new("codex")
        .args([
            "sandbox",
            "--include-managed-config",
            "-P",
            PROFILE,
            "-c",
            &format!("permissions={inline}"),
            "--",
        ])
        .arg(exe)
        .args(["setup", "codex", "--probe"])
        .arg(notes)
        .arg(git)
        .output()
        .map_err(|e| format!("couldn't run the Codex sandbox check: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "sandbox check failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(format!(
        "{} Saved approval routing is configured; this probe does not exercise an approval request or the desktop app's selected mode.",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

fn table_inline(item: &Item) -> Result<String, String> {
    let table = item
        .as_table()
        .ok_or("permission profiles must be tables")?;
    Ok(table.clone().into_inline_table().to_string())
}

// Resolve missing destinations through their nearest existing ancestor, so /tmp and
// /private/tmp name the same configuration before and after its first write.
fn destination_name(path: &Path) -> Result<String, String> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or("configuration has no filename")?,
        );
        ancestor = ancestor.parent().ok_or("configuration has no parent")?;
    }
    let mut resolved = ancestor.canonicalize().map_err(|e| e.to_string())?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(crate::registry::native(&resolved))
}

fn doc_contains_legacy(text: &str) -> Result<bool, String> {
    let doc = parse(text)?;
    Ok(doc.contains_key("sandbox_mode") || doc.contains_key("sandbox_workspace_write"))
}
