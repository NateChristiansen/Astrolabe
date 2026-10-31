/* Generated from protocol/schemas. Do not edit. */

export type Evidence = "executed" | "inferred";
/**
 * Payload versions evolve independently of the envelope version.
 */
export type PayloadVersion = "1";
export type Severity = "info" | "low" | "medium" | "high" | "critical";

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
