/* Generated from protocol/schemas. Do not edit. */

/**
 * Payload versions evolve independently of the envelope version.
 */
export type PayloadVersion = "1";

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
