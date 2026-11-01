//! The pre-command matcher, provider-neutral and pure: an accident guard, not a sandbox.
//! Its target is a command a careless agent would plausibly write, with one wrapping layer,
//! not one written to get past it. It reads what a command says, not what it would do. The
//! limits are rows in `tests/fixtures/guard-bypass.tsv`. A command that names a denied
//! program and can't be read is denied; everything else is allowed.
//! Later growth comes from a real entry in the denial log or a real incident, not from
//! imagined bypasses.
use serde_json::Value;

/// How deep wrappers and substitutions nest before the text counts as unparsed.
const MAX_DEPTH: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Sh,
    Ps,
    Cmd,
}

#[derive(Debug, Clone, Default)]
struct Word {
    /// What the shell would pass, as far as it can be told.
    text: String,
    /// Holds a variable or a substitution, so `text` isn't the final value.
    dynamic: bool,
}

/// What the next word is. A redirect's target or a here-string is only skipped.
enum Expect {
    Word,
    Skip,
    Heredoc,
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    dialect: Dialect,
    escape: char,
    commands: Vec<Vec<Word>>,
    broken: bool,
}

fn parse(text: &str, dialect: Dialect, depth: usize) -> Parser {
    let escape = match dialect {
        Dialect::Sh => '\\',
        Dialect::Ps => '`',
        Dialect::Cmd => '^',
    };
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
        dialect,
        escape,
        commands: Vec::new(),
        broken: false,
    };
    parser.list(None, depth);
    parser
}

/// Everything flat: `&&`, `;`, pipes, subshells and substitutions all just separate
/// commands, each of which is checked whether or not it would have run.
impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    /// `until` is what closes a substitution; running out before it means unparsed.
    fn list(&mut self, until: Option<char>, depth: usize) {
        if depth > MAX_DEPTH {
            self.broken = true;
            self.at = self.chars.len();
            return;
        }
        let mut command = Vec::new();
        let mut word: Option<Word> = None;
        let mut expect = Expect::Word;
        let mut heredocs = Vec::new();
        let mut parens = 0usize;
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                c if c == self.escape => {
                    let next = self.peek().filter(|next| *next != '\n');
                    self.at += 1;
                    if let Some(next) = next {
                        word.get_or_insert_default().text.push(next);
                    }
                }
                '\'' => {
                    let rest = &self.chars[self.at..];
                    let end = rest.iter().position(|c| *c == '\'');
                    self.broken |= end.is_none();
                    let end = end.unwrap_or(rest.len());
                    word.get_or_insert_default().text.extend(&rest[..end]);
                    self.at += end + 1;
                }
                '"' => self.double_quoted(word.get_or_insert_default(), depth),
                '$' => self.dollar(word.get_or_insert_default(), depth),
                ' ' | '\t' | '\r' => {
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                }
                '#' if word.is_none() && self.dialect != Dialect::Cmd => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.at += 1;
                    }
                }
                '<' | '>' => {
                    // The 2 in `2>`.
                    word.take_if(|w| w.text.chars().all(|c| c.is_ascii_digit()));
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                    let next = self.chars.get(self.at + 1);
                    let heredoc = c == '<' && self.peek() == Some('<') && next != Some(&'<');
                    while matches!(self.peek(), Some('<' | '>' | '&' | '|')) {
                        self.at += 1;
                    }
                    expect = Expect::Skip;
                    if heredoc {
                        expect = Expect::Heredoc;
                    }
                }
                c if matches!(c, '\n' | ';' | '|' | '&' | '(' | ')') || until == Some(c) => {
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                    self.finish(&mut command);
                    match c {
                        c if until == Some(c) && parens == 0 => return,
                        '(' => parens += 1,
                        ')' => parens = parens.saturating_sub(1),
                        '\n' => self.read_heredocs(&mut heredocs),
                        _ => {}
                    }
                }
                '`' if self.dialect == Dialect::Sh => {
                    self.substitution(word.get_or_insert_default(), '`', depth);
                }
                _ => word.get_or_insert_default().text.push(c),
            }
        }
        end_word(&mut word, &mut command, &mut expect, &mut heredocs);
        self.finish(&mut command);
        self.broken |= until.is_some();
    }

    fn finish(&mut self, command: &mut Vec<Word>) {
        if !command.is_empty() {
            self.commands.push(std::mem::take(command));
        }
    }

    /// A heredoc's body is text, skipped so that prose in it isn't read as commands. Tabs
    /// are stripped for `<<` as for `<<-`, so a plain one can end early.
    fn read_heredocs(&mut self, heredocs: &mut Vec<String>) {
        for delimiter in heredocs.drain(..) {
            while self.at < self.chars.len() {
                let rest = &self.chars[self.at..];
                let end = rest.iter().position(|c| *c == '\n').unwrap_or(rest.len());
                let line: String = rest[..end].iter().collect();
                self.at += end + 1;
                if line.trim_start_matches('\t').trim_end_matches('\r') == delimiter {
                    break;
                }
            }
        }
    }

    fn double_quoted(&mut self, word: &mut Word, depth: usize) {
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '"' => return,
                '\\' | '`' if c == self.escape => {
                    let Some(next) = self.peek() else { continue };
                    self.at += 1;
                    if c == '\\' && !matches!(next, '"' | '\\' | '$' | '`' | '\n') {
                        word.text.push('\\');
                    }
                    if next != '\n' {
                        word.text.push(next);
                    }
                }
                '`' if self.dialect == Dialect::Sh => self.substitution(word, '`', depth),
                '$' if self.dialect != Dialect::Cmd => self.dollar(word, depth),
                _ => word.text.push(c),
            }
        }
        self.broken = true;
    }

    /// Just past a `$`.
    fn dollar(&mut self, word: &mut Word, depth: usize) {
        if self.peek() == Some('(') {
            self.at += 1;
            self.substitution(word, ')', depth);
            return;
        }
        word.dynamic |= (self.peek())
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '{' | '_' | '?' | '@' | '*' | '!'));
        word.text.push('$');
    }

    fn substitution(&mut self, word: &mut Word, until: char, depth: usize) {
        word.dynamic = true;
        self.list(Some(until), depth + 1);
    }
}

fn end_word(
    word: &mut Option<Word>,
    command: &mut Vec<Word>,
    expect: &mut Expect,
    heredocs: &mut Vec<String>,
) {
    let Some(done) = word.take() else { return };
    match std::mem::replace(expect, Expect::Word) {
        Expect::Word => command.push(done),
        Expect::Skip => {}
        Expect::Heredoc => heredocs.push(done.text.trim_start_matches('-').to_owned()),
    }
}

/// Where a command runs. A missing root or cwd means no recursive delete can be judged
/// to be inside, so it is denied.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub root: Option<String>,
    pub cwd: Option<String>,
}

struct Context {
    root: Option<String>,
    /// None once a command has changed folder: not followed, so it stops being known.
    cwd: Option<String>,
    windows: bool,
}

/// The reason to deny, or None to stay out of the way. Never an allow: a hook's allow
/// would remove the client's approval prompt.
pub fn decide(tool: &str, input: &Value, scope: &Scope) -> Option<String> {
    // Their `command` isn't a shell's: Codex's patch tool puts the patch text there. A tool
    // unknown here that has a `command` is read as a shell, so a new one stays covered.
    if tool.starts_with("mcp__") || tool == "apply_patch" {
        return None;
    }
    let windows = (scope.root.as_deref()).is_some_and(|r| has_drive(r) || r.starts_with(r"\\"));
    let mut context = Context {
        root: scope.root.as_deref().map(|root| normal(root, windows)),
        cwd: scope.cwd.as_deref().map(|cwd| normal(cwd, windows)),
        windows,
    };
    // A tool's own working directory, which the hook's `cwd` doesn't reflect.
    if let Some(workdir) = ["workdir", "cwd", "working_directory"]
        .iter()
        .find_map(|key| input.get(*key)?.as_str())
    {
        context.cwd = resolve(workdir, &context);
    }
    let dialect = if tool.eq_ignore_ascii_case("powershell") {
        Dialect::Ps
    } else {
        Dialect::Sh
    };
    match input.get("command").or_else(|| input.get("cmd"))? {
        Value::String(text) => script(text, dialect, &mut context, 0),
        Value::Array(parts) => {
            let word = |part: &Value| Word {
                text: part.as_str().unwrap_or_default().to_owned(),
                dynamic: false,
            };
            let words: Vec<Word> = parts.iter().map(word).collect();
            simple(&words, dialect, &mut context, 0, false)
        }
        _ => None,
    }
}

/// For when nothing could be parsed at all: does this text name a denied program?
pub fn names_denied_program(text: &str) -> bool {
    const NAMES: &str = "pkill killall git gh rm find taskkill remove-item stop-process rmdir";
    text.to_ascii_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .any(|word| NAMES.split(' ').any(|name| name == word))
}

fn unparsed() -> String {
    "a command naming a denied program that could not be fully parsed".to_owned()
}

fn script(text: &str, dialect: Dialect, context: &mut Context, depth: usize) -> Option<String> {
    let parsed = parse(text, dialect, depth);
    (parsed.commands.iter())
        .find_map(|command| simple(command, dialect, context, depth, false))
        .or_else(|| (parsed.broken && names_denied_program(text)).then(unparsed))
}

const KEYWORDS: [&str; 12] = [
    "if", "then", "elif", "else", "fi", "do", "done", "while", "until", "!", "{", "}",
];

fn is_assignment(word: &Word) -> bool {
    let name = word.text.split_once('=').map(|(name, _)| name);
    name.is_some_and(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// Lowercase, no folder, no `.exe`. Case folded because the Mac's and Windows' file
/// systems would find the program either way.
fn program(word: &str) -> String {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let name = name.to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_owned()
}

fn join(words: &[Word]) -> String {
    let texts: Vec<&str> = words.iter().map(|word| word.text.as_str()).collect();
    texts.join(" ")
}

/// The letters of a `-abc`, not of a `--long`.
fn short_flags(text: &str) -> Option<&str> {
    text.strip_prefix('-').filter(|f| !f.starts_with('-'))
}

/// Past a wrapper's own options. `taking_value` lists those that consume the next word.
fn after_options<'a>(mut args: &'a [Word], taking_value: &[&str]) -> &'a [Word] {
    while args.first().is_some_and(|a| a.text.starts_with('-')) {
        let skip = 1 + usize::from(taking_value.contains(&args[0].text.as_str()));
        args = args.get(skip..).unwrap_or_default();
    }
    args
}

/// The command a plain wrapper runs, or None if `name` isn't one.
fn wrapped<'a>(name: &str, args: &'a [Word]) -> Option<&'a [Word]> {
    let values: &[&str] = match name {
        "env" => &["-u", "--unset", "-C", "--chdir"],
        "command"
            if args
                .first()
                .is_some_and(|a| matches!(a.text.as_str(), "-v" | "-V")) =>
        {
            return Some(&[]);
        }
        "exec" => &["-a"],
        "command" | "builtin" | "nohup" | "setsid" | "time" => &[],
        "nice" => &["-n", "--adjustment"],
        "timeout" => {
            let rest = after_options(args, &["-k", "-s", "--kill-after", "--signal"]);
            return Some(rest.get(1..).unwrap_or_default());
        }
        "sudo" => &["-u", "-g", "-h", "-p", "-C", "-D", "-T", "-U", "-r", "-t"],
        "xargs" => &["-I", "-n", "-P", "-L", "-s", "-d", "-E", "-J", "-R", "-S"],
        _ => return None,
    };
    Some(after_options(args, values))
}

/// One command. `piped` means more arguments arrive at run time, from `xargs` or `find`.
fn simple(
    words: &[Word],
    dialect: Dialect,
    context: &mut Context,
    depth: usize,
    piped: bool,
) -> Option<String> {
    let mut words = words;
    loop {
        match words {
            [first, ..] if KEYWORDS.contains(&first.text.as_str()) || is_assignment(first) => {
                words = &words[1..];
            }
            // PowerShell's `$x = git ...`.
            [first, second, ..] if first.dynamic && second.text == "=" => words = &words[2..],
            _ => break,
        }
    }
    let first = words.first()?;
    if depth > MAX_DEPTH {
        return names_denied_program(&join(words)).then(unparsed);
    }
    let name = program(&first.text);
    let args = &words[1..];
    let depth = depth + 1;
    // `env -C dir` and `sudo -D dir` change folder, like `cd`.
    let moves = |a: &Word| {
        ["-C", "-D", "--chdir"]
            .iter()
            .any(|o| a.text.starts_with(o))
    };
    if matches!(name.as_str(), "env" | "sudo") && args.iter().any(moves) {
        context.cwd = None;
    }
    if let Some(rest) = wrapped(&name, args) {
        return simple(rest, dialect, context, depth, piped || name == "xargs");
    }
    let lowered = |arg: &Word| arg.text.to_ascii_lowercase();
    match name.as_str() {
        "eval" | "iex" | "invoke-expression" => script(&join(args), dialect, context, depth),
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "ash" | "fish" => {
            let has_c = |a: &Word| short_flags(&a.text).is_some_and(|f| f.contains('c'));
            let flag = args.iter().position(has_c)?;
            script(&args.get(flag + 1)?.text, Dialect::Sh, context, depth)
        }
        "powershell" | "pwsh" | "cmd" => {
            let (flags, inner): (&[&str], _) = if name == "cmd" {
                (&["/c", "/k", "//c", "//k"], Dialect::Cmd)
            } else {
                (&["-command", "-c"], Dialect::Ps)
            };
            let flag = args.iter().position(|a| flags.contains(&&*lowered(a)))?;
            script(&join(&args[flag + 1..]), inner, context, depth)
        }
        "cd" | "pushd" | "popd" | "chdir" | "set-location" | "sl" | "pop-location" => {
            context.cwd = None;
            None
        }
        "pkill" | "killall" => Some(format!("machine-wide {name}")),
        "taskkill" => (args.iter().any(|a| matches!(&*lowered(a), "/im" | "//im")))
            .then(|| "machine-wide taskkill by name".to_owned()),
        "stop-process" | "spps" => {
            let by_id =
                |a: &Word| lowered(a) == "-id" || a.text.chars().all(|c| c.is_ascii_digit());
            (!args.iter().any(by_id)).then(|| "Stop-Process by name".to_owned())
        }
        "git" => git(args, piped),
        "gh" => gh(args),
        "find" => find(args, dialect, context, depth),
        "rm" | "remove-item" | "ri" | "rmdir" | "rd" | "del" | "erase" => {
            remove(&name, args, context, piped)
        }
        _ => None,
    }
}

fn unscoped(paths: &[Word]) -> bool {
    const UNSCOPED_PATHS: [&str; 7] = [".", "./", "*", ":/", ":", "/", ":/*"];
    (paths.iter()).any(|arg| UNSCOPED_PATHS.contains(&arg.text.as_str()))
}

fn git(args: &[Word], piped: bool) -> Option<String> {
    let mut rest = args;
    while let Some(first) = rest.first() {
        let text = first.text.as_str();
        if matches!(text, "-C" | "-c" | "--git-dir" | "--work-tree") {
            rest = rest.get(2..).unwrap_or_default();
        } else if text.starts_with('-') {
            rest = &rest[1..];
        } else {
            break;
        }
    }
    let (sub, args) = rest.split_first()?;
    let has = |option: &str| args.iter().any(|arg| arg.text == option);
    let (denied, what) = match sub.text.as_str() {
        _ if sub.dynamic => (true, "a git command hidden in a variable"),
        // Through `xargs`, arguments arrive at run time.
        "push" => {
            let force = args.iter().any(|arg| is_force_push_argument(&arg.text));
            (piped || force, "force-push")
        }
        "reset" => (has("--hard"), "reset --hard"),
        "checkout" => {
            let dashes = args.iter().position(|arg| arg.text == "--");
            let paths = &args[dashes.map_or(0, |at| at + 1)..];
            let all = (dashes.is_some() && paths.is_empty()) || unscoped(paths);
            (all, "an unscoped git checkout")
        }
        "restore" => {
            let staged_only = (has("--staged") || has("-S")) && !(has("--worktree") || has("-W"));
            let has_paths = args.iter().any(|a| !a.text.starts_with('-'));
            let all = !staged_only && (!has_paths || unscoped(args));
            (all, "an unscoped git restore")
        }
        _ => return None,
    };
    denied.then(|| what.to_owned())
}

fn is_force_push_argument(text: &str) -> bool {
    text.starts_with('+')
        || text == "--mirror"
        || text.starts_with("--force")
        || short_flags(text).is_some_and(|flags| flags.contains('f'))
}

/// By operation, reads named: anything else on `gh` posts, changes or deletes something,
/// or is new to this list, as a variable is: the same to a guard that can't tell.
fn gh(args: &[Word]) -> Option<String> {
    const READS: [&str; 13] = [
        "view", "list", "ls", "status", "diff", "checks", "watch", "download", "checkout", "clone",
        "get", "token", "verify",
    ];
    let mut positional = args.iter().filter(|arg| !arg.text.starts_with('-'));
    let denied = match positional.next()?.text.as_str() {
        "help" | "version" | "status" | "search" | "browse" | "completion" => false,
        "api" => gh_api_writes(args),
        _ => !READS.contains(&positional.next()?.text.as_str()),
    };
    denied.then(|| "a gh call that posts or changes something".to_owned())
}

/// All of `graphql` counts as a write: telling a query from a mutation isn't worth it.
fn gh_api_writes(args: &[Word]) -> bool {
    let mut method = None;
    let mut fields = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let text = arg.text.as_str();
        if text == "-X" || text == "--method" {
            method = iter.next().map(|value| value.text.to_ascii_uppercase());
        } else if let Some(value) = text.strip_prefix("--method=").or(text.strip_prefix("-X")) {
            method = Some(value.to_ascii_uppercase());
        }
        let key = text.split('=').next().unwrap_or(text);
        fields |= ["-f", "-F", "--field", "--raw-field", "--input"].contains(&key);
    }
    let graphql = args.iter().any(|arg| arg.text == "graphql");
    graphql || method.map_or(fields, |method| method != "GET" && method != "HEAD")
}

/// A plain check on the arguments, so `find . -name -delete` is denied too.
fn find(args: &[Word], dialect: Dialect, context: &mut Context, depth: usize) -> Option<String> {
    if args.iter().any(|arg| arg.text == "-delete") {
        return Some("find -delete".to_owned());
    }
    // What follows its `;` reads as more arguments, which only ever adds a denial.
    const EXEC: [&str; 4] = ["-exec", "-execdir", "-ok", "-okdir"];
    (args.iter().enumerate())
        .filter(|(_, arg)| EXEC.contains(&arg.text.as_str()))
        .find_map(|(at, _)| simple(&args[at + 1..], dialect, context, depth, true))
}

fn remove(name: &str, args: &[Word], context: &Context, piped: bool) -> Option<String> {
    let recursive = args.iter().any(|arg| {
        let text = arg.text.to_ascii_lowercase();
        // PowerShell's `-Force` isn't a cluster holding `r`.
        let cluster = |f: &str| name == "rm" && f != "force" && f.contains('r');
        short_flags(&text).is_some_and(cluster)
            || ["--recursive", "-r", "-recurse", "/s", "//s"].contains(&text.as_str())
    });
    let slash_options = matches!(name, "rmdir" | "rd" | "del" | "erase");
    let is_path = |arg: &&Word| {
        let slash_option = slash_options && arg.text.starts_with('/') && arg.text.len() <= 3;
        !arg.text.starts_with('-') && !slash_option
    };
    let paths: Vec<&Word> = args.iter().filter(is_path).collect();
    let inside = |part: &str| {
        let resolved = resolve(part, context).zip(context.root.as_ref());
        // `@(...)` is not read.
        !part.ends_with('@')
            && resolved.is_some_and(|(path, root)| path.starts_with(&format!("{root}/")))
    };
    // PowerShell takes `a,b` as two paths.
    let readable = |path: &&Word| {
        !path.dynamic
            && !path.text.contains('%')
            && !path.text.starts_with('~')
            && path.text.split(',').filter(|p| !p.is_empty()).all(inside)
    };
    let reason =
        "a recursive delete not plainly inside the worktree; after a cd, use an absolute path";
    let denied = piped || paths.is_empty() || !paths.iter().all(readable);
    (recursive && denied).then(|| reason.to_owned())
}

/// Lexical: an existing link isn't resolved. See the misses in the table.
fn resolve(text: &str, context: &Context) -> Option<String> {
    let rooted_windows = context.windows && (has_drive(text) || text.starts_with('\\'));
    if text.starts_with('/') || rooted_windows {
        Some(normal(text, context.windows))
    } else {
        let cwd = context.cwd.as_ref()?;
        Some(normal(&format!("{cwd}/{text}"), context.windows))
    }
}

fn has_drive(text: &str) -> bool {
    let mut chars = text.trim_start_matches(r"\\?\").chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.next() == Some(':')
}

/// Forward slashes with `.` and `..` gone. On Windows also: lowercase, `C:` not `/c`.
fn normal(path: &str, windows: bool) -> String {
    let mut text = path.to_owned();
    if windows {
        text = text.replace('\\', "/").to_ascii_lowercase();
        text = text.trim_start_matches("//?/").to_owned();
        if let [b'/', drive, tail @ ..] = text.as_bytes()
            && drive.is_ascii_alphabetic()
            && matches!(tail.first(), None | Some(b'/'))
        {
            text = format!("{}:{}", *drive as char, &text[2..]);
        }
    }
    let drive = windows && text.as_bytes().get(1) == Some(&b':');
    let (prefix, rest) = text.split_at(if drive { 2 } else { 0 });
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => parts.truncate(parts.len().saturating_sub(1)),
            part => parts.push(part),
        }
    }
    format!("{prefix}/{}", parts.join("/"))
}
