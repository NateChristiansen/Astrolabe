//! The owner's home: the board, the notes tree beside it, and each note as a page. Served on
//! localhost by `astrolabe home --serve`, and written as `home.html` at the notes checkout
//! root whenever the notes change on this machine, from the same rows `astrolabe efforts
//! --json` returns, so it opens from a file on any machine too.

use crate::notes::{self, Notes};
use chrono::{DateTime, FixedOffset};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub const FILE: &str = "home.html";
/// The machine's look (theme and palette), beside the sync record in the state folder.
const LOOK_FILE: &str = "home.json";

/// Light or dark: the system's choice, or one fixed for a viewer that can't ask (the
/// Claude app's file pane always renders light).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub fn parse(word: &str) -> Option<Theme> {
        match word {
            "auto" | "system" => Some(Theme::Auto),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// The colour family the page is drawn in, each with a light and a dark face.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Palette {
    /// Warm paper and terracotta, the board's first look.
    #[default]
    Ember,
    /// Cool paper and purple.
    Iris,
    /// Grey-blue paper and steel.
    Slate,
}

impl Palette {
    pub fn parse(word: &str) -> Option<Palette> {
        match word {
            "ember" => Some(Palette::Ember),
            "iris" => Some(Palette::Iris),
            "slate" => Some(Palette::Slate),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Palette::Ember => "ember",
            Palette::Iris => "iris",
            Palette::Slate => "slate",
        }
    }
}

/// How the page is drawn on this machine. A viewer may still override it on the page,
/// which that browser remembers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Look {
    pub theme: Theme,
    pub palette: Palette,
}

/// The saved look; the defaults for anything unset or unreadable.
pub fn look(notes: &Notes) -> Look {
    let saved = std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or(Value::Null);
    Look {
        theme: saved["theme"]
            .as_str()
            .and_then(Theme::parse)
            .unwrap_or_default(),
        palette: saved["palette"]
            .as_str()
            .and_then(Palette::parse)
            .unwrap_or_default(),
    }
}

/// Sets what's given and keeps the rest.
pub fn set_look(
    notes: &Notes,
    theme: Option<Theme>,
    palette: Option<Palette>,
) -> Result<(), String> {
    let mut have = look(notes);
    if let Some(t) = theme {
        have.theme = t;
    }
    if let Some(p) = palette {
        have.palette = p;
    }
    // The port and anything else kept there stay as they are.
    let mut saved = std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    saved["theme"] = have.theme.name().into();
    saved["palette"] = have.palette.name().into();
    notes::write_text(&notes.state.join(LOOK_FILE), &saved.to_string())
}

/// The port as saved, 0 and all; None when none was.
fn saved_port(notes: &Notes) -> Option<u16> {
    std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["port"].as_u64())
        .and_then(|p| u16::try_from(p).ok())
}

/// The port this machine serves on, beside the look; `SERVE_PORT` when unset.
pub fn port(notes: &Notes) -> u16 {
    std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["port"].as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .unwrap_or(SERVE_PORT)
}

pub fn set_port(notes: &Notes, port: u16) -> Result<(), String> {
    let mut saved = std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    saved["port"] = port.into();
    notes::write_text(&notes.state.join(LOOK_FILE), &saved.to_string())
}

/// The address the home is served at on this machine.
pub fn url(notes: &Notes) -> String {
    format!("http://localhost:{}/", port(notes))
}

/// The machine's one server holds this file; a second `--serve` sees it and leaves.
const LOCK_FILE: &str = "home.lock";
/// With no request this long, the server stops; the next change starts one again.
const IDLE_SECS: u64 = 24 * 3600;
/// Tests set a shorter idle, in seconds.
const IDLE_SECS_VAR: &str = "ASTROLABE_HOME_IDLE_SECS";
/// Tests ask for no process: a marker says the home was wanted.
const NO_DETACHED_VAR: &str = "ASTROLABE_NO_DETACHED_SYNC";
const REQUESTED_MARKER: &str = "home-requested";

/// The machine's server lock, held for the server's life; None when another holds it.
pub fn hold(notes: &Notes) -> Option<std::fs::File> {
    use fs4::fs_std::FileExt;
    let file = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(notes.state.join(LOCK_FILE))
        .ok()?;
    file.try_lock_exclusive().ok()?.then_some(file)
}

/// Whether a home answers on the port already: a server from before the lock, or one
/// started by hand.
fn answering(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    ) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(700)));
    if stream
        .write_all(format!("HEAD / HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").as_bytes())
        .is_err()
    {
        return false;
    }
    let mut head = [0u8; 15];
    stream.read_exact(&mut head).is_ok() && head.starts_with(b"HTTP/1.1 200")
}

/// Has the home up on this machine: nothing to do when a server holds the lock or answers,
/// else one is started as its own process, so the owner never starts it by hand. Called
/// after every change to the notes and at each session's start. The address, or None when
/// there is no notes checkout to serve.
pub fn ensure_serving(notes: &Notes) -> Option<String> {
    if !notes.root.is_dir() {
        return None;
    }
    let port = port(notes);
    if std::env::var_os(NO_DETACHED_VAR).is_some() {
        let _ = notes::write_text(&notes.state.join(REQUESTED_MARKER), &port.to_string());
        return Some(url(notes));
    }
    match hold(notes) {
        // Held by a running server.
        None => return Some(url(notes)),
        // Free: nothing holds it, so a start is wanted unless something answers anyway.
        Some(free) => drop(free),
    }
    if !answering(port) {
        notes.spawn_detached(
            &["home", "--serve", "--port", &port.to_string()],
            "home.log",
        );
        // Up before the address is named, so a browser opened on it finds it; a start that
        // fails is in home.log in the state folder.
        let deadline = Instant::now() + Duration::from_secs(3);
        while !answering(port) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Some(url(notes))
}

/// Done and abandoned efforts stay on the board this long after their last save.
const DONE_DAYS: i64 = 14;
/// How often the file reloads itself in a browser, and how often the served board does.
const FILE_REFRESH_SECS: u32 = 60;
const SERVE_REFRESH_SECS: u32 = 5;
/// The port `astrolabe home --serve` takes unless told otherwise.
pub const SERVE_PORT: u16 = 4747;
const SENTENCE_CHARS: usize = 160;

/// Renders and writes the board; the path written, or why not. Under the caller's lock.
pub fn write(notes: &Notes) -> Result<PathBuf, String> {
    // A git error is no licence to overwrite.
    if !notes.git(&["ls-files", "--", FILE])?.trim().is_empty() {
        return Err(format!(
            "{FILE} is tracked in the notes repo, so it isn't overwritten; run `git rm --cached {FILE}` there and the board is drawn at the next save"
        ));
    }
    let rows = crate::record::rows_in(notes, None, true);
    let path = notes.root.join(FILE);
    notes::write_text(
        &path,
        &render(&rows, &notes.root, notes::now_ms(), look(notes)),
    )?;
    Ok(path)
}

struct Card<'a> {
    row: &'a Value,
    updated_ms: Option<i64>,
}

impl Card<'_> {
    fn text(&self, key: &str) -> &str {
        self.row[key].as_str().unwrap_or_default()
    }
    fn status(&self) -> &str {
        self.text("status")
    }
    fn closed(&self) -> bool {
        matches!(self.status(), "done" | "abandoned")
    }
    fn priority_rank(&self) -> u8 {
        match self.text("priority") {
            "urgent" | "top" => 0,
            "high" => 1,
            "low" => 3,
            _ => 2,
        }
    }
}

/// Binds the board's localhost listener; the URL is printed before serving starts.
pub fn listen(port: u16) -> Result<(TcpListener, String), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("could not listen on 127.0.0.1:{port}: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("could not read the listener's address: {e}"))?
        .port();
    Ok((listener, format!("http://127.0.0.1:{port}/")))
}

/// Serves the home, drawn fresh for every request, until a day passes with no request or
/// the process is stopped. `hold` is the machine's lock, kept for the server's life. One
/// thread per connection, since browsers hold idle connections open; past the cap a
/// connection is told to come back.
pub fn serve(notes: &Notes, listener: TcpListener, hold: std::fs::File, requested: u16) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let busy = Arc::new(AtomicUsize::new(0));
    let last = Arc::new(AtomicI64::new(notes::now_ms()));
    let idle = std::env::var(IDLE_SECS_VAR)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(IDLE_SECS);
    let seen = Arc::clone(&last);
    let watched = Notes::new(notes.root.clone(), notes.state.clone());
    std::thread::spawn(move || {
        // The lock lives as long as this thread, which is as long as the process.
        let _hold = hold;
        let every = Duration::from_millis((idle * 1000 / 2).clamp(100, 60_000));
        loop {
            std::thread::sleep(every);
            let quiet = notes::now_ms() - seen.load(Ordering::SeqCst);
            // The lock goes with the process; the next change starts a server again, on
            // the port the machine now wants if that changed.
            let moved = saved_port(&watched).unwrap_or(SERVE_PORT) != requested;
            if moved || quiet >= (idle * 1000) as i64 {
                std::process::exit(0);
            }
        }
    });
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        last.store(notes::now_ms(), Ordering::SeqCst);
        if busy.load(Ordering::SeqCst) >= HANDLERS {
            let _ = stream.write_all(BUSY);
            continue;
        }
        let notes = Notes::new(notes.root.clone(), notes.state.clone());
        let count = Arc::clone(&busy);
        let seen = Arc::clone(&last);
        count.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new().spawn(move || {
            handle(&notes, stream, port, &seen);
            count.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            busy.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

const HANDLERS: usize = 32;
const BUSY: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const REQUEST_BOUND: usize = 16 * 1024;
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);

fn handle(notes: &Notes, mut stream: TcpStream, port: u16, seen: &AtomicI64) {
    let _ = stream.set_read_timeout(Some(REQUEST_DEADLINE));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let (status, kind, body, head_only) = match request(&mut stream) {
        Some(head) if is_events(&head, port) => {
            events(notes, stream, seen);
            return;
        }
        Some(head) => {
            let head_only = head.split_whitespace().next() == Some("HEAD");
            let (status, kind, body) = answer(notes, &head, port);
            (status, kind, body, head_only)
        }
        None => (
            "400 Bad Request",
            "text/plain",
            "bad request\n".to_owned(),
            false,
        ),
    };
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    );
    if !head_only {
        let _ = stream.write_all(body.as_bytes());
    }
    // A close with no FIN first reads as a reset on Windows; say the answer is whole.
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

/// A GET of `/events` addressed to this port.
fn is_events(head: &str, port: u16) -> bool {
    let mut words = head.lines().next().unwrap_or_default().split_whitespace();
    words.next() == Some("GET")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some("/events"))
        && host_ok(head, port)
}

/// Whether the request names this loopback port as its host, so a page on another origin
/// that reaches the socket by a rebound name gets nothing.
fn host_ok(head: &str, port: u16) -> bool {
    let host = head
        .lines()
        .skip(1)
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.trim().to_ascii_lowercase());
    let mut ours = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if port == 80 {
        // A browser leaves the default port out.
        ours.extend(["127.0.0.1".to_owned(), "localhost".to_owned()]);
    }
    host.is_some_and(|h| ours.contains(&h))
}

/// How often the notes are looked at for a change, and how often a quiet stream says it's
/// alive so a proxy or the browser doesn't drop it.
const WATCH_EVERY: Duration = Duration::from_secs(1);
const PING_EVERY: Duration = Duration::from_secs(15);

/// The stream a page holds open: `changed` whenever the notes change, a comment to keep it
/// alive meanwhile. The page re-reads itself on `changed`. A viewer holding the stream
/// counts as a request, so the server stays up for an open tab.
fn events(notes: &Notes, mut stream: TcpStream, seen: &AtomicI64) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    if stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nretry: 2000\n\n",
        )
        .is_err()
    {
        return;
    }
    let mut print = fingerprint(&notes.root);
    let mut pinged = Instant::now();
    // A page that went away is noticed by a read, since nothing is written until a change.
    let _ = stream.set_nonblocking(true);
    let mut probe = [0u8; 64];
    loop {
        std::thread::sleep(WATCH_EVERY);
        match stream.read(&mut probe) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
        seen.store(notes::now_ms(), Ordering::SeqCst);
        let now = fingerprint(&notes.root);
        let line = if now != print {
            print = now;
            format!("event: changed\ndata: {}\n\n", notes::now_ms())
        } else if pinged.elapsed() >= PING_EVERY {
            ": ping\n\n".to_owned()
        } else {
            continue;
        };
        pinged = Instant::now();
        if stream.write_all(line.as_bytes()).is_err() {
            return;
        }
    }
}

/// What a change to the notes moves: the newest change time and the count of effort and
/// journal files, and the board file's own, which every save rewrites.
fn fingerprint(root: &Path) -> (i64, usize, i64) {
    let mut newest = 0;
    let mut count = 0;
    let mut note = |path: &Path| {
        count += 1;
        if let Some(ms) = modified_ms(path) {
            newest = newest.max(ms);
        }
    };
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            if name == ".git" || name == ".obsidian" {
                continue;
            }
            for rel in notes::md_files(root, &name) {
                note(&root.join(rel));
            }
        } else if name.ends_with(".md") {
            note(&entry.path());
        }
    }
    (newest, count, modified_ms(&root.join(FILE)).unwrap_or(0))
}

fn modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// The request head, read until its blank line within one deadline for the whole of it; None
/// when it never comes or outgrows the bound.
fn request(stream: &mut TcpStream) -> Option<String> {
    let started = Instant::now();
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let left = REQUEST_DEADLINE.checked_sub(started.elapsed())?;
        let _ = stream.set_read_timeout(Some(left.max(Duration::from_millis(1))));
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        head.extend_from_slice(&buf[..n]);
        if head.len() > REQUEST_BOUND {
            return None;
        }
        if let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            return Some(String::from_utf8_lossy(&head[..end]).into_owned());
        }
    }
}

/// One response: the board at `/`, an effort as a page at its own path, nothing anywhere else.
/// Only a request addressed to this loopback port is answered, so a page on another origin
/// that reaches the socket by a rebound name gets nothing.
fn answer(notes: &Notes, head: &str, port: u16) -> (&'static str, &'static str, String) {
    let mut lines = head.lines();
    let mut words = lines.next().unwrap_or_default().split_whitespace();
    let (method, target, version) = (words.next(), words.next(), words.next());
    if !matches!(method, Some("GET" | "HEAD")) || !version.is_some_and(|v| v.starts_with("HTTP/1."))
    {
        return ("400 Bad Request", "text/plain", "bad request\n".to_owned());
    }
    if !host_ok(head, port) {
        return (
            "421 Misdirected Request",
            "text/plain",
            "not for this host\n".to_owned(),
        );
    }
    let target = target.unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/");
    if path == "/" {
        let rows = crate::record::rows_in(notes, None, true);
        let page = page(
            &rows,
            &notes.root,
            notes::now_ms(),
            look(notes),
            SERVE_REFRESH_SECS,
            Links::Served,
        );
        return ("200 OK", "text/html", page);
    }
    // A note by its path from the checkout root: plain components only, nothing a join could
    // carry elsewhere (a dot-dot, a drive letter's colon, a backslash), and `.md` at the end.
    if let Some(rel) = path.strip_prefix('/')
        && let Some(rel) = decode(rel)
        && rel.ends_with(".md")
        && !rel.contains([':', '\\'])
        && !rel.split('/').any(|part| part.is_empty() || part == ".git")
        && Path::new(&rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && inside(&notes.root, &rel)
        && let Ok(text) = std::fs::read_to_string(notes.root.join(&rel))
    {
        return (
            "200 OK",
            "text/html",
            note_page(
                &text,
                &rel,
                &crate::record::rows_in(notes, None, true),
                &notes.root,
                notes::now_ms(),
                look(notes),
            ),
        );
    }
    ("404 Not Found", "text/plain", "nothing here\n".to_owned())
}

/// A note as a page: its properties as pills, the Markdown rendered in the board's shell,
/// an outline beside it when it has sections. Raw HTML in the note is shown as text, since
/// notes are written by agents and the page runs in the owner's browser. A relative link
/// to another note becomes a link to that note's page.
fn note_page(text: &str, rel: &str, rows: &Value, root: &Path, now_ms: i64, look: Look) -> String {
    use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd, html};
    let text = text.replace("\r\n", "\n");
    let (front, body) = match text.strip_prefix("---\n").and_then(|rest| {
        rest.find("\n---\n")
            .map(|end| (&rest[..end], &rest[end + 5..]))
    }) {
        Some((front, body)) => (front, body),
        None => ("", text.as_str()),
    };
    let body = strip_comments(body);
    let prop = |key: &str| -> Option<String> {
        let value = front
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))?
            .trim()
            .trim_matches('"');
        (!value.is_empty() && value != "none").then(|| value.to_owned())
    };
    // What the owner reads sits under the title; what the program keeps (the save's
    // branch, head and the like) is a small list of facts beside the outline.
    let mut pills = Vec::new();
    for key in [
        "status",
        "repo",
        "priority",
        "initiative",
        "involvement",
        "authority",
        "waits_for",
    ] {
        let Some(value) = prop(key) else { continue };
        let class = match key {
            "status" => format!(" status-{}", esc(&value)),
            _ => String::new(),
        };
        pills.push(format!(
            "<span class=\"pill{class}\"><i>{}</i>{}</span>",
            esc(&key.replace('_', " ")),
            esc(tag(&value))
        ));
    }
    let mut facts = Vec::new();
    for key in ["updated", "saved_by", "branch", "head", "unpushed", "dirty"] {
        let Some(value) = prop(key) else { continue };
        let shown = match key {
            "updated" => esc(&when(&value, now_ms)),
            "branch" | "head" => format!("<code>{}</code>", esc(tag(&value))),
            _ => esc(tag(&value)),
        };
        facts.push(format!(
            "<dt>{}</dt><dd>{shown}</dd>",
            esc(&key.replace('_', " "))
        ));
    }
    facts.push(format!("<dt>file</dt><dd><code>{}</code></dd>", esc(rel)));
    let facts = format!("<div class=\"facts\"><dl>{}</dl></div>", facts.join(""));
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let title = body
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or(name.trim_end_matches(".md"))
        .trim()
        .to_owned();
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_WIKILINKS);
    let mut events: Vec<Event> = Parser::new_ext(&body, options).collect();
    // Headings get ids, and the second and third levels make the outline.
    let mut outline = Vec::new();
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Heading { level, .. }) = &events[i] {
            let level = *level;
            let mut heading = String::new();
            let mut j = i + 1;
            while j < events.len() {
                match &events[j] {
                    Event::End(TagEnd::Heading(_)) => break,
                    Event::Text(t) | Event::Code(t) => heading.push_str(t),
                    _ => {}
                }
                j += 1;
            }
            let id = heading_id(&heading);
            if let Event::Start(Tag::Heading { id: slot, .. }) = &mut events[i] {
                *slot = Some(id.clone().into());
            }
            match level {
                HeadingLevel::H2 => outline.push((2, heading, id)),
                HeadingLevel::H3 => outline.push((3, heading, id)),
                _ => {}
            }
        }
        i += 1;
    }
    let events = events.into_iter().map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        // A link may name a note, a page or a mail address, never a script or a file.
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            // A wikilink names a note by title, which nothing here resolves yet.
            let wiki = matches!(link_type, LinkType::WikiLink { .. });
            let dest_url = if wiki || !plain_url(&dest_url) {
                "#".into()
            } else if let Some(page) = resolve_link(root, rel, &dest_url) {
                page.into()
            } else {
                dest_url
            };
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            })
        }
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: if plain_url(&dest_url) {
                dest_url
            } else {
                "#".into()
            },
            title,
            id,
        }),
        other => other,
    });
    let mut rendered = String::new();
    html::push_html(&mut rendered, events);
    let aside = if outline.len() > 1 {
        let items: String = outline
            .iter()
            .map(|(level, text, id)| {
                format!(
                    "<a class=\"l{level}\" href=\"#{}\">{}</a>",
                    esc(id),
                    esc(text)
                )
            })
            .collect();
        format!("<aside class=\"outline\"><h4>On this page</h4>{items}{facts}</aside>")
    } else {
        String::new()
    };
    let props = if pills.is_empty() {
        String::new()
    } else {
        format!("<div class=\"props\">{}</div>", pills.join(""))
    };
    // The properties follow the title; a note without one starts with them.
    let rendered = match rendered.find("</h1>") {
        Some(i) => format!("{}{props}{}", &rendered[..i + 5], &rendered[i + 5..]),
        None => format!("{props}{rendered}"),
    };
    let rendered = if aside.is_empty() {
        format!("{rendered}{facts}")
    } else {
        rendered
    };
    let mut crumb = String::from("<a href=\"/\">Board</a>");
    let parts: Vec<&str> = rel.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        crumb += &format!("<span>›</span><span>{}</span>", esc(part));
    }
    crumb += &format!("<span>›</span><b>{}</b>", esc(name));
    let page = format!(
        "<div class=\"note{}\"><div class=\"prose\">{rendered}</div>{aside}</div>\n",
        if aside.is_empty() { " wide" } else { "" }
    );
    shell(
        look,
        &title,
        &crumb,
        &page,
        &tree_html(root, rows, Links::Served, Some(rel)),
        None,
        Links::Served,
    )
}

/// The text with its HTML comments taken out: a journal line's hidden tail is for the
/// program, not the page.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// A heading's anchor: lowercase words joined by hyphens.
fn heading_id(text: &str) -> String {
    let mut id = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            id.extend(c.to_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    let id = id.trim_end_matches('-').to_owned();
    if id.is_empty() {
        "section".to_owned()
    } else {
        id
    }
}

/// A relative link from the note at `from` to another note on disk, as that note's page;
/// None for a link elsewhere or to nothing.
fn resolve_link(root: &Path, from: &str, dest: &str) -> Option<String> {
    if dest.starts_with('#') || dest.contains(':') {
        return None;
    }
    let (path, fragment) = dest.split_once('#').unwrap_or((dest, ""));
    let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
    let mut candidates = vec![normalize(&format!("{dir}/{path}")), normalize(path)];
    candidates.dedup();
    for c in candidates {
        if c.is_empty()
            || c.split('/')
                .any(|p| p.is_empty() || p == ".." || p == ".git")
            || !root.join(&c).is_file()
        {
            continue;
        }
        let fragment = if fragment.is_empty() {
            String::new()
        } else {
            format!("#{}", heading_id(fragment))
        };
        return Some(format!("/{}{fragment}", percent(&c)));
    }
    None
}

/// `a/./b/../c` as `a/c`; a path climbing past the root keeps its `..` and so is refused.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// A save stamp as the owner reads it beside the clock: the time if today, "yesterday"
/// and the time, or the date.
fn when(stamp: &str, now_ms: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(ms) = stamp_ms(stamp).or_else(|| {
        chrono::NaiveDate::parse_from_str(stamp, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(12, 0, 0))
            .and_then(|t| Local.from_local_datetime(&t).single())
            .map(|t| t.timestamp_millis())
    }) else {
        return stamp.to_owned();
    };
    let dated = stamp.len() > 10;
    let (Some(then), Some(now)) = (
        Local.timestamp_millis_opt(ms).single(),
        Local.timestamp_millis_opt(now_ms).single(),
    ) else {
        return stamp.to_owned();
    };
    let days = (now.date_naive() - then.date_naive()).num_days();
    let time = if dated {
        then.format("%H:%M").to_string()
    } else {
        String::new()
    };
    match days {
        0 if dated => time,
        0 => "today".to_owned(),
        1 => format!("yesterday {time}").trim_end().to_owned(),
        _ => then.format("%b %-d").to_string(),
    }
}

/// Whether the note at `rel` really sits under the checkout once links are followed: a
/// symlink or junction inside it pointing out is not served.
fn inside(root: &Path, rel: &str) -> bool {
    let (Ok(real), Ok(root)) = (root.join(rel).canonicalize(), root.canonicalize()) else {
        return false;
    };
    real.starts_with(&root)
}

/// A link destination the note page may carry: relative, or http, https or mailto. Anything
/// with another scheme (javascript, data, file, vbscript) is not.
fn plain_url(url: &str) -> bool {
    let url = url.trim_matches(|c: char| c.is_whitespace() || c.is_control());
    let scheme_end = url.find(':');
    let first_break = url.find(['/', '?', '#']).unwrap_or(url.len());
    match scheme_end {
        Some(end) if end < first_break => {
            let scheme = url[..end].to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto")
        }
        _ => true,
    }
}

/// Percent-decoding of a path segment, the inverse of `percent`; None for a malformed escape
/// or bytes that aren't UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Where a link points: the file on disk, or the same note through the server (a browser
/// refuses a file link from an http page).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Links {
    File,
    Served,
}

impl Links {
    fn href(self, root: &Path, rel: &str) -> String {
        match self {
            Links::Served => format!("/{}", percent(rel)),
            Links::File => file_url(&root.join(rel)),
        }
    }
}

/// The board as a file. `now_ms` is passed in so a test can fix the clock.
pub fn render(rows: &Value, root: &Path, now_ms: i64, look: Look) -> String {
    page(rows, root, now_ms, look, FILE_REFRESH_SECS, Links::File)
}

fn page(
    rows: &Value,
    root: &Path,
    now_ms: i64,
    look: Look,
    refresh_secs: u32,
    links: Links,
) -> String {
    let crumb = "<b>Board</b>";
    shell(
        look,
        "Astrolabe home",
        crumb,
        &board_html(rows, root, now_ms, links),
        &tree_html(root, rows, links, None),
        // The file reloads itself; the served page holds a stream and swaps in place.
        (links == Links::File).then_some(refresh_secs),
        links,
    )
}

/// The board itself: the chips, the Needs-you band, the active grid with the ready, blocked
/// and ideas stacks beside it, today's journal and the open initiatives, the fortnight's
/// closed work as a list.
fn board_html(rows: &Value, root: &Path, now_ms: i64, links: Links) -> String {
    let cards: Vec<Card> = rows["efforts"]
        .as_array()
        .map(|a| a.iter())
        .into_iter()
        .flatten()
        .map(|row| Card {
            row,
            updated_ms: row["updated"].as_str().and_then(stamp_ms),
        })
        .collect();
    let mut columns: [Vec<&Card>; 6] = Default::default();
    for card in &cards {
        let slot = if card.row["needs_owner"] == true {
            0
        } else {
            match card.status() {
                "active" => 1,
                "ready" => 2,
                "blocked" => 3,
                "done" | "abandoned" => {
                    let fresh = card
                        .updated_ms
                        .is_some_and(|t| now_ms - t <= DONE_DAYS * 86_400_000);
                    if !fresh {
                        continue;
                    }
                    5
                }
                _ => 4,
            }
        };
        columns[slot].push(card);
    }
    for column in &mut columns {
        column.sort_by_key(|c| (c.priority_rank(), -c.updated_ms.unwrap_or(0)));
    }
    let heading = |title: &str, n: usize| format!("<h2>{title} <span class=\"n\">{n}</span></h2>");
    let cards_of = |column: &[&Card], compact: bool| {
        column
            .iter()
            .map(|c| card_html(c, root, now_ms, links, compact))
            .collect::<String>()
    };
    let mut board = String::new();
    if !columns[0].is_empty() {
        board += &format!(
            "<section class=\"col needsyou\">{}<div class=\"cards wide\">{}</div></section>\n",
            heading("Needs you", columns[0].len()),
            cards_of(&columns[0], false)
        );
    } else {
        // Kept in the page so a test and the live swap can find the column by name.
        board += "<section class=\"col needsyou\" hidden></section>\n";
    }
    // Ready, blocked and ideas stack beside the active grid; an empty one is a few words
    // under the work instead of a dashed box.
    let mut stack = String::new();
    let mut quiet = Vec::new();
    for (key, title, slot, none) in [
        ("ready", "Ready", 2, "nothing ready"),
        ("blocked", "Blocked", 3, "nothing blocked"),
        ("ideas", "Ideas", 4, "no ideas parked"),
    ] {
        let column = &columns[slot];
        if column.is_empty() {
            quiet.push(none);
        } else {
            stack += &format!(
                "<section class=\"col {key}\">{}<div class=\"cards\">{}</div></section>\n",
                heading(title, column.len()),
                cards_of(column, true)
            );
        }
    }
    let active = if columns[1].is_empty() {
        "<p class=\"empty\">nothing in flight</p>\n".to_owned()
    } else {
        cards_of(&columns[1], false)
    };
    board += &format!(
        "<div class=\"work{}\"><section class=\"col active\">{}<div class=\"cards grid\">{active}</div></section>\n{}</div>\n",
        if stack.is_empty() { " solo" } else { "" },
        heading("Active", columns[1].len()),
        if stack.is_empty() {
            String::new()
        } else {
            format!("<div class=\"stack\">{stack}</div>")
        }
    );
    if !quiet.is_empty() {
        let mut line = quiet.join(", ");
        if let Some(first) = line.get(..1) {
            line = first.to_uppercase() + &line[1..];
        }
        board += &format!("<p class=\"quiet\">{line}.</p>\n");
    }

    let by_number: std::collections::HashMap<u64, &Value> = cards
        .iter()
        .filter_map(|c| c.row["number"].as_u64().map(|n| (n, c.row)))
        .collect();
    let mut journal = String::new();
    for line in rows["journal_lines"].as_array().into_iter().flatten() {
        let effort = line["effort"].as_str().unwrap_or_default();
        let num = match effort.parse::<u64>().ok().and_then(|n| by_number.get(&n)) {
            Some(row) => format!(
                "<span class=\"num\"><a href=\"{}\">{}</a></span>",
                esc(&links.href(root, row["file"].as_str().unwrap_or_default())),
                row["number"].as_u64().unwrap_or(0)
            ),
            None => format!("<span class=\"num repo\">{}</span>", esc(effort)),
        };
        journal += &format!(
            "<li>{num}<span>{}</span></li>\n",
            esc(line["text"].as_str().unwrap_or_default())
        );
    }
    if journal.is_empty() {
        journal = "<li class=\"empty\">nothing yet today</li>\n".to_owned();
    }

    let mut initiatives: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for card in &cards {
        let name = tag(card.text("initiative"));
        if name.is_empty() {
            continue;
        }
        let entry = initiatives.entry(name.to_owned()).or_default();
        if card.closed() {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }
    // An initiative is listed while an effort of its is open; one whose efforts have all
    // closed has had its say and leaves the board with them.
    let mut initiative_lines = String::new();
    for (name, (open, done)) in &initiatives {
        if *open == 0 {
            continue;
        }
        initiative_lines += &format!(
            "<li class=\"ini\"><span>{}</span><span class=\"num\">{open} open · {done} done</span><span class=\"track\"><i style=\"width:{}%\"></i></span></li>\n",
            esc(name),
            100 * done / (open + done)
        );
    }
    if initiative_lines.is_empty() {
        initiative_lines = "<li class=\"empty\">nothing open</li>\n".to_owned();
    }
    board += &format!(
        "<div class=\"below\"><section class=\"col today\">{}<ul id=\"journal\">\n{journal}</ul></section><section class=\"col initiatives\">{}<ul>\n{initiative_lines}</ul></section></div>\n",
        heading(
            "Today",
            rows["journal_lines"].as_array().map_or(0, Vec::len)
        ),
        heading(
            "Initiatives",
            initiatives.values().filter(|(open, _)| *open > 0).count()
        )
    );

    let done = &columns[5];
    board += &format!(
        "<section class=\"col done\">{}\n{}</section>\n",
        heading("Done this fortnight", done.len()),
        if done.is_empty() {
            "<p class=\"empty\">nothing closed lately</p>\n".to_owned()
        } else {
            format!(
                "<ul class=\"closed\">{}</ul>\n",
                done.iter()
                    .map(|c| done_html(c, root, now_ms, links))
                    .collect::<String>()
            )
        }
    );

    // The chips: every count that isn't zero, then the notes' state.
    let active_n = columns[1].len() + columns[0].iter().filter(|c| c.status() == "active").count();
    let chip = |dot: &str, n: usize, label: &str| {
        format!("<span class=\"chip\"><span class=\"dot {dot}\"></span><b>{n}</b> {label}</span>")
    };
    let mut chips = String::new();
    if !columns[0].is_empty() {
        chips += &chip(
            "needs",
            columns[0].len(),
            if columns[0].len() == 1 {
                "needs you"
            } else {
                "need you"
            },
        );
    }
    if active_n > 0 {
        chips += &chip("active", active_n, "active");
    }
    for (dot, slot, label) in [
        ("ready", 2, "ready"),
        ("blocked", 3, "blocked"),
        ("idea", 4, "ideas"),
        ("done", 5, "done"),
    ] {
        if !columns[slot].is_empty() {
            chips += &chip(dot, columns[slot].len(), label);
        }
    }
    let warnings: Vec<&str> = rows["notes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    chips += &if warnings.is_empty() {
        "<span class=\"sync\">notes in sync</span>".to_owned()
    } else {
        format!(
            "<span class=\"sync warn\">{}</span>",
            esc(&warnings.join("; "))
        )
    };
    let date = chrono::DateTime::from_timestamp_millis(now_ms)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%A, %B %-d")
                .to_string()
        })
        .unwrap_or_else(|| today_of(rows));
    format!(
        "<div class=\"board\" id=\"board\"><div class=\"bhead\"><h1>{}</h1><div class=\"chips\">{chips}</div></div>\n{board}</div>\n",
        esc(&date)
    )
}

fn today_of(rows: &Value) -> String {
    rows["today"].as_str().unwrap_or_default().to_owned()
}

/// The document around a board or a note: the notes tree down the side, the top bar, the
/// look's tokens, the small script that keeps the tree and the viewer's overrides.
fn shell(
    look: Look,
    title: &str,
    crumb: &str,
    body: &str,
    tree: &str,
    refresh_secs: Option<u32>,
    links: Links,
) -> String {
    let theme_attr = match look.theme {
        Theme::Auto => String::new(),
        t => format!(" data-theme=\"{}\"", t.name()),
    };
    let scheme = match look.theme {
        Theme::Auto => "light dark",
        Theme::Light => "light",
        Theme::Dark => "dark",
    };
    let refresh = refresh_secs
        .map(|s| format!("<meta http-equiv=\"refresh\" content=\"{s}\">"))
        .unwrap_or_default();
    let csp = match links {
        Links::Served => {
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src http: https: data:; connect-src 'self'; form-action 'none'\">"
        }
        Links::File => "",
    };
    let swatches = [Palette::Ember, Palette::Iris, Palette::Slate]
        .iter()
        .map(|p| {
            format!(
                "<button class=\"sw\" data-p=\"{0}\" title=\"{0}\" aria-label=\"{0} palette\"></button>",
                p.name()
            )
        })
        .collect::<String>();
    format!(
        "<!doctype html><html lang=\"en\" data-palette=\"{palette}\"{theme_attr}><head><meta charset=\"utf-8\">{refresh}<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"{scheme}\">{csp}\n<title>{title}</title>\n<style>{PALETTES}{STYLE}</style></head><body>\n<div class=\"app\"><aside class=\"side\" id=\"side\"><div class=\"brand\"><span class=\"dot\"></span>Astrolabe<span class=\"live\" id=\"live\" hidden title=\"The page follows every save\">live</span></div><div class=\"search\"><input id=\"q\" type=\"search\" placeholder=\"Find a note\" aria-label=\"Find a note\"></div><nav class=\"tree\" id=\"tree\">\n{tree}</nav><div class=\"foot\"><label for=\"appearance\">Theme</label><select id=\"appearance\" aria-label=\"Appearance\"><option value=\"\">System</option><option value=\"light\">Light</option><option value=\"dark\">Dark</option></select><span class=\"swatches\">{swatches}</span></div></aside><div class=\"scrim\" id=\"scrim\"></div><main class=\"main\" id=\"main\"><div class=\"topbar\"><button class=\"btn menu-btn\" id=\"menu\" aria-label=\"Open the notes list\">☰</button><div class=\"crumb\" id=\"crumb\">{crumb}</div></div>\n{body}</main></div>\n<script>{SCRIPT}</script>\n</body></html>\n",
        palette = look.palette.name(),
        title = esc(title),
    )
}

/// A folder of notes as the tree shows it.
struct Node {
    name: String,
    path: String,
    dirs: Vec<Node>,
    files: Vec<String>,
    open: bool,
}

impl Node {
    fn new(name: &str, path: &str) -> Node {
        Node {
            name: name.to_owned(),
            path: path.to_owned(),
            dirs: Vec::new(),
            files: Vec::new(),
            open: false,
        }
    }

    fn insert(&mut self, rel: &str) {
        let inner = rel.strip_prefix(&self.path).unwrap_or(rel);
        let inner = inner.trim_start_matches('/');
        match inner.split_once('/') {
            Some((dir, _)) => {
                let path = format!("{}/{dir}", self.path);
                let child = match self.dirs.iter_mut().position(|d| d.name == dir) {
                    Some(i) => &mut self.dirs[i],
                    None => {
                        self.dirs.push(Node::new(dir, &path));
                        self.dirs.last_mut().unwrap()
                    }
                };
                child.insert(rel);
            }
            None => self.files.push(rel.to_owned()),
        }
    }

    fn sort(&mut self, newest_first: bool) {
        self.dirs.sort_by(|a, b| a.name.cmp(&b.name));
        self.files.sort();
        if newest_first {
            self.dirs.reverse();
            self.files.reverse();
        }
        for d in &mut self.dirs {
            d.sort(newest_first);
        }
    }

    /// Open down the first branch: the newest month of this year's efforts.
    fn open_latest(&mut self) {
        self.open = true;
        if let Some(first) = self.dirs.first_mut() {
            first.open_latest();
        }
    }

    fn count(&self) -> usize {
        self.files.len() + self.dirs.iter().map(Node::count).sum::<usize>()
    }
}

fn tree_html(root: &Path, rows: &Value, links: Links, current: Option<&str>) -> String {
    const FIRST: [&str; 5] = ["efforts", "journal", "knowledge", "lessons", "drafts"];
    let mut names: Vec<(bool, String)> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            if name == ".git" || name == ".obsidian" {
                continue;
            }
            names.push((name.starts_with('.'), name));
        } else if name.ends_with(".md") {
            files.push(name);
        }
    }
    names.sort_by_key(|(hidden, name)| {
        let rank = FIRST
            .iter()
            .position(|f| f == name)
            .unwrap_or(FIRST.len() + usize::from(*hidden));
        (rank, name.clone())
    });
    files.sort();
    let by_file: std::collections::HashMap<&str, &Value> = rows["efforts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["file"].as_str().map(|f| (f, e)))
        .collect();
    let mut shown = String::new();
    let mut hidden = String::new();
    let mut hidden_names = Vec::new();
    let mut hidden_count = 0;
    for (is_hidden, name) in &names {
        let mut node = Node::new(name, name);
        let mut any = false;
        for rel in notes::md_files(root, name) {
            node.insert(&rel);
            any = true;
        }
        if !any {
            continue;
        }
        let newest_first = matches!(name.as_str(), "efforts" | "journal" | "lessons");
        node.sort(newest_first);
        if matches!(name.as_str(), "efforts" | "journal") {
            node.open_latest();
        }
        let html = dir_html(&node, 0, None, &by_file, root, links, current);
        if *is_hidden {
            hidden += &html;
            hidden_names.push(name.clone());
            hidden_count += node.count();
        } else {
            shown += &html;
        }
    }
    let board_on = if current.is_none() { " on" } else { "" };
    let mut out = format!(
        "<a class=\"top{board_on}\" href=\"{}\"><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><rect x=\"3\" y=\"4\" width=\"18\" height=\"16\" rx=\"2\"/><path d=\"M9 4v16M15 4v16\"/></svg><span class=\"t\">Board</span></a>\n",
        match links {
            Links::Served => "/".to_owned(),
            Links::File => file_url(&root.join(FILE)),
        }
    );
    out += &shown;
    for name in &files {
        out += &file_row(name, 0, &by_file, root, links, current);
    }
    if !hidden.is_empty() {
        out += &format!(
            "<div class=\"hid\" id=\"hid\" hidden>{hidden}</div><button class=\"hidden-toggle\" id=\"hidtoggle\" data-names=\"{}\" data-n=\"{hidden_count}\">Show {} · {hidden_count} note{} agents wrote for each other</button>\n",
            esc(&hidden_names.join(", ")),
            esc(&hidden_names.join(", ")),
            if hidden_count == 1 { "" } else { "s" }
        );
    }
    out
}

/// `2026`, a shelf of months: its months are listed in its place, each carrying the year.
fn is_year(name: &str) -> bool {
    name.len() == 4 && name.chars().all(|c| c.is_ascii_digit())
}

/// `11-November` reads as `November`; a filed note's own number stays on its row.
fn label_of(name: &str) -> &str {
    name.split_once('-')
        .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .map_or(name, |(_, rest)| rest)
}

fn dir_html(
    node: &Node,
    depth: usize,
    year: Option<&str>,
    by_file: &std::collections::HashMap<&str, &Value>,
    root: &Path,
    links: Links,
    current: Option<&str>,
) -> String {
    if is_year(&node.name) && node.files.is_empty() {
        return node
            .dirs
            .iter()
            .map(|d| dir_html(d, depth, Some(&node.name), by_file, root, links, current))
            .collect();
    }
    let label = match year {
        Some(year) => format!("{} {year}", label_of(&node.name)),
        None => label_of(&node.name).to_owned(),
    };
    let mut out = format!(
        "<details{} data-key=\"{}\"><summary style=\"--d:{depth}\"><span class=\"chev\">›</span><span class=\"t\">{}</span><span class=\"n\">{}</span></summary>\n",
        if node.open { " open" } else { "" },
        esc(&node.path),
        esc(&label),
        node.count()
    );
    for d in &node.dirs {
        out += &dir_html(d, depth + 1, None, by_file, root, links, current);
    }
    for f in &node.files {
        out += &file_row(f, depth + 1, by_file, root, links, current);
    }
    out + "</details>\n"
}

fn file_row(
    rel: &str,
    depth: usize,
    by_file: &std::collections::HashMap<&str, &Value>,
    root: &Path,
    links: Links,
    current: Option<&str>,
) -> String {
    let on = if current == Some(rel) { " on" } else { "" };
    let href = esc(&links.href(root, rel));
    match by_file.get(rel) {
        Some(e) => {
            let status = if e["needs_owner"] == true {
                "needs".to_owned()
            } else {
                e["status"].as_str().unwrap_or_default().to_owned()
            };
            let title = e["title"].as_str().unwrap_or_default();
            format!(
                "<a class=\"f{on}\" style=\"--d:{depth}\" href=\"{href}\" title=\"{}\" data-rel=\"{}\"><span class=\"st s-{}\"></span><span class=\"num\">{}</span><span class=\"t\">{}</span></a>\n",
                esc(title),
                esc(rel),
                esc(&status),
                e["number"].as_u64().unwrap_or(0),
                esc(title)
            )
        }
        None => {
            let name = rel
                .rsplit('/')
                .next()
                .unwrap_or(rel)
                .trim_end_matches(".md");
            // `05-Thursday` reads as the day's number and its name.
            let (num, text) = match name.split_once('-') {
                Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
                    (format!("<span class=\"num\">{}</span>", esc(n)), rest)
                }
                _ => (String::new(), name),
            };
            format!(
                "<a class=\"f{on}\" style=\"--d:{depth}\" href=\"{href}\" title=\"{}\" data-rel=\"{}\">{num}<span class=\"t\">{}</span></a>\n",
                esc(name),
                esc(rel),
                esc(text)
            )
        }
    }
}

fn card_html(card: &Card, root: &Path, now_ms: i64, links: Links, compact: bool) -> String {
    let mut bits = Vec::new();
    let priority = card.text("priority");
    if matches!(priority, "high" | "urgent" | "top") {
        bits.push(format!("<span class=\"pri {priority}\">{priority}</span>"));
    }
    let initiative = tag(card.text("initiative"));
    if !initiative.is_empty() {
        bits.push(format!(
            "<span class=\"pill tag\">{}</span>",
            esc(initiative)
        ));
    }
    if !card.text("repo").is_empty() {
        bits.push(format!(
            "<span class=\"pill repo\">{}</span>",
            esc(card.text("repo"))
        ));
    }
    if card.status().is_empty() {
        bits.push("<span class=\"pill nostatus\">no status</span>".to_owned());
    }
    if let Some(t) = card.updated_ms {
        bits.push(format!(
            "<span class=\"age\" data-t=\"{t}\" title=\"{}\">{}</span>",
            esc(card.text("updated")),
            age(now_ms - t)
        ));
    }
    let pieces = match (
        card.row["pieces"]["done"].as_u64(),
        card.row["pieces"]["total"].as_u64(),
    ) {
        (Some(done), Some(total)) if total > 0 => format!(
            "<div class=\"bar\"><span class=\"track\"><i style=\"width:{}%\"></i></span><span>{done} of {total} pieces</span></div>\n",
            100 * done / total
        ),
        _ => String::new(),
    };
    let waits = card.text("waits_for");
    let waits = if waits.is_empty() {
        String::new()
    } else if let Some(ask) = waits.strip_prefix("owner:") {
        format!("<p class=\"ask\">{}</p>\n", esc(ask.trim()))
    } else {
        format!("<p class=\"waits\">waits for {}</p>\n", esc(waits))
    };
    let href = match card.row["file"].as_str() {
        Some(file) => links.href(root, file),
        None => "#".to_owned(),
    };
    let next = if compact || card.text("next").is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"next\">{}</p>\n",
            esc(&first_sentence(card.text("next")))
        )
    };
    // The card's left rail says its state without the column heading.
    let state = if card.row["needs_owner"] == true {
        "needs"
    } else {
        match card.status() {
            "" => "idea",
            s => s,
        }
    };
    format!(
        "<article class=\"card s-{}{}\" data-n=\"{}\"><h3><a href=\"{}\"><span class=\"num\">{}</span>{}</a></h3>\n{waits}<p class=\"meta\">{}</p>\n{pieces}{next}</article>\n",
        esc(state),
        if compact { " compact" } else { "" },
        card.row["number"].as_u64().unwrap_or(0),
        esc(&href),
        card.row["number"].as_u64().unwrap_or(0),
        esc(card.text("title")),
        bits.join(""),
    )
}

/// A closed effort as one line of the fortnight's list.
fn done_html(card: &Card, root: &Path, now_ms: i64, links: Links) -> String {
    let href = match card.row["file"].as_str() {
        Some(file) => links.href(root, file),
        None => "#".to_owned(),
    };
    let age = card
        .updated_ms
        .map(|t| {
            format!(
                "<span class=\"age\" data-t=\"{t}\" title=\"{}\">{}</span>",
                esc(card.text("updated")),
                age(now_ms - t)
            )
        })
        .unwrap_or_default();
    let repo = if card.text("repo").is_empty() {
        String::new()
    } else {
        format!(
            "<span class=\"pill repo\">{}</span>",
            esc(card.text("repo"))
        )
    };
    format!(
        "<li class=\"{}\" data-n=\"{}\"><svg class=\"tick\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M5 12l5 5L20 7\"/></svg><a href=\"{}\"><span class=\"num\">{}</span><span class=\"t\">{}</span></a>{repo}{age}</li>\n",
        esc(card.status()),
        card.row["number"].as_u64().unwrap_or(0),
        esc(&href),
        card.row["number"].as_u64().unwrap_or(0),
        esc(card.text("title")),
    )
}

/// A `file:///` link a browser follows: forward slashes, a Windows drive after the root,
/// the `\\?\` of a canonical Windows path dropped, everything but unreserved characters
/// percent-encoded.
fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix("\\\\?\\UNC\\")
        .map(|rest| format!("\\\\{rest}"))
        .unwrap_or_else(|| text.trim_start_matches("\\\\?\\").to_owned());
    let text = text.replace('\\', "/");
    let mut url = String::from("file://");
    if !text.starts_with('/') {
        url.push('/');
    }
    url + &percent(&text)
}

/// Percent-encodes everything but the unreserved characters, `/` and `:`.
fn percent(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `2026-11-04T16:30-07:00`, the stamp saves write, as epoch milliseconds.
fn stamp_ms(stamp: &str) -> Option<i64> {
    DateTime::<FixedOffset>::parse_from_str(stamp, "%Y-%m-%dT%H:%M%:z")
        .ok()
        .map(|t| t.timestamp_millis())
}

fn age(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s < 3600 {
        format!("{} min", s / 60)
    } else if s < 86_400 {
        format!("{} h", s / 3600)
    } else {
        format!("{} d", s / 86_400)
    }
}

/// The paragraph's first sentence, with a legacy `**Next:**` label dropped, cut short.
fn first_sentence(paragraph: &str) -> String {
    let mut text = paragraph.trim();
    while let Some(rest) = text.strip_prefix("**")
        && let Some((label, after)) = rest.split_once("**")
        && label.ends_with(':')
    {
        text = after.trim_start();
    }
    let text = text.replace('\n', " ");
    let end = text
        .char_indices()
        .find(|&(i, c)| c == '.' && text[i + 1..].starts_with(' '))
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len());
    let sentence = &text[..end];
    if sentence.chars().count() <= SENTENCE_CHARS {
        sentence.to_owned()
    } else {
        let cut: String = sentence.chars().take(SENTENCE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// `[[agent-tooling]]`, as Obsidian writes a link, reads as the name.
fn tag(value: &str) -> &str {
    value
        .trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .trim()
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The colour tokens of each palette, light and dark, chosen by the `data-palette` and
/// `data-theme` attributes on the document, so a viewer's override needs no new page.
const PALETTES: &str = r#"
:root{--bg:#f7f5f0;--pane:#fdfcfa;--card:#ffffff;--ink:#27251f;--mute:#7d7a71;--line:#e6e2d9;--line-2:#d8d3c8;--accent:#b35b35;--accent-ink:#ffffff;--needs:#fbf1ea;--tag:#efece5;--tag-ink:#4e4b44;--good:#3d7a4a;--warn:#9a6a12;--bad:#9b2f2f;--sel:#f1ede4;--code:#f3f0e9;--shadow:0 1px 2px rgba(50,40,20,.06),0 0 0 1px rgba(50,40,20,.03)}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){--bg:#171715;--pane:#1d1d1a;--card:#262623;--ink:#ebe7df;--mute:#9a968c;--line:#2f2e29;--line-2:#3b3a34;--accent:#e0895a;--accent-ink:#1b1b19;--needs:#30241c;--tag:#2e2f33;--tag-ink:#cfcbc2;--good:#7fc08c;--warn:#e0b25a;--bad:#e78a8a;--sel:#2a2a26;--code:#22221f;--shadow:none;color-scheme:dark}}
:root[data-theme="dark"]{--bg:#171715;--pane:#1d1d1a;--card:#262623;--ink:#ebe7df;--mute:#9a968c;--line:#2f2e29;--line-2:#3b3a34;--accent:#e0895a;--accent-ink:#1b1b19;--needs:#30241c;--tag:#2e2f33;--tag-ink:#cfcbc2;--good:#7fc08c;--warn:#e0b25a;--bad:#e78a8a;--sel:#2a2a26;--code:#22221f;--shadow:none;color-scheme:dark}
:root[data-palette="iris"]{--bg:#f5f3f9;--pane:#fcfbfe;--card:#ffffff;--ink:#26232e;--mute:#7b7689;--line:#e6e2ef;--line-2:#d6d0e3;--accent:#6b4fc2;--accent-ink:#ffffff;--needs:#f1ecfb;--tag:#ece8f4;--tag-ink:#4a4557;--sel:#eeeaf5;--code:#f1eef7;--good:#3a7a55;--warn:#9a6a12;--bad:#9b2f2f;--shadow:0 1px 2px rgba(40,30,70,.06),0 0 0 1px rgba(40,30,70,.03)}
@media (prefers-color-scheme:dark){:root[data-palette="iris"]:not([data-theme="light"]){--bg:#141319;--pane:#1a1921;--card:#242230;--ink:#e9e6f1;--mute:#9d98ad;--line:#2c2a37;--line-2:#3a3748;--accent:#a98bff;--accent-ink:#17151f;--needs:#2a2440;--tag:#2e2c3b;--tag-ink:#d2cde0;--sel:#272533;--code:#1f1e28;--good:#8ccf9a;--warn:#e0b25a;--bad:#e78a8a;--shadow:none;color-scheme:dark}}
:root[data-palette="iris"][data-theme="dark"]{--bg:#141319;--pane:#1a1921;--card:#242230;--ink:#e9e6f1;--mute:#9d98ad;--line:#2c2a37;--line-2:#3a3748;--accent:#a98bff;--accent-ink:#17151f;--needs:#2a2440;--tag:#2e2c3b;--tag-ink:#d2cde0;--sel:#272533;--code:#1f1e28;--good:#8ccf9a;--warn:#e0b25a;--bad:#e78a8a;--shadow:none;color-scheme:dark}
:root[data-palette="slate"]{--bg:#f3f5f7;--pane:#fbfcfd;--card:#ffffff;--ink:#1f262e;--mute:#6f7b88;--line:#e1e6ec;--line-2:#cfd7e0;--accent:#2f6f9f;--accent-ink:#ffffff;--needs:#e9f1f8;--tag:#e8edf2;--tag-ink:#3f4a56;--sel:#e9eef3;--code:#eef2f6;--good:#2f7a4f;--warn:#9a6a12;--bad:#9b2f2f;--shadow:0 1px 2px rgba(20,40,60,.06),0 0 0 1px rgba(20,40,60,.03)}
@media (prefers-color-scheme:dark){:root[data-palette="slate"]:not([data-theme="light"]){--bg:#121518;--pane:#171b1f;--card:#20262c;--ink:#e4e9ee;--mute:#8f9ba6;--line:#283036;--line-2:#354049;--accent:#7db6e0;--accent-ink:#0f161c;--needs:#1f2c37;--tag:#2a3239;--tag-ink:#c9d2da;--sel:#232a30;--code:#1b2126;--good:#86c89a;--warn:#e0b25a;--bad:#e78a8a;--shadow:none;color-scheme:dark}}
:root[data-palette="slate"][data-theme="dark"]{--bg:#121518;--pane:#171b1f;--card:#20262c;--ink:#e4e9ee;--mute:#8f9ba6;--line:#283036;--line-2:#354049;--accent:#7db6e0;--accent-ink:#0f161c;--needs:#1f2c37;--tag:#2a3239;--tag-ink:#c9d2da;--sel:#232a30;--code:#1b2126;--good:#86c89a;--warn:#e0b25a;--bad:#e78a8a;--shadow:none;color-scheme:dark}
"#;

/* Layout: a fixed notes tree on the left, one scrolling pane on the right that is the board
or a note. The same sheet draws the file copy. Type: 11 px labels, 12 and 13 meta, 14 body,
15.5 card titles, 28 the date; numbers tabular throughout. */
const STYLE: &str = r##"
*{box-sizing:border-box}html,body{height:100%}
body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 -apple-system,"Segoe UI",Inter,system-ui,sans-serif;-webkit-font-smoothing:antialiased;font-variant-numeric:tabular-nums}
a{color:inherit}button{font:inherit;color:inherit;background:none;border:0;cursor:pointer}
.app{display:flex;height:100%;min-height:0}
.side{width:268px;flex:0 0 auto;display:flex;flex-direction:column;border-right:1px solid var(--line);background:var(--bg);min-height:0}
.brand{display:flex;align-items:center;gap:10px;padding:16px 20px 12px;font-weight:600}
.brand .dot{width:9px;height:9px;border-radius:50%;background:var(--accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--accent) 22%,transparent)}
.brand .live{margin-left:auto;font-size:12px;font-weight:500;color:var(--mute);display:flex;align-items:center;gap:6px}.brand .live::before{content:"";width:6px;height:6px;border-radius:50%;background:var(--good)}.brand .live.off::before{background:var(--line-2)}
.search{margin:0 10px 6px}.search input{width:100%;height:32px;padding:0 10px;border:1px solid var(--line-2);border-radius:8px;background:var(--pane);color:var(--ink);font:13px inherit}
.tree{overflow:auto;flex:1;padding:4px 10px 16px;font-size:13px}
.tree .top{display:flex;align-items:center;gap:8px;height:30px;padding:0 10px;border-radius:7px;font-weight:500;text-decoration:none}
.tree .top svg{width:14px;height:14px;fill:none;stroke:currentColor;stroke-width:2;color:var(--mute)}
.tree .top.on,.tree .f.on{background:var(--sel)}.tree .f.on{font-weight:500}
.tree details{margin:0}
.tree>details{margin-top:14px}
.tree summary{list-style:none;cursor:pointer;display:flex;align-items:center;gap:6px;height:26px;padding:0 10px 0 calc(10px + var(--d,0) * 14px);border-radius:7px;color:var(--mute);font-size:12.5px;font-weight:500;min-width:0}
.tree summary::-webkit-details-marker{display:none}
.tree summary .chev{display:inline-block;width:10px;transition:transform .15s;color:var(--mute);flex:0 0 auto}
.tree details[open]>summary .chev{transform:rotate(90deg)}
.tree summary .t{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.tree summary .n{font-weight:400;opacity:.8}
.tree>details>summary{font-size:11px;line-height:16px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;height:24px;margin-bottom:2px}
.tree>details>summary .chev{display:none}
.tree>details>summary .n{letter-spacing:0;font-weight:500}
.tree .f{display:flex;align-items:center;gap:8px;height:30px;padding:0 10px 0 calc(10px + var(--d,1) * 14px);border-radius:7px;color:var(--ink);text-decoration:none;min-width:0}
.tree .f:hover,.tree summary:hover{background:var(--sel)}
.tree .f .num{color:var(--mute);font-size:11.5px;width:18px;flex:0 0 auto;text-align:right}
.tree .f .st{width:7px;height:7px;border-radius:50%;flex:0 0 auto;background:var(--line-2)}
.tree .f .st.s-active{background:var(--accent)}.tree .f .st.s-done{background:var(--good)}.tree .f .st.s-blocked{background:var(--bad)}.tree .f .st.s-needs{background:var(--warn)}.tree .f .st.s-ready{background:var(--line-2)}.tree .f .st.s-,.tree .f .st.s-proposed,.tree .f .st.s-abandoned{background:transparent;box-shadow:inset 0 0 0 1.5px var(--line-2)}
.tree .f .t{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.tree .hid summary{opacity:.7}
.tree .hidden-toggle{display:block;width:100%;text-align:left;margin:14px 0 0;padding:6px 10px;color:var(--mute);font-size:12px;line-height:1.4;border-radius:7px}
.tree .hidden-toggle:hover{background:var(--sel)}
.tree.filtering details{display:block}.tree.filtering .f.miss{display:none}
.side .foot{padding:10px 16px;border-top:1px solid var(--line);color:var(--mute);font-size:12px;display:flex;gap:8px;align-items:center;flex-wrap:wrap}
.side .foot select{font:12px inherit;color:var(--ink);background:var(--card);border:1px solid var(--line-2);border-radius:6px;padding:2px 4px}
.swatches{display:flex;gap:6px;align-items:center;margin-left:auto}
.sw{width:14px;height:14px;border-radius:50%;border:0;padding:0;box-shadow:0 0 0 2px var(--bg)}
.sw[data-p=ember]{background:#b35b35}.sw[data-p=iris]{background:#7c5fd3}.sw[data-p=slate]{background:#3f7fb0}.sw.on{box-shadow:0 0 0 2px var(--bg),0 0 0 3px var(--ink)}
.main{flex:1;min-width:0;overflow:auto;background:var(--pane)}
.topbar{position:sticky;top:0;z-index:5;display:flex;align-items:center;gap:12px;padding:10px 24px;background:color-mix(in srgb,var(--pane) 88%,transparent);backdrop-filter:blur(8px);border-bottom:1px solid var(--line);min-height:44px}
.topbar .crumb{display:flex;align-items:center;gap:6px;color:var(--mute);font-size:13px;min-width:0;overflow:hidden;white-space:nowrap}
.topbar .crumb b{color:var(--ink);font-weight:500}.topbar .crumb a{text-decoration:none}.topbar .crumb a:hover{text-decoration:underline}
.btn{padding:6px 11px;border:1px solid var(--line-2);border-radius:8px;background:var(--card);font-size:12.5px;font-weight:500}
.menu-btn{display:none}
.board{padding:28px 36px 56px;max-width:1240px}
.bhead{margin-bottom:28px}
.bhead h1{font-size:28px;line-height:1.15;margin:0;font-weight:600;letter-spacing:-.02em}
.chips{display:flex;gap:8px;margin:12px 0 0;flex-wrap:wrap;align-items:center}
.chip,.sync{display:inline-flex;align-items:center;gap:6px;height:24px;padding:0 10px;border-radius:999px;border:1px solid var(--line-2);font-size:12.5px;color:var(--mute)}
.chip b{color:var(--ink);font-weight:600}
.dot{width:6px;height:6px;border-radius:50%;background:var(--line-2);flex:0 0 auto}
.dot.active{background:var(--accent)}.dot.done{background:var(--good)}.dot.blocked{background:var(--bad)}.dot.needs{background:var(--warn)}.dot.idea{background:transparent;box-shadow:inset 0 0 0 1.5px var(--line-2)}
.sync::before{content:"";width:6px;height:6px;border-radius:50%;background:var(--good);flex:0 0 auto}.sync.warn{color:var(--warn);border-color:var(--warn)}.sync.warn::before{background:var(--warn)}
.col h2{font-size:11.5px;line-height:16px;font-weight:600;text-transform:uppercase;letter-spacing:.07em;color:var(--mute);margin:0 0 12px;display:flex;align-items:center;gap:8px}
.col h2 .n{font-weight:500;letter-spacing:0;opacity:.8}
.needsyou{margin-bottom:28px}.needsyou h2{color:var(--accent)}
.cards{display:flex;flex-direction:column;gap:10px}
.cards.wide{display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:12px}
.cards.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:12px;align-items:start}
.work{display:grid;grid-template-columns:minmax(0,2fr) minmax(0,1fr);gap:28px;align-items:start}
.work.solo{grid-template-columns:minmax(0,1fr)}
.stack{display:flex;flex-direction:column;gap:24px}
.card{position:relative;background:var(--card);border:1px solid var(--line);border-radius:12px;padding:14px 16px 14px 20px;display:flex;flex-direction:column;gap:8px;min-width:0;box-shadow:var(--shadow);transition:box-shadow .2s}
.card::before{content:"";position:absolute;left:0;top:12px;bottom:12px;width:3px;border-radius:0 3px 3px 0;background:var(--line-2)}
.card.s-active::before{background:var(--accent)}.card.s-blocked::before{background:var(--bad)}.card.s-ready::before{background:var(--good)}.card.s-needs::before{background:var(--warn)}
.card.s-needs{background:var(--needs);border-color:var(--line-2)}
.card.compact{padding:11px 14px 11px 18px;gap:5px;border-radius:10px}
.card:hover{border-color:var(--line-2)}
.card h3{margin:0;font-size:15.5px;line-height:1.3;font-weight:600}.card.compact h3{font-size:14px}
.card h3 a{text-decoration:none;display:flex;gap:8px;align-items:baseline}.card h3 a:hover{text-decoration:underline}
.num{color:var(--mute);font-weight:400}.card h3 .num{font-size:12px;font-weight:500;flex:0 0 auto}
.meta{display:flex;flex-wrap:wrap;align-items:center;gap:8px;font-size:12px;color:var(--mute);margin:0}
.pill{padding:1px 8px;border-radius:999px;background:var(--tag);color:var(--tag-ink);font-weight:500;font-size:11.5px}
.pill.repo{background:transparent;border:1px solid var(--line-2);color:var(--mute)}
.pill.nostatus{background:transparent;border:1px dashed var(--line-2);color:var(--mute)}
.pri{color:var(--warn);font-weight:600;font-size:11px;letter-spacing:.05em;text-transform:uppercase}.pri.urgent{color:var(--bad)}
.meta .age{margin-left:auto}
.card .ask{margin:0;color:var(--accent);font-weight:500;font-size:13.5px;line-height:1.45}
.card .waits{margin:0;color:var(--mute);font-size:13.5px;line-height:1.45}
.card .next{margin:0;font-size:13.5px;line-height:1.45}
.bar{display:flex;align-items:center;gap:8px;font-size:11.5px;color:var(--mute)}
.bar .track{flex:1;max-width:160px;height:4px;border-radius:2px;background:var(--line);overflow:hidden}.bar .track i{display:block;height:100%;background:var(--good);border-radius:2px}
.card.flash{box-shadow:0 0 0 2px color-mix(in srgb,var(--accent) 55%,transparent);transition:box-shadow .3s}
.empty{color:var(--mute);font-style:italic;font-size:13px;margin:0}
.quiet{margin:14px 0 0;font-size:13px;color:var(--mute)}
.below{display:grid;grid-template-columns:minmax(0,2fr) minmax(0,1fr);gap:28px;margin-top:36px}
.below ul{margin:0;padding:0;list-style:none}
.today ul{max-width:66ch}
.today li{display:flex;gap:14px;padding:9px 0;border-top:1px solid var(--line);font-size:14px;line-height:1.5}
.today li:first-child{border-top:0;padding-top:0}
.today li .num{flex:0 0 auto;min-width:34px;height:22px;padding:0 6px;border-radius:6px;background:var(--tag);color:var(--tag-ink);font-size:11.5px;font-weight:500;display:flex;align-items:center;justify-content:center;margin-top:1px}
.today li .num a{text-decoration:none}
.today li .num.repo{background:transparent;border:1px solid var(--line-2);color:var(--mute)}
.today li.empty{padding:0;border:0}
.initiatives li{display:grid;grid-template-columns:1fr auto;gap:4px 12px;padding:8px 0;border-top:1px solid var(--line);font-size:13px}
.initiatives li:first-child{border-top:0;padding-top:0}
.initiatives li .num{font-size:12px}
.initiatives .track{grid-column:1 / -1;height:4px;border-radius:2px;background:var(--line);overflow:hidden}.initiatives .track i{display:block;height:100%;background:var(--accent);border-radius:2px}
.col.done{margin-top:36px}
.closed{margin:0;padding:0;list-style:none;columns:2;column-gap:36px}
.closed li{display:flex;align-items:center;gap:12px;height:34px;border-top:1px solid var(--line);font-size:13.5px;color:var(--mute);break-inside:avoid}
.closed li:first-child{border-top:0}
.closed a{flex:1;min-width:0;display:flex;gap:12px;align-items:center;text-decoration:none}.closed a:hover .t{text-decoration:underline}
.closed .t{color:var(--ink);flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.closed .num{width:22px;text-align:right;font-size:12px;flex:0 0 auto}
.closed .tick{width:14px;height:14px;fill:none;stroke:var(--good);stroke-width:2.5;flex:0 0 auto}
.closed .abandoned .tick{stroke:var(--line-2)}
.closed .age{font-size:12px}
.note{display:grid;grid-template-columns:minmax(0,1fr) 220px;gap:36px;padding:26px 32px 60px;max-width:1120px}
.note.wide{grid-template-columns:minmax(0,1fr)}
.props{display:flex;flex-wrap:wrap;gap:6px;margin:-6px 0 22px;font:12px/1.5 -apple-system,"Segoe UI",Inter,system-ui,sans-serif}
.props .pill{display:inline-flex;align-items:center;gap:5px}
.props .pill i{font-style:normal;color:var(--mute)}
.props .pill.status-active{background:color-mix(in srgb,var(--accent) 16%,var(--card));color:var(--accent)}.props .pill.status-active i{color:inherit;opacity:.75}
.props .pill.status-done{background:color-mix(in srgb,var(--good) 16%,var(--card));color:var(--good)}.props .pill.status-done i{color:inherit;opacity:.75}
.props .pill.status-blocked{background:color-mix(in srgb,var(--bad) 16%,var(--card));color:var(--bad)}.props .pill.status-blocked i{color:inherit;opacity:.75}
.facts{margin:16px 0 0;padding:12px 0 0;border-top:1px solid var(--line);font:12px/1.7 -apple-system,"Segoe UI",Inter,system-ui,sans-serif;color:var(--mute)}
.facts dl{margin:0;display:grid;grid-template-columns:auto 1fr;gap:0 10px}.facts dt{margin:0}.facts dd{margin:0;color:var(--ink);overflow-wrap:anywhere}.facts code{font:11px ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
.prose .facts{max-width:40ch}
.prose{font:16px/1.65 Charter,"Iowan Old Style","Source Serif 4",Georgia,serif;max-width:70ch;min-width:0}
.prose h1{font:600 28px/1.2 -apple-system,"Segoe UI",Inter,system-ui,sans-serif;letter-spacing:-.015em;margin:0 0 16px;text-wrap:balance}
.prose h2{font:600 19px/1.3 -apple-system,"Segoe UI",Inter,system-ui,sans-serif;margin:32px 0 10px;padding-top:14px;border-top:1px solid var(--line)}
.prose h3{font:600 16px/1.35 -apple-system,"Segoe UI",Inter,system-ui,sans-serif;margin:22px 0 8px}
.prose h4{font:600 14px/1.4 -apple-system,"Segoe UI",Inter,system-ui,sans-serif;margin:18px 0 6px;color:var(--mute);text-transform:uppercase;letter-spacing:.04em}
.prose h5,.prose h6{font:600 14px/1.4 -apple-system,"Segoe UI",Inter,system-ui,sans-serif;margin:14px 0 4px}
.prose p{margin:0 0 14px}.prose ul,.prose ol{padding-left:22px;margin:0 0 14px}.prose li{margin:4px 0}.prose li>p{margin:0}
.prose input[type=checkbox]{accent-color:var(--accent);margin:0 6px 0 -2px;vertical-align:-1px}
.prose li:has(input:checked){color:var(--mute)}
.prose a{color:var(--accent);text-decoration:none;border-bottom:1px solid color-mix(in srgb,var(--accent) 35%,transparent)}.prose a:hover{border-bottom-color:var(--accent)}
.prose a[href="#"]{color:var(--mute);border-bottom-style:dotted;cursor:default}
.prose code{font:13.5px ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;background:var(--code);border:1px solid var(--line);border-radius:5px;padding:1px 5px}
.prose pre{background:var(--code);border:1px solid var(--line);border-radius:10px;padding:14px 16px;overflow:auto;font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;margin:0 0 16px}
.prose pre code{background:none;border:0;padding:0;font-size:inherit}
.prose blockquote{margin:0 0 14px;padding:4px 0 4px 16px;border-left:3px solid var(--line-2);color:var(--mute);font-style:italic}
.prose table{border-collapse:collapse;font:14px -apple-system,"Segoe UI",Inter,system-ui,sans-serif;margin:0 0 16px;display:block;overflow-x:auto;max-width:100%}
.prose th,.prose td{border:1px solid var(--line);padding:6px 10px;text-align:left;vertical-align:top}.prose th{background:var(--code);font-weight:600}
.prose hr{border:0;border-top:1px solid var(--line);margin:24px 0}.prose strong{font-weight:600}.prose img{max-width:100%;border-radius:8px}
.outline{position:sticky;top:60px;align-self:start;font-size:12.5px;color:var(--mute);max-height:calc(100vh - 84px);overflow:auto}
.outline h4{margin:0 0 8px;font-size:11px;line-height:16px;text-transform:uppercase;letter-spacing:.07em;font-weight:600}
.outline a{display:block;padding:3px 0 3px 10px;border-left:2px solid var(--line);text-decoration:none;color:var(--mute);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.outline a.l3{padding-left:20px}.outline a:hover{color:var(--ink);border-left-color:var(--accent)}
.scrim{display:none}
@media (max-width:980px){.work,.below{grid-template-columns:minmax(0,1fr)}.closed{columns:1}}
@media (max-width:820px){
.side{position:fixed;inset:0 auto 0 0;z-index:30;transform:translateX(-100%);transition:transform .2s;box-shadow:0 0 40px rgba(0,0,0,.25)}
.side.open{transform:none}
.scrim{display:block;position:fixed;inset:0;background:rgba(0,0,0,.3);z-index:25;opacity:0;pointer-events:none;transition:opacity .2s}.scrim.on{opacity:1;pointer-events:auto}
.menu-btn{display:inline-flex}
.board{padding:16px 16px 40px}.bhead h1{font-size:24px}.note{grid-template-columns:minmax(0,1fr);padding:18px 16px 50px;gap:0}.outline{display:none}.topbar{padding:8px 16px}
}
@media (prefers-reduced-motion:reduce){*{transition:none!important}}
:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
"##;

/// What the page keeps for the viewer: which folders are open, the look they chose, the
/// scroll position across the file copy's reload; and the search box, the hidden folders'
/// toggle, the ages ticking, the drawer on a narrow screen.
const SCRIPT: &str = r#"(function(){
var $=function(s){return document.querySelector(s)};
var read=function(k){try{return JSON.parse(localStorage.getItem(k)||'{}')}catch(e){return {}}};
var save=function(k,v){try{localStorage.setItem(k,JSON.stringify(v))}catch(e){}};
var root=document.documentElement;
var tree=$('#tree');
var open=read('astrolabe-tree');
var filter=function(){var q=$('#q').value.trim().toLowerCase();tree.classList.toggle('filtering',!!q);tree.querySelectorAll('.f').forEach(function(a){a.classList.toggle('miss',!!q&&a.textContent.toLowerCase().indexOf(q)<0&&(a.title||'').toLowerCase().indexOf(q)<0&&(a.dataset.rel||'').toLowerCase().indexOf(q)<0)});if(q)tree.querySelectorAll('details').forEach(function(d){d.open=true})};
var bindTree=function(){
tree.querySelectorAll('details[data-key]').forEach(function(d){var k=d.dataset.key;if(k in open)d.open=!!open[k];d.addEventListener('toggle',function(){open[k]=d.open;save('astrolabe-tree',open)})});
var hid=$('#hid'),ht=$('#hidtoggle');
if(ht){var show=function(on){hid.hidden=!on;ht.textContent=(on?'Hide ':'Show ')+ht.dataset.names+' · '+ht.dataset.n+' note'+(ht.dataset.n==='1'?'':'s')+' agents wrote for each other'};show(!!read('astrolabe-hidden').on);ht.addEventListener('click',function(){var on=hid.hidden;save('astrolabe-hidden',{on:on});show(on)})}
filter();
};
bindTree();
$('#q').addEventListener('input',filter);
/* The served page follows every save: the server says `changed`, the page reads itself
again and swaps the tree and the board or the note in place, so nothing scrolls or
closes, and a card that changed is marked for a moment. */
if(location.protocol!=='file:'&&window.EventSource){
var live=$('#live');live.hidden=false;live.classList.add('off');
var swap=function(sel,doc){var a=document.querySelector(sel),b=doc.querySelector(sel);if(a&&b)a.innerHTML=b.innerHTML};
var refresh=function(){fetch(location.pathname,{cache:'no-store'}).then(function(r){return r.text()}).then(function(html){
var doc=new DOMParser().parseFromString(html,'text/html');
var before={};document.querySelectorAll('.card[data-n]').forEach(function(c){before[c.dataset.n]=c.innerHTML});
swap('#tree',doc);bindTree();
var board=$('#board');if(board){swap('#board',doc);document.querySelectorAll('.card[data-n]').forEach(function(c){if(before[c.dataset.n]!==c.innerHTML){c.classList.add('flash');setTimeout(function(){c.classList.remove('flash')},1500)}})}
var note=document.querySelector('.note');if(note){var fresh=doc.querySelector('.note');if(fresh&&fresh.innerHTML!==note.innerHTML)note.innerHTML=fresh.innerHTML}
tick();
}).catch(function(){})};
var es=new EventSource('/events');
var opened=false;es.onopen=function(){live.classList.remove('off');if(opened)refresh();opened=true};
es.onerror=function(){live.classList.add('off')};
es.addEventListener('changed',refresh);
}
var look=read('astrolabe-look');
var apply=function(){if(look.palette)root.dataset.palette=look.palette;if(look.appearance)root.dataset.theme=look.appearance;else if(look.appearance==='')root.removeAttribute('data-theme');$('#appearance').value=look.appearance||(root.dataset.theme||'');document.querySelectorAll('.sw').forEach(function(b){b.classList.toggle('on',(root.dataset.palette||'ember')===b.dataset.p)})};
$('#appearance').addEventListener('change',function(e){look.appearance=e.target.value;save('astrolabe-look',look);apply()});
document.querySelectorAll('.sw').forEach(function(b){b.addEventListener('click',function(){look.palette=b.dataset.p;save('astrolabe-look',look);apply()})});
apply();
var age=function(ms){var s=Math.max(0,ms)/1000;if(s<60)return 'just now';if(s<3600)return Math.floor(s/60)+' min';if(s<86400)return Math.floor(s/3600)+' h';return Math.floor(s/86400)+' d'};
var tick=function(){document.querySelectorAll('.age[data-t]').forEach(function(a){a.textContent=age(Date.now()-(+a.dataset.t))})};
tick();setInterval(tick,60000);
var main=$('#main');
var key='astrolabe-scroll:'+location.pathname;
try{var y=sessionStorage.getItem(key);if(y)main.scrollTop=+y}catch(e){}
main.addEventListener('scroll',function(){try{sessionStorage.setItem(key,main.scrollTop)}catch(e){}});
$('#menu').addEventListener('click',function(){$('#side').classList.add('open');$('#scrim').classList.add('on')});
$('#scrim').addEventListener('click',function(){$('#side').classList.remove('open');$('#scrim').classList.remove('on')});
})();"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_793_916_000_000; // 2026-11-05T15:00-07:00

    fn row(n: u64, status: &str, extra: Value) -> Value {
        let mut v = json!({
            "number": n, "title": format!("Effort {n}"), "file": format!("efforts/{n:04}-e.md"),
            "status": status, "priority": "none", "initiative": "", "repo": "demo",
            "waits_for": "", "needs_owner": false, "updated": "2026-11-05T12:00-07:00",
            "next": "Next thing. Then more.", "pieces": null,
        });
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    }

    fn section<'a>(page: &'a str, key: &str) -> &'a str {
        let start = page.find(&format!("class=\"col {key}\"")).unwrap();
        let end = page[start + 1..]
            .find("<section")
            .map_or(page.len(), |i| start + 1 + i);
        &page[start..end]
    }

    #[test]
    fn cards_land_in_their_columns() {
        let rows = json!({
            "today": "2026-11-05",
            "efforts": [
                row(1, "active", json!({"title": "A <b>&</b> \"q\""})),
                row(2, "active", json!({"waits_for": "owner: pick one", "needs_owner": true})),
                row(3, "blocked", json!({"waits_for": "the merge driver"})),
                row(4, "ready", json!({})),
                row(5, "proposed", json!({})),
                row(6, "", json!({"updated": null})),
                row(7, "done", json!({"updated": "2026-11-02T09:00-07:00"})),
                row(8, "done", json!({"updated": "2026-10-10T09:00-07:00", "initiative": "[[finished]]"})),
                row(9, "active", json!({"priority": "high", "pieces": {"done": 2, "total": 5}, "initiative": "[[agent-tooling]]", "next": "**Next:** Measure it. Then decide."})),
                row(10, "x\" onmouseover=\"alert(1)", json!({})),
            ],
            "journal_lines": [{"effort": "0001", "text": "Finished <it>."}],
            "notes": [],
        });
        let page = render(&rows, Path::new("/n"), NOW, Look::default());
        assert!(page.contains("A &lt;b&gt;&amp;&lt;/b&gt; &quot;q&quot;"));
        assert!(!page.contains("<b>&</b>"));
        let needs = section(&page, "needsyou");
        assert!(needs.contains("Effort 2") && needs.contains("class=\"ask\">pick one<"));
        let active = section(&page, "active");
        assert!(active.contains("&quot;q&quot;") && active.contains("Effort 9"));
        assert!(!active.contains("Effort 2"));
        // Priority first, then the newest save.
        assert!(active.find("Effort 9").unwrap() < active.find("&quot;q&quot;").unwrap());
        assert!(active.contains("class=\"pri high\">high<"));
        assert!(active.contains(">2 of 5 pieces<"));
        assert!(active.contains("class=\"pill tag\">agent-tooling<"));
        assert!(active.contains("class=\"next\">Measure it.<"));
        assert!(active.contains(">3 h</span>"));
        assert!(active.contains("href=\"file:///n/efforts/0001-e.md\""));
        let blocked = section(&page, "blocked");
        assert!(blocked.contains("Effort 3") && blocked.contains(">waits for the merge driver<"));
        assert!(section(&page, "ready").contains("Effort 4"));
        let ideas = section(&page, "ideas");
        assert!(ideas.contains("Effort 5") && ideas.contains("Effort 6"));
        assert!(ideas.contains("no status"));
        assert!(
            ideas.contains("class=\"card s-x&quot; onmouseover=&quot;alert(1)\"")
                && !page.contains("onmouseover=\""),
            "a status stays inside its class: {ideas}"
        );
        let done = section(&page, "done");
        assert!(done.contains("Effort 7"));
        assert!(
            !page.contains("Effort 8"),
            "an old done effort is off the board"
        );
        assert!(page.contains("<b>Board</b>"), "{page}");
        assert!(
            page.contains("<b>1</b> needs you</span>")
                && page.contains("<b>3</b> active</span>")
                && page.contains("<b>1</b> blocked</span>")
                && page.contains("<b>1</b> done</span>"),
            "{page}"
        );
        assert!(page.contains("notes in sync"));
        assert!(page.contains("<a href=\"file:///n/efforts/0001-e.md\">1</a></span><span>Finished &lt;it&gt;.</span></li>"));
        assert!(page.contains("1 open · 0 done"));
        assert!(
            !page.contains("finished"),
            "an initiative with nothing open is off the board"
        );
        assert!(
            !page.contains("<details"),
            "done is a column like the others, open"
        );
    }

    #[test]
    fn empty_board_and_warnings() {
        let rows = json!({"today": "2026-11-05", "efforts": [], "journal_lines": [], "notes": ["2 notes commit(s) not yet pushed."]});
        let page = render(&rows, Path::new("/n"), NOW, Look::default());
        assert!(page.contains("class=\"sync warn\">2 notes commit(s) not yet pushed.<"));
        assert!(page.contains("nothing yet today"));
        assert!(page.contains("nothing open"));
        assert!(
            page.contains("class=\"empty\">nothing in flight<"),
            "{page}"
        );
        assert!(
            page.contains("class=\"quiet\">Nothing ready, nothing blocked, no ideas parked.<"),
            "{page}"
        );
        assert!(!page.contains("<b>0</b>"), "a zero is no chip: {page}");
        assert!(page.contains("nothing closed lately"));
    }

    #[test]
    fn sentences_and_ages() {
        assert_eq!(first_sentence("One. Two."), "One.");
        assert_eq!(first_sentence("**Next:** go\nnow. x"), "go now.");
        assert_eq!(first_sentence("No stop"), "No stop");
        let long = "x".repeat(200);
        let cut = first_sentence(&long);
        assert_eq!(cut.chars().count(), SENTENCE_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(age(5 * 60_000), "5 min");
        assert_eq!(age(3 * 3_600_000 + 1), "3 h");
        assert_eq!(age(50 * 3_600_000), "2 d");
        assert_eq!(stamp_ms("2026-11-05T12:00-07:00"), Some(1_793_905_200_000));
        assert_eq!(stamp_ms("nope"), None);
    }

    #[test]
    fn themes_and_palettes() {
        let rows = json!({"today": "2026-11-05", "efforts": [], "journal_lines": [], "notes": []});
        let auto = render(&rows, Path::new("/n"), NOW, Look::default());
        assert!(
            auto.contains("<html lang=\"en\" data-palette=\"ember\"><head>")
                && auto.contains("content=\"light dark\"")
                && auto.contains("prefers-color-scheme:dark"),
            "{auto}"
        );
        let dark = render(
            &rows,
            Path::new("/n"),
            NOW,
            Look {
                theme: Theme::Dark,
                palette: Palette::Iris,
            },
        );
        assert!(dark.contains("data-palette=\"iris\" data-theme=\"dark\""));
        assert!(dark.contains("content=\"dark\""));
        assert!(dark.contains(":root[data-palette=\"iris\"]"));
        assert_eq!(Theme::parse("system"), Some(Theme::Auto));
        assert_eq!(Theme::parse("blue"), None);
        assert_eq!(Palette::parse("slate"), Some(Palette::Slate));
        assert_eq!(Palette::parse("mauve"), None);
    }

    fn note_page_at(text: &str) -> String {
        note_page(
            text,
            "efforts/0001-x.md",
            &json!({"efforts": []}),
            Path::new("/n"),
            NOW,
            Look::default(),
        )
    }

    #[test]
    fn links_resolve_to_notes_and_headings_get_ids() {
        let dir = std::env::temp_dir().join(format!("astrolabe-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("efforts/2026/11-November")).unwrap();
        std::fs::create_dir_all(dir.join("knowledge")).unwrap();
        std::fs::write(dir.join("knowledge/owner.md"), "# Owner\n").unwrap();
        std::fs::write(dir.join("efforts/2026/11-November/0002-b.md"), "# b\n").unwrap();
        let rel = "efforts/2026/11-November/0001-a.md";
        let text = "# 0001 A\n\n## Current state\n\nSee [b](0002-b.md), [owner](../../../knowledge/owner.md#Boundaries), [root](knowledge/owner.md), [gone](nope.md), [[Rough Sketch]].\n\n### Pieces & bits\n\n- [ ] one <!-- hidden -->\n";
        let page = note_page(
            text,
            rel,
            &json!({"efforts": []}),
            &dir,
            NOW,
            Look::default(),
        );
        assert!(
            page.contains("href=\"/efforts/2026/11-November/0002-b.md\""),
            "{page}"
        );
        assert!(
            page.contains("href=\"/knowledge/owner.md#boundaries\""),
            "{page}"
        );
        assert_eq!(
            page.matches("href=\"/knowledge/owner.md").count(),
            3,
            "{page}"
        );
        assert!(
            page.contains("href=\"nope.md\""),
            "a link to nothing is left as written"
        );
        assert!(page.contains("href=\"#\">Rough Sketch</a>"), "{page}");
        assert!(page.contains("<h2 id=\"current-state\">"), "{page}");
        assert!(page.contains("<h3 id=\"pieces-bits\">"), "{page}");
        assert!(
            page.contains("class=\"l2\" href=\"#current-state\""),
            "an outline: {page}"
        );
        assert!(
            !page.contains("hidden -->"),
            "a comment is not shown: {page}"
        );
        assert!(
            page.contains("<b>0001-a.md</b>"),
            "the crumb ends in the file: {page}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_note_is_served_only_from_inside_the_checkout() {
        let dir = std::env::temp_dir().join(format!("astrolabe-inside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("notes/knowledge")).unwrap();
        std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
        std::fs::write(dir.join("notes/knowledge/owner.md"), "# o\n").unwrap();
        std::fs::write(dir.join("elsewhere/secret.md"), "# s\n").unwrap();
        let root = dir.join("notes");
        assert!(inside(&root, "knowledge/owner.md"));
        assert!(!inside(&root, "knowledge/nope.md"), "nothing there");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("elsewhere"), root.join("out")).unwrap();
            std::os::unix::fs::symlink(
                dir.join("elsewhere/secret.md"),
                root.join("knowledge/alias.md"),
            )
            .unwrap();
            assert!(!inside(&root, "out/secret.md"), "a folder link out");
            assert!(!inside(&root, "knowledge/alias.md"), "a file link out");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_look_keeps_the_port_and_the_port_keeps_the_look() {
        let dir = std::env::temp_dir().join(format!("astrolabe-look-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state")).unwrap();
        let notes = Notes::new(dir.join("notes"), dir.join("state"));
        assert_eq!(port(&notes), SERVE_PORT);
        set_port(&notes, 5050).unwrap();
        set_look(&notes, Some(Theme::Dark), Some(Palette::Slate)).unwrap();
        assert_eq!(port(&notes), 5050);
        set_port(&notes, 5051).unwrap();
        assert_eq!(
            look(&notes),
            Look {
                theme: Theme::Dark,
                palette: Palette::Slate
            }
        );
        assert_eq!(saved_port(&notes), Some(5051));
        set_port(&notes, 0).unwrap();
        assert_eq!(port(&notes), SERVE_PORT, "0 means the default");
        assert_eq!(saved_port(&notes), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tree_lists_shelves_newest_first_and_hides_dot_folders() {
        let dir = std::env::temp_dir().join(format!("astrolabe-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for p in [
            "efforts/2026/10-October/0001-a.md",
            "efforts/2026/11-November/0002-b.md",
            "journal/2026/11-November/05-Thursday.md",
            "knowledge/owner.md",
            ".agents/brief.md",
            "README.md",
        ] {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "# x\n").unwrap();
        }
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/config"), "").unwrap();
        let rows = json!({"efforts": [
            {"number": 2, "title": "B", "file": "efforts/2026/11-November/0002-b.md", "status": "active", "needs_owner": false},
        ]});
        let tree = tree_html(
            &dir,
            &rows,
            Links::Served,
            Some("efforts/2026/11-November/0002-b.md"),
        );
        let at = |s: &str| tree.find(s).unwrap_or_else(|| panic!("{s} in {tree}"));
        assert!(at("data-key=\"efforts\"") < at("data-key=\"journal\""));
        assert!(at("data-key=\"journal\"") < at("data-key=\"knowledge\""));
        assert!(at("November") < at("October"), "newest month first: {tree}");
        assert!(
            tree.contains("<details open data-key=\"efforts/2026/11-November\">"),
            "{tree}"
        );
        assert!(
            tree.contains("<details data-key=\"efforts/2026/10-October\">"),
            "{tree}"
        );
        assert!(
            tree.contains(
                "class=\"f on\" style=\"--d:2\" href=\"/efforts/2026/11-November/0002-b.md\""
            ),
            "{tree}"
        );
        assert!(tree.contains("class=\"st s-active\""));
        assert!(tree.contains("<span class=\"num\">2</span><span class=\"t\">B</span>"));
        assert!(tree.contains("href=\"/README.md\""));
        assert!(!tree.contains(".git/"), "{tree}");
        let hid = &tree[at("id=\"hid\" hidden")..];
        assert!(hid.contains("href=\"/.agents/brief.md\""), "{tree}");
        assert!(
            !tree.contains("data-key=\"efforts/2026\""),
            "a year is a shelf of months, not a level: {tree}"
        );
        assert!(tree.contains(">November 2026</span>"), "{tree}");
        assert!(tree.contains("Show .agents · 1 note agents wrote for each other"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_note_is_a_page_with_its_properties_and_no_raw_html() {
        let text = "---\nstatus: active\nrepo: demo\nwaits_for: \"owner: pick\"\nupdated: \"2026-11-05T12:00-07:00\"\n---\n# 0001 Smooth <ink>\n\n## Current state\n\nDone & dusted. <script>alert(1)</script>\n\n- [x] one\n";
        let page = note_page_at(text);
        assert!(
            page.contains("<title>0001 Smooth &lt;ink&gt;</title>"),
            "{page}"
        );
        assert!(
            page.contains("<h1 id=\"0001-smooth\">0001 Smooth &lt;ink&gt;</h1>"),
            "{page}"
        );
        assert!(
            page.contains("class=\"pill status-active\"><i>status</i>active</span>")
                && page.contains("<i>waits for</i>owner: pick</span>"),
            "{page}"
        );
        assert!(page.contains("<h2 id=\"current-state\">Current state</h2>"));
        assert!(page.contains("Done &amp; dusted."), "{page}");
        assert!(
            !page.contains("<script>alert") && page.contains("&lt;script&gt;alert(1)"),
            "raw html is text: {page}"
        );
        assert!(page.contains("checked"), "task lists render: {page}");
        assert!(page.contains("href=\"/\""), "a way back to the board");
        assert!(
            !page.contains("http-equiv=\"refresh\""),
            "a note doesn't reload under the reader"
        );
        // No frontmatter: the whole text is the body.
        assert!(note_page_at("# Bare\n\ntext\n").contains("<h1 id=\"bare\">Bare</h1>"));
        // Links keep to pages and mail; a script or file link loses its destination.
        let links = note_page_at(
            "[a](javascript:alert(1)) [b](JAVASCRIPT:x) [c](&#106;avascript:y) [d](vbscript:z) [e](file:///etc/passwd) [f](data:text/html,x) ![g](javascript:w) [h](https://example.com/x?y#z) [i](mailto:o@example.com) [j](/efforts/0002-x.md) [k](other.md) [l](a/b:c)\n",
        );
        // The note's own part of the page: the shell's style and script have their own
        // `href="#"` rule and the tree its own links.
        assert!(links.contains("Content-Security-Policy"), "{links}");
        let links = &links[links.find("class=\"prose\"").unwrap()..links.find("</main>").unwrap()];
        assert!(
            !links.to_ascii_lowercase().contains("javascript:"),
            "{links}"
        );
        for bad in ["vbscript:", "file:", "data:text"] {
            assert!(!links.contains(bad), "{bad}: {links}");
        }
        assert_eq!(links.matches("href=\"#\"").count(), 6, "{links}");
        assert!(links.contains("src=\"#\""), "{links}");
        for good in [
            "href=\"https://example.com/x?y#z\"",
            "href=\"mailto:o@example.com\"",
            "href=\"/efforts/0002-x.md\"",
            "href=\"other.md\"",
            "href=\"a/b:c\"",
        ] {
            assert!(links.contains(good), "{good}: {links}");
        }
    }

    #[test]
    fn served_names_round_trip() {
        assert_eq!(
            decode(&percent("0001-caf\u{e9} b.md")).as_deref(),
            Some("0001-caf\u{e9} b.md")
        );
        assert_eq!(decode("a%2"), None);
        assert_eq!(decode("%ff"), None);
        assert_eq!(decode("%+A"), None);
    }

    #[test]
    fn file_links() {
        assert_eq!(
            file_url(Path::new("/Users/n/My Notes/efforts/1-a#b.md")),
            "file:///Users/n/My%20Notes/efforts/1-a%23b.md"
        );
        assert_eq!(
            file_url(Path::new("/n/caf\u{e9}.md")),
            "file:///n/caf%C3%A9.md"
        );
        assert_eq!(
            file_url(Path::new("C:\\Users\\n\\notes\\x.md")),
            "file:///C:/Users/n/notes/x.md"
        );
        assert_eq!(
            file_url(Path::new("\\\\?\\C:\\n\\x.md")),
            "file:///C:/n/x.md"
        );
    }
}
