//! A launch from inside a Claude Code session and an ordinary one, through the real driver and a real
//! child process. Its own binary with one test, because it sets this process's
//! environment, which is unsound with another thread running.
mod common;

use astrolabe::acp::{AcpClient, ClientEvent};
use astrolabe::driver::claude;
use common::{Harness, TempDir};
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MARK: &str = "CLAUDECODE";
/// Named here on purpose, not read from the driver: dropping one must fail.
const NESTED: [&str; 10] = [
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_ACCOUNT_UUID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_HOST_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_ORGANIZATION_UUID",
    "CLAUDE_CODE_SDK_HAS_HOST_AUTH_REFRESH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_USER_EMAIL",
];
/// A login of the user's own, and what finds it.
const KEPT: [&str; 3] = ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "PATH"];

fn adapters(dir: &TempDir) -> PathBuf {
    let package = dir
        .0
        .join("node_modules/@agentclientprotocol/claude-agent-acp");
    std::fs::create_dir_all(package.join("dist")).unwrap();
    let manifest = json!({ "bin": { "claude-agent-acp": "dist/index.js" } });
    std::fs::write(package.join("package.json"), manifest.to_string()).unwrap();
    std::fs::write(package.join("dist/index.js"), "").unwrap();
    dir.0.clone()
}

/// The names of what reached the adapter, through the real driver.
fn launched_env() -> Vec<String> {
    let h = Harness::new();
    let mut spec = claude::launch(&adapters(&h.dir), None, None).unwrap();
    let profile = std::env::current_exe().unwrap();
    let profile = profile.parent().unwrap().parent().unwrap();
    let fake = format!("fake_agent{}", std::env::consts::EXE_SUFFIX);
    spec.command = profile.join("examples").join(fake).into();
    spec.args = vec!["--dump-env".into()];

    let mut client = AcpClient::launch(&h.engine, &spec, &h.dir.0).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while client.next_event(Duration::from_millis(50)).unwrap() != Some(ClientEvent::Ready) {
        assert!(Instant::now() < deadline, "no Ready");
    }
    client.shutdown().unwrap();
    let names = std::fs::read_to_string(h.dir.0.join("env.txt")).unwrap();
    names.lines().map(str::to_owned).collect()
}

#[test]
fn only_a_nested_launch_loses_the_parents_identity_and_both_keep_the_login() {
    // Windows keeps its own spelling, such as `Path`, and matches names in any case.
    let has = |names: &[String], name: &str| names.iter().any(|n| n.eq_ignore_ascii_case(name));
    // SAFETY: the only test in this binary. What an earlier launch left running only
    // reads and writes pipes, never the environment.
    unsafe {
        // This suite may itself run inside a Claude Code session.
        std::env::remove_var(MARK);
        for name in NESTED.iter().chain(&KEPT[..2]) {
            std::env::set_var(name, "from-the-parent");
        }
    }
    let ordinary = launched_env();
    for name in NESTED.iter().chain(&KEPT) {
        assert!(
            has(&ordinary, name),
            "{name} was taken from an ordinary launch"
        );
    }

    // SAFETY: as above.
    unsafe { std::env::set_var(MARK, "1") };
    let nested = launched_env();
    assert!(!has(&nested, MARK), "the mark reached the adapter");
    for name in NESTED {
        assert!(!has(&nested, name), "{name} reached the adapter");
    }
    for name in KEPT {
        assert!(has(&nested, name), "{name} was taken away");
    }
    assert!(has(&nested, "DISABLE_AUTOUPDATER"));
}
