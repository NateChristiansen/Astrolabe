const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {spawnSync} = require('node:child_process');

for (const scenario of ['full-turn', 'approval-allow', 'approval-reject', 'two-turns', 'resume', 'cancel-tool']) {
  test(`scrub ${scenario} preserves replay shape and drops private fields`, () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'scrub-codex-'));
    try {
      const expected = fs.readFileSync(path.join(__dirname, '../protocol/fixtures/acp/codex', scenario + '.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
      const raw = [{kind: 'before', config: 'PRIVATE_SENTINEL'}];
      for (const row of expected) {
        if (row.exit_code !== undefined) raw.push({kind: 'exit', process: row.process, code: row.exit_code});
        else if (row.stderr !== undefined) raw.push({kind: 'wire', process: row.process, direction: 'stderr', line: 'PRIVATE_SENTINEL\n' + row.stderr});
        else {
          const message = structuredClone(row.message);
          message.private = 'PRIVATE_SENTINEL';
          if (message.params?.update) message.params.update.private = 'PRIVATE_SENTINEL';
          raw.push({kind: 'wire', ...row, line: JSON.stringify(message) + '\n', bytes: 'PRIVATE_SENTINEL'});
        }
      }
      const input = path.join(dir, 'private.jsonl'), output = path.join(dir, 'public.jsonl');
      fs.writeFileSync(input, raw.map(r => JSON.stringify(r)).join('\n'));
      const run = () => spawnSync(process.execPath, [path.join(__dirname, 'scrub-codex.cjs'), scenario, input, output]);
      assert.equal(run().status, 0);
      assert.deepEqual(fs.readFileSync(output, 'utf8').trim().split('\n').map(JSON.parse), expected);
      assert.notEqual(run().status, 0, 'must refuse overwrite');
      fs.unlinkSync(output);
      fs.appendFileSync(input, '\n' + JSON.stringify({kind: 'wire', process: 0, direction: 'from_agent', elapsed_ms: 1, line: JSON.stringify({method: 'unknown/private', params: {secret: 'PRIVATE_SENTINEL'}})}));
      assert.notEqual(run().status, 0);
      assert.equal(fs.existsSync(output), false, 'unknown shape must not produce partial output');
    } finally {
      fs.rmSync(dir, {recursive: true, force: true});
    }
  });
}
