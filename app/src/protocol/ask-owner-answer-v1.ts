/* Generated from protocol/schemas. Do not edit. */

/**
 * Payload versions evolve independently of the envelope version.
 */
export type PayloadVersion = "1";

export interface AskOwnerAnswer {
  answer: string;
  payload_version: PayloadVersion;
  request_id: string;
  [k: string]: unknown;
}
