//! Install an already downloaded, verified executable without touching notes or app settings.
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

const RECEIPT: &str = ".astrolabe-install.json";
const PENDING: &str = ".astrolabe-install-pending";
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set by CI for every published build; a local `cargo build` has none.
const COMMIT: Option<&str> = option_env!("ASTROLABE_COMMIT");

#[derive(Deserialize, Serialize)]
struct Receipt {
    format: u8,
    version: String,
    sha256: String,
    /// The commit a published build came from. Receipts before this field have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
}

/// The installed program and its receipt.
fn names() -> [&'static str; 2] {
    if cfg!(windows) {
        ["astrolabe.exe", RECEIPT]
    } else {
        ["astrolabe", RECEIPT]
    }
}

/// The copy that installers up to 0.5.0 also wrote. A later install retires it when the
/// receipt shows it is the installer's own.
fn retired() -> &'static str {
    if cfg!(windows) {
        "astrolabe-trial.exe"
    } else {
        "astrolabe-trial"
    }
}

fn digest(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn regular(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(true),
        Ok(_) => Err(format!(
            "{} is not a regular file; move it aside yourself first",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("inspect {}: {e}", path.display())),
    }
}

pub fn command(args: &[&str]) -> Result<String, String> {
    let mut directory = None;
    let mut force = false;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match *word {
            "--dir" => directory = Some(PathBuf::from(words.next().ok_or("--dir needs a folder")?)),
            "--force" => force = true,
            _ => return Err("usage: astrolabe install [--dir <folder>] [--force]".into()),
        }
    }
    let directory = match directory {
        Some(path) => path,
        None => default_directory()?,
    };
    let source = std::env::current_exe().map_err(|e| e.to_string())?;
    if COMMIT.is_none()
        && !force
        && let Some(published) = published_install(&directory)
    {
        return Err(format!(
            "{} holds {published}, and {} is a local build, which may be older. Run `astrolabe update` for the newest published build, or rebuild and pass --force to install this one.",
            directory.join(names()[0]).display(),
            source.display()
        ));
    }
    let result = install(&source, &directory)?;
    Ok(format!("{result}{}", path_warning(&directory)))
}

/// How the installed program reads when CI built it: its receipt's commit, or for a
/// receipt from before that field, a nightly's own version line. None for a local build,
/// a release receipted before the field, or no installation.
fn published_install(directory: &Path) -> Option<String> {
    let receipt = read_receipt(directory).ok()??;
    if let Some(commit) = receipt.commit {
        return Some(format!(
            "the published build {}",
            &commit[..commit.len().min(7)]
        ));
    }
    let out = std::process::Command::new(directory.join(names()[0]))
        .arg("--version")
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    line.contains("(nightly ")
        .then(|| line.trim_start_matches("astrolabe ").to_owned())
}

pub(crate) fn default_directory() -> Result<PathBuf, String> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|v| !v.is_empty())
        .ok_or("cannot find your home folder")?;
    Ok(PathBuf::from(home).join(".local/bin"))
}

// Only compare PATH with the final destination, never the download's temporary stage.
fn path_warning(directory: &Path) -> String {
    let Some(path) = std::env::var_os("PATH") else {
        return String::new();
    };
    let installed = directory.join(names()[0]);
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    } else {
        vec![String::new()]
    };
    let found = std::env::split_paths(&path).find_map(|dir| {
        extensions.iter().find_map(|ext| {
            let file = dir.join(format!("astrolabe{ext}"));
            let metadata = fs::metadata(&file).ok()?;
            if !metadata.is_file() {
                return None;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            Some(file)
        })
    });
    match found {
        Some(found) if fs::canonicalize(&found).ok() != fs::canonicalize(&installed).ok() => {
            format!(
                "\nPATH finds {} before the installed program. Run {} directly or adjust PATH to use the installed copy.",
                found.display(),
                installed.display()
            )
        }
        _ => String::new(),
    }
}

fn check_pending(directory: &Path) -> Result<(), String> {
    let pending = directory.join(PENDING);
    if fs::symlink_metadata(&pending).is_ok() {
        return Err(format!(
            "an installation was interrupted; recover using {} before retrying",
            pending.display()
        ));
    }
    Ok(())
}

fn read_receipt(directory: &Path) -> Result<Option<Receipt>, String> {
    if regular(&directory.join(RECEIPT))? {
        let bytes = fs::read(directory.join(RECEIPT)).map_err(|e| e.to_string())?;
        let receipt: Receipt = serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid installation receipt: {e}"))?;
        if receipt.format != 1 {
            return Err("unknown installation receipt format".into());
        }
        Ok(Some(receipt))
    } else {
        Ok(None)
    }
}

fn verify_program(directory: &Path, receipt: &Option<Receipt>) -> Result<bool, String> {
    let exists = regular(&directory.join(names()[0]))?;
    if exists {
        let path = directory.join(names()[0]);
        match receipt {
            Some(receipt) if digest(&path)? != receipt.sha256 => {
                return Err(format!(
                    "{} changed outside this installer; move it aside yourself before updating",
                    path.display()
                ));
            }
            Some(_) => {}
            None => {
                return Err(format!(
                    "{} already exists without an installation receipt; move it aside yourself before installing",
                    path.display()
                ));
            }
        }
    }
    Ok(exists)
}

/// Read-only ownership check for update routing. An unreceipted build is not an install;
/// a damaged or interrupted installation is an error, never a reason to select another.
pub(crate) fn installed_program(directory: &Path) -> Result<Option<PathBuf>, String> {
    check_pending(directory)?;
    let receipt = read_receipt(directory)?;
    if receipt.is_none() {
        return Ok(None);
    }
    if !verify_program(directory, &receipt)? {
        return Err(format!(
            "{} has an installation receipt but its program is missing; repair the installation before updating",
            directory.display()
        ));
    }
    Ok(Some(
        fs::canonicalize(directory)
            .map_err(|e| e.to_string())?
            .join(names()[0]),
    ))
}

/// The installed program's hash, taken before an update so a failure can be compared.
pub(crate) fn program_digest(directory: &Path) -> Option<String> {
    digest(&directory.join(names()[0])).ok()
}

/// What a failed installer run left in `directory`, where the program hashed to `before`.
pub(crate) fn left_after_failure(directory: &Path, before: Option<&str>) -> String {
    let pending = directory.join(PENDING);
    let waiting = fs::symlink_metadata(&pending).is_ok();
    let now = program_digest(directory);
    if now.is_some() && now.as_deref() == before {
        return if waiting {
            format!(
                "the program file is unchanged, but the attempt was not fully undone; recover as {} says before retrying",
                pending.display()
            )
        } else {
            "the previous program is still in place".into()
        };
    }
    if waiting {
        let receipted = read_receipt(directory).ok().flatten().map(|r| r.sha256);
        return if now.is_some() && now == receipted {
            format!(
                "the new program is in place, but {} couldn't be removed; remove that file, then run `astrolabe update` again for the plugin",
                pending.display()
            )
        } else {
            format!(
                "the previous program was not restored; recover it as {} says before retrying",
                pending.display()
            )
        };
    }
    match now {
        Some(_) => format!(
            "the program in {} is not the one that was there before; check `astrolabe --version` before retrying",
            directory.display()
        ),
        None => format!(
            "there is no program in {} now; run the installer from the install guide again",
            directory.display()
        ),
    }
}

/// Remove this installer's backup folders other than `keep`, the newest, so one rollback
/// copy stays. Nothing goes while a marker says a recovery is pending, since it names a
/// backup to restore from. Returns how many were removed.
fn prune(directory: &Path, keep: &Path) -> usize {
    if fs::symlink_metadata(directory.join(PENDING)).is_ok() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            // Only a name this installer gives: the prefix, then its twelve random bytes
            // in hex. A folder someone else put under the prefix is theirs.
            let ours = entry
                .file_name()
                .to_string_lossy()
                .strip_prefix(".astrolabe-backup-")
                .is_some_and(|id| {
                    id.len() == 24 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                });
            ours && entry.path() != keep
                // A real folder only: this doesn't follow a link, which is never removed.
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
        })
        .filter(|entry| fs::remove_dir_all(entry.path()).is_ok())
        .count()
}

/// Only files owned by this installer are replaced; anything else is left for the user to
/// move aside.
pub fn install(source: &Path, directory: &Path) -> Result<String, String> {
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let directory = fs::canonicalize(directory).map_err(|e| e.to_string())?;
    let lock_path = directory.join(".astrolabe-install.lock");
    // Do not follow a pre-existing symlink for the lock or any managed file.
    regular(&lock_path)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| e.to_string())?;
    if !FileExt::try_lock_exclusive(&lock).map_err(|e| e.to_string())? {
        return Err("another Astrolabe installer is running".into());
    }
    check_pending(&directory)?;
    let pending = directory.join(PENDING);
    let files = names();
    let receipt = read_receipt(&directory)?;
    let existed = [verify_program(&directory, &receipt)?, receipt.is_some()];
    // The old compatibility copy goes only when it is the receipted build; anything else
    // under that name is not this installer's to touch.
    let old_copy = directory.join(retired());
    let old_copy_present = fs::symlink_metadata(&old_copy).is_ok();
    let retire = match &receipt {
        Some(receipt) if old_copy_present && regular(&old_copy).unwrap_or(false) => {
            digest(&old_copy)? == receipt.sha256
        }
        _ => false,
    };
    let kept = if old_copy_present && !retire {
        format!(
            "\n{} was left in place: it isn't this installer's copy. Nothing calls it now; remove it yourself once you've checked what it is.",
            old_copy.display()
        )
    } else {
        String::new()
    };
    let checksum = digest(source)?;
    if receipt.as_ref().is_some_and(|r| r.sha256 == checksum) && existed[0] && !retire {
        return Ok(format!(
            "Astrolabe {VERSION} is already installed in {}. Refresh the matching plugin separately.{kept}",
            directory.display()
        ));
    }

    let mut random = [0u8; 12];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let backup = directory.join(format!(".astrolabe-backup-{id}"));
    fs::create_dir(&backup).map_err(|e| e.to_string())?;
    let stage = backup.join("new");
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let dest = stage.join(files[0]);
    fs::copy(source, &dest).map_err(|e| e.to_string())?;
    if digest(&dest)? != checksum {
        return Err("staged executable checksum mismatch".into());
    }
    let receipt = Receipt {
        format: 1,
        version: VERSION.into(),
        sha256: checksum,
        commit: COMMIT.map(str::to_owned),
    };
    fs::write(
        stage.join(RECEIPT),
        serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // The marker and the original files remain available for recovery after a hard stop.
    let mut moved_out: Vec<&str> = files
        .iter()
        .zip(existed)
        .filter(|(_, present)| *present)
        .map(|(name, _)| *name)
        .collect();
    if retire {
        moved_out.push(retired());
    }
    fs::write(&pending, format!("Backup: {}\nOriginal files moved there: {moved_out:?}\nClose Astrolabe sessions before recovery. See docs/releases.md.\n", backup.display()))
        .map_err(|e| e.to_string())?;
    let result = replace(&directory, &backup, &stage, &moved_out, &files);
    match result {
        Ok(()) => {
            fs::remove_file(&pending).map_err(|e| {
                format!("installed, but could not remove {}: {e}", pending.display())
            })?;
            let pruned = match prune(&directory, &backup) {
                0 => String::new(),
                1 => "\nRemoved 1 older backup folder.".into(),
                n => format!("\nRemoved {n} older backup folders."),
            };
            Ok(format!(
                "Installed Astrolabe {VERSION} in {}.\nPrevious files: {}{}{pruned}{kept}\nNext: install or refresh the matching plugin, review its hooks, and start a fresh session. Notes and app settings were not changed.",
                directory.display(),
                backup.display(),
                if retire {
                    format!("\nRetired the old {} copy into that folder.", retired())
                } else {
                    String::new()
                }
            ))
        }
        Err((why, recovered)) => {
            if recovered {
                let _ = fs::remove_file(&pending);
            }
            Err(format!(
                "installation failed: {why}. Close Astrolabe sessions and its board process before retrying. {}",
                if recovered {
                    "The previous installation was restored.".into()
                } else {
                    format!(
                        "Recovery files are in {}; see {}.",
                        backup.display(),
                        pending.display()
                    )
                }
            ))
        }
    }
}

/// Move `out` into the backup, then the staged `files` into place; undo both on failure.
fn replace(
    directory: &Path,
    backup: &Path,
    stage: &Path,
    out: &[&str],
    files: &[&str],
) -> Result<(), (String, bool)> {
    let mut saved = Vec::new();
    let mut installed = Vec::new();
    let operation = (|| -> std::io::Result<()> {
        for name in out {
            fs::rename(directory.join(name), backup.join(name))?;
            saved.push(*name);
        }
        for name in files {
            fs::rename(stage.join(name), directory.join(name))?;
            installed.push(*name);
        }
        Ok(())
    })();
    if let Err(error) = operation {
        let mut recovered = true;
        for name in installed.iter().rev() {
            recovered &= fs::remove_file(directory.join(name)).is_ok();
        }
        for name in saved.iter().rev() {
            recovered &= fs::rename(backup.join(name), directory.join(name)).is_ok();
        }
        return Err((error.to_string(), recovered));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_after_replacing_binaries_restores_every_original() {
        let root =
            std::env::temp_dir().join(format!("astrolabe-install-rollback-{}", std::process::id()));
        fs::create_dir_all(root.join("backup/new")).unwrap();
        let files = names();
        let out = [files[0], files[1], retired()];
        for name in out {
            fs::write(root.join(name), format!("old {name}")).unwrap();
        }
        fs::write(root.join("backup/new").join(files[0]), "replacement").unwrap();
        // No staged receipt: fail after the replacement binary has moved.
        let result = replace(
            &root,
            &root.join("backup"),
            &root.join("backup/new"),
            &out,
            &files,
        );
        assert!(matches!(result, Err((_, true))));
        for name in out {
            assert_eq!(
                fs::read_to_string(root.join(name)).unwrap(),
                format!("old {name}")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("astrolabe-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// Install again into a folder an earlier install used. A process another test is
    /// starting holds a copy of that install's lock handle for a moment, so wait it out.
    fn reinstall(build: &Path, directory: &Path) -> String {
        for _ in 0..200 {
            match install(build, directory) {
                Err(why) if why.contains("another Astrolabe installer") => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                result => return result.unwrap(),
            }
        }
        panic!("the install lock was never released");
    }

    fn backups(directory: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(directory)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(".astrolabe-backup-"))
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn an_install_keeps_only_the_newest_backup() {
        let root = fs::canonicalize(scratch("prune")).unwrap();
        let directory = root.join("bin");
        let build = root.join("build");
        let mut last = String::new();
        for round in 1..=3 {
            fs::write(&build, format!("build {round}")).unwrap();
            last = reinstall(&build, &directory);
        }
        let kept = backups(&directory);
        assert_eq!(kept.len(), 1, "{kept:?}");
        // The one kept holds the build the last install replaced, and the output names it.
        assert_eq!(
            fs::read_to_string(kept[0].join(names()[0])).unwrap(),
            "build 2"
        );
        assert!(
            last.contains(&format!("Previous files: {}", kept[0].display())),
            "{last}"
        );
        assert!(last.contains("Removed 1 older backup folder."), "{last}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_prune_leaves_everything_while_recovery_is_pending_and_other_files_always() {
        let root = scratch("prune-pending");
        // The shape this installer gives a backup: twelve random bytes in hex.
        let old = root.join(".astrolabe-backup-0123456789abcdef01234567");
        let named = root.join(".astrolabe-backup-89abcdef0123456789abcdef");
        let newest = root.join(".astrolabe-backup-fedcba9876543210fedcba98");
        for folder in [&old, &named, &newest] {
            fs::create_dir(folder).unwrap();
            fs::write(folder.join(names()[0]), "a program").unwrap();
        }
        // Not this installer's: a file under the backup prefix, another folder, and
        // folders someone made under the prefix with names the installer never gives.
        fs::write(root.join(".astrolabe-backup-file"), "kept").unwrap();
        fs::create_dir(root.join("unrelated")).unwrap();
        let manual = root.join(".astrolabe-backup-manual");
        let longer = root.join(".astrolabe-backup-0123456789abcdef01234567-mine");
        let upper = root.join(".astrolabe-backup-0123456789ABCDEF0123456Z");
        for folder in [&manual, &longer, &upper] {
            fs::create_dir(folder).unwrap();
            fs::write(folder.join("precious"), "kept").unwrap();
        }
        // A link under an installer-shaped name is never followed or removed.
        #[cfg(unix)]
        let link = root.join(".astrolabe-backup-00112233445566778899aabb");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&manual, &link).unwrap();
        fs::write(root.join(PENDING), format!("Backup: {}\n", named.display())).unwrap();
        assert_eq!(prune(&root, &newest), 0);
        assert!(
            old.is_dir() && named.is_dir() && newest.is_dir(),
            "nothing goes while a marker exists"
        );

        fs::remove_file(root.join(PENDING)).unwrap();
        assert_eq!(prune(&root, &newest), 2);
        assert!(newest.is_dir() && !old.exists() && !named.exists());
        assert!(root.join(".astrolabe-backup-file").is_file());
        assert!(root.join("unrelated").is_dir());
        for folder in [&manual, &longer, &upper] {
            assert_eq!(
                fs::read_to_string(folder.join("precious")).unwrap(),
                "kept",
                "{}",
                folder.display()
            );
        }
        #[cfg(unix)]
        assert!(fs::symlink_metadata(&link).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_install_is_described_by_what_it_left() {
        let root = scratch("left");
        let directory = root.join("bin");
        let build = root.join("build");
        fs::write(&build, "build 1").unwrap();
        install(&build, &directory).unwrap();
        let before = program_digest(&directory);
        let before = before.as_deref();
        let program = directory.join(names()[0]);
        let pending = directory.join(PENDING);

        assert_eq!(
            left_after_failure(&directory, before),
            "the previous program is still in place"
        );

        // A rollback that stopped partway: the marker stays even though the program does.
        fs::write(&pending, "Backup: somewhere\n").unwrap();
        let left = left_after_failure(&directory, before);
        assert!(
            left.contains("not fully undone") && left.contains(&pending.display().to_string()),
            "{left}"
        );
        assert!(!left.contains("still in place"), "{left}");

        // A rollback that failed: something else sits where the program was.
        fs::write(&program, "half an install").unwrap();
        let left = left_after_failure(&directory, before);
        assert!(
            left.contains("was not restored") && left.contains(&pending.display().to_string()),
            "{left}"
        );
        fs::write(&program, "build 1").unwrap();

        // The new program went in and only the marker's removal failed.
        fs::remove_file(&pending).unwrap();
        fs::write(&build, "build 2").unwrap();
        reinstall(&build, &directory);
        fs::write(&pending, "Backup: somewhere\n").unwrap();
        let left = left_after_failure(&directory, before);
        assert!(
            left.contains("the new program is in place") && left.contains("remove that file"),
            "{left}"
        );

        // Nothing where the program was.
        fs::remove_file(&pending).unwrap();
        fs::remove_file(&program).unwrap();
        let left = left_after_failure(&directory, before);
        assert!(left.contains("no program"), "{left}");
        fs::remove_dir_all(root).unwrap();
    }
}
