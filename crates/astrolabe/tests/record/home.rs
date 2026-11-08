use super::*;

#[test]
fn efforts_json_is_what_the_home_view_reads() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: which?\" <<'ASTROLABE'\nActive: g.\n\nNext: ask.\nASTROLABE\n").1);
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--repo", "demo"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let e = &v["efforts"][0];
    assert_eq!(e["number"], 1);
    assert_eq!(e["status"], "blocked");
    assert_eq!(e["needs_owner"], true);
    assert_eq!(e["state"], "Active: g.");
    assert_eq!(e["next"], "Next: ask.");
    assert_eq!(e["repo"], "demo");
    assert_eq!(v["journal"], "");
    // The reader takes both journal shapes.
    let mixed = "- 14:02 done 0012 Smooth ink: merged the ink MR <!-- mac claude e:7f3a -->\n- Finished the ink. <!-- 0013 mac 5 e:9b -->\n- 12:30 is when it broke: fixed. <!-- 0014 pc 6 e:9c -->\n- Found the Deck preset. <!-- demo mac 7 e:aa -->\n- Fixed. <!-- 0015 mac e:9d -->\n- 12:30 done fixing it. <!-- 0016 mac e:9e -->\n";
    let lines = astrolabe::notes::parse_journal(mixed);
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.effort.as_str(), l.text.as_str(), l.id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("0012", "merged the ink MR", "7f3a"),
            ("0013", "Finished the ink.", "9b"),
            ("0014", "12:30 is when it broke: fixed.", "9c"),
            ("demo", "Found the Deck preset.", "aa"),
            ("0015", "Fixed.", "9d"),
            ("0016", "12:30 done fixing it.", "9e")
        ]
    );
    // Tidy keeps the latest line per effort, as a union merge of two replacements leaves
    // both; findings and first-shape lines stay.
    let doubled = "- 09:00 started 0001 A: a <!-- mac claude e:l1 -->\n- 09:05 done 0001 A: a <!-- mac claude e:l2 -->\n- Started A. <!-- 0001 mac 10 e:a1 -->\n- Started B. <!-- 0002 pc 11 e:b1 -->\n- Unstamped C. <!-- 0003 mac e:c0 -->\n- Finished A. <!-- 0001 mac 20 e:a2 -->\n- Finished B. <!-- 0002 pc 21 e:b2 -->\n- Finished C. <!-- 0003 mac 22 e:c1 -->\n- Found x. <!-- demo mac 30 e:f1 -->\n- Found y. <!-- demo pc 31 e:f2 -->\n";
    assert_eq!(
        astrolabe::notes::tidy_journal(doubled),
        "- 09:00 started 0001 A: a <!-- mac claude e:l1 -->\n- 09:05 done 0001 A: a <!-- mac claude e:l2 -->\n- Finished A. <!-- 0001 mac 20 e:a2 -->\n- Finished B. <!-- 0002 pc 21 e:b2 -->\n- Finished C. <!-- 0003 mac 22 e:c1 -->\n- Found x. <!-- demo mac 30 e:f1 -->\n- Found y. <!-- demo pc 31 e:f2 -->\n"
    );
}

#[test]
fn the_board_follows_every_save_and_stays_out_of_git() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let board = mac.notes.join("home.html");
    let page = std::fs::read_to_string(&board).expect("the board is written at effort new");
    assert!(column(&page, "active").contains("Smooth ink"), "{page}");
    assert!(page.contains("<span class=\"pill repo\">demo</span>"));
    // Excluded in the checkout's own exclude file, so the board never reaches origin and
    // nothing is committed or left dirty for it.
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]).trim(), "");
    assert!(!mac.notes.join(".gitignore").exists());
    let exclude = git(&mac.notes, &["rev-parse", "--git-path", "info/exclude"]);
    assert!(
        std::fs::read_to_string(mac.notes.join(exclude.trim()))
            .unwrap()
            .contains("/home.html")
    );
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: which cap?\" <<'ASTROLABE'\nWaiting.\n\nNo choice yet.\n\nAsk.\nASTROLABE\n").1);
    let page = std::fs::read_to_string(&board).unwrap();
    let needs = column(&page, "needsyou");
    assert!(needs.contains("Smooth ink") && needs.contains("class=\"ask\">which cap?<"));
    assert!(!column(&page, "active").contains("Smooth ink"));
    assert!(mac.bash("s1", "astrolabe effort save --status done --journal \"Smoothed the ink.\" <<'ASTROLABE'\nDone.\n\nNothing chosen.\n\nNothing left.\nASTROLABE\n").1);
    let page = std::fs::read_to_string(&board).unwrap();
    assert!(column(&page, "done").contains("Smooth ink"));
    assert!(
        page.contains("<span>Smoothed the ink.</span></li>"),
        "{page}"
    );
    // The plain command redraws and names the file.
    std::fs::remove_file(&board).unwrap();
    let (out, ok) = mac.run(&mac.notes, &["home"], "");
    assert!(ok, "{out}");
    let file = out.lines().next().unwrap_or_default().trim();
    // The registered checkout may be the canonical path of the one the test knows.
    assert!(
        file.ends_with("home.html") && Path::new(file).exists(),
        "{out}"
    );
    // Then the address, and the server was asked for (a marker stands in for the process
    // under test).
    assert!(
        out.lines()
            .nth(1)
            .unwrap_or_default()
            .starts_with("http://localhost:"),
        "{out}"
    );
    assert!(mac.state().join("home-requested").exists());
    assert!(board.exists());
    // The other machine redraws its own copy when the saves arrive.
    assert!(mac.sync().1);
    let pc = world.machine("pc");
    assert!(!pc.notes.join("home.html").exists());
    assert!(pc.sync().1);
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(column(&page, "done").contains("Smooth ink"));
    assert_eq!(git(&pc.notes, &["status", "--porcelain"]).trim(), "");
    assert!(mac.bash("s1", "astrolabe effort save --status active <<'ASTROLABE'\nBack on it.\n\nNothing chosen.\n\nOne more pass.\nASTROLABE\n").1);
    assert!(mac.sync().1);
    assert!(pc.sync().1);
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(column(&page, "active").contains("Smooth ink"), "{page}");
    assert!(!column(&page, "done").contains("Smooth ink"));
    // An ignore rule that unignores it stops the draw, and says so.
    std::fs::write(pc.notes.join(".gitignore"), "!home.html\n").unwrap();
    let (out, ok) = pc.run(&pc.notes, &["home"], "");
    assert!(!ok && out.contains("overrides the exclude file"), "{out}");
    std::fs::remove_file(pc.notes.join(".gitignore")).unwrap();
    assert!(pc.run(&pc.notes, &["home"], "").1);
    // A fixed theme is kept per machine and carried by every later draw.
    let (out, ok) = pc.run(&pc.notes, &["home", "--theme", "dark"], "");
    assert!(ok, "{out}");
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(page.contains("content=\"dark\"") && page.contains(" data-theme=\"dark\">"));
    assert!(pc.sync().1);
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(page.contains("content=\"dark\""));
    // A palette is kept the same way, beside the theme.
    let (out, ok) = pc.run(&pc.notes, &["home", "--palette", "iris"], "");
    assert!(ok, "{out}");
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(
        page.contains("data-palette=\"iris\" data-theme=\"dark\">"),
        "{page}"
    );
    let (out, ok) = pc.run(&pc.notes, &["home", "--palette", "mauve"], "");
    assert!(
        !ok && out.contains("iris, ember, slate, moss or graphite"),
        "{out}"
    );
    for palette in ["moss", "graphite"] {
        let (out, ok) = pc.run(&pc.notes, &["home", "--palette", palette], "");
        assert!(ok, "{out}");
        let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
        assert!(page.contains(&format!("data-palette=\"{palette}\" data-theme=\"dark\">")));
    }
    let (out, ok) = pc.run(&pc.notes, &["home", "--theme", "blue"], "");
    assert!(!ok && out.contains("auto, light or dark"), "{out}");
    assert!(pc.run(&pc.notes, &["home", "--theme", "auto"], "").1);
    assert!(
        std::fs::read_to_string(pc.notes.join("home.html"))
            .unwrap()
            .contains("prefers-color-scheme")
    );
    // A tracked board is left alone and said so.
    std::fs::write(pc.notes.join("home.html"), "mine").unwrap();
    git(&pc.notes, &["add", "-f", "home.html"]);
    git(&pc.notes, &["commit", "-qm", "tracked board"]);
    assert!(pc.sync().1);
    assert_eq!(
        std::fs::read_to_string(pc.notes.join("home.html")).unwrap(),
        "mine"
    );
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert!(status.contains("home.html is tracked"), "{status}");
}

/// What a reset connection left that the recovery poll can use: the saturated server's 503,
/// whole or cut short anywhere in its status line (seen on Windows CI). The poll takes
/// anything but a 200 as "not yet" and asks again.
fn busy_before_reset(out: &str) -> bool {
    const BUSY: &str = "HTTP/1.1 503 Service Unavailable\r\n";
    !out.is_empty() && (out.starts_with(BUSY) || BUSY.starts_with(out))
}

#[test]
fn a_reset_mid_status_line_still_counts_as_busy() {
    assert!(busy_before_reset("HTTP/1.1 503 Serv"));
    assert!(busy_before_reset("HTTP/1.1 503 Service Unavailable\r"));
    assert!(busy_before_reset(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n"
    ));
    assert!(!busy_before_reset(""));
    assert!(!busy_before_reset("HTTP/1.1 200 OK\r\n"));
    assert!(!busy_before_reset(
        "HTTP/1.1 503 Service Unavailable, no line end"
    ));
}

#[test]
fn the_board_is_served_fresh_on_localhost() {
    use std::io::{BufRead, BufReader, Read};
    use std::time::Duration;
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let mut child = Kill(
        mac.command(&mac.notes, &["home", "--serve", "--port", "0"])
            .env("ASTROLABE_HOME_IDLE_SECS", "6")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut url = String::new();
        let _ = BufReader::new(stdout).read_line(&mut url);
        let _ = tx.send(url);
    });
    let url = rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|e| server_failed(&mut child.0, &format!("no server URL: {e}")));
    let addr = url
        .trim()
        .strip_prefix("http://")
        .and_then(|u| u.strip_suffix('/'))
        .unwrap_or_else(|| server_failed(&mut child.0, &format!("invalid server URL: {url:?}")))
        .to_owned();
    // A second server on this machine leaves at once, naming the address.
    let (out, ok) = mac.run(&mac.notes, &["home", "--serve", "--port", "0"], "");
    assert!(
        ok && out.contains("already serving") && out.contains("http://localhost:"),
        "{out}"
    );
    // One request; a connection the runner's stack resets before any byte arrives (seen
    // once on Windows CI) is tried again, since that's the runner, not the server.
    let get = |target: &str, host: &str| {
        let mut last = String::new();
        for _ in 0..3 {
            let mut s = std::net::TcpStream::connect(&addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            s.write_all(format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
                .unwrap();
            let mut out = String::new();
            match s.read_to_string(&mut out) {
                Ok(_) => return out,
                // A saturated listener can close with the request still unread.
                // Keep its status for the recovery poll even if Windows resets the connection.
                Err(e)
                    if busy_before_reset(&out)
                        && matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) =>
                {
                    return out;
                }
                Err(e)
                    if out.is_empty()
                        && matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) =>
                {
                    last = format!("{e}");
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => panic!("{e}: {out}"),
            }
        }
        panic!("reset three times: {last}")
    };
    let page = get("/", &addr);
    assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
    assert!(page.contains("Cache-Control: no-store"));
    // The style sheet and script are matched without whitespace, so formatting them can't
    // make these checks pass or fail.
    let squeezed: String = page.split_whitespace().collect();
    assert!(
        !page.contains("http-equiv=\"refresh\"") && squeezed.contains("newEventSource('/events')"),
        "a served page follows the stream instead of reloading: {page}"
    );
    // A failed refresh is announced in the sidebar's status region, styled as a warning.
    assert!(
        page.contains("<span class=\"live\" id=\"live\" role=\"status\" hidden>")
            && squeezed.contains(".live.stale{")
            && squeezed.contains("classList.toggle('stale',"),
        "{page}"
    );
    assert!(column(&page, "active").contains("Smooth ink"));
    // The card's title leads with its number, shown rather than hidden.
    assert!(
        page.contains("<span class=\"num\">1</span> Smooth ink")
            && !squeezed.contains(".cardh3.num{display:none"),
        "{page}"
    );
    // The card links through the server, since a browser refuses file links from a page.
    assert!(page.contains("href=\"/efforts/"), "{page}");
    // The card's own link, past the style sheet and the tree.
    let link = page[page.find("class=\"card").unwrap()..]
        .split("href=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned();
    let note = get(&link, &addr);
    assert!(
        note.starts_with("HTTP/1.1 200 OK\r\n") && note.contains("text/html"),
        "{note}"
    );
    assert!(
        // The page drops the file's padding; the anchor keeps it so old links still land.
        note.contains("<h1 id=\"0001-smooth-ink\">1 Smooth ink</h1>")
            && note.contains("<title>1 Smooth ink"),
        "{note}"
    );
    // A page holding the stream hears of a save within a few seconds; meanwhile the stream
    // says nothing but that it's alive.
    let mut events = std::net::TcpStream::connect(&addr).unwrap();
    events
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    events
        .write_all(format!("GET /events HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
        .unwrap();
    let mut heard = String::new();
    let mut buf = [0u8; 1024];
    let n = events.read(&mut buf).unwrap();
    heard.push_str(&String::from_utf8_lossy(&buf[..n]));
    assert!(
        heard.starts_with("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream"),
        "{heard}"
    );
    std::thread::sleep(Duration::from_millis(1500));
    // Drawn per request: a save shows on the next fetch with no redraw of the file.
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: which cap?\" <<'ASTROLABE'\nWaiting.\n\nNo choice yet.\n\nAsk.\nASTROLABE\n").1);
    let page = get("/", &addr);
    assert!(column(&page, "needsyou").contains("which cap?"), "{page}");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !heard.contains("event: changed") && std::time::Instant::now() < deadline {
        match events.read(&mut buf) {
            Ok(n) if n > 0 => heard.push_str(&String::from_utf8_lossy(&buf[..n])),
            _ => break,
        }
    }
    assert!(heard.contains("event: changed\ndata: "), "{heard}");
    drop(events);
    // Another origin can't hold the stream either.
    let mut other = std::net::TcpStream::connect(&addr).unwrap();
    other
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    other
        .write_all(b"GET /events HTTP/1.1\r\nHost: evil.example:80\r\n\r\n")
        .unwrap();
    let mut out = String::new();
    other.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 421"), "{out}");
    // A lesson in the notes' inbox is counted, listed and served as a page.
    assert!(!page.contains("to triage"), "{page}");
    std::fs::create_dir_all(mac.notes.join("lessons")).unwrap();
    std::fs::write(
        mac.notes.join("lessons/2026-11-05-gates.md"),
        "# Gates did not stop\n\nWhat happened.\n",
    )
    .unwrap();
    let page = get("/", &addr);
    assert!(page.contains("<b>1</b> lesson to triage"), "{page}");
    assert!(
        page.contains("href=\"/lessons/2026-11-05-gates.md\">Gates did not stop</a>"),
        "{page}"
    );
    let lesson = get("/lessons/2026-11-05-gates.md", &addr);
    assert!(
        lesson.starts_with("HTTP/1.1 200 OK\r\n") && lesson.contains("Gates did not stop"),
        "{lesson}"
    );
    assert!(get("/lessons/../knowledge/owner.md", &addr).starts_with("HTTP/1.1 404"));
    // An open draft is counted; the program never removes one.
    assert!(!page.contains("open draft"), "{page}");
    std::fs::create_dir_all(mac.notes.join("drafts")).unwrap();
    std::fs::write(mac.notes.join("drafts/.gitkeep"), "").unwrap();
    std::fs::write(
        mac.notes.join("drafts/2026-11-05-release-note.md"),
        "# Release note\n\nText to post.\n",
    )
    .unwrap();
    let page = get("/", &addr);
    assert!(page.contains("<b>1</b> open draft</span>"), "{page}");
    assert!(get("/favicon.ico", &addr).starts_with("HTTP/1.1 404"));
    assert!(get("/efforts/C:owner.md", &addr).starts_with("HTTP/1.1 404"));
    assert!(get("/efforts/..%2Fknowledge%2Fowner.md", &addr).starts_with("HTTP/1.1 404"));
    // HEAD gets the headers and no body.
    let mut s = std::net::TcpStream::connect(&addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(format!("HEAD / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(
        out.starts_with("HTTP/1.1 200 OK") && out.ends_with("\r\n\r\n"),
        "{out}"
    );
    assert!(get("/efforts/../knowledge/owner.md", &addr).starts_with("HTTP/1.1 404"));
    // Any note by its path is a page, since the board is where the owner reads them; a
    // dot folder (the plugin's own state, an app's config) is not.
    assert!(get("/knowledge/owner.md", &addr).starts_with("HTTP/1.1 200"));
    assert!(get("/.git/config", &addr).starts_with("HTTP/1.1 404"));
    assert!(get("/.agents/x.md", &addr).starts_with("HTTP/1.1 404"));
    assert!(
        get("/", "evil.example:80").starts_with("HTTP/1.1 421"),
        "another host gets nothing"
    );
    // A connection that says nothing gets a bad-request reply, not a hung server.
    let mut quiet = std::net::TcpStream::connect(&addr).unwrap();
    quiet
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    quiet.write_all(b"GET / HTTP/1.1\r\n").unwrap();
    quiet.shutdown(std::net::Shutdown::Write).unwrap();
    let mut out = String::new();
    quiet.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    assert!(
        get("/", &addr).starts_with("HTTP/1.1 200"),
        "still serving after a bad request"
    );
    // A head that never ends and outgrows the bound is refused, not buffered.
    let mut big = std::net::TcpStream::connect(&addr).unwrap();
    big.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let _ = big.write_all(format!("GET / HTTP/1.1\r\nX: {}\r\n", "y".repeat(20_000)).as_bytes());
    let mut out = String::new();
    let _ = big.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    // Past the handler cap an idle connection is told to come back, and the rest still serve
    // once the idle ones go.
    let idle: Vec<_> = (0..32)
        .map(|_| std::net::TcpStream::connect(&addr).unwrap())
        .collect();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut out = String::new();
    while !out.starts_with("HTTP/1.1 503") {
        assert!(
            std::time::Instant::now() < deadline,
            "no 503 past the cap: {out}"
        );
        let mut extra = std::net::TcpStream::connect(&addr).unwrap();
        extra
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        out.clear();
        let _ = extra.read_to_string(&mut out);
    }
    drop(idle);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !get("/", &addr).starts_with("HTTP/1.1 200") {
        assert!(
            std::time::Instant::now() < deadline,
            "not serving again after the idle ones left"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // With nothing asking for a while, the server stops on its own and the lock goes with
    // it; the next change to the notes starts one again.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let exited = loop {
        if let Ok(Some(status)) = child.0.try_wait() {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        exited.is_some_and(|s| s.success()),
        "the server stops when idle: {exited:?}"
    );
}

#[test]
fn a_replaced_program_takes_over_on_the_same_port() {
    use std::io::{BufRead, BufReader, Read};
    use std::time::{Duration, Instant, SystemTime};
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The server runs from a copy, so the test can replace it the way an install does.
    let built = Path::new(env!("CARGO_BIN_EXE_astrolabe"));
    let bin = mac.dir.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join(built.file_name().unwrap());
    std::fs::copy(built, &exe).unwrap();
    let mut child = Kill(spawn_fresh_copy(
        mac.command_from(&exe, &mac.notes, &["home", "--serve", "--port", "0"])
            .env("ASTROLABE_HOME_IDLE_SECS", "8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    ));
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut url = String::new();
        let _ = BufReader::new(stdout).read_line(&mut url);
        let _ = tx.send(url);
    });
    let url = rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|e| server_failed(&mut child.0, &format!("no server URL: {e}")));
    let addr = url
        .trim()
        .strip_prefix("http://")
        .and_then(|u| u.strip_suffix('/'))
        .unwrap_or_else(|| server_failed(&mut child.0, &format!("invalid server URL: {url:?}")))
        .to_owned();
    let serving = || {
        let Ok(mut s) = std::net::TcpStream::connect(&addr) else {
            return false;
        };
        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = s.write_all(format!("GET / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes());
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out.starts_with("HTTP/1.1 200")
    };
    assert!(serving(), "the first server answers");
    // An install's swap: the old file renamed aside, the new one renamed into its place.
    let new = bin.join("astrolabe.new");
    std::fs::copy(built, &new).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&new)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    std::fs::rename(&exe, bin.join("astrolabe.old")).unwrap();
    std::fs::rename(&new, &exe).unwrap();
    // The first server hands over and leaves; the one it started answers on the same port.
    let deadline = Instant::now() + Duration::from_secs(20);
    let exited = loop {
        if let Ok(Some(status)) = child.0.try_wait() {
            break Some(status);
        }
        if Instant::now() > deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        exited.is_some_and(|s| s.success()),
        "the replaced server leaves: {exited:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !serving() {
        let log = std::fs::read_to_string(mac.state().join("home.log")).unwrap_or_default();
        assert!(
            Instant::now() < deadline,
            "nothing took over on {addr}; home.log: {log}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn a_board_left_open_pulls_the_other_machines_saves() {
    use std::io::{BufRead, BufReader, Read};
    use std::time::{Duration, Instant};
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    let mut child = Kill(
        pc.command(&pc.notes, &["home", "--serve", "--port", "0"])
            .env("ASTROLABE_HOME_IDLE_SECS", "60")
            .env("ASTROLABE_HOME_PULL_SECS", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut url = String::new();
        let _ = BufReader::new(stdout).read_line(&mut url);
        let _ = tx.send(url);
    });
    let url = rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|e| server_failed(&mut child.0, &format!("no server URL: {e}")));
    let addr = url
        .trim()
        .strip_prefix("http://")
        .and_then(|u| u.strip_suffix('/'))
        .unwrap_or_else(|| server_failed(&mut child.0, &format!("invalid server URL: {url:?}")))
        .to_owned();
    let arrived = || pc.notes.join(shelf("0001-smooth-ink.md")).exists();
    let owner = pc.notes.join("knowledge/owner.md");
    // A pull in flight resets the tree before it puts hand edits back: wait for the rebase
    // to be gone and the edit to be back before looking.
    let settled = || {
        let deadline = Instant::now() + Duration::from_secs(15);
        while (pc.notes.join(".git/rebase-merge").exists()
            || std::fs::read_to_string(&owner).unwrap_or_default() != "OWNER-TEXT edited by hand")
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    // The other machine saves and pushes; with no page open here, nothing is fetched. The
    // owner's seat is mid-edit meanwhile.
    write(&owner, "OWNER-TEXT edited by hand");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!arrived(), "no page open, no pull");
    // A page holding the stream: the save is pulled within the interval, and the page hears.
    let mut events = std::net::TcpStream::connect(&addr).unwrap();
    events
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    events
        .write_all(format!("GET /events HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
        .unwrap();
    let mut heard = String::new();
    let mut buf = [0u8; 1024];
    let deadline = Instant::now() + Duration::from_secs(15);
    while !(arrived() && heard.contains("event: changed")) && Instant::now() < deadline {
        if let Ok(n) = events.read(&mut buf) {
            heard.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
    if !arrived() {
        server_failed(
            &mut child.0,
            &format!("the open board didn't pull the other machine's save: {heard}"),
        );
    }
    assert!(heard.contains("event: changed"), "{heard}");
    // The hand edit survived the pull and nothing was left stashed.
    settled();
    assert_eq!(
        std::fs::read_to_string(&owner).unwrap(),
        "OWNER-TEXT edited by hand"
    );
    assert!(git(&pc.notes, &["stash", "list"]).trim().is_empty());
    // With nothing new on origin, later ticks fetch but leave the tree and HEAD alone.
    let reflog = git(&pc.notes, &["reflog", "--format=%H"]);
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(
        git(&pc.notes, &["reflog", "--format=%H"]),
        reflog,
        "no rebase ran"
    );
    assert!(git(&pc.notes, &["status", "--porcelain"]).contains("owner.md"));
    // The next save arrives too, and the hand edit survives it.
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: which cap?\" <<'ASTROLABE'\nWaiting.\n\nNo choice yet.\n\nAsk.\nASTROLABE\n").1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let file = pc.notes.join(shelf("0001-smooth-ink.md"));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !std::fs::read_to_string(&file)
        .unwrap_or_default()
        .contains("which cap?")
        && Instant::now() < deadline
    {
        let _ = events.read(&mut buf);
    }
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("which cap?")
    );
    settled();
    assert_eq!(
        std::fs::read_to_string(&owner).unwrap(),
        "OWNER-TEXT edited by hand"
    );
    drop(events);
}

fn server_failed(child: &mut std::process::Child, why: &str) -> ! {
    use std::io::Read;
    let _ = child.kill();
    let status = child.wait();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    panic!("{why}; status {status:?}; stderr: {stderr}");
}

#[test]
fn a_preview_serves_beside_the_machine_and_saves_nothing() {
    use std::io::{BufRead, BufReader, Read};
    use std::time::Duration;
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let before = std::fs::read_to_string(mac.state().join("home.json")).ok();
    let mut child = Kill(
        mac.command(&mac.notes, &["home", "--preview", "--theme", "light"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut url = String::new();
        let _ = BufReader::new(stdout).read_line(&mut url);
        let _ = tx.send(url);
    });
    let url = rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|e| server_failed(&mut child.0, &format!("no preview URL: {e}")));
    let addr = url
        .trim()
        .strip_prefix("http://")
        .and_then(|u| u.strip_suffix('/'))
        .unwrap_or_else(|| server_failed(&mut child.0, &format!("invalid preview URL: {url:?}")))
        .to_owned();
    let mut s = std::net::TcpStream::connect(&addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(format!("GET / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
        .unwrap();
    let mut page = String::new();
    let _ = s.read_to_string(&mut page);
    assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
    // The live notes, in the look it was given.
    assert!(column(&page, "active").contains("Smooth ink"), "{page}");
    assert!(page.contains(" data-theme=\"light\">"), "{page}");
    // The machine's port, look and lock are as they were.
    assert_eq!(
        std::fs::read_to_string(mac.state().join("home.json")).ok(),
        before
    );
    assert!(!mac.state().join("home.lock").exists());
    // With the machine's server, not in place of it.
    let (out, ok) = mac.run(&mac.notes, &["home", "--preview", "--serve"], "");
    assert!(!ok && out.contains("leave out --serve"), "{out}");
}

/// Spawns a program this process just wrote. While `fs::copy` holds the new file open for
/// writing, another test thread can fork; its child keeps that handle until it execs, and
/// Linux refuses to run a file open for writing ("text file busy"). The handle is gone once
/// that child execs, so trying again is enough (rust-lang/rust#114554).
fn spawn_fresh_copy(command: &mut Command) -> std::process::Child {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match command.spawn() {
            Err(e)
                if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            spawned => return spawned.unwrap(),
        }
    }
}
