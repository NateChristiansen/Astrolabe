//! Repository registration and matching by checkout or portable remote.
use super::state_dir;
use super::tickets::write_json;
use crate::effort::frontmatter;
use crate::hook::comparable;
use crate::registry::AGENT;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// The notes checkout and local repos by name, in the state folder shared by hooks and
/// terminal commands.
/// One file per repo, `repos/<name>.json`: each add replaces only its own record, so two at
/// once can't lose each other's.
const BOM: char = '\u{feff}';

pub(super) const REPOS: &str = "repos";

pub(crate) struct Repos(pub(crate) Map<String, Value>);

impl Repos {
    /// None written yet is no repos. A record that can't be read is an error, never skipped.
    pub(super) fn read(dir: &Path) -> Result<Self, String> {
        let folder = dir.join(REPOS);
        let entries = match std::fs::read_dir(&folder) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self(Map::new())),
            Err(e) => return Err(format!("{}: {e}", folder.display())),
            Ok(entries) => entries,
        };
        let mut map = Map::new();
        for entry in entries {
            let path = entry
                .map_err(|e| format!("{}: {e}", folder.display()))?
                .path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let bad = || format!("{} isn't a repo record this program wrote", path.display());
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let record: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
            let (Some(name), Some(checkout)) =
                (record["name"].as_str(), record["checkout"].as_str())
            else {
                return Err(bad());
            };
            map.insert(name.to_owned(), checkout.into());
        }
        Ok(Self(map))
    }

    pub(crate) fn checkout(&self, name: &str) -> Option<PathBuf> {
        self.0.get(name)?.as_str().map(PathBuf::from)
    }
}

/// `astrolabe repo add` for local bindings. This record is the plugin's authority.
pub fn add_repo(name: &str, checkout: &Path) -> Result<(), String> {
    let dir = state_dir().ok_or("Astrolabe has no home folder for its state")?;
    check_repo_name(name)?;
    let record = json!({"name": name, "checkout": checkout.display().to_string()});
    write_json(&dir.join(REPOS).join(format!("{name}.json")), &record)
}

/// A repo name is a file name in the plugin's records, so checked before anything is written.
pub fn check_repo_name(name: &str) -> Result<(), String> {
    let named = !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if name.is_empty() || !named {
        return Err(format!(
            "repo name {name:?} must be letters, digits, '-', '_' or '.'"
        ));
    }
    Ok(())
}

/// The notes checkout as the plugin's own records name it, checked to be a git checkout.
pub(super) fn notes_checked() -> Result<PathBuf, String> {
    let (_, notes) = notes_and_repos()?;
    common_dir(&notes).map(|_| notes.clone()).ok_or_else(|| {
        format!("the notes checkout on record, {}, isn't a git checkout here; run `astrolabe repo add agent <notes checkout>`", notes.display())
    })
}

pub(crate) fn common_dir(dir: &Path) -> Option<String> {
    let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"].map(AsRef::as_ref);
    let out = crate::worktree::git(dir, &args).ok()?;
    Some(comparable(Path::new(out.trim())))
}

/// A git remote in one form for comparing, so the scp, ssh and https forms of one hosted repo
/// match: no scheme or `user@`, `host:path` as `host/path`, no trailing `.git` or `/`, the
/// host lowercased and the path's case kept. An scp path from the root (`host:/srv/game`)
/// stays apart from one from the home folder (`host:srv/game`), as an ssh server can serve
/// different repos at the two. None for anything that isn't a network remote: a local path
/// or a `file:` URL names a folder on one machine only, so that repo is registered instead.
pub(super) fn normalize_remote(url: &str) -> Option<String> {
    const SCHEMES: [&str; 6] = ["ssh", "git", "http", "https", "git+ssh", "ssh+git"];
    let url = url.trim();
    if url.is_empty() || url.contains(char::is_whitespace) || url.contains('\\') {
        return None;
    }
    let (host, path, rooted) = match url.split_once("://") {
        Some((scheme, rest)) => {
            if !SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
                return None;
            }
            let (host, path) = rest.split_once('/')?;
            (host, path, false)
        }
        // `host:path`, unless the part before the colon is a folder or a Windows drive.
        None => {
            let (host, path) = url.split_once(':').filter(|(h, _)| !h.contains('/'))?;
            (host, path, path.starts_with('/'))
        }
    };
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let drive = host.len() == 1 && host.chars().all(|c| c.is_ascii_alphabetic());
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if host.is_empty() || drive || path.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some(if rooted {
        format!("{host}:/{path}")
    } else {
        format!("{host}/{path}")
    })
}

/// The URL each of this folder's remotes fetches from, by name: `git remote -v` adds a
/// partial clone's filter after its URL, so its lines aren't read. Only the first
/// `remote.<name>.url`, as git fetches from that one alone and pushes to the rest, so a
/// push-only URL can't make this folder another repo. A remote with no fetch URL (only a
/// `pushurl`) is skipped, as there's nothing it fetches; some git versions then give the
/// remote's own name, which never reads as a remote URL. A worktree shares its checkout's
/// config, so it has the same ones.
pub(super) fn folder_remotes(dir: &Path) -> Result<Vec<String>, String> {
    let fail = |why: String| format!("the remotes of {} can't be read: {why}", dir.display());
    let names = crate::worktree::git(dir, &["remote".as_ref()]).map_err(fail)?;
    let mut urls: Vec<String> = Vec::new();
    for name in names.lines().map(str::trim).filter(|n| !n.is_empty()) {
        let args = ["remote", "get-url", name].map(AsRef::as_ref);
        let Ok(out) = crate::worktree::git(dir, &args) else {
            continue;
        };
        let url = out.trim();
        if !url.is_empty() && !urls.iter().any(|u| u == url) {
            urls.push(url.to_owned());
        }
    }
    Ok(urls)
}

/// The remotes a repo note lists under `remote:`, as written. One that can't be read refuses,
/// naming the file, because skipping it could match a folder to the wrong repo or to none.
pub(super) fn parse_remotes(file: &str, text: &str) -> Result<Vec<String>, String> {
    let text = text.replace("\r\n", "\n");
    let props = frontmatter(&text);
    let Some((_, value)) = props.iter().find(|(k, _)| k == "remote") else {
        return Ok(Vec::new());
    };
    let bad = |what: String| {
        format!(
            "{file}: {what}; the owner fixes it in the note's properties, as in `remote: git@example.com:team/game.git` or a list of them, because a remote that can't be read could match this repo to the wrong folder or to none"
        )
    };
    if value.is_empty() {
        return Err(bad("`remote` has no URL".to_owned()));
    }
    let urls: Vec<String> = value.split(',').map(|u| u.trim().to_owned()).collect();
    match urls.iter().find(|u| normalize_remote(u).is_none()) {
        Some(url) => Err(bad(format!("remote {url:?} isn't a git remote URL"))),
        None => Ok(urls),
    }
}

/// `knowledge/repos/<name>.md`'s remotes. No note is no remotes.
pub(crate) fn note_remotes(notes: &Path, name: &str) -> Result<Vec<String>, String> {
    let file = format!("knowledge/repos/{name}.md");
    match std::fs::read_to_string(notes.join(&file)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{file}: {e}")),
        Ok(text) => parse_remotes(&file, &text),
    }
}

/// Every repo note that lists a remote: its name (the file's stem) and its remotes, normalized.
pub(super) fn remote_notes(notes: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let dir = notes.join("knowledge/repos");
    let entries = match std::fs::read_dir(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
        Ok(entries) => entries,
    };
    let mut found = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let (Some(stem), Some("md")) = (
            path.file_stem().and_then(|s| s.to_str()),
            path.extension().and_then(|e| e.to_str()),
        ) else {
            continue;
        };
        let file = format!("knowledge/repos/{stem}.md");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
        let remotes = parse_remotes(&file, &text)?;
        if remotes.is_empty() {
            continue;
        }
        check_repo_name(stem).map_err(|why| {
            format!("{file} lists a `remote:`, but its file name is the repo's name and {why}, so effort notes couldn't name it in `repo:`; the owner renames the note")
        })?;
        let remotes = remotes.iter().filter_map(|r| normalize_remote(r)).collect();
        found.push((stem.to_owned(), remotes));
    }
    Ok(found)
}

/// The repo names this folder is, as a checkout or a worktree: those registered on this
/// machine for its git repo (`here`, its common directory), and those whose repo note lists
/// one of its remotes. A name can be both, even for different folders; this folder is every
/// name it matches.
pub(crate) fn repo_names(
    notes: &Path,
    registry: &Repos,
    cwd: &Path,
    here: &str,
) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = (registry.0.keys())
        .filter(|name| {
            let checkout = registry.checkout(name);
            checkout.and_then(|c| common_dir(&c)).as_deref() == Some(here)
        })
        .cloned()
        .collect();
    let remotes: Vec<String> = (folder_remotes(cwd)?.iter())
        .filter_map(|r| normalize_remote(r))
        .collect();
    if !remotes.is_empty() {
        for (name, theirs) in remote_notes(notes)? {
            if theirs.iter().any(|r| remotes.contains(r)) {
                names.push(name);
            }
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// The effort's repo this session's folder is in, as a checkout or a worktree of it.
pub(super) fn session_repo(
    notes: &Path,
    registry: &Repos,
    repos: &[String],
    cwd: &Path,
) -> Result<String, String> {
    let here = common_dir(cwd).ok_or_else(|| {
        format!("this session's folder {} isn't in a git repo, so it can't be matched to the effort's repo", cwd.display())
    })?;
    // The notes checkout is the owner's seat: every effort loads there, as its first repo.
    if in_notes(cwd)
        && let Some(first) = repos.first()
    {
        return Ok(first.clone());
    }
    let names = repo_names(notes, registry, cwd, &here)?;
    if let Some(name) = repos.iter().find(|r| names.contains(r)) {
        return Ok(name.clone());
    }
    let mut unknown = Vec::new();
    for name in repos {
        if registry.checkout(name).is_none() && note_remotes(notes, name)?.is_empty() {
            unknown.push(format!("`astrolabe repo add {name}`"));
        }
    }
    let mut why = format!(
        "this session's repo isn't the effort's ({}), so it's refused; switch on from a session in that repo, or the owner adds this one to the effort's `repo`",
        repos.join(", ")
    );
    if !unknown.is_empty() {
        why += &format!(
            ". Neither registered on this machine nor given a `remote:` in its repo note, so no folder can match it: {}, run from a folder of that repo, adds it",
            unknown.join(", ")
        );
    }
    Err(why)
}

/// Where a remote goes when `repo add` writes it: `origin` if the folder has one, else the
/// first it lists. Only remotes another machine could match.
pub(super) fn chosen_remote(dir: &Path) -> Result<Option<String>, String> {
    let args = ["remote", "get-url", "origin"].map(AsRef::as_ref);
    let origin = crate::worktree::git(dir, &args).ok();
    let origin = origin.map(|o| o.trim().to_owned());
    let mut remotes = folder_remotes(dir)?;
    remotes.retain(|r| normalize_remote(r).is_some());
    Ok(match origin.filter(|o| remotes.contains(o)) {
        Some(origin) => Some(origin),
        None => remotes.into_iter().next(),
    })
}

/// `remotes` as the note's `remote:` property, put in place of the one it has, or added.
pub(super) fn with_remotes(text: &str, remotes: &[String]) -> String {
    let crlf = text.contains("\r\n");
    // Kept, and the properties after it found as `frontmatter` finds them.
    let bom = text.starts_with(BOM);
    let text = text.trim_start_matches(BOM).replace("\r\n", "\n");
    let block = match remotes {
        [one] => format!("remote: {one}\n"),
        _ => format!(
            "remote:\n{}",
            remotes
                .iter()
                .map(|r| format!("  - {r}\n"))
                .collect::<String>()
        ),
    };
    let out = match crate::effort::header_bounds(&text) {
        None => format!("---\n{block}---\n\n{text}"),
        Some(at) => {
            let lines: Vec<&str> = text[at.start..at.end].split_inclusive('\n').collect();
            let end = lines.len();
            let key = (0..end).find(|&i| {
                let line = lines[i];
                !line.starts_with(' ')
                    && line
                        .split_once(':')
                        .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case("remote"))
            });
            let (from, to) = match key {
                Some(i) => {
                    let items = lines[i + 1..end]
                        .iter()
                        .take_while(|l| l.trim_start().starts_with("- "))
                        .count();
                    (i, i + 1 + items)
                }
                None => (end, end),
            };
            // A header that never closes can end without a line break, as can an opening
            // line that is the whole note; the block needs one before it.
            let before = [&text[..at.start], &lines[..from].concat()].concat();
            let gap = if before.ends_with('\n') { "" } else { "\n" };
            [&before, gap, &block, &lines[to..].concat(), &text[at.end..]].concat()
        }
    };
    let out = if bom { format!("{BOM}{out}") } else { out };
    if crlf { out.replace('\n', "\r\n") } else { out }
}

/// `astrolabe repo add <name> [<checkout>]` by remote: when `folder` has a remote another
/// machine could match, writes it into `knowledge/repos/<name>.md` in the notes checkout,
/// making a stub note if there's none, and says what it wrote. None when the folder has no
/// such remote, so the caller registers it on this machine instead. Never commits.
pub fn add_repo_note(name: &str, folder: &Path) -> Result<Option<Vec<String>>, String> {
    check_repo_name(name)?;
    common_dir(folder).ok_or_else(|| format!("{} isn't in a git repo", folder.display()))?;
    let Some(remote) = chosen_remote(folder)? else {
        return Ok(None);
    };
    let notes = notes_checked().map_err(|why| {
        format!("{why}. It's needed because a repo with a remote is added as a note in the notes checkout, which every machine shares")
    })?;
    let file = format!("knowledge/repos/{name}.md");
    let path = notes.join(&file);
    let unsaved = "Not committed or pushed: the notes checkout is the owner's to commit.";
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(fail(e)),
        Ok(text) => Some(text),
    };
    let Some(text) = text else {
        let stub = format!(
            "---\nremote: {remote}\n---\n\n# {name}\n\nCreated by `astrolabe repo add`, so sessions in this repo match it by its remote on every machine.\n"
        );
        std::fs::create_dir_all(path.parent().unwrap_or(&notes)).map_err(fail)?;
        std::fs::write(&path, stub).map_err(fail)?;
        return Ok(Some(vec![
            format!(
                "Wrote {}, a new note for repo {name} with remote {remote}: a session in a checkout or worktree with that remote is {name} on any machine.",
                path.display()
            ),
            unsaved.to_owned(),
        ]));
    };
    let mut listed = parse_remotes(&file, &text)?;
    let ours: Vec<String> = (folder_remotes(folder)?.iter())
        .filter_map(|r| normalize_remote(r))
        .collect();
    if let Some(known) =
        (listed.iter()).find(|r| normalize_remote(r).is_some_and(|r| ours.contains(&r)))
    {
        return Ok(Some(vec![format!(
            "{} already lists {known}, a remote of this folder, so nothing was written.",
            path.display()
        )]));
    }
    let before = listed.join(", ");
    listed.push(remote.clone());
    std::fs::write(&path, with_remotes(&text, &listed)).map_err(fail)?;
    let said = if before.is_empty() {
        format!(
            "Added remote {remote} to {}, which listed none: a session with that remote is {name} on any machine.",
            path.display()
        )
    } else {
        format!(
            "Added remote {remote} to {}, which listed {before} and still does: a session with any of them is {name}.",
            path.display()
        )
    };
    Ok(Some(vec![said, unsaved.to_owned()]))
}

/// A checkout's main folder from any folder in it or its worktrees, for a registry record
/// that outlives a worktree.
pub fn main_checkout(folder: &Path) -> Result<PathBuf, String> {
    let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"].map(AsRef::as_ref);
    let common = crate::worktree::git(folder, &args)
        .map_err(|why| format!("{} isn't in a git repo: {why}", folder.display()))?;
    let common = PathBuf::from(common.trim());
    if common.file_name().is_some_and(|n| n == ".git")
        && let Some(parent) = common.parent()
    {
        return Ok(parent.to_path_buf());
    }
    let args = ["rev-parse", "--show-toplevel"].map(AsRef::as_ref);
    let top = crate::worktree::git(folder, &args)
        .map_err(|why| format!("{} has no checkout folder: {why}", folder.display()))?;
    Ok(PathBuf::from(top.trim()))
}

/// The notes checkout on record, for commands run outside a hook.
pub fn notes_root() -> Result<PathBuf, String> {
    notes_and_repos().map(|(_, notes)| notes)
}

pub(crate) fn notes_and_repos() -> Result<(Repos, PathBuf), String> {
    let dir = state_dir().ok_or("Astrolabe has no home folder for its state")?;
    let repos = Repos::read(&dir).map_err(|e| format!("this machine's Astrolabe repos: {e}"))?;
    let notes = repos.checkout(AGENT).ok_or(
        "no notes checkout is set on this machine; run `astrolabe repo add agent <notes checkout>`",
    )?;
    Ok((repos, notes))
}

/// Whether this folder is the registered notes checkout or one of its worktrees, by the
/// registry and git's common directory: the owner's steering seat, where any effort loads
/// whatever its repo and a new one names the repo its work is for.
pub(crate) fn in_notes(cwd: &Path) -> bool {
    notes_identity(cwd).unwrap_or(false)
}

/// A failed identity read is unknown, not proof that this is a product checkout.
/// Fingerprints must preserve that distinction when a hook runs out of read time.
pub(crate) fn notes_identity(cwd: &Path) -> Option<bool> {
    let Some((_, notes)) = notes_and_repos().ok() else {
        return Some(false);
    };
    Some(common_dir(cwd)? == common_dir(&notes)?)
}

#[cfg(test)]
mod remote_tests {
    use super::with_remotes;
    use crate::effort::frontmatter;

    fn listed(text: &str) -> Option<String> {
        frontmatter(text)
            .into_iter()
            .find(|(k, _)| k == "remote")
            .map(|(_, v)| v)
    }

    #[test]
    fn remotes_are_written_inside_the_header_every_reader_sees() {
        let one = ["git@example.com:a/b.git".to_owned()];
        let two = [one[0].clone(), "https://example.com/c/d".to_owned()];
        let list = "remote:\n  - git@example.com:a/b.git\n  - https://example.com/c/d\n";
        for (text, remotes, out) in [
            // Well formed: added before the closing line, or put in place of the old value.
            (
                "---\nstatus: x\n---\n\n# Demo\n",
                &one[..],
                "---\nstatus: x\nremote: git@example.com:a/b.git\n---\n\n# Demo\n".to_owned(),
            ),
            (
                "---\nRemote: old\nstatus: x\n---\n# Demo\n",
                &two[..],
                format!("---\n{list}status: x\n---\n# Demo\n"),
            ),
            (
                "---\nremote:\n  - old\n  - older\nstatus: x\n---\n# Demo\n",
                &one[..],
                "---\nremote: git@example.com:a/b.git\nstatus: x\n---\n# Demo\n".to_owned(),
            ),
            (
                "# Demo\n",
                &one[..],
                "---\nremote: git@example.com:a/b.git\n---\n\n# Demo\n".to_owned(),
            ),
            (
                "\u{feff}---\r\nstatus: x\r\n---\r\n# Demo\r\n",
                &one[..],
                "\u{feff}---\r\nstatus: x\r\nremote: git@example.com:a/b.git\r\n---\r\n# Demo\r\n"
                    .to_owned(),
            ),
            // Spaces on either `---` line, and an empty header.
            (
                "---  \nstatus: x\n--- \n# Demo\n",
                &one[..],
                "---  \nstatus: x\nremote: git@example.com:a/b.git\n--- \n# Demo\n".to_owned(),
            ),
            (
                "---\n---\n# Demo\n",
                &one[..],
                "---\nremote: git@example.com:a/b.git\n---\n# Demo\n".to_owned(),
            ),
            // A header that never closes is all header: no second one goes in front of it.
            (
                "---\nstatus: x\n# Demo",
                &one[..],
                "---\nstatus: x\n# Demo\nremote: git@example.com:a/b.git\n".to_owned(),
            ),
            (
                "---\nremote: old\n# Demo\n",
                &one[..],
                "---\nremote: git@example.com:a/b.git\n# Demo\n".to_owned(),
            ),
            // An opening line that is the whole note, with no line break after it.
            (
                "---",
                &one[..],
                "---\nremote: git@example.com:a/b.git\n".to_owned(),
            ),
            ("\u{feff}--- ", &two[..], format!("\u{feff}--- \n{list}")),
            (
                "---\r\nstatus: x",
                &one[..],
                "---\r\nstatus: x\r\nremote: git@example.com:a/b.git\r\n".to_owned(),
            ),
        ] {
            let written = with_remotes(text, remotes);
            assert_eq!(written, out, "{text:?}");
            assert_eq!(listed(&written), Some(remotes.join(", ")), "{written:?}");
        }
    }
}
