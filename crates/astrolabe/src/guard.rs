//! The pre-command matcher, provider-neutral and pure apart from `guard/scratch.rs`: an
//! accident guard, not a sandbox.
//! Its target is a command a careless agent would plausibly write, with one wrapping layer,
//! not one written to get past it. It reads what a command says, not what it would do. The
//! limits are rows in `tests/fixtures/guard-bypass.tsv`. A command that names a denied
//! program and can't be read is denied; everything else is allowed.
//! Later growth comes from a real entry in the denial log or a real incident, not from
//! imagined bypasses.
use serde_json::Value;

mod launch;
mod scratch;
pub use launch::{LAUNCH, correctable, launch_tool};
use std::collections::HashMap;

/// How deep wrappers and substitutions nest before the text counts as unparsed.
const MAX_DEPTH: usize = 8;
/// Marks a POSIX `$NAME` inside a word's text, around the name.
const VAR: char = '\u{0}';
/// The most values a variable or folder is tracked through before it counts as unknown.
const MAX_VALUES: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    Sh,
    Ps,
    Cmd,
}

#[derive(Debug, Clone, Default)]
struct Word {
    /// What the shell would pass, as far as it can be told. A POSIX `$NAME` is kept as the
    /// name between two `VAR` marks.
    text: String,
    /// Holds a variable or a substitution, so `text` isn't the final value.
    dynamic: bool,
    /// Holds what can't be expanded here: a substitution, a special parameter, or a
    /// PowerShell or cmd expansion.
    opaque: bool,
    substitutions: usize,
    /// `text`'s length when a `$(mktemp -d)` came, if one did.
    fresh_at: Option<usize>,
}

/// One command as parsed, and whether it certainly runs in the shell that reads the next:
/// at top level, not after `&&`, `||` or `|`, not in a subshell, block or substitution, and
/// not piped or backgrounded.
struct Command {
    words: Vec<Word>,
    definite: bool,
    /// At top level, outside any subshell, block, substitution, pipeline or background.
    top: bool,
    /// Joined to the command before by `&&`, so it runs only if that one succeeded.
    chained: bool,
    /// From here on a loop or function has begun.
    looped: bool,
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
    commands: Vec<Command>,
    broken: bool,
    literal_error: Option<&'static str>,
    /// Open `if`, `case`, loop and `{` blocks.
    block: usize,
    /// A loop or a function: a back edge, so order in the text isn't order of running.
    loops: bool,
    /// The next command starts a list: after the start, `;` or a newline.
    lead: bool,
    /// The last separator was `&` or `|`, so a newline continues its list.
    joined: bool,
    /// The last separator was `&&`.
    and: bool,
    depth: usize,
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
        literal_error: None,
        block: 0,
        loops: false,
        lead: true,
        joined: false,
        and: false,
        depth,
    };
    parser.list(None, depth);
    parser
}

const ONE_COMMAND: &str =
    "run one literal command alone, without chaining, redirection or expansion";

/// The guard's parser, restricted to a single command whose words are all literal.
/// Parsing retains structural errors even when only one nonempty command remains.
pub(crate) fn literal_words(text: &str, dialect: Dialect) -> Result<Vec<String>, String> {
    let parsed = parse(text, dialect, 0);
    if let Some(why) = parsed.literal_error {
        return Err(why.into());
    }
    if parsed.broken {
        return Err("an unfinished quote".into());
    }
    let words = || parsed.commands.iter().flat_map(|command| &command.words);
    if words().any(|word| word.dynamic) {
        return Err("shell expansion isn't a literal command".into());
    }
    let [command] = parsed.commands.as_slice() else {
        return Err(ONE_COMMAND.into());
    };
    Ok(command.words.iter().map(|word| word.text.clone()).collect())
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
        let mut opened = None;
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                c if c == self.escape => {
                    if self.peek().is_none() {
                        self.literal_error.get_or_insert("an unfinished escape");
                    }
                    let next = self.peek().filter(|next| *next != '\n');
                    self.at += 1;
                    if let Some(next) = next {
                        word.get_or_insert_default().text.push(next);
                    }
                }
                '@' if self.dialect == Dialect::Ps && word.is_none() && self.here_string() => {
                    word = self.here_body(depth);
                }
                '\'' => self.single_quoted(word.get_or_insert_default()),
                '"' => self.double_quoted(word.get_or_insert_default(), depth),
                '$' => self.dollar(word.get_or_insert_default(), depth),
                ' ' | '\t' | '\r' => {
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                }
                '#' if word.is_none() && self.dialect != Dialect::Cmd => {
                    self.literal_error.get_or_insert(ONE_COMMAND);
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.at += 1;
                    }
                }
                '<' | '>' => {
                    self.literal_error.get_or_insert(ONE_COMMAND);
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
                    self.literal_error.get_or_insert(ONE_COMMAND);
                    end_word(&mut word, &mut command, &mut expect, &mut heredocs);
                    // A redirect with no target word before this must not skip the next
                    // command's program, as in `cat <(git push --force)`.
                    expect = Expect::Word;
                    let top = until.is_none() && parens == 0;
                    // A lone `|` or `&`, not `||` or `&&`: a pipeline or the background.
                    let doubled = |at: usize| self.chars.get(at) == Some(&c);
                    let second = matches!(c, '|' | '&') && self.at >= 2 && doubled(self.at - 2);
                    let apart = matches!(c, '|' | '&') && !doubled(self.at) && !second;
                    let and = c == '&' && doubled(self.at);
                    self.finish(&mut command, top, apart);
                    self.lead = matches!(c, ';') || (c == '\n' && !self.joined);
                    self.joined = matches!(c, '|' | '&') || (self.joined && c == '\n');
                    if !second && !(c == '\n' && self.joined) {
                        self.and = and;
                    }
                    match c {
                        c if until == Some(c) && parens == 0 => return,
                        '(' => {
                            parens += 1;
                            opened = Some(self.at);
                        }
                        ')' => {
                            // `name()`: a function, run later from wherever it's called.
                            let body = opened.map(|at| &self.chars[at..self.at - 1]);
                            self.loops |= body.is_some_and(|b| b.iter().all(|c| c.is_whitespace()));
                            parens = parens.saturating_sub(1);
                        }
                        '\n' => self.read_heredocs(&mut heredocs),
                        _ => {}
                    }
                }
                '`' if self.dialect == Dialect::Sh => {
                    self.substitution(word.get_or_insert_default(), '`', depth);
                }
                '*' | '?' | '[' | '{' | '~' if self.dialect == Dialect::Sh => {
                    self.literal_error
                        .get_or_insert("shell expansion isn't a literal command");
                    word.get_or_insert_default().text.push(c);
                }
                '@' if self.dialect == Dialect::Ps && word.is_none() => {
                    self.literal_error
                        .get_or_insert("shell expansion isn't a literal command");
                    word.get_or_insert_default().text.push(c);
                }
                '%' | '!' if self.dialect == Dialect::Cmd => {
                    self.literal_error
                        .get_or_insert("shell expansion isn't a literal command");
                    word.get_or_insert_default().text.push(c);
                }
                _ => word.get_or_insert_default().text.push(c),
            }
        }
        end_word(&mut word, &mut command, &mut expect, &mut heredocs);
        self.finish(&mut command, until.is_none() && parens == 0, false);
        self.broken |= until.is_some();
    }

    /// `top`: outside any subshell or substitution; `apart`: piped or backgrounded.
    fn finish(&mut self, command: &mut Vec<Word>, top: bool, apart: bool) {
        let Some(first) = command.first() else { return };
        let top = top && self.depth == 0 && self.block == 0;
        match first.text.as_str() {
            "for" | "while" | "until" | "select" | "function" => {
                self.loops = true;
                self.block += 1;
            }
            "if" | "case" | "{" => self.block += 1,
            "fi" | "esac" | "done" | "}" => self.block = self.block.saturating_sub(1),
            _ => {}
        }
        self.commands.push(Command {
            words: std::mem::take(command),
            definite: top && self.lead && !apart,
            top: top && !apart,
            chained: self.and,
            looped: self.loops,
        });
        self.joined = false;
        self.and = false;
    }

    /// Just past a PowerShell `@`: whether a here-string starts, `@'` or `@"` and then
    /// nothing but the line's end.
    fn here_string(&self) -> bool {
        let rest = &self.chars[self.at..];
        let Some((quote, rest)) = rest.split_first() else {
            return false;
        };
        let end = rest.iter().position(|c| *c == '\n');
        matches!(quote, '\'' | '"')
            && end.is_some_and(|end| rest[..end].iter().all(|c| matches!(c, ' ' | '\t' | '\r')))
    }

    /// A here-string's body, as one word: text, so prose or a script in it isn't read as
    /// commands, though a `@"` one's `$(...)` still is. It ends at a line starting `'@` or
    /// `"@`.
    fn here_body(&mut self, depth: usize) -> Option<Word> {
        self.literal_error
            .get_or_insert("shell expansion isn't a literal command");
        let quote = self.chars[self.at];
        while self.peek().is_some_and(|c| c != '\n') {
            self.at += 1;
        }
        self.at += 1;
        let mut word = Word::default();
        let mut line_start = true;
        let mut indent = false;
        while let Some(c) = self.peek() {
            let closes = c == quote && self.chars.get(self.at + 1) == Some(&'@');
            if line_start && closes {
                self.at += 2;
                return Some(word);
            }
            // An indented close may end the string in some PowerShell, so where the body
            // ends isn't certain: read as unparsed, a denied program in it is denied.
            if indent && closes {
                self.broken = true;
            }
            self.at += 1;
            indent = (line_start || indent) && matches!(c, ' ' | '\t');
            line_start = c == '\n';
            match c {
                '`' if quote == '"' => {
                    if let Some(next) = self.peek() {
                        self.at += 1;
                        word.text.push(next);
                    }
                }
                '$' if quote == '"' => self.dollar(&mut word, depth),
                _ => word.text.push(c),
            }
        }
        self.broken = true;
        Some(word)
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

    fn single_quoted(&mut self, word: &mut Word) {
        while let Some(c) = self.peek() {
            self.at += 1;
            if c == '\'' {
                if self.dialect != Dialect::Ps || self.peek() != Some('\'') {
                    return;
                }
                self.at += 1;
            }
            word.text.push(c);
        }
        self.broken = true;
    }

    fn double_quoted(&mut self, word: &mut Word, depth: usize) {
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '"' if self.dialect == Dialect::Ps && self.peek() == Some('"') => {
                    self.at += 1;
                    word.text.push('"');
                }
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
        self.literal_error
            .get_or_insert("shell expansion isn't a literal command");
        if self.peek() == Some('(') {
            self.at += 1;
            self.substitution(word, ')', depth);
            return;
        }
        if self.dialect == Dialect::Sh
            && let Some(name) = self.variable_name()
        {
            word.dynamic = true;
            word.text
                .extend([VAR].into_iter().chain(name.chars()).chain([VAR]));
            // zsh reads `$S:h` as a modifier and `$S[1]` as a subscript, so neither is the
            // plain value followed by text.
            let next = self.chars.get(self.at + 1);
            let modifier =
                self.peek() == Some(':') && next.is_some_and(|c| c.is_ascii_alphabetic());
            word.opaque |= modifier || self.peek() == Some('[');
            return;
        }
        let dynamic = (self.peek())
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '{' | '_' | '?' | '@' | '*' | '!'));
        word.dynamic |= dynamic;
        word.opaque |= dynamic;
        word.text.push('$');
    }

    /// The `NAME` of a `$NAME` or `${NAME}` just past its `$`, consumed; None for anything
    /// else, such as `$1`, `$?` or `${NAME:-x}`.
    fn variable_name(&mut self) -> Option<String> {
        let braced = self.peek() == Some('{');
        let start = self.at + usize::from(braced);
        let name_char = |c: &char| c.is_ascii_alphanumeric() || *c == '_';
        let len = self.chars[start..]
            .iter()
            .take_while(|c| name_char(c))
            .count();
        let first = self.chars.get(start)?;
        if len == 0
            || first.is_ascii_digit()
            || (braced && self.chars.get(start + len) != Some(&'}'))
        {
            return None;
        }
        self.at = start + len + usize::from(braced);
        Some(self.chars[start..start + len].iter().collect())
    }

    fn substitution(&mut self, word: &mut Word, until: char, depth: usize) {
        word.dynamic = true;
        word.opaque = true;
        word.substitutions += 1;
        let start = self.commands.len();
        let (lead, joined, block, and) = (self.lead, self.joined, self.block, self.and);
        self.lead = true;
        self.list(Some(until), depth + 1);
        (self.lead, self.joined, self.block, self.and) = (lead, joined, block, and);
        if let [inner] = &self.commands[start..]
            && makes_temp_folder(&inner.words)
        {
            word.fresh_at = Some(word.text.len());
        }
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
/// to be inside, so it is denied. `session` is the id the app gave the hook, which names
/// the session's own scratch folder (`guard/scratch.rs`).
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub root: Option<String>,
    pub cwd: Option<String>,
    pub session: Option<String>,
}

/// A variable's possible value: a path, or a folder `mktemp -d` just made.
#[derive(Debug, Clone, PartialEq)]
enum Possible {
    Path(String),
    Fresh,
}

struct Context {
    root: Option<String>,
    /// Every folder the command could be in by now, as a `cd` adds one rather than
    /// replacing, so a failed or subshell `cd` is judged too. None once one can't be told.
    cwds: Option<Vec<String>>,
    windows: bool,
    session: Option<String>,
    /// POSIX variables set in this command, each with every value it could hold; None for
    /// one that could hold anything. Unset here means the environment's: unknown.
    vars: HashMap<String, Option<Vec<Possible>>>,
    /// A loop, a function or a `source` was seen: order stops meaning anything, so no
    /// variable or `cd` is followed from here on.
    loops: bool,
    /// The folders a top-level `cd` led to, while the `&&` chain after it lasts: there it
    /// must have succeeded.
    chain: Option<Vec<String>>,
    /// Variables set in the current top-level `&&` chain, which hold in the rest of it.
    chain_vars: HashMap<String, Option<Vec<Possible>>>,
    /// The command being read is at top level, and joined by `&&` to the one before.
    top: bool,
    chained: bool,
    /// The folder the hook gave, before any `cd`.
    start: Option<String>,
}

/// The name of the field a shell tool's command is in: `command` if the input has one at
/// all, else `cmd`. Both apps send `command`; `cmd`, a command given as a list and a folder
/// inside the tool's input are shapes no app is recorded sending (`docs/hosts.md`), kept
/// because they cost little and deny safely.
pub fn command_key(input: &Value) -> &'static str {
    if input.get("command").is_some() {
        "command"
    } else {
        "cmd"
    }
}

/// A shell tool's command, as the guard reads it. The one reader, so whatever checks,
/// rewrites, records or approves a command looks at the field the guard judges.
pub fn command_field(input: &Value) -> Option<&Value> {
    input.get(command_key(input))
}

/// The folder a tool's own input names for its command, if it names one. The one reader:
/// the guard resolves it against the hook's `cwd`, as `hook::folder` does for the rest.
pub fn tool_folder(input: &Value) -> Option<&str> {
    ["workdir", "cwd", "working_directory"]
        .iter()
        .find_map(|key| input.get(*key)?.as_str())
}

/// Whether the guard can judge this command: text, or a list of words that are all text.
pub fn readable_command(command: &Value) -> bool {
    match command {
        Value::String(_) => true,
        Value::Array(parts) => parts.iter().all(Value::is_string),
        _ => false,
    }
}

/// A command as one line of text, whichever form it came in.
pub fn command_text(command: &Value) -> String {
    match command {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Tools whose `command` isn't a shell's: Codex's patch tool puts the patch text there. A
/// tool unknown here that has a `command` is read as a shell, so a new one stays covered.
pub fn not_a_shell(tool: &str) -> bool {
    tool.starts_with("mcp__") || tool == "apply_patch"
}

/// Whether a tool's commands are PowerShell: the one answer for the guard's parse, the
/// proof steps, the denial log and the ticket rewrite. A tool named for it, in any case,
/// or Codex on Windows, which runs its shell commands there.
pub fn powershell(tool: &str, codex: bool) -> bool {
    tool.eq_ignore_ascii_case("powershell") || (cfg!(windows) && codex)
}

/// `decide_in` for a caller that knows only the tool's name.
pub fn decide(tool: &str, input: &Value, scope: &Scope) -> Option<String> {
    decide_in(powershell(tool, false), tool, input, scope)
}

/// The reason to deny, or None to stay out of the way. Never an allow: a hook's allow
/// would remove the client's approval prompt. `powershell` is `powershell`'s answer for
/// the session the command came from.
pub fn decide_in(powershell: bool, tool: &str, input: &Value, scope: &Scope) -> Option<String> {
    if let Some(reason) = launch::session_tool(tool, input) {
        return Some(reason);
    }
    if not_a_shell(tool) {
        return None;
    }
    let windows = (scope.root.as_deref()).is_some_and(|r| has_drive(r) || r.starts_with(r"\\"));
    let mut context = Context {
        root: scope.root.as_deref().map(|root| normal(root, windows)),
        cwds: scope.cwd.as_deref().map(|cwd| vec![normal(cwd, windows)]),
        windows,
        session: scope.session.clone(),
        vars: HashMap::new(),
        loops: false,
        chain: None,
        chain_vars: HashMap::new(),
        top: false,
        chained: false,
        start: None,
    };
    // A tool's own working directory, which the hook's `cwd` doesn't reflect.
    if let Some(workdir) = tool_folder(input) {
        context.cwds = resolve(workdir, &context);
    }
    context.start = context.cwds.as_ref().and_then(|cwds| cwds.first().cloned());
    let dialect = if powershell { Dialect::Ps } else { Dialect::Sh };
    match command_field(input)? {
        Value::String(text) => script(text, dialect, &mut context, 0),
        Value::Array(parts) => {
            let word = |part: &Value| Word {
                text: part.as_str().unwrap_or_default().to_owned(),
                ..Default::default()
            };
            let words: Vec<Word> = parts.iter().map(word).collect();
            simple(&words, false, dialect, &mut context, 0, false)
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
    // PowerShell's and cmd's loops and script blocks aren't parsed: any brace, or cmd at all,
    // counts as one.
    let blocks = match dialect {
        Dialect::Sh => false,
        Dialect::Ps => text.contains('{'),
        Dialect::Cmd => true,
    };
    context.loops |= blocks;
    // A `cd` anywhere in a loop can repeat before a command written above it, so once a loop
    // begins with one in what follows, the folder is no longer known.
    let looped = parsed.commands.iter().position(|command| command.looped);
    let moves = looped.is_some_and(|at| {
        let words = parsed.commands[at..]
            .iter()
            .flat_map(|command| &command.words);
        words
            .map(|word| program(&word.text))
            .any(|name| CHANGES_FOLDER.contains(&name.as_str()))
    });
    if blocks
        && text
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .any(|w| CHANGES_FOLDER.contains(&w))
    {
        context.cwds = None;
    }
    let run = |command: &Command| {
        if command.looped && !context.loops && moves {
            context.cwds = None;
            context.chain = None;
        }
        context.loops |= command.looped;
        // Only a `cd` earlier in the same `&&` chain at top level fixes the folder.
        if !(command.top && command.chained) {
            context.chain = None;
        }
        context.top = command.top;
        context.chained = command.top && command.chained;
        if !context.chained {
            context.chain_vars.clear();
        }
        simple(
            &command.words,
            command.definite,
            dialect,
            context,
            depth,
            false,
        )
    };
    (parsed.commands.iter())
        .find_map(run)
        .or_else(|| (parsed.broken && names_denied_program(text)).then(unparsed))
}

/// `mktemp -d`, which makes a new folder, as against `-u`, which only names one.
fn makes_temp_folder(words: &[Word]) -> bool {
    let Some((first, args)) = words.split_first() else {
        return false;
    };
    let flag = |short: char, long: &str| {
        (args.iter())
            .any(|a| short_flags(&a.text).is_some_and(|f| f.contains(short)) || a.text == long)
    };
    program(&first.text) == "mktemp"
        && !words.iter().any(|w| w.dynamic)
        && flag('d', "--directory")
        && !flag('u', "--dry-run")
}

/// Every value a word could have, or None if that can't be told. A bare `$T` set from
/// `$(mktemp -d)` is the folder it made; anything appended to it isn't, as a failed
/// `mktemp` leaves `$T/x` as `/x`.
fn expand(word: &Word, context: &Context) -> Option<Vec<Possible>> {
    if word.substitutions == 1 && word.fresh_at == Some(0) && word.text.is_empty() {
        return Some(vec![Possible::Fresh]);
    }
    if word.opaque {
        return None;
    }
    if !word.dynamic {
        return Some(vec![Possible::Path(word.text.clone())]);
    }
    if context.loops {
        return None;
    }
    let parts: Vec<&str> = word.text.split(VAR).collect();
    if parts.len().is_multiple_of(2) {
        return None;
    }
    let lookup = |name: &str| {
        let value = context.chain_vars.get(name).or(context.vars.get(name));
        value.cloned().flatten()
    };
    if let ["", name, ""] = parts[..] {
        return lookup(name);
    }
    let mut values = vec![String::new()];
    for (at, part) in parts.iter().enumerate() {
        if at % 2 == 0 {
            values.iter_mut().for_each(|v| v.push_str(part));
            continue;
        }
        let mut next = Vec::new();
        for value in lookup(part)? {
            let Possible::Path(path) = value else {
                return None;
            };
            next.extend(values.iter().map(|v| format!("{v}{path}")));
        }
        values = next;
        if values.len() > MAX_VALUES {
            return None;
        }
    }
    Some(values.into_iter().map(Possible::Path).collect())
}

/// `NAME=value`, run where the rest of the command sees it if `definite`; otherwise it may
/// not have run, so the old values stay possible.
fn assign(word: &Word, definite: bool, context: &mut Context) {
    let Some((name, value)) = word.text.split_once('=') else {
        return;
    };
    let prefix = name.len() + 1;
    let value = Word {
        text: value.to_owned(),
        fresh_at: word.fresh_at.and_then(|at| at.checked_sub(prefix)),
        ..word.clone()
    };
    // `S=a:~/x` holds the home folder.
    let new = expand(&value, context).filter(|_| !value.text.contains('~'));
    if context.chained {
        context.chain_vars.insert(name.to_owned(), new.clone());
    }
    let merged = match (definite, context.vars.get(name).cloned().flatten(), new) {
        (true, _, new) => new,
        (false, Some(mut old), Some(new)) => {
            for value in new {
                if !old.contains(&value) {
                    old.push(value);
                }
            }
            Some(old)
        }
        _ => None,
    };
    let merged = merged.filter(|values| values.len() <= MAX_VALUES);
    context.vars.insert(name.to_owned(), merged);
}

/// A command that names a variable set here may change it (`read S`, `for S in`, `unset
/// S`, `S+=x`), so it's unknown from then on.
fn forget_named(words: &[Word], context: &mut Context) {
    let named = |name: &str, word: &Word| {
        let rest = word.text.strip_prefix(name);
        rest.is_some_and(|rest| rest.is_empty() || rest.starts_with("+=") || rest.starts_with('['))
    };
    for (name, values) in context.vars.iter_mut().chain(context.chain_vars.iter_mut()) {
        if words.iter().any(|word| named(name, word)) {
            *values = None;
        }
    }
}

/// A `cd`: each folder it could lead to is added to those the command could be in.
/// `certain`: the `cd` itself runs whenever what follows it in an `&&` chain does: at the
/// start of a list or joined by `&&`, and not negated. After `||` or `!` it may not have run.
fn change_folder(
    name: &str,
    args: &[Word],
    certain: bool,
    dialect: Dialect,
    context: &mut Context,
) {
    let mut iter = args.iter();
    let mut target = None;
    while let Some(arg) = iter.next() {
        let text = arg.text.to_ascii_lowercase();
        if matches!(text.as_str(), "-path" | "-literalpath" | "-lp" | "--") {
            target = iter.next();
            break;
        }
        // `-P` resolves links before a `..`, which the guard doesn't. `-` alone is the last
        // folder, which isn't known.
        if short_flags(&arg.text).is_some_and(|flags| flags.contains('P')) {
            target = None;
            break;
        }
        if !text.starts_with('-') {
            target = Some(arg);
            break;
        }
    }
    let ends = |value: &Possible| match value {
        Possible::Path(p) if !p.is_empty() && !p.contains('~') => resolve(p, context),
        _ => None,
    };
    let leads = target
        .filter(|_| !context.loops && dialect != Dialect::Cmd)
        .filter(|_| !matches!(name, "popd" | "pop-location"))
        .and_then(|target| expand(target, context))
        .and_then(|values| values.iter().map(ends).collect::<Option<Vec<_>>>());
    let leads: Option<Vec<String>> = leads.map(|leads| leads.into_iter().flatten().collect());
    context.chain = leads.clone().filter(|_| context.top && certain);
    context.cwds = match (leads, context.cwds.take()) {
        (Some(leads), Some(mut cwds)) => {
            for folder in leads {
                if !cwds.contains(&folder) {
                    cwds.push(folder);
                }
            }
            Some(cwds).filter(|cwds| cwds.len() <= MAX_VALUES)
        }
        _ => None,
    };
}

const CHANGES_FOLDER: [&str; 8] = [
    "cd",
    "pushd",
    "popd",
    "chdir",
    "set-location",
    "sl",
    "push-location",
    "pop-location",
];

const KEYWORDS: [&str; 12] = [
    "if", "then", "elif", "else", "fi", "do", "done", "while", "until", "!", "{", "}",
];

fn is_assignment(word: &Word) -> bool {
    let name = word.text.split_once('=').map(|(name, _)| name);
    name.is_some_and(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// Lowercase, no folder, no `.exe`, `.cmd` or `.ps1` (npm installs a CLI on Windows as all
/// three). Case folded because the Mac's and Windows' file systems would find the program
/// either way.
fn program(word: &str) -> String {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let name = name.to_ascii_lowercase();
    [".exe", ".cmd", ".ps1"]
        .iter()
        .find_map(|ext| name.strip_suffix(ext))
        .unwrap_or(&name)
        .to_owned()
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
        "command" if args.first().is_some_and(|a| a.text == "-v") => return Some(&[]),
        "exec" => &["-a"],
        "command" | "builtin" | "nohup" | "setsid" | "time" => &[],
        "npx" => &["-p", "--package", "-c", "--call"],
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

/// One command. `piped` means more arguments arrive at run time, from `xargs` or `find`;
/// `definite`, that the shell reading the next command sees what this one sets.
fn simple(
    words: &[Word],
    definite: bool,
    dialect: Dialect,
    context: &mut Context,
    depth: usize,
    piped: bool,
) -> Option<String> {
    let mut words = words;
    let mut assignments = Vec::new();
    let mut negated = false;
    loop {
        match words {
            [first, ..] if KEYWORDS.contains(&first.text.as_str()) => {
                negated |= first.text == "!";
                words = &words[1..];
            }
            [first, ..] if is_assignment(first) => {
                assignments.push(first);
                words = &words[1..];
            }
            // PowerShell's `$x = git ...`.
            [first, second, ..] if first.dynamic && second.text == "=" => words = &words[2..],
            _ => break,
        }
    }
    // Only a command of assignments alone sets them for what follows.
    if dialect == Dialect::Sh && words.is_empty() {
        for word in assignments {
            assign(word, definite, context);
        }
    }
    forget_named(words, context);
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
        context.cwds = None;
    }
    if let Some(rest) = wrapped(&name, args) {
        return simple(
            rest,
            false,
            dialect,
            context,
            depth,
            piped || name == "xargs",
        );
    }
    let lowered = |arg: &Word| arg.text.to_ascii_lowercase();
    match name.as_str() {
        "export" | "declare" | "local" | "readonly" | "typeset" if dialect == Dialect::Sh => {
            // An option can change what the value means (`-n` makes a name a reference to
            // another variable), so with one the names are unknown.
            let plain = !args.iter().any(|arg| arg.text.starts_with(['-', '+']));
            for arg in args.iter().filter(|arg| is_assignment(arg)) {
                if plain {
                    assign(arg, definite, context);
                } else if let Some((name, _)) = arg.text.split_once('=') {
                    context.vars.insert(name.to_owned(), None);
                    context.chain_vars.insert(name.to_owned(), None);
                }
            }
            None
        }
        // What runs isn't read here and could set anything.
        "source" | "." => {
            context.loops = true;
            context.cwds = None;
            None
        }
        "eval" | "iex" | "invoke-expression" => {
            if args.iter().any(|arg| arg.dynamic) {
                context.loops = true;
                context.cwds = None;
            }
            script(&join(args), dialect, context, depth)
        }
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
        name if CHANGES_FOLDER.contains(&name) => {
            let certain = (definite || context.chained) && !negated;
            change_folder(name, args, certain, dialect, context);
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
        "start-process" | "saps" | "start" => started(args, dialect, context, depth, piped),
        "codex" => launch::codex(args),
        "claude" => launch::claude(args),
        "git" => git(args, piped),
        "gh" => gh(args, piped),
        "find" => find(args, dialect, context, depth),
        "rm" | "remove-item" | "ri" | "rmdir" | "rd" | "del" | "erase" => {
            remove(&name, args, context, piped)
        }
        _ => None,
    }
}

/// PowerShell's `Start-Process <file> -ArgumentList <args>`: the program it starts with its
/// arguments, read as one command. A list such as `'a','b'` arrives as one word `a,b`.
fn started(
    args: &[Word],
    dialect: Dialect,
    context: &mut Context,
    depth: usize,
    piped: bool,
) -> Option<String> {
    let mut file = None;
    let mut list = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.text.to_ascii_lowercase().as_str() {
            "-filepath" | "-file" | "-path" => file = iter.next(),
            "-argumentlist" | "-args" | "-arguments" => list.extend(iter.next()),
            // A switch takes no value; any other option is taken to, which at worst drops a
            // word and so never adds a denial.
            "-wait" | "-nonewwindow" | "-passthru" | "-usenewenvironment" => {}
            t if t.starts_with('-') => {
                iter.next();
            }
            _ if file.is_none() => file = Some(arg),
            _ => list.push(arg),
        }
    }
    let mut words = vec![file?.clone()];
    for arg in list {
        for part in arg.text.split([',', ' ']).filter(|p| !p.is_empty()) {
            words.push(Word {
                text: part.to_owned(),
                ..arg.clone()
            });
        }
    }
    simple(&words, false, dialect, context, depth, piped)
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

/// By operation, reads and a plain `gh run rerun` named: anything else on `gh` posts, changes
/// or deletes something, or is new here, as a variable is: the same to a guard that can't tell.
fn gh(args: &[Word], piped: bool) -> Option<String> {
    const READS: [&str; 13] = [
        "view", "list", "ls", "status", "diff", "checks", "watch", "download", "checkout", "clone",
        "get", "token", "verify",
    ];
    let r = (args.get(..2)).is_some_and(|a| a.iter().all(|w| !w.dynamic) && join(a) == "run rerun");
    let mut positional = args.iter().filter(|arg| !arg.text.starts_with('-'));
    let denied = match positional.next()?.text.as_str() {
        "help" | "version" | "status" | "search" | "browse" | "completion" => false,
        "api" => gh_api_writes(args),
        _ => !READS.contains(&positional.next()?.text.as_str()),
    };
    (denied && (piped || !r)).then(|| "a gh call that posts or changes something".to_owned())
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
        .find_map(|(at, _)| simple(&args[at + 1..], false, dialect, context, depth, true))
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
    // In the worktree, or in this session's own scratch folder by its real location; there
    // a `..` could climb out of a link, so none is taken.
    let in_reach = |path: &str| {
        let root = context.root.as_ref();
        let in_root = |p: &String| root.is_some_and(|r| p.starts_with(&format!("{r}/")));
        let climbs = path.split(['/', '\\']).any(|part| part == "..");
        let session = context.session.as_deref();
        // The guard looks before the command runs, so a link the command makes first isn't
        // there yet: only a plain last name may be missing, with nothing after it that `rm`
        // would follow (a trailing separator) or expand (a glob).
        let follows = path.ends_with(['/', '\\'])
            || path.ends_with("/.")
            || path.ends_with("\\.")
            || path.contains(['*', '?', '[']);
        let own = |p: &String| {
            !climbs && !follows && session.is_some_and(|s| scratch::owned(p, s, context.windows))
        };
        // A `cd` may have gone through a link, which `..` then climbs out of for real.
        let moved = || {
            let cwds = context.chain.as_ref().or(context.cwds.as_ref());
            cwds.is_some_and(|cwds| cwds.iter().any(|cwd| Some(cwd) != context.start.as_ref()))
        };
        if climbs && moved() && !is_rooted(path, context) {
            return false;
        }
        resolve(path, context).is_some_and(|ends| ends.iter().all(|p| in_root(p) || own(p)))
    };
    // `@(...)` is not read. PowerShell takes `a,b` as two paths.
    let inside = |part: &str| !part.ends_with('@') && in_reach(part);
    // A variable's value is split and globbed if unquoted, so one holding a space or a
    // pattern isn't taken as one path.
    let plain = |path: &Word, value: &str| {
        !path.dynamic || !value.contains(|c: char| c.is_whitespace() || "*?[".contains(c))
    };
    let readable = |path: &&Word| {
        expand(path, context).is_some_and(|values| {
            values.iter().all(|value| match value {
                Possible::Fresh => true,
                Possible::Path(text) => {
                    plain(path, text)
                        && !text.contains('%')
                        // Bash expands a `~` after `=` or `:` too.
                        && !text.contains('~')
                        && text.split(',').filter(|p| !p.is_empty()).all(inside)
                }
            })
        })
    };
    let reason =
        "a recursive delete not plainly inside the worktree or this session's own scratch folder";
    let denied = piped || paths.is_empty() || !paths.iter().all(readable);
    (recursive && denied).then(|| reason.to_owned())
}

/// Where a path leads from each folder the command could be in. Lexical: an existing link
/// isn't resolved. See the misses in the table.
fn resolve(text: &str, context: &Context) -> Option<Vec<String>> {
    if is_rooted(text, context) {
        Some(vec![normal(text, context.windows)])
    } else {
        let at = |cwd: &String| normal(&format!("{cwd}/{text}"), context.windows);
        let cwds = context.chain.as_ref().or(context.cwds.as_ref())?;
        Some(cwds.iter().map(at).collect())
    }
}

fn is_rooted(text: &str, context: &Context) -> bool {
    text.starts_with('/') || (context.windows && (has_drive(text) || text.starts_with('\\')))
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

#[cfg(test)]
mod literal_tests {
    use super::{Dialect, literal_words};

    #[test]
    fn literal_words_follow_the_shells_quote_and_escape_rules() {
        for (dialect, command, expected) in [
            (
                Dialect::Sh,
                "gh issue create --title 'Owner'\\''s issue'",
                "Owner's issue",
            ),
            (
                Dialect::Ps,
                "gh issue create --title 'Owner''s issue'",
                "Owner's issue",
            ),
            (
                Dialect::Ps,
                "gh issue create --title \"Say \"\"yes\"\"\"",
                "Say \"yes\"",
            ),
            (
                Dialect::Sh,
                "gh issue create --title \"Say \\\"yes\\\"\"",
                "Say \"yes\"",
            ),
            (
                Dialect::Sh,
                "gh issue create --title 'a; $b > c'",
                "a; $b > c",
            ),
            (Dialect::Sh, "gh issue create --title one\\\ntwo", "onetwo"),
            (Dialect::Sh, "gh issue create --title ''", ""),
        ] {
            let words = literal_words(command, dialect).unwrap();
            assert_eq!(
                words,
                ["gh", "issue", "create", "--title", expected],
                "{command}"
            );
        }
    }

    #[test]
    fn only_a_single_complete_literal_command_can_get_an_approval() {
        for command in [
            "git push --force;",
            "git push --force &&",
            "(git push --force)",
            "git push --force > output",
            "git push --force 2>&1",
            "git push --force # comment",
            "git push --force\necho done",
            "git push --force $remote",
            "git push --force $(echo origin)",
            "git push --force `echo origin`",
            "git push --force *",
            "git push --force {a,b}",
            "git push --force 'unfinished",
            "git push --force \"unfinished",
            "git push --force \\",
            "",
        ] {
            assert!(literal_words(command, Dialect::Sh).is_err(), "{command}");
        }
        for command in [
            "gh @args",
            "gh issue create --title $title",
            "gh issue create --title 'unfinished",
        ] {
            assert!(literal_words(command, Dialect::Ps).is_err(), "{command}");
        }
    }
}
