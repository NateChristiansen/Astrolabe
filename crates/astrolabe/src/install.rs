//! Install an already downloaded, verified executable without touching notes or app settings.
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

const RECEIPT: &str = ".astrolabe-install.json";
const PENDING: &str = ".astrolabe-install-pending";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize, Serialize)]
struct Receipt {
    format: u8,
    version: String,
    sha256: String,
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
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match *word {
            "--dir" => directory = Some(PathBuf::from(words.next().ok_or("--dir needs a folder")?)),
            _ => return Err("usage: astrolabe install [--dir <folder>]".into()),
        }
    }
    let directory = match directory {
        Some(path) => path,
        None => default_directory()?,
    };
    let result = install(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        &directory,
    )?;
    Ok(format!("{result}{}", path_warning(&directory)))
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
            Ok(format!(
                "Installed Astrolabe {VERSION} in {}.\nPrevious files: {}{}{kept}\nNext: install or refresh the matching plugin, review its hooks, and start a fresh session. Notes and app settings were not changed.",
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
}
