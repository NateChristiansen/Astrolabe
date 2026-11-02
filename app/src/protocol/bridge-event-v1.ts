/* Generated from protocol/schemas. Do not edit. */

export type BridgeEvent =
  | {
      state: BridgeState;
      type: "state";
      [k: string]: unknown;
    }
  | {
      items: TranscriptItem[];
      session: number;
      type: "transcript";
      [k: string]: unknown;
    }
  | {
      item: TranscriptItem;
      session: number;
      type: "item";
      [k: string]: unknown;
    }
  | {
      message: string;
      type: "problem";
      [k: string]: unknown;
    };
export type BridgeProvider = "claude" | "codex";
export type SessionState =
  "starting" | "could_not_start" | "working" | "needs_you" | "your_turn" | "ended" | "failed" | "interrupted";
export type TranscriptRole = "you" | "agent" | "tool" | "note";

export interface BridgeState {
  plain: PlainRow[];
  providers: BridgeProvider[];
  repos: string[];
  /**
   * What this core's restart decided, when it decided anything.
   */
  restart?: string | null;
  sessions: SessionRow[];
  [k: string]: unknown;
}
/**
 * A session outside the app whose hooks reported in.
 */
export interface PlainRow {
  at_ms: number;
  folder: string;
  [k: string]: unknown;
}
export interface SessionRow {
  can_prompt: boolean;
  can_stop: boolean;
  cards: StallRow[];
  id: number;
  provider: BridgeProvider;
  repo: string;
  requests: ApprovalRow[];
  state: SessionState;
  title?: string | null;
  /**
   * Why it could not start or failed, in the core's own words.
   */
  why?: string | null;
  [k: string]: unknown;
}
export interface StallRow {
  id: number;
  summary: string;
  [k: string]: unknown;
}
export interface ApprovalRow {
  asking: string;
  /**
   * When not live, what happened to the session.
   */
  decided?: string | null;
  /**
   * What it asks to run, as the adapter sent it. A title can differ.
   */
  detail?: string | null;
  id: string;
  /**
   * False when the process that asked has gone, so no answer can reach it.
   */
  live: boolean;
  [k: string]: unknown;
}
export interface TranscriptItem {
  role: TranscriptRole;
  /**
   * Rises through a session's transcript.
   */
  seq: number;
  status?: string | null;
  text: string;
  /**
   * A tool's updates share this; the latest of each field stands.
   */
  tool?: string | null;
  [k: string]: unknown;
}
