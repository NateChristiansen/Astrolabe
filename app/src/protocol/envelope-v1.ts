/* Generated from protocol/schemas. Do not edit. */

/**
 * The discriminator selects exactly one typed payload.
 */
export type Envelope =
  | {
      body: string;
      id: string;
      kind: "review_finding";
      links?: Links | null;
      payload: ReviewFinding;
      producer: string;
      reply_to?: string | null;
      schema_version: EnvelopeVersion;
      /**
       * Software-supplied RFC 3339 timestamp in UTC.
       */
      timestamp: string;
      [k: string]: unknown;
    }
  | {
      body: string;
      id: string;
      kind: "ask_owner_request";
      links?: Links | null;
      payload: AskOwnerRequest;
      producer: string;
      reply_to?: string | null;
      schema_version: EnvelopeVersion;
      /**
       * Software-supplied RFC 3339 timestamp in UTC.
       */
      timestamp: string;
      [k: string]: unknown;
    }
  | {
      body: string;
      id: string;
      kind: "ask_owner_answer";
      links?: Links | null;
      payload: AskOwnerAnswer;
      producer: string;
      reply_to?: string | null;
      schema_version: EnvelopeVersion;
      /**
       * Software-supplied RFC 3339 timestamp in UTC.
       */
      timestamp: string;
      [k: string]: unknown;
    };
export type Evidence = "executed" | "inferred";
/**
 * Payload versions evolve independently of the envelope version.
 */
export type PayloadVersion = "1";
export type Severity = "info" | "low" | "medium" | "high" | "critical";
export type EnvelopeVersion = "1";

export interface Links {
  effort?: string | null;
  revision?: string | null;
  run?: string | null;
  [k: string]: unknown;
}
export interface ReviewFinding {
  actual: string;
  confidence?: Confidence | null;
  evidence: Evidence;
  expected: string;
  file: string;
  impact: string;
  /**
   * One-based source line.
   */
  line: number;
  payload_version: PayloadVersion;
  revision: string;
  severity: Severity;
  trigger: string;
  [k: string]: unknown;
}
/**
 * Uncalibrated self-report; never use as a probability or approval threshold.
 */
export interface Confidence {
  method: string;
  model: string;
  value: number;
  [k: string]: unknown;
}
export interface AskOwnerRequest {
  context: string;
  options?: string[] | null;
  payload_version: PayloadVersion;
  /**
   * Exactly one self-contained question, never a list of questions.
   */
  question: string;
  recommendation: Recommendation;
  what_waits: string;
  [k: string]: unknown;
}
export interface Recommendation {
  action: string;
  reason: string;
  [k: string]: unknown;
}
export interface AskOwnerAnswer {
  answer: string;
  payload_version: PayloadVersion;
  request_id: string;
  [k: string]: unknown;
}
