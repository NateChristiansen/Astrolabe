/* Generated from protocol/schemas. Do not edit. */

/**
 * One local WebSocket between the app and the core. Text frames, one JSON message each.
 * `Auth` must be the first frame; nothing else is read or sent before it succeeds.
 */
export type BridgeRequest =
  | {
      secret: string;
      type: "auth";
      [k: string]: unknown;
    }
  | {
      provider: BridgeProvider;
      repo: string;
      type: "start";
      [k: string]: unknown;
    }
  | {
      session: number;
      type: "open";
      [k: string]: unknown;
    }
  | {
      session: number;
      text: string;
      type: "prompt";
      [k: string]: unknown;
    }
  | {
      choice: BridgeChoice;
      request: string;
      type: "answer";
      [k: string]: unknown;
    }
  | {
      request: string;
      type: "dismiss";
      [k: string]: unknown;
    }
  | {
      card: number;
      type: "keep_waiting";
      [k: string]: unknown;
    }
  | {
      session: number;
      type: "stop";
      [k: string]: unknown;
    };
export type BridgeProvider = "claude" | "codex";
export type BridgeChoice =
  | {
      kind: "allow";
      [k: string]: unknown;
    }
  | {
      kind: "reject";
      [k: string]: unknown;
    }
  | {
      kind: "reject_and_say";
      words: string;
      [k: string]: unknown;
    };
