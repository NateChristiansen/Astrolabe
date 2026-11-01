const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {spawnSync} = require('node:child_process');

test('resume catalog response drops private contents and requires the matching request shape', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'scrub-codex-resume-'));
  try {
    const input = path.join(dir, 'private.jsonl'), output = path.join(dir, 'public.jsonl');
    const request = {jsonrpc: '2.0', id: 2, method: 'session/resume',
      params: {cwd: 'PRIVATE_SENTINEL', sessionId: 'PRIVATE_SENTINEL', mcpServers: []}};
    const response = {jsonrpc: '2.0', id: 2, result: {
      models: {currentModelId: 'PRIVATE_SENTINEL'},
      modes: {availableModes: [{name: 'PRIVATE_SENTINEL'}]},
      configOptions: [{value: 'PRIVATE_SENTINEL'}],
    }};
    const cases = [
      {name: 'matching resume', request, response, ok: true},
      {name: 'wrong method', request: {...request, method: 'session/load'}, response},
      {name: 'wrong id', request, response: {...response, id: 3}},
      {name: 'wrong process', request, response, responseProcess: 1},
      {name: 'extra result key', request, response: {...response,
        result: {...response.result, secret: 'PRIVATE_SENTINEL'}}},
      {name: 'missing catalog', request, response: {...response,
        result: {models: response.result.models, modes: response.result.modes}}},
      {name: 'invalid catalog type', request, response: {...response,
        result: {...response.result, models: []}}},
    ];
    for (const c of cases) {
      const raw = [
        {kind: 'wire', process: 0, elapsed_ms: 0, direction: 'to_agent', line: JSON.stringify(c.request)},
        {kind: 'wire', process: c.responseProcess || 0, elapsed_ms: 1, direction: 'from_agent', line: JSON.stringify(c.response)},
      ];
      fs.writeFileSync(input, raw.map(r => JSON.stringify(r)).join('\n'));
      const run = spawnSync(process.execPath, [path.join(__dirname, 'scrub-codex.cjs'), 'resume', input, output]);
      if (c.ok) {
        assert.equal(run.status, 0, c.name);
        const scrubbed = fs.readFileSync(output, 'utf8');
        assert.equal(scrubbed.includes('PRIVATE_SENTINEL'), false);
        assert.deepEqual(scrubbed.trim().split('\n').map(JSON.parse)[1].message,
          {jsonrpc: '2.0', id: 2, result: {}});
        fs.unlinkSync(output);
      } else {
        assert.notEqual(run.status, 0, c.name);
        assert.equal(fs.existsSync(output), false, c.name);
      }
    }
  } finally {
    fs.rmSync(dir, {recursive: true, force: true});
  }
});

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
