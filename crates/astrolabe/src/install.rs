//! Install an already downloaded, verified executable without touching notes or app settings.
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RECEIPT: &str = ".astrolabe-install.json";
const PENDING: &str = ".astrolabe-install-pending";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize, Serialize)]
struct Receipt {
    format: u8,
    version: String,
    sha256: String,
}

fn names() -> [&'static str; 3] {
    if cfg!(windows) {
        ["astrolabe.exe", "astrolabe-trial.exe", RECEIPT]
    } else {
        ["astrolabe", "astrolabe-trial", RECEIPT]
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
    let mut migrate = false;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match *word {
            "--migrate-trial" => migrate = true,
            "--dir" => directory = Some(PathBuf::from(words.next().ok_or("--dir needs a folder")?)),
            _ => return Err("usage: astrolabe install [--migrate-trial] [--dir <folder>]".into()),
        }
    }
    let directory = match directory {
        Some(path) => path,
        None => {
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .filter(|v| !v.is_empty())
                .ok_or("cannot find your home folder")?;
            PathBuf::from(home).join(".local/bin")
        }
    };
    install(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        &directory,
        migrate,
    )
}

/// Both command names contain the same bytes. A compatibility copy also works on Windows
/// filesystems without symlink privileges. Only files owned by this installer are updated
/// automatically; adopting an older, unreceipted build requires an explicit migration.
pub fn install(source: &Path, directory: &Path, migrate: bool) -> Result<String, String> {
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
    let pending = directory.join(PENDING);
    if fs::symlink_metadata(&pending).is_ok() {
        return Err(format!(
            "an installation was interrupted; recover using {} before retrying",
            pending.display()
        ));
    }
    let files = names();
    let mut existed = [false; 3];
    for (i, name) in files.iter().enumerate() {
        existed[i] = regular(&directory.join(name))?;
    }
    let receipt = if existed[2] {
        let bytes = fs::read(directory.join(RECEIPT)).map_err(|e| e.to_string())?;
        let receipt: Receipt = serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid installation receipt: {e}"))?;
        if receipt.format != 1 {
            return Err("unknown installation receipt format".into());
        }
        Some(receipt)
    } else {
        None
    };
    for (i, name) in files[..2].iter().enumerate() {
        if !existed[i] {
            continue;
        }
        let path = directory.join(name);
        if let Some(receipt) = &receipt {
            if digest(&path)? != receipt.sha256 {
                return Err(format!(
                    "{} changed outside this installer; move it aside yourself before updating",
                    path.display()
                ));
            }
        } else if !migrate {
            return Err(format!(
                "{} already exists without an installation receipt; use --migrate-trial to adopt an existing Astrolabe build",
                path.display()
            ));
        } else {
            // This runs only after the user explicitly chose to adopt these local programs.
            let output = Command::new(&path)
                .arg("--version")
                .stdin(Stdio::null())
                .output()
                .map_err(|e| format!("check {}: {e}", path.display()))?;
            let text = String::from_utf8_lossy(&output.stdout);
            let version = text.trim().strip_prefix("astrolabe ").unwrap_or("");
            let parts: Vec<_> = version.split('.').collect();
            if !output.status.success()
                || parts.len() != 3
                || !parts
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err(format!(
                    "{} did not identify itself as Astrolabe; it was not replaced",
                    path.display()
                ));
            }
        }
    }
    let checksum = digest(source)?;
    if receipt.as_ref().is_some_and(|r| r.sha256 == checksum) && existed.iter().all(|v| *v) {
        return Ok(format!(
            "Astrolabe {VERSION} is already installed in {}. Refresh the matching plugin separately.",
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
    for name in &files[..2] {
        let dest = stage.join(name);
        fs::copy(source, &dest).map_err(|e| e.to_string())?;
        if digest(&dest)? != checksum {
            return Err("staged executable checksum mismatch".into());
        }
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
    fs::write(&pending, format!("Backup: {}\nOriginal files present: {existed:?}\nClose Astrolabe sessions before recovery. See docs/releases.md.\n", backup.display()))
        .map_err(|e| e.to_string())?;
    let result = replace(&directory, &backup, &stage, &files, existed);
    match result {
        Ok(()) => {
            fs::remove_file(&pending).map_err(|e| {
                format!("installed, but could not remove {}: {e}", pending.display())
            })?;
            Ok(format!(
                "Installed Astrolabe {VERSION} in {}.\nPrevious files: {}\nThe astrolabe-trial compatibility name uses the same build.\nNext: install or refresh the matching plugin, review its hooks, and start a fresh session. Notes and app settings were not changed.",
                directory.display(),
                backup.display()
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

fn replace(
    directory: &Path,
    backup: &Path,
    stage: &Path,
    files: &[&str; 3],
    existed: [bool; 3],
) -> Result<(), (String, bool)> {
    let mut saved = Vec::new();
    let mut installed = Vec::new();
    let operation = (|| -> std::io::Result<()> {
        for (i, name) in files.iter().enumerate() {
            if existed[i] {
                fs::rename(directory.join(name), backup.join(name))?;
                saved.push(*name);
            }
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
        for name in files {
            fs::write(root.join(name), format!("old {name}")).unwrap();
        }
        for name in &files[..2] {
            fs::write(root.join("backup/new").join(name), "replacement").unwrap();
        }
        // No staged receipt: fail after both replacement binaries have moved.
        let result = replace(
            &root,
            &root.join("backup"),
            &root.join("backup/new"),
            &files,
            [true; 3],
        );
        assert!(matches!(result, Err((_, true))));
        for name in files {
            assert_eq!(
                fs::read_to_string(root.join(name)).unwrap(),
                format!("old {name}")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
