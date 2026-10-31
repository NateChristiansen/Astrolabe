const { test } = require('node:test');
const assert = require('node:assert/strict');
const { mkdtempSync, writeFileSync, readFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { spawnSync } = require('node:child_process');
const { detect, loadMarkers, run } = require('./scan');

const secret = ['-----BEGIN', 'OPENSSH', 'PRIVATE KEY-----'].join(' ');
const work = ['SYNTHETIC', 'PRIVATE', 'WORK'].join('_');
function repo(t) {
  const cwd = mkdtempSync(join(tmpdir(), 'astrolabe-scan-'));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const git = (...args) => {
    const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  git('init', '-b', 'main');
  git('config', 'user.name', 'Synthetic Fixture');
  git('config', 'user.email', 'fixture.invalid');
  const markers = join(cwd, 'local-markers');
  writeFileSync(markers, `${work}\n`);
  const scan = (mode, input) => run([mode, '--markers', markers], { cwd, input, warn: () => {} });
  const stage = (content) => {
    writeFileSync(join(cwd, 'example.txt'), content);
    git('add', 'example.txt');
  };
  return { cwd, git, markers, scan, stage };
}

test('generic rules detect synthetic keys, token prefixes, and entropy', () => {
  assert.deepEqual(detect(secret, []), ['private-key']);
  for (const prefix of ['ghp_', 'github_pat_', 'sk-ant-', 'xoxb-', 'AKIA']) {
    const suffix = prefix === 'AKIA' ? 'ABCDEFGHIJKLMNOP' : 'a'.repeat(32);
    assert.ok(detect(prefix + suffix, []).includes('token-prefix'));
  }
  const value = ['Ab9', 'zY8', 'xW7', 'vU6', 'tS5', 'rQ4', 'pN3', 'mK2'].join('');
  assert.ok(detect(`opaque = "${value}"`, []).includes('high-entropy-assignment'));
  assert.ok(detect(`"api_key": "${value}"`, []).includes('high-entropy-assignment'));
  assert.deepEqual(detect('ordinary = "hello"', []), []);
  assert.deepEqual(detect('checksum = "' + '0123456789abcdef'.repeat(4) + '"', []), []);
  assert.ok(detect('token = "' + '0123456789abcdef'.repeat(4) + '"', []).includes('high-entropy-assignment'));
});

test('private markers ignore case and trim marker-file whitespace', (t) => {
  const { markers, scan, stage } = repo(t);
  writeFileSync(markers, `  ${work}   \n  \n`);
  assert.deepEqual(loadMarkers(markers, () => {}), [work]);
  assert.deepEqual(detect(work.toLowerCase(), [work]), ['private-marker']);
  assert.deepEqual(detect(work, [work.toLowerCase()]), ['private-marker']);
  stage(work.toLowerCase());
  assert.equal(scan('--staged'), 1);
});

for (const [name, format] of [
  ['inline JSON password', (value) => `prefix {"password":"${value}"}`],
  ['inline API key assignment', (value) => `prefix api_key="${value}"`],
  ['inline authorization bearer header', (value) => `prefix 'Authorization: Bearer ${value}'`],
]) {
  test(`sensitive keys detect ${name}`, () => {
    const value = ['Ab9', 'zY8', 'xW7', 'vU6', 'tS5', 'rQ4', 'pN3', 'mK2'].join('');
    assert.deepEqual(detect(format(value), []), ['high-entropy-assignment']);
  });
}

test('scanner source and tests do not trigger generic rules', () => {
  for (const file of ['scan', 'scan.test.cjs', 'seed.test.cjs']) {
    assert.deepEqual(detect(readFileSync(join(__dirname, file), 'utf8'), []), [], file);
  }
});

test('missing marker file warns; other read failures fail closed', (t) => {
  const { cwd } = repo(t);
  const warnings = [];
  assert.deepEqual(loadMarkers(join(cwd, 'missing'), (m) => warnings.push(m)), []);
  assert.equal(warnings.length, 1);
  assert.throws(() => loadMarkers(cwd, () => {}));
});

test('staged key and temp-file work marker reject; clean index passes', (t) => {
  const { cwd, stage, scan } = repo(t);
  stage(secret + '\nsynthetic block\n');
  assert.equal(scan('--staged'), 1);
  stage(work);
  assert.equal(scan('--staged'), 1);
  stage('Clean public content.\n');
  // Working-tree contamination must not change the index scan result.
  writeFileSync(join(cwd, 'example.txt'), secret);
  assert.equal(scan('--staged'), 0);
});

test('push scans intermediate commits and supports updates/deletions/multiple refs', (t) => {
  const { git, stage, scan } = repo(t);
  stage('Clean public content.');
  git('commit', '-m', 'Clean base');
  const base = git('rev-parse', 'HEAD');
  stage(secret);
  git('commit', '-m', 'Synthetic planted secret');
  stage('Clean again.');
  git('commit', '-m', 'Remove synthetic secret');
  const tip = git('rev-parse', 'HEAD');
  const zero = '0'.repeat(40);
  const update = (local, remote) => `refs/heads/main ${local} refs/heads/main ${remote}\n`;
  assert.equal(scan('--push', update(tip, zero)), 1);
  assert.equal(scan('--push', update(tip, base)), 1);
  assert.equal(scan('--push', update(base, zero)), 0);
  assert.equal(scan('--push', update(zero, tip)), 0);
  assert.equal(scan('--push', update(base, zero) + update(tip, base)), 1);
  assert.equal(scan('--all'), 1);
  assert.throws(() => scan('--push', 'malformed'));
});

test('scanner entrypoint returns nonzero and diagnostics redact content', (t) => {
  const { cwd, markers, stage } = repo(t);
  stage(work);
  const result = spawnSync(process.execPath, [join(__dirname, 'scan'), '--markers', markers], {
    cwd, encoding: 'utf8',
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /private-marker/);
  assert.ok(!result.stderr.includes(work));
});
