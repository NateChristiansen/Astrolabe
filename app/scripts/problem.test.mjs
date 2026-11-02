// A problem belongs to the session open when it arrived, so it can't sit over another one.
import assert from "node:assert/strict";
import { test } from "node:test";
import { shownProblem } from "../src/core.ts";

test("a problem shows on the session it arrived on, and no other", () => {
  const problem = { message: "That session has ended.", session: 1 };
  assert.equal(shownProblem(problem, 1), problem.message);
  assert.equal(shownProblem(problem, 2), undefined);
  assert.equal(shownProblem(problem, undefined), undefined);
});

test("a problem from before any session was open goes when one opens", () => {
  const problem = { message: "No repo." };
  assert.equal(shownProblem(problem, undefined), problem.message);
  assert.equal(shownProblem(problem, 1), undefined);
  assert.equal(shownProblem(undefined, 1), undefined);
});
