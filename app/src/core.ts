// The window's one connection to the core, and the reducer for what it hears.
import type { BridgeEvent, TranscriptItem } from "./protocol/bridge-event-v1";
import type { BridgeRequest } from "./protocol/bridge-request-v1";

type Invoke = (command: string) => Promise<[number, string]>;
declare global {
  interface Window {
    __TAURI__?: { core: { invoke: Invoke } };
  }
}

export type Link = "connecting" | "connected" | "down" | "stopping" | "stopped";

export interface Core {
  /** False when the socket isn't open, so nothing went. */
  send(request: BridgeRequest): boolean;
  /** After a quit, the owner's word: finds or starts a core again. */
  start(): void;
  close(): void;
}

const RETRY_MS = 1000;

/** A core that dies comes back, unless it said it was going or this window is. */
export const retries = ({ told, closed }: { told: boolean; closed: boolean }) => !told && !closed;

/** Finds or starts the core, and comes back on its own after a restart: new secret, new port. */
export function connect(
  onEvent: (event: BridgeEvent) => void,
  onLink: (link: Link, why?: string) => void,
): Core {
  let socket: WebSocket | undefined;
  let closed = false;
  let told = false;
  const open = async () => {
    onLink("connecting");
    try {
      const invoke = window.__TAURI__?.core.invoke;
      if (!invoke) throw new Error("this page isn't running inside the app");
      const [port, secret] = await invoke("core_connection");
      if (closed) return;
      socket = new WebSocket(`ws://127.0.0.1:${port}`);
      socket.onopen = () => {
        socket?.send(JSON.stringify({ type: "auth", secret }));
        onLink("connected");
      };
      socket.onmessage = (message) => {
        const event: BridgeEvent = JSON.parse(message.data as string);
        if (event.type === "quitting") {
          told = true;
          onLink("stopping");
        }
        onEvent(event);
      };
      socket.onclose = () => {
        onLink(told ? "stopped" : "down");
        if (retries({ told, closed })) setTimeout(open, RETRY_MS);
      };
    } catch (why) {
      onLink("down", String(why));
      if (retries({ told, closed })) setTimeout(open, RETRY_MS);
    }
  };
  void open();
  return {
    send: (request) => {
      if (socket?.readyState !== WebSocket.OPEN) return false;
      socket.send(JSON.stringify(request));
      return true;
    },
    start: () => {
      told = false;
      void open();
    },
    close: () => {
      closed = true;
      socket?.close();
    },
  };
}

/** One bubble's worth: agent chunks run together, a tool's updates land on its one line. */
export function addItem(items: TranscriptItem[], item: TranscriptItem): TranscriptItem[] {
  if (items.some((seen) => seen.seq === item.seq)) return items;
  const last = items[items.length - 1];
  if (item.role === "agent" && last?.role === "agent") {
    return [...items.slice(0, -1), { ...last, text: last.text + item.text, seq: item.seq }];
  }
  if (item.tool != null) {
    const at = items.findIndex((seen) => seen.tool === item.tool);
    if (at >= 0) {
      const merged = {
        ...items[at],
        text: item.text || items[at].text,
        status: item.status ?? items[at].status,
      };
      return [...items.slice(0, at), merged, ...items.slice(at + 1)];
    }
  }
  return [...items, item];
}

export interface Problem {
  message: string;
  session?: number;
}

/** A problem names no session: it belongs to the one open when it arrived. */
export const shownProblem = (problem: Problem | undefined, open: number | undefined) =>
  problem && problem.session !== open ? undefined : problem?.message;

export const stateWords: Record<string, string> = {
  starting: "Starting",
  could_not_start: "Couldn't start",
  working: "Working",
  needs_you: "Needs you",
  your_turn: "Your turn",
  ended: "Ended",
  failed: "Failed",
  interrupted: "Interrupted",
  still_running: "Still running",
};

export const providerWords: Record<string, string> = { claude: "Claude", codex: "Codex" };

export const toolWords: Record<string, string> = {
  pending: "waiting",
  in_progress: "running",
  completed: "done",
  failed: "failed",
};
