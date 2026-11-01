//! Records a scenario against the real Claude adapter, through the real client.
//! A manual Mac tool. Usage:
//! `record_claude <scenario> <cwd> <out-prefix> <adapters-dir> [client-path]`
//! Writes `<out-prefix>.jsonl` (a second process: `<out-prefix>.2.jsonl`) and prints
//! each client event with its elapsed time. A step that does not finish exits 1.
//! Scrubbing is a separate step: see `protocol/fixtures/acp/claude/README.md`.
//! Internal: `record_claude --tee <out> <command> <args...>` is the wire tap.
use astrolabe::acp::{AcpClient, ClientEvent, Launch};
use astrolabe::driver::claude;
use astrolabe::model::{Alert, Request};
use astrolabe::{Clock, Engine, OsProcessProbe, STALL_AFTER_MS, SystemClock};
use serde_json::{Value, json};
use std::cell::Cell;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TURN_LIMIT: Duration = Duration::from_secs(120);
/// The cancel scenario's tool. Not bare `sleep`: Claude Code backgrounds that.
const SLOW_TOOL: &str = "python3 -c \"import time; time.sleep(47)\"";

/// System time plus a jump, to see a long idle without waiting for it.
#[derive(Clone, Default)]
struct JumpClock(Rc<Cell<i64>>);

impl Clock for JumpClock {
    fn now_ms(&self) -> i64 {
        SystemClock.now_ms() + self.0.get()
    }
}

struct Run<'e> {
    client: AcpClient<'e>,
    start: Instant,
}

/// What the scenario does when a request arrives.
#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Allow,
    Reject,
}

impl Run<'_> {
    fn pump(&mut self, wait: Duration) -> Option<ClientEvent> {
        let event = self.client.next_event(wait).unwrap()?;
        let line = match &event {
            ClientEvent::Update(message) => summary(message),
            other => format!("{other:?}"),
        };
        println!("{:>7} ms {line}", self.start.elapsed().as_millis());
        Some(event)
    }

    fn ready(&mut self) {
        self.until(|event, _| matches!(event, ClientEvent::Ready), None);
    }

    /// Reads until `done`, or the process ends, which fails the scenario loudly.
    fn until(&mut self, done: impl Fn(&ClientEvent, &Self) -> bool, answer: Option<Answer>) {
        let deadline = Instant::now() + TURN_LIMIT;
        while Instant::now() < deadline {
            let Some(event) = self.pump(Duration::from_millis(200)) else {
                continue;
            };
            if let ClientEvent::Alert(Alert::NeedsYou(request)) = &event
                && let Some(answer) = answer
            {
                self.answer(request, answer);
            }
            if done(&event, self) {
                return;
            }
            if let ClientEvent::Exited { .. } | ClientEvent::PromptError { .. } = event {
                fail("the scenario step did not finish");
            }
        }
        fail("step limit reached");
    }

    fn answer(&mut self, request: &Request, answer: Answer) {
        let option = match answer {
            Answer::Reject => request.reject_option().map(str::to_owned),
            Answer::Allow => request
                .options
                .iter()
                .find(|option| option.kind == "allow_once")
                .map(|option| option.option_id.clone()),
        };
        let option = option.expect("the request offers this choice");
        println!("answering {option} to {}", request.title);
        self.client.answer(&request.id, &option).unwrap();
    }

    fn turn(&mut self, text: &str, answer: Option<Answer>) {
        println!("prompt: {text}");
        self.client.prompt(text).unwrap();
        self.until(turn_over, answer);
    }

    fn close(self) {
        self.client.shutdown().unwrap();
        println!("closed at {} ms", self.start.elapsed().as_millis());
    }
}

fn turn_over(event: &ClientEvent, _: &Run) -> bool {
    matches!(event, ClientEvent::TurnEnded { .. })
}

fn fail(message: &str) -> ! {
    eprintln!("record_claude: {message}");
    std::process::exit(1);
}

fn summary(message: &Value) -> String {
    let update = &message["params"]["update"];
    let mut line = update["sessionUpdate"].as_str().unwrap_or("?").to_owned();
    for key in ["toolCallId", "status"] {
        if let Some(value) = update[key].as_str() {
            line.push_str(&format!(" {key}={value}"));
        }
    }
    if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
        line.push_str(&format!(" {text:?}"));
    }
    let session = message["params"]["sessionId"].as_str().unwrap_or("?");
    format!("{line} [{session}]")
}

fn tapped(launch: &Launch, out: &Path) -> Launch {
    let mut args: Vec<OsString> = vec!["--tee".into(), out.into(), launch.command.clone()];
    args.extend(launch.args.iter().cloned());
    Launch {
        command: std::env::current_exe().unwrap().into(),
        args,
        env: launch.env.clone(),
        meta: launch.meta.clone(),
    }
}

/// Forwards bytes unchanged and logs each line with its direction.
fn tee(out: &str, command: &str, args: &[String]) -> i32 {
    let log = Arc::new(Mutex::new(std::fs::File::create(out).unwrap()));
    let start = Instant::now();
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let record = move |log: &Mutex<std::fs::File>, dir: &str, line: &str| {
        let message: Value = serde_json::from_str(line).unwrap_or_else(|_| json!(line));
        let t = start.elapsed().as_millis() as u64;
        let entry = json!({ "dir": dir, "t": t, "msg": message });
        writeln!(log.lock().unwrap(), "{entry}").unwrap();
    };
    let mut to_child = child.stdin.take().unwrap();
    let from_child = child.stdout.take().unwrap();
    let input_log = log.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            record(&input_log, "in", &line);
            if writeln!(to_child, "{line}")
                .and_then(|()| to_child.flush())
                .is_err()
            {
                break;
            }
        }
        // Dropping the pipe here passes the client's close on to the adapter.
    });
    let mut stdout = std::io::stdout().lock();
    for line in BufReader::new(from_child).lines() {
        let Ok(line) = line else { break };
        record(&log, "out", &line);
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
    let code = child.wait().unwrap().code().unwrap_or(-1);
    let t = start.elapsed().as_millis() as u64;
    let entry = json!({ "dir": "exit", "t": t, "code": code });
    writeln!(log.lock().unwrap(), "{entry}").unwrap();
    code
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args[0] == "--tee" {
        std::process::exit(tee(&args[1], &args[2], &args[3..]));
    }
    if !cfg!(unix) {
        fail("this tool needs pgrep and kill; it runs on Mac only");
    }
    let (scenario, cwd, prefix, adapters, client) = match &args[..] {
        [s, c, p, a] => (s, c, p, a, None),
        [s, c, p, a, client] => (s, c, p, a, Some(Path::new(client))),
        _ => fail("usage: record_claude <scenario> <cwd> <out-prefix> <adapters-dir> [client]"),
    };
    let cwd = PathBuf::from(cwd);
    let dir = std::env::temp_dir().join(format!("astrolabe-record-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let clock = JumpClock::default();
    let engine = Engine::open(&dir.join("a.db"), clock.clone(), OsProcessProbe).unwrap();
    // The owner's plugins and settings stay out of the recording.
    let options = json!({ "promptSuggestions": false, "settingSources": [] });
    let client = if scenario == "launch_refused" {
        Some(Path::new("/nonexistent"))
    } else {
        client
    };
    let mut launch = claude::launch(Path::new(adapters), client, Some(options)).unwrap();
    // Cheapest model. `options.model` is not enough: on resume the adapter puts the
    // owner's `settings.model` back. This variable outranks that.
    launch.env.push(("ANTHROPIC_MODEL".into(), "haiku".into()));
    let wire = if scenario == "crash_direct" {
        launch.clone()
    } else {
        tapped(&launch, Path::new(&format!("{prefix}.jsonl")))
    };
    let open = |spec: &Launch| {
        let client = AcpClient::launch(&engine, spec, &cwd).unwrap();
        Run {
            client,
            start: Instant::now(),
        }
    };
    let mut run = open(&wire);
    if scenario != "launch_refused" {
        run.ready();
    }
    match scenario.as_str() {
        "full_turn" => run.turn("Reply with just OK.", None),
        "approve" => run.turn(
            "Use the Bash tool to run exactly: touch approved.txt\nThen reply with just DONE.",
            Some(Answer::Allow),
        ),
        "reject" => run.turn(
            "Use the Bash tool to run exactly: touch rejected.txt\nIf it is refused, reply with just REFUSED.",
            Some(Answer::Reject),
        ),
        "two_turns" => {
            run.turn("Reply with just ONE.", None);
            run.turn("Reply with just TWO.", None);
        }
        "resume" => {
            run.turn("The word is OWL. Reply with just OK.", None);
            let session = run.client.session_id();
            run.close();
            let second = tapped(&launch, Path::new(&format!("{prefix}.2.jsonl")));
            let client = AcpClient::resume(&engine, session, &second, &cwd).unwrap();
            run = Run {
                client,
                start: Instant::now(),
            };
            run.ready();
            run.turn("What was the word? Reply with just the word.", None);
        }
        "cancel" => {
            let text = format!("Use the Bash tool to run exactly: {SLOW_TOOL}\nThen reply DONE.");
            println!("prompt: {text}");
            run.client.prompt(&text).unwrap();
            run.until(
                |event, _| matches!(event, ClientEvent::Alert(Alert::NeedsYou(_))),
                Some(Answer::Allow),
            );
            // Let the tool really start, then stop it.
            let until = Instant::now() + Duration::from_secs(4);
            while Instant::now() < until {
                run.pump(Duration::from_millis(100));
            }
            println!("tool processes before cancel: {}", tool_count());
            println!("cancel at {} ms", run.start.elapsed().as_millis());
            run.client.cancel().unwrap();
            run.until(turn_over, None);
            std::thread::sleep(Duration::from_secs(1));
            println!("tool processes after cancel: {}", tool_count());
        }
        "long_tool" => run.turn(
            "Use the Bash tool to run exactly: python3 -c \"import time; time.sleep(35)\"\nThen reply DONE.",
            Some(Answer::Allow),
        ),
        "child" => run.turn(
            "Use the Agent tool to start one subagent. Tell it to use the Bash tool to run exactly: touch child.txt\nThen reply with just DONE.",
            Some(Answer::Allow),
        ),
        "idle" => {
            // No prompt, so no tokens. Real quiet first, then jump past the limit.
            let session = run.client.session_id();
            let until = Instant::now() + Duration::from_secs(45);
            while Instant::now() < until {
                run.pump(Duration::from_millis(500));
                let alerts = engine.tick().unwrap();
                assert!(alerts.is_empty(), "card during real quiet: {alerts:?}");
            }
            clock.0.set(STALL_AFTER_MS + 1_000);
            let alerts = engine.tick().unwrap();
            println!("after a jump of {STALL_AFTER_MS} ms: {alerts:?}");
            println!("display: {:?}", engine.inspect(session).unwrap().display);
        }
        "launch_refused" => {
            run.until(|event, _| matches!(event, ClientEvent::Exited { .. }), None);
        }
        "crash_direct" => {
            // No tap, and mid-tool: the exit status the client really gets, and
            // whether the tool goes with the adapter.
            let text = format!("Use the Bash tool to run exactly: {SLOW_TOOL}\nThen reply DONE.");
            run.client.prompt(&text).unwrap();
            run.until(
                |event, _| matches!(event, ClientEvent::Alert(Alert::NeedsYou(_))),
                Some(Answer::Allow),
            );
            let until = Instant::now() + Duration::from_secs(4);
            while Instant::now() < until {
                run.pump(Duration::from_millis(100));
            }
            println!("tool processes before the kill: {}", tool_count());
            let pid = run.client.pid().to_string();
            println!("killing {pid}");
            Command::new("/bin/kill")
                .args(["-KILL", &pid])
                .status()
                .unwrap();
            run.until(|event, _| matches!(event, ClientEvent::Exited { .. }), None);
            std::thread::sleep(Duration::from_secs(1));
            let left = Command::new("/usr/bin/pgrep")
                .args(["-g", &pid])
                .output()
                .unwrap();
            let left = String::from_utf8_lossy(&left.stdout).lines().count();
            println!("left in the group: {left}");
            println!("tool processes after the exit: {}", tool_count());
            let view = engine.inspect(run.client.session_id()).unwrap();
            println!("state: {:?} {:?}", view.run.state, view.session.lifecycle);
        }
        "close_mid_tool" => {
            // The owner closes the session while a tool runs.
            let text = format!("Use the Bash tool to run exactly: {SLOW_TOOL}\nThen reply DONE.");
            run.client.prompt(&text).unwrap();
            run.until(
                |event, _| matches!(event, ClientEvent::Alert(Alert::NeedsYou(_))),
                Some(Answer::Allow),
            );
            let until = Instant::now() + Duration::from_secs(4);
            while Instant::now() < until {
                run.pump(Duration::from_millis(100));
            }
            println!("tool processes before the close: {}", tool_count());
            run.client.shutdown().unwrap();
            std::thread::sleep(Duration::from_secs(1));
            println!("tool processes after the close: {}", tool_count());
            return;
        }
        "crash" => {
            // Kills the real adapter under the tap: an exit nobody asked for.
            let pid = run.client.pid();
            let children = Command::new("/usr/bin/pgrep")
                .args(["-P", &pid.to_string()])
                .output()
                .unwrap();
            let adapter = String::from_utf8_lossy(&children.stdout);
            println!("killing {}", adapter.trim());
            Command::new("/bin/kill")
                .args(["-KILL", adapter.trim()])
                .status()
                .unwrap();
            run.until(|event, _| matches!(event, ClientEvent::Exited { .. }), None);
        }
        other => panic!("unknown scenario {other}"),
    }
    run.close();
    let _ = std::fs::remove_dir_all(dir);
}

fn tool_count() -> usize {
    let found = Command::new("/usr/bin/pgrep")
        .args(["-f", "time.sleep\\(47\\)"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&found.stdout).lines().count()
}
