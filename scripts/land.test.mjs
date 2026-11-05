import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('./land.mjs', import.meta.url));
const branch = 'feature/topic';
const ref = `refs/heads/${branch}`;

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'astrolabe-land-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const checkout = join(root, 'main');
  const worktree = join(root, 'finished work');
  const remote = join(root, 'remote.git');
  const env = {
    ...process.env,
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: join(root, 'empty-config'),
    GIT_AUTHOR_NAME: 'Astrolabe Test',
    GIT_AUTHOR_EMAIL: 'test@example.invalid',
    GIT_COMMITTER_NAME: 'Astrolabe Test',
    GIT_COMMITTER_EMAIL: 'test@example.invalid',
  };
  function run(command, args, cwd = root, status = 0) {
    const result = spawnSync(command, args, { cwd, env, encoding: 'utf8' });
    if (result.error) throw result.error;
    assert.equal(result.status, status, `${command} ${args.join(' ')}\n${result.stdout}${result.stderr}`);
    return result;
  }
  function git(cwd, ...args) {
    return run('git', args, cwd).stdout.trim();
  }
  function tip(cwd, name) {
    const result = spawnSync('git', ['rev-parse', '--verify', '--quiet', name], {
      cwd, env, encoding: 'utf8',
    });
    assert.ok(result.status === 0 || result.status === 1, result.stderr);
    return result.stdout.trim();
  }
  function hook(name, body) {
    const path = join(remote, 'hooks', name);
    writeFileSync(path, `#!/bin/sh\n${body}\n`);
    chmodSync(path, 0o755);
  }

  git(root, 'init', '--bare', '-b', 'main', remote);
  git(root, 'init', '-b', 'main', checkout);
  writeFileSync(join(checkout, '.gitignore'), 'build.log\n');
  git(checkout, 'add', '.gitignore');
  git(checkout, 'commit', '-m', 'base');
  const base = tip(checkout, 'HEAD');
  git(checkout, 'remote', 'add', 'origin', remote);
  git(checkout, 'push', 'origin', 'main');
  git(checkout, 'worktree', 'add', '-b', branch, worktree);
  writeFileSync(join(worktree, 'feature.txt'), 'landed');
  git(worktree, 'add', 'feature.txt');
  git(worktree, 'commit', '-m', 'feature');
  const checked = tip(worktree, 'HEAD');
  git(worktree, 'push', '-u', 'origin', branch);
  return {
    checkout, worktree, remote, env, git, tip, hook, base, checked,
    land({ status = 0, retry = false, cwd = worktree, name = branch, sha = checked } = {}) {
      const args = retry ? ['--cleanup-only', name, sha] : [name, sha];
      return run(process.execPath, [script, ...args], cwd, status);
    },
  };
}

test('lands and removes only the source refs, keeping the worktree files', (t) => {
  const f = fixture(t);
  writeFileSync(join(f.worktree, 'build.log'), 'keep me');
  f.land();
  assert.equal(f.tip(f.remote, 'refs/heads/main'), f.checked);
  assert.equal(f.tip(f.remote, ref), '');
  assert.equal(f.tip(f.checkout, ref), '');
  assert.equal(f.tip(f.worktree, 'HEAD'), f.checked);
  assert.equal(readFileSync(join(f.worktree, 'feature.txt'), 'utf8'), 'landed');
  assert.equal(readFileSync(join(f.worktree, 'build.log'), 'utf8'), 'keep me');
  f.land({ retry: true });
});

test('rejects main as a source and skips cleanup when the checked commit has not landed', (t) => {
  const f = fixture(t);
  f.land({ status: 1, retry: true, name: 'MAIN', sha: f.base });
  assert.match(f.land({ status: 1, retry: true }).stderr, /missing from origin\/main/);
  assert.equal(f.tip(f.remote, 'refs/heads/main'), f.base);
  assert.equal(f.tip(f.remote, ref), f.checked);
  assert.equal(f.tip(f.checkout, ref), f.checked);
});

test('a rejected main push leaves both branches in place', (t) => {
  const f = fixture(t);
  f.hook('pre-receive', 'exit 1');
  const result = f.land({ status: 1 });
  assert.doesNotMatch(result.stdout, /Landed/);
  assert.equal(f.tip(f.remote, 'refs/heads/main'), f.base);
  assert.equal(f.tip(f.remote, ref), f.checked);
  assert.equal(f.tip(f.checkout, ref), f.checked);
});

test('the source branch on GitHub must match the checked commit before landing', (t) => {
  const f = fixture(t);
  f.git(f.remote, 'update-ref', ref, f.base);
  assert.match(f.land({ status: 1 }).stderr, /both match the checked commit/);
  assert.equal(f.tip(f.remote, 'refs/heads/main'), f.base);
  assert.equal(f.tip(f.remote, ref), f.base);
  assert.equal(f.tip(f.checkout, ref), f.checked);
});

test('the lease preserves a remote branch that advances after the main push', (t) => {
  const f = fixture(t);
  const tree = f.git(f.worktree, 'rev-parse', 'HEAD^{tree}');
  const advanced = f.git(f.worktree, 'commit-tree', tree, '-p', f.checked, '-m', 'concurrent work');
  f.git(f.worktree, 'push', 'origin', `${advanced}:refs/heads/other`);
  f.env.ASTROLABE_TEST_ADVANCED = advanced;
  f.hook('post-receive', `while read old new ref; do
  if test "$ref" = refs/heads/main; then
    git update-ref ${ref} "$ASTROLABE_TEST_ADVANCED"
  fi
done`);

  const result = f.land({ status: 1 });
  assert.match(result.stdout, /Landed on origin\/main/);
  assert.match(result.stderr, /GitHub cleanup pending/);
  assert.equal(f.tip(f.remote, ref), advanced);
  assert.equal(f.tip(f.remote, 'refs/heads/other'), advanced);
  assert.equal(f.tip(f.remote, 'refs/heads/main'), f.checked);
});

test('a dirty worktree keeps its branch until a retry can detach it', (t) => {
  const f = fixture(t);
  const dirty = join(f.worktree, 'local.txt');
  writeFileSync(dirty, 'in progress');
  const result = f.land({ status: 1 });
  assert.match(result.stderr, /Local cleanup pending.*uncommitted files/);
  assert.equal(f.tip(f.remote, ref), '');
  assert.equal(f.tip(f.checkout, ref), f.checked);
  assert.equal(readFileSync(dirty, 'utf8'), 'in progress');
  rmSync(dirty);
  f.land({ retry: true });
  assert.equal(f.tip(f.checkout, ref), '');
});

test('a retry from another checkout leaves an occupied branch in place', (t) => {
  const f = fixture(t);
  f.git(f.worktree, 'push', 'origin', 'HEAD:main');
  f.git(f.checkout, 'merge', '--ff-only', 'origin/main');
  const result = f.land({ status: 1, retry: true, cwd: f.checkout });
  assert.match(result.stderr, /Local cleanup pending/);
  assert.equal(f.tip(f.checkout, ref), f.checked);
  assert.equal(f.git(f.worktree, 'symbolic-ref', 'HEAD'), ref);
  f.land({ retry: true });
  assert.equal(f.tip(f.checkout, ref), '');
});

test('a remote deletion failure can be retried without pushing main again', (t) => {
  const f = fixture(t);
  f.git(f.remote, 'config', 'receive.denyDeletes', 'true');
  const result = f.land({ status: 1 });
  assert.match(result.stdout, /Landed on origin\/main/);
  assert.match(result.stderr, /GitHub cleanup pending/);
  assert.equal(f.tip(f.remote, ref), f.checked);
  assert.equal(f.tip(f.checkout, ref), '');
  f.git(f.remote, 'config', 'receive.denyDeletes', 'false');
  const tree = f.git(f.worktree, 'rev-parse', 'HEAD^{tree}');
  const advanced = f.git(f.worktree, 'commit-tree', tree, '-p', f.checked, '-m', 'another landing');
  f.git(f.worktree, 'push', 'origin', `${advanced}:refs/heads/main`);
  f.hook('pre-receive', `read old new ref
test "$ref" = ${ref}`);
  f.land({ retry: true });
  assert.equal(f.tip(f.remote, 'refs/heads/main'), advanced);
  assert.equal(f.tip(f.remote, ref), '');
});
