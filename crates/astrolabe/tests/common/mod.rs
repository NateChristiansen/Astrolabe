//! A unique fixture folder, removed when the test ends.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// A unique fixture directory, removed when the test ends.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "astrolabe-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a child printed and how long it took once `stdin` was written to it, or None when it
/// was still running at `limit` and was killed: a hook that never answers fails its test
/// instead of hanging the run.
#[allow(dead_code)]
pub fn within(
    mut child: std::process::Child,
    stdin: Vec<u8>,
    limit: std::time::Duration,
) -> Option<(std::process::Output, std::time::Duration)> {
    use std::io::{Read, Write};
    let started = std::time::Instant::now();
    let pipe = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(mut pipe) = pipe {
            let _ = pipe.write_all(&stdin);
        }
    });
    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    }
    let (stdout, stderr) = (drain(child.stdout.take()), drain(child.stderr.take()));
    loop {
        match child.try_wait().unwrap() {
            Some(status) => {
                let took = started.elapsed();
                let _ = writer.join();
                let output = std::process::Output {
                    status,
                    stdout: stdout.join().unwrap(),
                    stderr: stderr.join().unwrap(),
                };
                return Some((output, took));
            }
            None if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
}
