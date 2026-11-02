import { useEffect, useRef, useState } from "react";
import type { BridgeEvent, BridgeState, SessionRow, TranscriptItem } from "./protocol/bridge-event-v1";
import type { BridgeProvider } from "./protocol/bridge-request-v1";
import { addItem, connect, providerWords, stateWords, toolWords } from "./core";
import type { Core, Link } from "./core";

function NewSession({ state, core }: { state: BridgeState; core: Core }) {
  const [repo, setRepo] = useState("");
  const [provider, setProvider] = useState<BridgeProvider | "">("");
  const chosenRepo = repo || state.repos[0] || "";
  const chosenProvider = provider || state.providers[0] || "";
  if (state.repos.length === 0) {
    return <p className="hint">No repos yet. Add one with: astrolabe repo add &lt;name&gt; &lt;folder&gt;</p>;
  }
  return (
    <form
      className="new"
      onSubmit={(event) => {
        event.preventDefault();
        if (chosenRepo && chosenProvider) {
          core.send({ type: "start", repo: chosenRepo, provider: chosenProvider });
        }
      }}
    >
      <select aria-label="Repo" value={chosenRepo} onChange={(e) => setRepo(e.target.value)}>
        {state.repos.map((name) => (
          <option key={name}>{name}</option>
        ))}
      </select>
      <select
        aria-label="Agent"
        value={chosenProvider}
        onChange={(e) => setProvider(e.target.value as BridgeProvider)}
      >
        {state.providers.map((name) => (
          <option key={name} value={name}>
            {providerWords[name]}
          </option>
        ))}
      </select>
      <button>New session</button>
    </form>
  );
}

function Approval({ row, core }: { row: SessionRow["requests"][number]; core: Core }) {
  const [saying, setSaying] = useState(false);
  const [words, setWords] = useState("");
  const answer = (choice: Parameters<typeof send>[0]) => send(choice);
  function send(choice: { kind: "allow" } | { kind: "reject" } | { kind: "reject_and_say"; words: string }) {
    core.send({ type: "answer", request: row.id, choice });
  }
  return (
    <section className="card approval">
      <h3>The agent is asking</h3>
      <p>{row.asking}</p>
      {row.detail && <pre>{row.detail}</pre>}
      {row.live ? (
        <>
          <div className="actions">
            <button className="go" onClick={() => answer({ kind: "allow" })}>
              Allow
            </button>
            <button onClick={() => answer({ kind: "reject" })}>Reject</button>
            <button onClick={() => setSaying(!saying)}>Reject and say what instead</button>
          </div>
          {saying && (
            <form
              className="say"
              onSubmit={(event) => {
                event.preventDefault();
                if (words.trim()) answer({ kind: "reject_and_say", words });
              }}
            >
              <input
                autoFocus
                value={words}
                placeholder="What should it do instead?"
                onChange={(e) => setWords(e.target.value)}
              />
              <button>Send</button>
            </form>
          )}
        </>
      ) : (
        <>
          <p className="hint">{row.decided}</p>
          <div className="actions">
            <button onClick={() => core.send({ type: "dismiss", request: row.id })}>Dismiss</button>
          </div>
        </>
      )}
    </section>
  );
}

function Chat({ row, items, core }: { row: SessionRow; items: TranscriptItem[]; core: Core }) {
  const [text, setText] = useState("");
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => end.current?.scrollIntoView({ block: "end" }), [items.length, row.requests.length]);
  return (
    <main className="chat">
      <header>
        <h2>
          {row.repo} · {providerWords[row.provider]}
        </h2>
        <span className={`state ${row.state}`}>{stateWords[row.state]}</span>
        {row.can_stop && <button onClick={() => core.send({ type: "stop", session: row.id })}>Stop</button>}
      </header>
      <div className="transcript">
        {row.why && <p className="why">{row.why}</p>}
        {items.map((item) =>
          item.role === "tool" ? (
            <p key={item.tool} className="tool">
              {item.text || "A tool"} <span>{toolWords[item.status ?? ""] ?? item.status}</span>
            </p>
          ) : (
            <p key={item.seq} className={item.role}>
              {item.text}
            </p>
          ),
        )}
        {row.requests.map((request) => (
          <Approval key={request.id} row={request} core={core} />
        ))}
        {row.cards.map((card) => (
          <section key={card.id} className="card stall">
            <h3>This may be stuck</h3>
            <p>{card.summary}</p>
            <div className="actions">
              <button onClick={() => core.send({ type: "keep_waiting", card: card.id })}>Keep waiting</button>
              {row.can_stop && <button onClick={() => core.send({ type: "stop", session: row.id })}>Stop</button>}
            </div>
          </section>
        ))}
        <div ref={end} />
      </div>
      <form
        className="prompt"
        onSubmit={(event) => {
          event.preventDefault();
          if (text.trim() && row.can_prompt) {
            core.send({ type: "prompt", session: row.id, text });
            setText("");
          }
        }}
      >
        <textarea
          aria-label="Message"
          rows={2}
          value={text}
          placeholder={row.can_prompt ? "Say what to do" : "You can type when it's your turn"}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              e.currentTarget.form?.requestSubmit();
            }
          }}
        />
        <button disabled={!row.can_prompt || !text.trim()}>Send</button>
      </form>
    </main>
  );
}

export function App() {
  const [core, setCore] = useState<Core>();
  const [link, setLink] = useState<{ link: Link; why?: string }>({ link: "connecting" });
  const [state, setState] = useState<BridgeState>();
  const [problem, setProblem] = useState<string>();
  const [open, setOpen] = useState<number>();
  const [transcripts, setTranscripts] = useState<Record<number, TranscriptItem[]>>({});
  const opened = useRef<number | undefined>(undefined);
  const live = useRef<Core | undefined>(undefined);
  opened.current = open;

  useEffect(() => {
    const onEvent = (event: BridgeEvent) => {
      if (event.type === "state") {
        setState(event.state);
      } else if (event.type === "problem") {
        setProblem(event.message);
      } else if (event.type === "transcript") {
        const items = event.items.reduce(addItem, [] as TranscriptItem[]);
        setTranscripts((all) => ({ ...all, [event.session]: items }));
      } else {
        setTranscripts((all) => {
          const items = all[event.session];
          return items ? { ...all, [event.session]: addItem(items, event.item) } : all;
        });
      }
    };
    const made = connect(onEvent, (next, why) => {
      setLink({ link: next, why });
      // A new connection may have missed lines: read the open transcript again.
      if (next === "connected" && opened.current !== undefined) {
        live.current?.send({ type: "open", session: opened.current });
      }
    });
    live.current = made;
    setCore(made);
    return () => made.close();
  }, []);

  useEffect(() => {
    if (open !== undefined && link.link === "connected") core?.send({ type: "open", session: open });
  }, [open, core, link.link]);

  const sessions = state?.sessions ?? [];
  const row = sessions.find((session) => session.id === open);
  return (
    <div className="app">
      <aside>
        <h1>Astrolabe</h1>
        {state && core && <NewSession state={state} core={core} />}
        <ul>
          {sessions.map((session) => (
            <li key={session.id}>
              <button className={session.id === open ? "row open" : "row"} onClick={() => setOpen(session.id)}>
                <strong>{session.title ?? `${session.repo} · new`}</strong>
                <small>
                  {session.repo} · {providerWords[session.provider]}
                </small>
                <span className={`state ${session.state}`}>{stateWords[session.state]}</span>
              </button>
            </li>
          ))}
          {state?.plain.map((plain) => (
            <li key={`${plain.folder}${plain.at_ms}`} className="row plain">
              <strong>{plain.folder.split(/[\\/]/).pop()}</strong>
              <small>Plain shell, reported in {new Date(plain.at_ms).toLocaleString()}</small>
            </li>
          ))}
        </ul>
      </aside>
      <div className="main">
        {link.link !== "connected" && (
          <p className="banner">{link.why ?? "Connecting to the core…"}</p>
        )}
        {state?.restart && <p className="banner">{state.restart}</p>}
        {problem && (
          <p className="banner problem" onClick={() => setProblem(undefined)}>
            {problem}
          </p>
        )}
        {row && core ? (
          <Chat row={row} items={transcripts[row.id] ?? []} core={core} />
        ) : (
          <p className="empty">Pick a session, or start a new one.</p>
        )}
      </div>
    </div>
  );
}
