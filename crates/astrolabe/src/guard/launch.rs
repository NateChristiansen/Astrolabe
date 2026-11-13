//! Worker launches: a command or tool call that starts an agent session must name its model
//! and its reasoning effort, because a launch that names neither inherits the host's
//! defaults, and those can be the costliest model at a high effort (lesson, 2026-11-12).
//! Read from the command line or the tool's input only, never from host config: a profile
//! or a config file can hide what a launch inherits, so neither counts as naming it.
//! Provider-specific by necessity; each provider's rules sit in its own function.
use super::Word;
use serde_json::Value;

/// What every launch denial starts with, so the hook can tell it from the guard's other
/// denials: the fix is to add the flags, not to ask the owner.
pub const LAUNCH: &str = "a worker launch with no explicit model and effort";

/// The defaults a denial suggests. Update them when a provider ships a newer model.
const CODEX_DEFAULT: &str = "-m gpt-6.1-sol -c model_reasoning_effort=medium";
const CLAUDE_DEFAULT: &str = "--model opus --effort medium";
const SESSION_DEFAULT: &str = "model \"claude-opus-5-5\" and effort \"medium\"";

/// A tool that starts a session, so its input is checked even when the hook can't read it
/// whole.
pub fn launch_tool(tool: &str) -> bool {
    tool.ends_with("__start_session")
}

/// Whether the hook should answer this denial as one the agent fixes itself.
pub fn correctable(reason: &str) -> bool {
    reason.starts_with(LAUNCH)
}

fn denial(what: &str, fix: &str) -> String {
    format!(
        "{LAUNCH}: {what}. Add {fix} (the default), or a higher model or effort with the reason in the brief"
    )
}

/// One declared value: the last one given wins, as in both CLIs, and it must be literal.
#[derive(Default)]
struct Declared {
    value: Option<(String, bool)>,
}

impl Declared {
    fn set(&mut self, word: Option<&Word>, text: Option<&str>) {
        let dynamic = word.is_some_and(|w| w.dynamic);
        let text = text.or(word.map(|w| w.text.as_str())).unwrap_or_default();
        self.value = Some((unquote(text).to_owned(), dynamic));
    }

    /// Named, literal and not a word that means "whatever the default is".
    fn explicit(&self) -> bool {
        self.value.as_ref().is_some_and(|(text, dynamic)| {
            !dynamic && !text.is_empty() && !text.eq_ignore_ascii_case("default")
        })
    }
}

fn unquote(text: &str) -> &str {
    let text = text.trim();
    ['"', '\'']
        .iter()
        .find_map(|q| text.strip_prefix(*q)?.strip_suffix(*q))
        .unwrap_or(text)
        .trim()
}

/// `codex …` subcommands that start no agent turn.
const CODEX_QUIET: [&str; 24] = [
    "agents",
    "login",
    "logout",
    "mcp",
    "mcp-server",
    "plugin",
    "app-server",
    "remote-control",
    "app",
    "completion",
    "update",
    "doctor",
    "sandbox",
    "debug",
    "apply",
    "a",
    "queue",
    "archive",
    "delete",
    "unarchive",
    "migrate-rollouts",
    "migrate-nfs-db",
    "exec-server",
    "features",
];

/// Codex options, global or a subcommand's, that take the next word as their value.
const CODEX_VALUES: [&str; 28] = [
    "-c",
    "--config",
    "--enable",
    "--disable",
    "--remote",
    "--remote-auth-token-env",
    "-i",
    "--image",
    "-m",
    "--model",
    "--local-provider",
    "-p",
    "--profile",
    "-s",
    "--sandbox",
    "-C",
    "--cd",
    "--add-dir",
    "-a",
    "--ask-for-approval",
    "--output-schema",
    "--color",
    "-o",
    "--output-last-message",
    "--thread-source",
    "--cyber-access-program",
    "--base",
    "--commit",
];

/// `codex [options] [subcommand] [options] [prompt]`. Bare `codex`, a prompt, `exec`/`e`
/// (with its `resume`, `fork` and `review`), `review`, `resume` and `fork` start a turn;
/// `cloud` runs on the provider's servers and isn't read here. A config override counts:
/// `-c model=…` for the model and `-c model_reasoning_effort=…` for the effort.
pub(super) fn codex(args: &[Word]) -> Option<String> {
    let mut model = Declared::default();
    let mut effort = Declared::default();
    let mut sub: Option<&str> = None;
    let mut at = 0;
    while let Some(word) = args.get(at) {
        at += 1;
        let text = word.text.as_str();
        if text == "--" {
            break;
        }
        if matches!(text, "-h" | "--help" | "-V" | "--version") {
            return None;
        }
        let (flag, attached) = match text.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => match text {
                t if t.len() > 2
                    && (t.starts_with("-m") || t.starts_with("-c"))
                    && !t.starts_with("--") =>
                {
                    (&t[..2], Some(&t[2..]))
                }
                t => (t, None),
            },
        };
        if flag.starts_with('-') {
            let value_word = match attached {
                Some(_) => None,
                None if CODEX_VALUES.contains(&flag) => {
                    at += 1;
                    args.get(at - 1)
                }
                None => None,
            };
            let dynamic = word.dynamic || value_word.is_some_and(|w| w.dynamic);
            let value = attached.or(value_word.map(|w| w.text.as_str()));
            match flag {
                "-m" | "--model" => model.set(value_word.or(Some(word)), value),
                "-c" | "--config" => {
                    let Some((key, setting)) = value.and_then(|v| v.split_once('=')) else {
                        continue;
                    };
                    let target = match unquote(key) {
                        "model" => &mut model,
                        "model_reasoning_effort" => &mut effort,
                        _ => continue,
                    };
                    target.value = Some((unquote(setting).to_owned(), dynamic));
                }
                _ => {}
            }
            continue;
        }
        if sub.is_none() {
            if word.dynamic {
                return Some(denial(
                    "a codex command hidden in a variable",
                    CODEX_DEFAULT,
                ));
            }
            sub = Some(text);
            if CODEX_QUIET.contains(&text) || text == "help" || text == "cloud" {
                return None;
            }
        }
    }
    let missing = match (model.explicit(), effort.explicit()) {
        (true, true) => return None,
        (false, false) => "a codex launch names neither a model nor an effort",
        (false, true) => "a codex launch names no model",
        (true, false) => "a codex launch names no reasoning effort",
    };
    Some(denial(missing, &format!("`{CODEX_DEFAULT}`")))
}

/// `claude …` subcommands that start no session here.
const CLAUDE_QUIET: [&str; 22] = [
    "agents",
    "attach",
    "auth",
    "auto-mode",
    "doctor",
    "gateway",
    "import",
    "install",
    "logs",
    "mcp",
    "plugin",
    "plugins",
    "purge",
    "respawn",
    "rm",
    "setup-token",
    "stop",
    "kill",
    "ultrareview",
    "update",
    "upgrade",
    "help",
];

/// `claude [options] [prompt]` starts or resumes a session, interactive or not, in the
/// foreground or with `--bg`. Not a launch: a management subcommand as the first word, help
/// or version, a cloud start (`--cloud`, `--environment`), which runs elsewhere, and the
/// desktop handoff `--desktop --resume <id>`, which moves an idle session and takes no flags.
pub(super) fn claude(args: &[Word]) -> Option<String> {
    if let Some(first) = args.first()
        && !first.text.starts_with('-')
        && CLAUDE_QUIET.contains(&first.text.as_str())
    {
        return None;
    }
    let mut model = Declared::default();
    let mut effort = Declared::default();
    let mut desktop = false;
    let mut resume = false;
    let mut at = 0;
    while let Some(word) = args.get(at) {
        at += 1;
        let text = word.text.as_str();
        if text == "--" {
            break;
        }
        let (flag, attached) = match text.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (text, None),
        };
        match flag {
            "-h" | "--help" | "-v" | "--version" | "--cloud" | "--environment" => return None,
            "--desktop" => desktop = true,
            "-r" | "--resume" => resume = true,
            "--model" | "--effort" => {
                let value_word = match attached {
                    Some(_) => None,
                    None => {
                        at += 1;
                        args.get(at - 1)
                    }
                };
                let target = if flag == "--model" {
                    &mut model
                } else {
                    &mut effort
                };
                target.set(
                    value_word.or(Some(word)),
                    attached.or(value_word.map(|w| w.text.as_str())),
                );
            }
            _ => {}
        }
    }
    if desktop && resume {
        return None;
    }
    let missing = match (model.explicit(), effort.explicit()) {
        (true, true) => return None,
        (false, false) => "a claude launch names neither a model nor an effort",
        (false, true) => "a claude launch names no model (`default` doesn't count)",
        (true, false) => "a claude launch names no effort",
    };
    Some(denial(missing, &format!("`{CLAUDE_DEFAULT}`")))
}

/// A desktop app's session-starting tool, by the end of its name. `fork` can take no model
/// or effort, so it always inherits the caller's. A fresh local or SSH start, or a proposal
/// of one, must name both. Not read: starting a proposal by `proposal_id` (its values were
/// fixed when it was proposed), a title-only `offer`, and a cloud start, which ignores both.
pub(super) fn session_tool(tool: &str, input: &Value) -> Option<String> {
    if !launch_tool(tool) {
        return None;
    }
    let given = |key: &str| {
        input[key]
            .as_str()
            .is_some_and(|v| !v.trim().is_empty() && !v.eq_ignore_ascii_case("default"))
    };
    if input["proposal_id"].as_str().is_some_and(|p| !p.is_empty()) {
        return None;
    }
    if input["initiation"].as_str() == Some("offer") && input["prompt"].is_null() {
        return None;
    }
    if input["target"].as_str() == Some("cloud") {
        return None;
    }
    if input["context"].as_str() == Some("fork") {
        return Some(denial(
            "a forked session inherits this session's model and effort and can't take its own",
            &format!("context \"fresh\" with a background, {SESSION_DEFAULT}"),
        ));
    }
    let missing = match (given("model"), given("effort")) {
        (true, true) => return None,
        (false, false) => "a session start names neither a model nor an effort",
        (false, true) => "a session start names no model",
        (true, false) => "a session start names no effort",
    };
    Some(denial(missing, SESSION_DEFAULT))
}
