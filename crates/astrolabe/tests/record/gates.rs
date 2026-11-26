//! Gate for any cut to save replies or activation text: a worker killed mid-task, with no
//! Stop hook and a save left half-written, is picked up by a fresh session that gets the
//! next runnable action, the open hypotheses and its permission scope from the notes alone.
//! Run it before and after a wording change; it checks what reaches the agent, not phrasing.
use super::*;

const CURRENT: &str = "Half done: the jitter is gone on frames 1 to 11 and still there on 12.\n\nI kept the 0.3 threshold over 0.2, because 0.2 blurred the fast strokes. Open: frame 12 may be the cache, not the threshold; the cache isn't ruled out yet.\n\nNext: run the frame-12 check with the cache off. Not checked on the long clip.";
const LATER: &str = "Nearly done: the cache was the cause on frame 12, and turning it off fixes it.\n\nI kept the 0.3 threshold. Open: whether the cache should be rebuilt or dropped.\n\nNext: rebuild the cache and rerun all 40 frames. Not checked on the long clip.";

fn save_body(body: &str) -> String {
    format!("astrolabe effort save --status active <<'ASTROLABE'\n{body}\nASTROLABE\n")
}

/// The binding a dispatcher sends a worker, whichever session runs it.
const BIND: &str = "astrolabe activate --effort 1 --worker --model opus --reasoning medium";

/// A command as an agent runs it: again, unchanged, when the reply says to, as it does when
/// a loaded machine slows the hook's pull past its deadline.
fn agent(m: &Machine, session: &str, command: &str) -> (String, bool) {
    let mut reply = m.bash(session, command);
    for _ in 0..6 {
        let again = [
            "Run the same command again",
            "run the same command again",
            "ran out of time",
        ];
        if !again.iter().any(|s| reply.0.contains(s)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
        reply = m.bash(session, command);
    }
    reply
}

/// What a fresh worker must get to carry on: the state paragraphs in full, its pieces,
/// its settings and role, its own guard challenge, and how to save.
fn picks_up(text: &str, current: &str) {
    for paragraph in current.split("\n\n") {
        assert!(
            text.contains(paragraph),
            "state lost: {paragraph}\n---\n{text}"
        );
    }
    for piece in ["- [ ] Smooth frames 1 to 11", "- [ ] Smooth frame 12"] {
        assert!(text.contains(piece), "piece lost: {piece}\n---\n{text}");
    }
    assert!(
        text.contains("authority: edit") && text.contains("don't push"),
        "scope lost: {text}"
    );
    assert!(
        text.contains("worker under that effort"),
        "role lost: {text}"
    );
    assert!(
        text.contains("astrolabe-guard-probe") && text.contains("which must run: "),
        "a fresh session must prove its own guard: {text}"
    );
    assert!(
        text.contains("astrolabe effort save"),
        "how to save lost: {text}"
    );
}

#[test]
fn gate_a_killed_worker_is_picked_up_from_its_notes_with_scope_and_next_step() {
    let world = World::new();
    let mac = world.machine("mac");
    // The dispatcher files the effort; a worker binds and saves its state and pieces.
    let (out, ok) = agent(&mac, "d1", NEW);
    assert!(ok, "{out}");
    let (out, ok) = agent(&mac, "w1", BIND);
    assert!(ok, "{out}");
    let pieces = mac.dir.0.join("pieces.md");
    write(
        &pieces,
        "- [ ] Smooth frames 1 to 11\n- [ ] Smooth frame 12\n",
    );
    let command = format!(
        "astrolabe effort save --status active --pieces-file \"{}\" <<'ASTROLABE'\n{CURRENT}\nASTROLABE\n",
        pieces.display().to_string().replace('\\', "/")
    );
    let (out, ok) = agent(&mac, "w1", &command);
    assert!(ok && out.contains("Saved effort 1"), "{out}");
    // The save's detached push outlives the session; this stands in for it.
    assert!(mac.sync().1);
    // Then it edits, and dies mid-save: the file is written, nothing committed, the reply
    // never reached it, and no Stop hook fired.
    mac.edit("w1", "Edit");
    let rel = shelf("0001-smooth-ink.md");
    let saved = std::fs::read_to_string(mac.notes.join(&rel)).unwrap();
    let half = saved.replace(CURRENT, LATER);
    assert_ne!(half, saved);
    let intent = json!({
        "key": "killed-w1", "files": [[rel, half]], "renames": [], "journal": [],
        "message": "0001: checkpoint", "base": null, "states": []
    });
    write(&mac.state().join("ops/killed-w1.json"), &intent.to_string());
    std::fs::write(mac.notes.join(&rel), &half).unwrap();

    // The app comes back on the same session: it's still that effort's worker, with the
    // newest state on this machine.
    let mut resumed = mac.session_start("w1", "resume");
    for _ in 0..6 {
        if !resumed.contains("ran out of time") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
        resumed = mac.session_start("w1", "resume");
    }
    assert!(
        resumed.contains("worker under that effort")
            && resumed.contains(LATER.split("\n\n").last().unwrap()),
        "{resumed}"
    );

    // A fresh session on the same machine, bound the way the dispatcher binds any worker.
    let (fresh, ok) = agent(&mac, "w2", BIND);
    assert!(ok, "{fresh}");
    picks_up(&fresh, LATER);
    // Its first write finishes the dead session's save, once.
    let (out, ok) = agent(&mac, "w2", &save_body(LATER));
    assert!(ok, "{out}");
    let log = mac.log();
    // The first save, the dead session's finished once, and this one's own.
    assert_eq!(log.matches("0001: checkpoint").count(), 3, "{log}");
    let left: Vec<Value> = std::fs::read_dir(mac.state().join("ops"))
        .map(|dir| dir.flatten().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| serde_json::from_slice(&std::fs::read(e.path()).unwrap()).unwrap())
        .collect();
    assert!(left.iter().all(|op| op["result"].is_string()), "{left:?}");
    // The scope came with it: a worker still can't journal or touch another effort.
    let (out, ok) = agent(&mac, "w2", "astrolabe journal \"x\"");
    assert!(!ok && out.contains("worker under effort 1"), "{out}");

    // On another machine, after the push, a fresh worker gets the same.
    assert!(mac.sync().1);
    let pc = world.machine("pc");
    pc.session_start("w3", "startup");
    let (fresh, ok) = agent(&pc, "w3", BIND);
    assert!(ok, "{fresh}");
    picks_up(&fresh, LATER);
}
