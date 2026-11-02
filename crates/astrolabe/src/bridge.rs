//! The one local WebSocket to the app: loopback only, its own origins only, and a secret
//! made fresh each start, handed over in an owner-only file and sent as the first frame.
use crate::error::{Error, Result};
use protocol::BridgeRequest;
use std::fs;
use std::io::{ErrorKind, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::{Message, WebSocket};

/// Tauri's own page on macOS and on Windows.
const APP_ORIGINS: [&str; 2] = ["tauri://localhost", "http://tauri.localhost"];
const AUTH_WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);

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
/// On Unix owner-only from its creation. Windows has no mode to set: the file takes the
/// folder's permissions, and `%LOCALAPPDATA%` is the user's own.
pub fn write_file(file: &Path, port: u16, secret: &str) -> Result<()> {
    let aside = file.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut out = options.open(&aside)?;
    writeln!(out, "{port}\n{secret}")?;
    drop(out);
    fs::rename(&aside, file)?;
    Ok(())
}

pub fn start(data_dir: &Path, inbound: Sender<Inbound>) -> Result<()> {
    let file = data_dir.join("bridge.txt");
    let _ = fs::remove_file(&file);
    let listener = bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    let secret = new_secret()?;
    write_file(&file, listener.local_addr()?.port(), &secret)?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (secret, inbound) = (secret.clone(), inbound.clone());
            thread::spawn(move || {
                client(stream, &secret, &inbound);
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
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                if let Ok(request) = serde_json::from_str(text.as_str()) {
                    let _ = inbound.send(Inbound::Request(request));
                }
            }
            Ok(Message::Close(_)) => return None,
            Ok(_) => {}
            Err(tungstenite::Error::Io(why))
                if matches!(why.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return None,
        }
        while let Ok(text) = outgoing.try_recv() {
            socket.send(Message::text(text)).ok()?;
        }
    }
}
