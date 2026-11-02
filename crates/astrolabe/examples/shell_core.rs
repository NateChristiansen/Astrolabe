//! The real core with recordings for agents, as its own killable process, for `cargo test`.
//! Usage: `shell_core <db> <fixtures dir> <clock file>`. The repo named on a start picks the
//! scenario. The clock file holds milliseconds to add to the time, re-read at each look.
use astrolabe::acp::Launch;
use astrolabe::launch::Prepared;
use astrolabe::shell::Prepare;
use astrolabe::{Clock, Error, SystemClock};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

struct FileClock(PathBuf);

impl Clock for FileClock {
    fn now_ms(&self) -> i64 {
        let extra = std::fs::read_to_string(&self.0).ok();
        SystemClock.now_ms() + extra.and_then(|text| text.trim().parse().ok()).unwrap_or(0)
    }
}

fn example(name: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    exe.parent().unwrap().join(file)
}

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [db, fixtures, clock] = args.as_slice() else {
        panic!("usage: shell_core <db> <fixtures dir> <clock file>");
    };
    let (root, cwd) = (fixtures.clone(), db.parent().unwrap().join("agents"));
    std::fs::create_dir_all(&cwd).unwrap();
    let prepare: Prepare = Arc::new(move |repo, _, _, say| {
        say("preparing".to_owned());
        let (command, args) = match repo {
            "refused-before-launch" => {
                return Err(Error::Setup(
                    "the receipt never came, in these words".to_owned(),
                ));
            }
            // Silent, and stays when its pipe closes: nothing but the stall watch notices.
            "silent" => (example("fake_agent"), vec!["--ignore-eof".into()]),
            recording => {
                let file = root.join(format!("{recording}.jsonl"));
                (example("replay_agent"), vec![file.into_os_string()])
            }
        };
        let meta = repo.starts_with("claude/").then(|| {
            let options = json!({ "promptSuggestions": false, "settingSources": [] });
            json!({ "claudeCode": { "options": options } })
        });
        let launch = Launch {
            command: command.into(),
            args,
            meta,
            ..Launch::default()
        };
        Ok(Prepared {
            launch,
            cwd: cwd.clone(),
            worktree: None,
        })
    });
    let tick = Duration::from_millis(100);
    if let Err(why) = astrolabe::serve_with(db, FileClock(clock.clone()), tick, prepare) {
        eprintln!("{why}");
        std::process::exit(1);
    }
}
