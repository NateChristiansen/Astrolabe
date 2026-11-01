//! A fake ACP adapter that runs as a real child process over stdio. Flags choose
//! how it starts; the prompt text chooses how the turn goes. Session state lives in
//! a file in its working directory, so a resume in a new process can prove itself.
use serde_json::{Value, json};
use std::io::{self, Write};
use std::path::Path;

const SESSION: &str = "fake-session";

struct Agent {
    input: io::Lines<io::StdinLock<'static>>,
    resume_method: Option<&'static str>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|arg| arg == name);
    if flag("--dump-env") {
        let names: Vec<_> = std::env::vars_os().map(|(name, _)| name).collect();
        let names: Vec<_> = names.iter().map(|name| name.to_string_lossy()).collect();
        std::fs::write("env.txt", names.join("\n")).unwrap();
    }
    if flag("--noisy-stderr") {
        // As the real adapter does, with chatter and a real reason among it.
        for n in 0..20 {
            eprintln!("chatter {n} from a healthy adapter, of no interest here");
        }
        eprintln!("the useful reason");
        eprintln!("connection failed durationMs=2000 cause=ECONNREFUSED");
        eprintln!("[session/create] sessionId=s failed durationMs=9 cause=ENOENT");
        for n in 0..20 {
            eprintln!("[session/create] sessionId=s phase=p{n} durationMs=0 totalMs=0");
        }
    }
    if flag("--sleep") {
        std::thread::sleep(std::time::Duration::from_secs(10));
        return;
    }
    if flag("--hold-stdout") {
        // A tool left behind by the adapter keeps the pipes open after it dies.
        spawn_tool();
        std::process::exit(4);
    }
    if flag("--stop-reading") {
        // Alive and answering nothing: the handshake is done, then stdin goes unread.
        let mut input = io::stdin().lines();
        for reply in [
            json!({ "protocolVersion": 1, "agentCapabilities": {} }),
            json!({ "sessionId": SESSION }),
        ] {
            let line = input.next().unwrap().unwrap();
            let message: Value = serde_json::from_str(&line).unwrap();
            send(&result(message["id"].clone(), reply));
        }
        std::thread::sleep(std::time::Duration::from_secs(10));
        return;
    }
    if flag("--ignore-eof") {
        // Started a tool, and will not leave when its pipe closes.
        spawn_tool();
        std::thread::sleep(std::time::Duration::from_secs(10));
        return;
    }
    let resume_method = if flag("--load") {
        Some("session/load")
    } else if flag("--no-resume") {
        None
    } else {
        Some("session/resume")
    };
    let mut agent = Agent {
        input: io::stdin().lines(),
        resume_method,
    };
    while let Some(message) = agent.read() {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
                if flag("--dump-init") {
                    std::fs::write("init.json", message.to_string()).unwrap();
                }
                if flag("--hang-initialize") {
                    continue;
                }
                if flag("--stderr-then-exit") {
                    eprintln!("adapter cannot start");
                    std::process::exit(5);
                }
                if flag("--fail-initialize") {
                    send(&failure(id, "cannot initialize"));
                    continue;
                }
                if flag("--stray-error") {
                    send(&json!({ "id": null, "error": { "code": -32700, "message": "stray" } }));
                }
                send(&result(id, agent.capabilities()));
            }
            "session/new" if flag("--dump-open") => {
                dump_open(&message);
                send(&result(id, json!({ "sessionId": SESSION })));
            }
            "session/resume" if flag("--dump-open") => {
                dump_open(&message);
                send(&result(id, json!({})));
            }
            "session/new" => {
                if flag("--fail-new") {
                    send(&failure(id, "cannot create a session"));
                    continue;
                }
                if flag("--exit-after-new") {
                    send(&result(id, json!({ "sessionId": SESSION })));
                    std::process::exit(0);
                }
                let meta = message["params"]["_meta"].to_string();
                std::fs::write("meta.txt", meta).unwrap();
                std::fs::write("turns.txt", "0").unwrap();
                if flag("--receipt-new") {
                    report_start();
                }
                let mut opened = json!({ "sessionId": SESSION });
                if flag("--mode-agent") {
                    opened["modes"] = json!({ "currentModeId": "agent" });
                } else if flag("--mode-safe") {
                    opened["modes"] = json!({ "currentModeId": "workspace-write" });
                }
                send(&result(id, opened));
            }
            "session/resume" | "session/load" => {
                assert_eq!(message["params"]["sessionId"], SESSION);
                // Replayed history, including a tool that looks like it is running.
                send(&say(SESSION, "replayed"));
                send(&tool_call(SESSION, "old-call", "in_progress"));
                send(&result(id, json!({})));
            }
            "session/prompt" => {
                if flag("--hang-prompt") {
                    continue;
                }
                if flag("--receipt-prompt") {
                    report_start();
                }
                if flag("--switch-mode") {
                    send(
                        &json!({ "jsonrpc": "2.0", "method": "session/update", "params": {
                            "sessionId": SESSION,
                            "update": { "sessionUpdate": "current_mode_update", "currentModeId": "agent" },
                        }}),
                    );
                }
                agent.turn(id, &message);
            }
            _ => {}
        }
    }
}

/// What the client's hook would leave.
fn report_start() {
    let dir = astrolabe::hook::reports_dir().unwrap();
    astrolabe::hook::write_report(
        &dir,
        "session-start",
        Some(SESSION),
        &std::env::current_dir().unwrap(),
    )
    .unwrap();
}

/// What each open route was sent, one line per open, for a test to read.
fn dump_open(message: &Value) {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("open.jsonl")
        .unwrap();
    writeln!(file, "{message}").unwrap();
}

impl Agent {
    fn capabilities(&self) -> Value {
        let agent = match self.resume_method {
            Some("session/load") => json!({ "loadSession": true }),
            Some(_) => json!({ "sessionCapabilities": { "resume": {} } }),
            None => json!({}),
        };
        json!({ "protocolVersion": 1, "agentCapabilities": agent })
    }

    fn read(&mut self) -> Option<Value> {
        let line = self.input.next()?.ok()?;
        Some(serde_json::from_str(&line).expect("the client must write valid JSON"))
    }

    fn turn(&mut self, id: Value, message: &Value) {
        let turns: u32 = std::fs::read_to_string("turns.txt")
            .unwrap()
            .parse()
            .unwrap();
        std::fs::write("turns.txt", (turns + 1).to_string()).unwrap();
        let scenario = message["params"]["prompt"][0]["text"].as_str().unwrap();
        match scenario {
            "answer-exit" => {
                send(&permission("7", SESSION));
                self.answer_to("7");
                std::process::exit(0);
            }
            "approve" | "cancel" | "cancel-held" => {
                send(&tool_call(SESSION, "call-1", "pending"));
                // A string id: the answer must echo it with its type.
                send(&permission("7", SESSION));
                let outcome = self.answer_to("7");
                if outcome["outcome"] == "cancelled" {
                    // Held until the test has looked at the state before the answer.
                    while scenario == "cancel-held" && !Path::new("release").exists() {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    send(&result(id, json!({ "stopReason": "cancelled" })));
                    return;
                }
                let chosen = outcome["optionId"].as_str().unwrap();
                send(&say(SESSION, &format!("answered {chosen}")));
                let status = if chosen == "yes" {
                    "completed"
                } else {
                    "failed"
                };
                send(&tool_update("call-1", status));
            }
            "request-then-crash" => {
                send(&permission("7", SESSION));
                std::process::exit(3);
            }
            "crash" => {
                send(&tool_call(SESSION, "call-1", "in_progress"));
                eprintln!("adapter crashed");
                std::process::exit(3);
            }
            "malformed" => {
                write_raw(b"{not json\n");
                write_raw(b"[1,2]\n");
                write_raw(b"{\"jsonrpc\":\"2.0\"}\n");
                write_raw(b"\xff\xfe\n");
                write_raw(format!("{}\r\n", say(SESSION, "after noise")).as_bytes());
            }
            "unsupported" => {
                send(&json!({
                    "jsonrpc": "2.0", "id": 50, "method": "fs/read_text_file",
                    "params": { "sessionId": SESSION, "path": "x" },
                }));
                let reply = self.read_response(&json!(50));
                send(&say(SESSION, &format!("code {}", reply["error"]["code"])));
            }
            "error" => {
                send(&failure(id, "prompt refused"));
                return;
            }
            "refused-with-details" => {
                let data = json!({ "details": "login expired" });
                let error = json!({ "code": -32603, "message": "Internal error", "data": data });
                send(&json!({ "jsonrpc": "2.0", "id": id, "error": error }));
                return;
            }
            "wait-for-cancel" => {
                // Only `session/cancel` stops this tool, as with a real Bash tool.
                send(&tool_call(SESSION, "call-1", "in_progress"));
                while let Some(message) = self.read() {
                    if message["method"] == "session/cancel" {
                        std::fs::write("cancelled.txt", "").unwrap();
                        send(&tool_update("call-1", "failed"));
                        send(&result(id, json!({ "stopReason": "cancelled" })));
                        return;
                    }
                }
                return;
            }
            "silent" => {
                // Never answers, cancel or not, until its pipe closes.
                while self.read().is_some() {}
                return;
            }
            "no-stop-reason" => {
                send(&result(id, json!({})));
                return;
            }
            "unrelated-error" => {
                send(&failure(json!(999), "unrelated"));
                send(&result(json!(998), json!({ "stopReason": "end_turn" })));
                send(&tool_call(SESSION, "call-2", "in_progress"));
                // Holds the turn open so the test can see the run survived.
                send(&permission("9", SESSION));
                self.answer_to("9");
                send(&say(SESSION, "still going"));
            }
            "flood-exit" => {
                for n in 0..3 {
                    send(&say(SESSION, &format!("line {n}")));
                }
                send(&result(id, json!({ "stopReason": "end_turn" })));
                std::process::exit(0);
            }
            "child" => {
                send(&tool_call("child-1", "child-call", "in_progress"));
                send(&permission("8", "child-1"));
                self.answer_to("8");
                send(&tool_update_in("child-1", "child-call", "completed"));
                // A completion that is not the parent's, then a hold.
                send(&result(json!(997), json!({ "stopReason": "end_turn" })));
                send(&permission("9", SESSION));
                self.answer_to("9");
            }
            _ => send(&say(SESSION, &format!("turn {}", turns + 1))),
        }
        send(&result(id, json!({ "stopReason": "end_turn" })));
    }

    fn answer_to(&mut self, id: &str) -> Value {
        let reply = self.read_response(&json!(id));
        reply["result"]["outcome"].clone()
    }

    /// Other traffic can arrive while waiting; only `session/cancel` may, here.
    fn read_response(&mut self, id: &Value) -> Value {
        loop {
            let message = self.read().expect("the client must answer");
            if message.get("method").is_none() {
                assert_eq!(&message["id"], id, "the answer must echo the raw id");
                return message;
            }
        }
    }
}

/// Starts a process that inherits the pipes, and leaves its pid in the cwd. It is
/// deliberately never waited on: outliving this process is the point.
#[allow(clippy::zombie_processes)]
fn spawn_tool() {
    let exe = std::env::current_exe().unwrap();
    let tool = std::process::Command::new(exe)
        .arg("--sleep")
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::fs::write("tool.pid", tool.id().to_string()).unwrap();
}

fn send(message: &Value) {
    write_raw(format!("{message}\n").as_bytes());
}

fn write_raw(bytes: &[u8]) {
    let mut out = io::stdout().lock();
    out.write_all(bytes).unwrap();
    out.flush().unwrap();
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn failure(id: Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": message } })
}

fn update(session: &str, update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": session, "update": update },
    })
}

fn say(session: &str, text: &str) -> Value {
    let content = json!({ "type": "text", "text": text });
    update(
        session,
        json!({ "sessionUpdate": "agent_message_chunk", "content": content }),
    )
}

fn tool_call(session: &str, id: &str, status: &str) -> Value {
    update(
        session,
        json!({ "sessionUpdate": "tool_call", "toolCallId": id, "title": id, "status": status }),
    )
}

fn tool_update(id: &str, status: &str) -> Value {
    tool_update_in(SESSION, id, status)
}

fn tool_update_in(session: &str, id: &str, status: &str) -> Value {
    update(
        session,
        json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": status }),
    )
}

fn permission(id: &str, session: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "session/request_permission",
        "params": {
            "sessionId": session,
            "toolCall": { "toolCallId": "call-1", "title": "git push" },
            "options": [
                { "optionId": "yes", "name": "Allow", "kind": "allow_once" },
                { "optionId": "always", "name": "Always", "kind": "allow_always" },
                { "optionId": "no", "name": "Reject", "kind": "reject_once" },
            ],
        },
    })
}
