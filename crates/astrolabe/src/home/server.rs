//! Local HTTP lifecycle, bounded requests and live event streams.
use super::FILE;
use super::markdown::note_page;
use super::render::{Links, SERVE_REFRESH_SECS, page};
use super::settings::{SERVE_PORT, look, port, saved_port, url};
use crate::notes::{self, Notes};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The machine's one server holds this file; a second `--serve` sees it and leaves.
pub(super) const LOCK_FILE: &str = "home.lock";

/// With no request this long, the server stops; the next change starts one again.
pub(super) const IDLE_SECS: u64 = 24 * 3600;

/// Tests set a shorter idle, in seconds.
pub(super) const IDLE_SECS_VAR: &str = "ASTROLABE_HOME_IDLE_SECS";

/// Tests ask for no process: a marker says the home was wanted.
pub(super) const NO_DETACHED_VAR: &str = "ASTROLABE_NO_DETACHED_SYNC";

pub(super) const REQUESTED_MARKER: &str = "home-requested";

/// How often the notes are pulled while a page holds the stream, so a machine that's only
/// being looked at still shows the other machines' saves.
pub(super) const PULL_EVERY_SECS: u64 = 60;

/// A page load asks for a pull at most this often.
pub(super) const ASK_GAP: Duration = Duration::from_secs(10);

/// Tests set a shorter pull interval, in seconds.
pub(super) const PULL_EVERY_VAR: &str = "ASTROLABE_HOME_PULL_SECS";

/// What the server's threads share: when a request last came, how many pages hold the
/// stream, and whether a page load asked for a pull.
#[derive(Default)]
pub(super) struct Watch {
    seen: AtomicI64,
    viewers: AtomicUsize,
    wanted: AtomicBool,
}

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
pub(super) fn answering(port: u16) -> bool {
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
    let watch = Arc::new(Watch::default());
    watch.seen.store(notes::now_ms(), Ordering::SeqCst);
    let idle = std::env::var(IDLE_SECS_VAR)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(IDLE_SECS);
    let seen = Arc::clone(&watch);
    let watched = Notes::new(notes.root.clone(), notes.state.clone());
    std::thread::spawn(move || {
        // The lock lives as long as this thread, which is as long as the process.
        let _hold = hold;
        let every = Duration::from_millis((idle * 1000 / 2).clamp(100, 60_000));
        loop {
            std::thread::sleep(every);
            let quiet = notes::now_ms() - seen.seen.load(Ordering::SeqCst);
            // The lock goes with the process; the next change starts a server again, on
            // the port the machine now wants if that changed.
            let moved = saved_port(&watched).unwrap_or(SERVE_PORT) != requested;
            if moved || quiet >= (idle * 1000) as i64 {
                std::process::exit(0);
            }
        }
    });
    let puller = Arc::clone(&watch);
    let pulled = Notes::new(notes.root.clone(), notes.state.clone());
    std::thread::spawn(move || pull_while_watched(&pulled, &puller));
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        watch.seen.store(notes::now_ms(), Ordering::SeqCst);
        if busy.load(Ordering::SeqCst) >= HANDLERS {
            let _ = stream.write_all(BUSY);
            continue;
        }
        let notes = Notes::new(notes.root.clone(), notes.state.clone());
        let count = Arc::clone(&busy);
        let seen = Arc::clone(&watch);
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

/// Pulls the notes every `PULL_EVERY_SECS` while a page holds the stream, and soon after a
/// page load asks. A pull that brings commits moves the files, so the open pages re-read
/// through the stream as they would for a local save. Never pushes: that stays with sessions.
fn pull_while_watched(notes: &Notes, watch: &Watch) {
    let every = std::env::var(PULL_EVERY_VAR)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(PULL_EVERY_SECS)
        .max(1);
    let every = Duration::from_secs(every);
    let mut last: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let since = last.map(|at| at.elapsed());
        // A page reloading after a pull it caused doesn't ask for another straight away.
        let asked = watch.wanted.load(Ordering::SeqCst)
            && since.is_none_or(|t| t >= every.min(ASK_GAP))
            && watch.wanted.swap(false, Ordering::SeqCst);
        let due = watch.viewers.load(Ordering::SeqCst) > 0 && since.is_none_or(|t| t >= every);
        if asked || due {
            // A writer holding the lock is mid-save and pulled first itself; this one is tried
            // again on the next tick, once the lock is free.
            if crate::application::background_pull(notes).is_some() {
                last = Some(Instant::now());
            }
        }
    }
}

pub(super) const HANDLERS: usize = 32;

pub(super) const BUSY: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

pub(super) const REQUEST_BOUND: usize = 16 * 1024;

pub(super) const REQUEST_DEADLINE: Duration = Duration::from_secs(5);

pub(super) fn handle(notes: &Notes, mut stream: TcpStream, port: u16, watch: &Watch) {
    let _ = stream.set_read_timeout(Some(REQUEST_DEADLINE));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let (status, kind, body, head_only) = match request(&mut stream) {
        Some(head) if is_events(&head, port) => {
            watch.viewers.fetch_add(1, Ordering::SeqCst);
            events(notes, stream, &watch.seen);
            watch.viewers.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        Some(head) => {
            let head_only = head.split_whitespace().next() == Some("HEAD");
            let (status, kind, body) = answer(notes, &head, port);
            // A page loaded: pull soon, without the answer waiting on the network.
            if !head_only && status.starts_with("200") {
                watch.wanted.store(true, Ordering::SeqCst);
            }
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
pub(super) fn is_events(head: &str, port: u16) -> bool {
    let mut words = head.lines().next().unwrap_or_default().split_whitespace();
    words.next() == Some("GET")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some("/events"))
        && host_ok(head, port)
}

/// Whether the request names this loopback port as its host, so a page on another origin
/// that reaches the socket by a rebound name gets nothing.
pub(super) fn host_ok(head: &str, port: u16) -> bool {
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
pub(super) const WATCH_EVERY: Duration = Duration::from_secs(1);

pub(super) const PING_EVERY: Duration = Duration::from_secs(15);

/// The stream a page holds open: `changed` whenever the notes change, a comment to keep it
/// alive meanwhile. The page re-reads itself on `changed`. A viewer holding the stream
/// counts as a request, so the server stays up for an open tab.
pub(super) fn events(notes: &Notes, mut stream: TcpStream, seen: &AtomicI64) {
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
pub(super) fn fingerprint(root: &Path) -> (i64, usize, i64) {
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

pub(super) fn modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// The request head, read until its blank line within one deadline for the whole of it; None
/// when it never comes or outgrows the bound.
pub(super) fn request(stream: &mut TcpStream) -> Option<String> {
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
pub(super) fn answer(notes: &Notes, head: &str, port: u16) -> (&'static str, &'static str, String) {
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

/// Percent-decoding of a path segment, the inverse of `percent`; None for a malformed escape
/// or bytes that aren't UTF-8.
pub(super) fn decode(text: &str) -> Option<String> {
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

/// Whether the note at `rel` really sits under the checkout once links are followed: a
/// symlink or junction inside it pointing out is not served.
pub(super) fn inside(root: &Path, rel: &str) -> bool {
    let (Ok(real), Ok(root)) = (root.join(rel).canonicalize(), root.canonicalize()) else {
        return false;
    };
    real.starts_with(&root)
}
