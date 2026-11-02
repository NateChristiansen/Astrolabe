//! The one local WebSocket to the app: loopback only, its own origins only, and a secret
//! made fresh each start, handed over in an owner-only file and sent as the first frame.
use crate::error::{Error, Result};
use protocol::{BridgeEvent, BridgeRequest};
use std::fs;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::{Message, WebSocket};

/// Tauri's own page on macOS and on Windows.
const APP_ORIGINS: [&str; 2] = ["tauri://localhost", "http://tauri.localhost"];
const AUTH_WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);
/// Read by each client's own thread, so a stuck session can't keep a window from hearing.
static QUITTING: AtomicBool = AtomicBool::new(false);
static CLOSING: AtomicBool = AtomicBool::new(false);
static CLIENTS: AtomicUsize = AtomicUsize::new(0);

pub enum Inbound {
    /// A client passed both checks. Whatever the core has to say goes here.
    Connected(Sender<String>),
    Request(BridgeRequest),
}

/// Refuses anything but a loopback address, whatever the caller was told.
pub fn bind(addr: SocketAddr) -> Result<TcpListener> {
    if !addr.ip().is_loopback() {
        let why = format!("the bridge binds to loopback only, not {addr}");
        return Err(Error::Setup(why));
    }
    Ok(TcpListener::bind(addr)?)
}

fn new_secret() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|why| Error::Setup(format!("no randomness: {why}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Port on the first line, secret on the second. Aside and renamed, so it appears whole.
pub fn write_file(file: &Path, port: u16, secret: &str) -> std::io::Result<()> {
    let aside = file.with_extension(format!("{}.tmp", std::process::id()));
    gone(&aside)?;
    crate::private_file::write(&aside, &format!("{port}\n{secret}\n"))?;
    fs::rename(&aside, file)?;
    Ok(())
}

fn gone(path: &Path) -> std::io::Result<()> {
    let missing = |why: &std::io::Error| why.kind() == ErrorKind::NotFound;
    fs::remove_file(path).or_else(|why| if missing(&why) { Ok(()) } else { Err(why) })
}

/// Every client, and any that arrives from now on, hears the quit within `POLL`.
pub fn tell_all() {
    QUITTING.store(true, SeqCst);
}

/// Closes each socket by the WebSocket handshake and waits for it, up to `wait`. An exit with a
/// frame unread can reset the socket, and on Windows a reset discards what was not yet read:
/// that window would then restart a core. The peer's reply shows it read past the quit.
pub fn close_all(file: &Path, wait: Duration) {
    CLOSING.store(true, SeqCst);
    let deadline = Instant::now() + wait;
    while CLIENTS.load(SeqCst) > 0 && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    let _ = gone(file);
}

pub fn start(data_dir: &Path, inbound: Sender<Inbound>) -> Result<()> {
    let file = data_dir.join("bridge.txt");
    let listener = bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    let secret = new_secret()?;
    let port = listener.local_addr()?.port();
    let at = file.display();
    let refused = |why| {
        Error::Setup(format!(
            "the core will not serve: {at} can't be yours alone: {why}"
        ))
    };
    gone(&file)
        .and_then(|()| write_file(&file, port, &secret))
        .map_err(refused)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (secret, inbound) = (secret.clone(), inbound.clone());
            CLIENTS.fetch_add(1, SeqCst);
            thread::spawn(move || {
                client(stream, &secret, &inbound);
                CLIENTS.fetch_sub(1, SeqCst);
            });
        }
    });
    Ok(())
}

fn same(a: &str, b: &str) -> bool {
    let differs = a
        .bytes()
        .zip(b.bytes())
        .fold(0, |all, (x, y)| all | (x ^ y));
    a.len() == b.len() && differs == 0
}

/// The first frame must be the secret, or the socket closes having sent nothing.
fn authenticated(socket: &mut WebSocket<TcpStream>, secret: &str) -> bool {
    let Ok(Message::Text(first)) = socket.read() else {
        return false;
    };
    match serde_json::from_str(first.as_str()) {
        Ok(BridgeRequest::Auth { secret: given }) => same(&given, secret),
        _ => false,
    }
}

/// A read that only ran out of time. Windows CI once gave raw error 997, `WSA_IO_PENDING`, on a
/// read with a timeout; Rust has no kind for it. No other system has that number.
pub fn quiet_read(why: &std::io::Error) -> bool {
    const WSA_IO_PENDING: i32 = 997;
    matches!(why.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
        || why.raw_os_error() == Some(WSA_IO_PENDING)
}

#[allow(clippy::result_large_err)] // tungstenite's own callback type
fn client(stream: TcpStream, secret: &str, inbound: &Sender<Inbound>) -> Option<()> {
    stream.set_read_timeout(Some(AUTH_WAIT)).ok()?;
    let check = |request: &Request, response: Response| {
        let origin = request.headers().get("origin");
        match origin.and_then(|origin| origin.to_str().ok()) {
            Some(origin) if APP_ORIGINS.contains(&origin) => Ok(response),
            _ => Err(ErrorResponse::new(Some("origin refused".to_owned()))),
        }
    };
    let mut socket = tungstenite::accept_hdr(stream, check).ok()?;
    if !authenticated(&mut socket, secret) {
        let _ = socket.close(None);
        let _ = socket.flush();
        return None;
    }
    socket.get_ref().set_read_timeout(Some(POLL)).ok()?;
    let (out, outgoing) = mpsc::channel();
    let _ = inbound.send(Inbound::Connected(out));
    let (mut told, mut closing) = (false, false);
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                if let Ok(request) = serde_json::from_str(text.as_str()) {
                    let _ = inbound.send(Inbound::Request(request));
                }
            }
            Ok(Message::Close(_)) => return None,
            Ok(_) => {}
            Err(tungstenite::Error::Io(why)) if quiet_read(&why) => {}
            Err(_) => return None,
        }
        // First, so a queue of other frames can't hold it up.
        if QUITTING.load(SeqCst) && !told {
            told = true;
            let text = serde_json::to_string(&BridgeEvent::Quitting).ok()?;
            socket.send(Message::text(text)).ok()?;
        }
        // Nothing can follow a close frame, and trying would end this before the reply.
        while let (false, Ok(text)) = (closing, outgoing.try_recv()) {
            socket.send(Message::text(text)).ok()?;
        }
        if told && CLOSING.load(SeqCst) && !closing {
            closing = true;
            socket.close(None).ok()?;
        }
    }
}
