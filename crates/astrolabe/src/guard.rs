//! The pre-command matcher, provider-neutral and pure: an accident guard, not a sandbox.
//! It reads what a command says, not what it would do, so a program it can't see (a
//! script file, an interpreter, encoded text, a variable) gets past it. Those limits are
//! rows in `tests/fixtures/guard-bypass.tsv`. A command that names a denied program and
//! can't be fully parsed is denied; everything else is allowed.
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
    /// The same with backslashes kept, for Windows paths under a POSIX reading.
    raw: String,
    /// Holds a variable or a substitution, so `text` isn't the final value.
    dynamic: bool,
}

#[derive(Default)]
struct SimpleCommand {
    words: Vec<Word>,
    /// Heredoc or here-string text, which a shell would run.
    stdin: Option<String>,
}

#[derive(Default)]
struct Parsed {
    commands: Vec<SimpleCommand>,
    broken: bool,
}

enum Expect {
    Word,
    RedirectTarget,
    HereString,
    Heredoc { strip_tabs: bool },
}

struct Heredoc {
    delimiter: String,
    strip_tabs: bool,
    /// None while its command is still being read.
    command: Option<usize>,
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    dialect: Dialect,
    out: Parsed,
}

fn parse(text: &str, dialect: Dialect, depth: usize) -> Parsed {
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
        dialect,
        out: Parsed::default(),
    };
    parser.list(false, depth);
    parser.out
}

/// Everything flat: `&&`, `;`, pipes, subshells and substitutions all just separate
/// commands, each of which is checked whether or not it would have run.
impl Parser {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    fn list(&mut self, in_substitution: bool, depth: usize) {
        if depth > MAX_DEPTH {
            self.out.broken = true;
            self.at = self.chars.len();
            return;
        }
        let mut command = SimpleCommand::default();
        let mut word: Option<Word> = None;
        let mut expect = Expect::Word;
        let mut heredocs: Vec<Heredoc> = Vec::new();
        let mut parens = 0usize;
        while let Some(c) = self.peek(0) {
            match c {
                '\'' => {
                    self.at += 1;
                    let word = word.get_or_insert_default();
                    match self.chars[self.at..].iter().position(|c| *c == '\'') {
                        Some(end) => {
                            let inner: String = self.chars[self.at..self.at + end].iter().collect();
                            word.text.push_str(&inner);
                            word.raw.push_str(&inner);
                            self.at += end + 1;
                        }
                        None => {
                            self.out.broken = true;
                            self.at = self.chars.len();
                        }
                    }
                }
                '"' => {
                    self.at += 1;
                    self.double_quoted(word.get_or_insert_default(), depth);
                }
                '\\' if self.dialect == Dialect::Sh => {
                    self.at += 1;
                    match self.peek(0) {
                        Some('\n') => self.at += 1,
                        Some(next) => {
                            let word = word.get_or_insert_default();
                            word.text.push(next);
                            word.raw.push('\\');
                            word.raw.push(next);
                            self.at += 1;
                        }
                        None => {}
                    }
                }
                '`' if self.dialect == Dialect::Ps => {
                    self.at += 1;
                    if let Some(next) = self.peek(0) {
                        self.at += 1;
                        if next != '\n' {
                            push(word.get_or_insert_default(), next);
                        }
                    }
                }
                '^' if self.dialect == Dialect::Cmd => {
                    self.at += 1;
                    if let Some(next) = self.peek(0) {
                        self.at += 1;
                        push(word.get_or_insert_default(), next);
                    }
                }
                '`' => {
                    self.at += 1;
                    self.backticks(word.get_or_insert_default(), depth);
                }
                '$' => self.dollar(word.get_or_insert_default(), depth),
                ' ' | '\t' | '\r' => {
                    self.at += 1;
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                }
                '#' if word.is_none() && self.dialect != Dialect::Cmd => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.at += 1;
                    }
                }
                '<' | '>' => {
                    if word
                        .as_ref()
                        .is_some_and(|w| w.text.chars().all(|c| c.is_ascii_digit()))
                    {
                        word = None;
                    }
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                    expect = self.redirect();
                }
                '&' if self.peek(1) == Some('>') => {
                    self.at += 1;
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                    expect = self.redirect();
                }
                '\n' | ';' | '|' | '&' | '(' | ')' => {
                    if c == ')' && parens == 0 && in_substitution {
                        self.at += 1;
                        end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                        self.finish(&mut command, &mut heredocs);
                        return;
                    }
                    match c {
                        '(' => parens += 1,
                        ')' => parens = parens.saturating_sub(1),
                        _ => {}
                    }
                    self.at += 1;
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                    self.finish(&mut command, &mut heredocs);
                    if c == '\n' {
                        self.read_heredocs(&mut heredocs);
                    }
                }
                _ => {
                    self.at += 1;
                    push(word.get_or_insert_default(), c);
                }
            }
        }
        end_word(&mut word, &mut command, &mut expect, &mut heredocs);
        self.finish(&mut command, &mut heredocs);
        if in_substitution {
            self.out.broken = true;
        }
    }

    fn finish(&mut self, command: &mut SimpleCommand, heredocs: &mut [Heredoc]) {
        let index = self.out.commands.len();
        for heredoc in heredocs.iter_mut().filter(|h| h.command.is_none()) {
            heredoc.command = Some(index);
        }
        if !command.words.is_empty() {
            self.out.commands.push(std::mem::take(command));
        }
    }

    /// Consumes the operator and says what the next word is. A `|` in `>|` too.
    fn redirect(&mut self) -> Expect {
        let first = self.peek(0);
        self.at += 1;
        if first == Some('<') && self.peek(0) == Some('<') {
            self.at += 1;
            if self.peek(0) == Some('<') {
                self.at += 1;
                return Expect::HereString;
            }
            let strip_tabs = self.peek(0) == Some('-');
            if strip_tabs {
                self.at += 1;
            }
            return Expect::Heredoc { strip_tabs };
        }
        while matches!(self.peek(0), Some('>' | '&' | '|')) {
            self.at += 1;
        }
        Expect::RedirectTarget
    }

    fn read_heredocs(&mut self, heredocs: &mut Vec<Heredoc>) {
        for heredoc in heredocs.drain(..) {
            let mut body = String::new();
            while self.at < self.chars.len() {
                let mut line = String::new();
                while let Some(c) = self.peek(0) {
                    self.at += 1;
                    if c == '\n' {
                        break;
                    }
                    line.push(c);
                }
                let compared = if heredoc.strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if compared.trim_end_matches('\r') == heredoc.delimiter {
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            if let Some(command) = heredoc.command.and_then(|i| self.out.commands.get_mut(i)) {
                command.stdin = Some(body);
            }
        }
    }

    fn double_quoted(&mut self, word: &mut Word, depth: usize) {
        while let Some(c) = self.peek(0) {
            match c {
                '"' if self.dialect != Dialect::Sh && self.peek(1) == Some('"') => {
                    self.at += 2;
                    push(word, '"');
                }
                '"' => {
                    self.at += 1;
                    return;
                }
                '\\' if self.dialect == Dialect::Sh => {
                    self.at += 1;
                    if let Some(next) = self.peek(0) {
                        self.at += 1;
                        if matches!(next, '"' | '\\' | '$' | '`') {
                            word.text.push(next);
                            word.raw.push(next);
                        } else if next != '\n' {
                            word.text.push('\\');
                            word.text.push(next);
                            word.raw.push('\\');
                            word.raw.push(next);
                        }
                    }
                }
                '`' if self.dialect == Dialect::Ps => {
                    self.at += 1;
                    if let Some(next) = self.peek(0) {
                        self.at += 1;
                        push(word, next);
                    }
                }
                '`' if self.dialect == Dialect::Sh => {
                    self.at += 1;
                    self.backticks(word, depth);
                }
                '$' if self.dialect != Dialect::Cmd => self.dollar(word, depth),
                _ => {
                    self.at += 1;
                    push(word, c);
                }
            }
        }
        self.out.broken = true;
    }

    fn dollar(&mut self, word: &mut Word, depth: usize) {
        self.at += 1;
        match self.peek(0) {
            Some('(') => {
                self.at += 1;
                word.dynamic = true;
                self.list(true, depth + 1);
            }
            Some(next)
                if next.is_alphanumeric() || matches!(next, '{' | '_' | '?' | '@' | '*' | '!') =>
            {
                word.dynamic = true;
                push(word, '$');
            }
            _ => push(word, '$'),
        }
    }

    fn backticks(&mut self, word: &mut Word, depth: usize) {
        word.dynamic = true;
        let mut inner = String::new();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match c {
                '`' => {
                    let nested = parse(&inner, self.dialect, depth + 1);
                    self.out.broken |= nested.broken;
                    self.out.commands.extend(nested.commands);
                    return;
                }
                '\\' => {
                    if let Some(next) = self.peek(0) {
                        self.at += 1;
                        inner.push(next);
                    }
                }
                _ => inner.push(c),
            }
        }
        self.out.broken = true;
    }
}

fn push(word: &mut Word, c: char) {
    word.text.push(c);
    word.raw.push(c);
}

fn end_word(
    word: &mut Option<Word>,
    command: &mut SimpleCommand,
    expect: &mut Expect,
    heredocs: &mut Vec<Heredoc>,
) {
    let Some(done) = word.take() else { return };
    match std::mem::replace(expect, Expect::Word) {
        Expect::Word => command.words.push(done),
        Expect::RedirectTarget => {}
        Expect::HereString => command.stdin = Some(done.text),
        Expect::Heredoc { strip_tabs } => heredocs.push(Heredoc {
            delimiter: done.text,
            strip_tabs,
            command: None,
        }),
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
    cwd: Option<String>,
    windows: bool,
}

/// The reason to deny, or None to stay out of the way. Never an allow: a hook's allow
/// would remove the client's approval prompt.
pub fn decide(tool: &str, input: &Value, scope: &Scope) -> Option<String> {
    if tool.starts_with("mcp__") {
        return None;
    }
    let windows = scope.root.as_deref().is_some_and(has_drive);
    let mut context = Context {
        root: scope.root.as_deref().map(|root| normal(root, windows)),
        cwd: scope.cwd.as_deref().map(|cwd| normal(cwd, windows)),
        windows,
    };
    let dialect = if tool.eq_ignore_ascii_case("powershell") {
        Dialect::Ps
    } else {
        Dialect::Sh
    };
    let command = input.get("command").or_else(|| input.get("cmd"))?;
    match command {
        Value::String(text) => script(text, dialect, &mut context, 0),
        Value::Array(parts) => {
            let words: Vec<Word> = (parts.iter())
                .map(|part| {
                    let text = part.as_str().unwrap_or_default();
                    Word {
                        text: text.to_owned(),
                        raw: text.to_owned(),
                        dynamic: false,
                    }
                })
                .collect();
            simple(&words, None, dialect, &mut context, 0, false)
        }
        _ => None,
    }
}

/// For when nothing could be parsed at all: does this text name a denied program?
pub fn names_denied_program(text: &str) -> bool {
    const NAMES: [&str; 10] = [
        "pkill",
        "killall",
        "git",
        "gh",
        "rm",
        "find",
        "taskkill",
        "remove-item",
        "stop-process",
        "rmdir",
    ];
    text.to_ascii_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .any(|word| NAMES.contains(&word))
}

fn unparsed() -> String {
    "a command naming a denied program that could not be fully parsed".to_owned()
}

fn script(text: &str, dialect: Dialect, context: &mut Context, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return names_denied_program(text).then(unparsed);
    }
    let parsed = parse(text, dialect, depth);
    for command in &parsed.commands {
        let stdin = command.stdin.as_deref();
        if let Some(reason) = simple(&command.words, stdin, dialect, context, depth, false) {
            return Some(reason);
        }
    }
    (parsed.broken && names_denied_program(text)).then(unparsed)
}

const KEYWORDS: [&str; 12] = [
    "if", "then", "elif", "else", "fi", "do", "done", "while", "until", "!", "{", "}",
];

fn is_assignment(word: &Word) -> bool {
    let Some((name, _)) = word.text.split_once('=') else {
        return false;
    };
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Lowercase, no folder, no Windows extension. Case folded because the Mac's and
/// Windows' file systems would find the program either way.
fn program(word: &str) -> String {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let name = name.to_ascii_lowercase();
    for extension in [".exe", ".cmd", ".bat"] {
        if let Some(stem) = name.strip_suffix(extension) {
            return stem.to_owned();
        }
    }
    name
}

fn join(words: &[Word]) -> String {
    (words.iter())
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Past a wrapper's own options. `taking_value` lists those that consume the next word.
fn after_options<'a>(mut args: &'a [Word], taking_value: &[&str]) -> &'a [Word] {
    while let Some(first) = args.first() {
        if first.text == "--" {
            return &args[1..];
        }
        if !first.text.starts_with('-') || first.text.len() == 1 {
            break;
        }
        let skip = if taking_value.contains(&first.text.as_str()) {
            2
        } else {
            1
        };
        args = args.get(skip..).unwrap_or_default();
    }
    args
}

/// True if `word` abbreviates `option`, as PowerShell allows: `-enc`, `-Comm`.
fn abbreviates(word: &str, option: &str) -> bool {
    let word = word.to_ascii_lowercase();
    word.len() >= 2 && option.starts_with(&word)
}

/// One command. `piped` means more arguments arrive at run time, from `xargs` or `find`.
fn simple(
    words: &[Word],
    stdin: Option<&str>,
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
    let wrapped = |rest: &[Word], context: &mut Context, piped: bool| {
        simple(rest, None, dialect, context, depth, piped)
    };
    match name.as_str() {
        "env" => {
            let mut rest = args;
            loop {
                rest = after_options(rest, &["-u", "--unset", "-C", "--chdir"]);
                match rest {
                    [first, ..] if is_assignment(first) => rest = &rest[1..],
                    _ => break,
                }
            }
            if let Some(at) = args
                .iter()
                .position(|a| a.text == "-S" || a.text == "--split-string")
                && let Some(text) = args.get(at + 1)
            {
                return script(&text.text, dialect, context, depth);
            }
            wrapped(rest, context, piped)
        }
        "command" | "builtin" => {
            if args
                .first()
                .is_some_and(|a| a.text == "-v" || a.text == "-V")
            {
                return None;
            }
            wrapped(after_options(args, &[]), context, piped)
        }
        "exec" => wrapped(after_options(args, &["-a"]), context, piped),
        "nohup" | "setsid" | "time" | "caffeinate" | "busybox" => {
            wrapped(after_options(args, &["-t", "-w"]), context, piped)
        }
        "nice" | "ionice" => wrapped(
            after_options(args, &["-n", "-c", "-p", "--adjustment"]),
            context,
            piped,
        ),
        "stdbuf" => wrapped(after_options(args, &["-i", "-o", "-e"]), context, piped),
        "watch" => wrapped(after_options(args, &["-n", "--interval"]), context, piped),
        "timeout" => {
            let rest = after_options(args, &["-k", "-s", "--kill-after", "--signal"]);
            wrapped(rest.get(1..).unwrap_or_default(), context, piped)
        }
        "sudo" | "doas" => wrapped(
            after_options(
                args,
                &["-u", "-g", "-h", "-p", "-C", "-T", "-U", "-r", "-t"],
            ),
            context,
            piped,
        ),
        "xargs" => wrapped(
            after_options(
                args,
                &["-I", "-n", "-P", "-L", "-s", "-d", "-E", "-J", "-R", "-S"],
            ),
            context,
            true,
        ),
        "eval" | "iex" | "invoke-expression" => {
            let rest = after_options(args, &["-Command"]);
            script(&join(rest), dialect, context, depth)
        }
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "ash" | "fish" => {
            let flag = args.iter().position(|arg| {
                let text = &arg.text;
                text.starts_with('-')
                    && !text.starts_with("--")
                    && text[1..].chars().all(|c| c.is_ascii_alphabetic())
                    && text.contains('c')
            });
            if let Some(at) = flag {
                let inner = args.get(at + 1)?;
                return script(&inner.text, Dialect::Sh, context, depth);
            }
            stdin.and_then(|text| script(text, Dialect::Sh, context, depth))
        }
        "powershell" | "pwsh" => powershell(args, stdin, context, depth),
        "cmd" => {
            let flag = args.iter().position(|arg| {
                let text = arg.text.to_ascii_lowercase();
                matches!(text.as_str(), "/c" | "/k" | "//c" | "//k")
                    || text.starts_with("/c ")
                    || text.starts_with("//c ")
            })?;
            let text = args[flag].text.to_ascii_lowercase();
            let inline = text.len() > 2 && text.starts_with("/c ");
            let rest = &args[flag + usize::from(!inline)..];
            let mut joined = join(rest);
            if inline {
                joined = joined.trim_start_matches(['/', 'c', 'C']).to_owned();
            }
            script(&joined, Dialect::Cmd, context, depth)
        }
        "cd" | "pushd" | "chdir" | "set-location" | "sl" => {
            let target = args.iter().find(|arg| !arg.text.starts_with('-'));
            context.cwd = match target {
                Some(target) if !target.dynamic && !target.text.starts_with('~') => {
                    resolve(target, context)
                }
                _ => None,
            };
            None
        }
        "popd" | "pop-location" => {
            context.cwd = None;
            None
        }
        "pkill" => Some("machine-wide pkill".to_owned()),
        "killall" => Some("machine-wide killall".to_owned()),
        "kill" => (args.iter().skip(1).any(|arg| arg.text == "-1"))
            .then(|| "a kill of every process".to_owned()),
        "taskkill" => (args.iter().any(|arg| {
            let text = arg.text.to_ascii_lowercase();
            text == "/im" || text == "//im"
        }))
        .then(|| "machine-wide taskkill by name".to_owned()),
        "stop-process" | "spps" => {
            let by_id = args.iter().any(|arg| {
                abbreviates(&arg.text, "-id") || arg.text.chars().all(|c| c.is_ascii_digit())
            });
            (!by_id).then(|| "Stop-Process by name".to_owned())
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

fn powershell(
    args: &[Word],
    stdin: Option<&str>,
    context: &mut Context,
    depth: usize,
) -> Option<String> {
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        let text = arg.text.as_str();
        if abbreviates(text, "-command") || text.eq_ignore_ascii_case("/c") {
            return script(&join(&args[at + 1..]), Dialect::Ps, context, depth);
        }
        // Encoded text and script files are known misses.
        if abbreviates(text, "-encodedcommand") || abbreviates(text, "-file") {
            return None;
        }
        if ["-executionpolicy", "-windowstyle", "-workingdirectory"]
            .iter()
            .any(|option| abbreviates(text, option))
            || ["-inputformat", "-outputformat", "-configurationname"]
                .iter()
                .any(|option| abbreviates(text, option))
        {
            at += 2;
        } else if text.starts_with('-') {
            at += 1;
        } else {
            // A bare argument runs as a command, on Windows PowerShell.
            return script(&join(&args[at..]), Dialect::Ps, context, depth);
        }
    }
    stdin.and_then(|text| script(text, Dialect::Ps, context, depth))
}

const UNSCOPED_PATHS: [&str; 7] = [".", "./", "*", ":/", ":", "/", ":/*"];

fn git(args: &[Word], piped: bool) -> Option<String> {
    let mut rest = args;
    while let Some(first) = rest.first() {
        let text = first.text.as_str();
        if text == "-c" && rest.get(1).is_some_and(|c| c.text.starts_with("alias.")) {
            return Some("a git alias defined on the command line".to_owned());
        }
        let takes_value = matches!(
            text,
            "-C" | "-c"
                | "--git-dir"
                | "--work-tree"
                | "--namespace"
                | "--exec-path"
                | "--super-prefix"
                | "--config-env"
        );
        if takes_value {
            rest = rest.get(2..).unwrap_or_default();
        } else if text.starts_with('-') {
            rest = &rest[1..];
        } else {
            break;
        }
    }
    let (sub, args) = rest.split_first()?;
    if sub.dynamic {
        return Some("a git command hidden in a variable".to_owned());
    }
    match sub.text.as_str() {
        "push" => {
            if piped {
                return Some("a git push whose arguments arrive at run time".to_owned());
            }
            (args.iter().any(|arg| is_force_push_argument(&arg.text)))
                .then(|| "force-push".to_owned())
        }
        "reset" => (args.iter().any(|arg| arg.text == "--hard")).then(|| "reset --hard".to_owned()),
        "checkout" => {
            let dashes = args.iter().position(|arg| arg.text == "--");
            let unscoped = match dashes {
                Some(at) => {
                    args.len() == at + 1
                        || args[at + 1..]
                            .iter()
                            .any(|arg| UNSCOPED_PATHS.contains(&arg.text.as_str()))
                }
                None => args
                    .iter()
                    .any(|arg| UNSCOPED_PATHS.contains(&arg.text.as_str())),
            };
            unscoped.then(|| "an unscoped git checkout".to_owned())
        }
        "restore" => {
            let staged = args.iter().any(|a| a.text == "--staged" || a.text == "-S");
            let worktree = args
                .iter()
                .any(|a| a.text == "--worktree" || a.text == "-W");
            if staged && !worktree {
                return None;
            }
            let paths = after_options(args, &["-s", "--source"]);
            let has_paths = paths.iter().any(|a| !a.text.starts_with('-'));
            (!has_paths
                || paths
                    .iter()
                    .any(|a| UNSCOPED_PATHS.contains(&a.text.as_str())))
            .then(|| "an unscoped git restore".to_owned())
        }
        _ => None,
    }
}

fn is_force_push_argument(text: &str) -> bool {
    if text.starts_with('+') || text == "--mirror" {
        return true;
    }
    if let Some(long) = text.strip_prefix("--") {
        return long == "force" || long.starts_with("force-");
    }
    // `origin +main:main` is above; short flags may cluster, as in `-uf`.
    text.strip_prefix('-')
        .is_some_and(|short| short.contains('f') && short.chars().all(|c| c.is_ascii_alphabetic()))
        || text.contains(":+")
}

/// By operation, reads named: anything else on `gh` posts, changes or deletes something,
/// or is new to this list, which is the same to a guard that can't tell.
fn gh(args: &[Word]) -> Option<String> {
    let denied = || Some("a gh call that posts or changes something".to_owned());
    let positional: Vec<&Word> = {
        let mut positional = Vec::new();
        let mut rest = args;
        while let Some((first, tail)) = rest.split_first() {
            if matches!(first.text.as_str(), "-R" | "--repo") {
                rest = tail.get(1..).unwrap_or_default();
                continue;
            }
            if !first.text.starts_with('-') {
                positional.push(first);
            }
            rest = tail;
        }
        positional
    };
    let group = positional.first()?;
    if group.dynamic {
        return denied();
    }
    match group.text.as_str() {
        "help" | "version" | "status" | "search" | "browse" | "completion" => return None,
        "api" => return gh_api(args, denied),
        _ => {}
    }
    let verb = positional.get(1)?;
    let read = [
        "view", "list", "ls", "status", "diff", "checks", "watch", "download", "checkout", "clone",
        "get", "token", "verify",
    ];
    if !verb.dynamic && read.contains(&verb.text.as_str()) {
        None
    } else {
        denied()
    }
}

fn gh_api(args: &[Word], denied: impl Fn() -> Option<String>) -> Option<String> {
    let mut method = None;
    let mut fields = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let text = arg.text.as_str();
        if text == "-X" || text == "--method" {
            let value = iter.next()?;
            if value.dynamic {
                return denied();
            }
            method = Some(value.text.to_ascii_uppercase());
        } else if let Some(value) = text.strip_prefix("--method=").or(text.strip_prefix("-X")) {
            method = Some(value.to_ascii_uppercase());
        } else if ["-f", "-F", "--field", "--raw-field", "--input"].contains(&text)
            || text.starts_with("--field=")
            || text.starts_with("--raw-field=")
            || text.starts_with("--input=")
        {
            fields = true;
        }
    }
    let graphql = args.iter().any(|arg| arg.text == "graphql");
    if graphql {
        let hidden = args.iter().any(|arg| arg.dynamic);
        let mutation = args.iter().any(|arg| arg.text.contains("mutation"));
        return if hidden || mutation { denied() } else { None };
    }
    match method.as_deref() {
        Some("GET" | "HEAD") => None,
        Some(_) => denied(),
        None if fields => denied(),
        None => None,
    }
}

fn find(args: &[Word], dialect: Dialect, context: &mut Context, depth: usize) -> Option<String> {
    if args.iter().any(|arg| arg.text == "-delete") {
        return Some("find -delete".to_owned());
    }
    let exec = args
        .iter()
        .position(|arg| matches!(arg.text.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))?;
    let rest = &args[exec + 1..];
    let end = rest
        .iter()
        .position(|arg| arg.text == ";" || arg.text == "+")
        .unwrap_or(rest.len());
    simple(&rest[..end], None, dialect, context, depth, true)
}

fn remove(name: &str, args: &[Word], context: &Context, piped: bool) -> Option<String> {
    let lowered = |word: &Word| word.text.to_ascii_lowercase();
    let recursive = args.iter().any(|arg| {
        let text = lowered(arg);
        match name {
            "rm" => {
                text == "--recursive"
                    || abbreviates(&text, "-recurse")
                    || (text.starts_with('-')
                        && !text.starts_with("--")
                        && !abbreviates(&text, "-force")
                        && text.contains('r'))
            }
            "rmdir" | "rd" => {
                text == "/s" || text == "//s" || abbreviates(&text, "-recurse") || text == "--r"
            }
            "del" | "erase" => text == "/s" || text == "//s" || abbreviates(&text, "-recurse"),
            _ => abbreviates(&text, "-recurse"),
        }
    });
    if !recursive {
        return None;
    }
    let mut ended = false;
    let paths: Vec<&Word> = args
        .iter()
        .filter(|arg| {
            if arg.text == "--" && !ended {
                ended = true;
                return false;
            }
            let text = arg.text.as_str();
            let single_letter_slash = text.len() <= 3
                && text.starts_with('/')
                && matches!(name, "rmdir" | "rd" | "del" | "erase");
            ended || !(text.starts_with('-') || single_letter_slash)
        })
        .collect();
    let outside = || Some("a recursive delete outside the worktree".to_owned());
    if piped || paths.is_empty() {
        return Some("a recursive delete whose paths can't be read".to_owned());
    }
    for path in paths {
        if path.dynamic || path.text.contains('%') || path.text.starts_with('~') {
            return Some("a recursive delete whose paths can't be read".to_owned());
        }
        let Some(resolved) = resolve(path, context) else {
            return outside();
        };
        let Some(root) = &context.root else {
            return outside();
        };
        if !resolved.starts_with(&format!("{root}/")) {
            return outside();
        }
    }
    None
}

/// Lexical: an existing link isn't followed, which is also what `rm -rf` does to one.
fn resolve(path: &Word, context: &Context) -> Option<String> {
    let text = if context.windows {
        &path.raw
    } else {
        &path.text
    };
    let absolute = text.starts_with('/') || (context.windows && (is_rooted_windows(text)));
    if absolute {
        Some(normal(text, context.windows))
    } else {
        let cwd = context.cwd.as_ref()?;
        Some(normal(&format!("{cwd}/{text}"), context.windows))
    }
}

fn is_rooted_windows(text: &str) -> bool {
    has_drive(text) || text.starts_with('\\')
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
        let bytes = text.as_bytes();
        if bytes.len() >= 2
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && (bytes.len() == 2 || bytes[2] == b'/')
        {
            let drive = bytes[1] as char;
            text = format!("{drive}:{}", &text[2..]);
        }
    }
    let (prefix, rest) = if windows && text.as_bytes().get(1) == Some(&b':') {
        text.split_at(2)
    } else {
        ("", text.as_str())
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("{prefix}/{}", parts.join("/"))
}
