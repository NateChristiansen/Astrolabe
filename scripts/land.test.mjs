import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import {
  chmodSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('./land.mjs', import.meta.url));
const branch = 'feature/topic';
const ref = `refs/heads/${branch}`;
// Windows can't remove the folder the shell is in, so the worktree goes on a
// retry from the main checkout there.
const windows = process.platform === 'win32';

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
    retire({ status = 0, cwd = checkout, folder = worktree } = {}) {
      return run(process.execPath, [script, '--retire', folder], cwd, status);
    },
    land({ status = 0, retry = false, cwd = worktree, name = branch, sha = checked } = {}) {
      const args = retry ? ['--cleanup-only', name, sha] : [name, sha];
      return run(process.execPath, [script, ...args], cwd, status);
    },
    finish(options = {}) {
      const result = this.land({ ...options, status: windows ? 1 : 0 });
      if (windows) {
        assert.match(result.stderr, /Worktree cleanup pending/);
        this.land({ retry: true, cwd: checkout });
      }
      assert.equal(existsSync(worktree), false);
      return result;
    },
  };
}

test('lands, removes the source refs and the finished worktree with its build output', (t) => {
  const f = fixture(t);
  writeFileSync(join(f.worktree, 'build.log'), 'build output');
  f.finish();
  assert.equal(f.tip(f.remote, 'refs/heads/main'), f.checked);
  assert.equal(f.tip(f.remote, ref), '');
  assert.equal(f.tip(f.checkout, ref), '');
  assert.equal(existsSync(f.worktree), false);
  assert.doesNotMatch(f.git(f.checkout, 'worktree', 'list'), /finished work/);
  f.land({ retry: true, cwd: f.checkout });
});

test('a locked worktree keeps its files after the branch is gone', (t) => {
  const f = fixture(t);
  f.git(f.checkout, 'worktree', 'lock', f.worktree);
  const result = f.land({ status: 1 });
  assert.match(result.stderr, /Worktree cleanup pending/);
  assert.equal(f.tip(f.checkout, ref), '');
  assert.equal(f.tip(f.worktree, 'HEAD'), f.checked);
  assert.equal(readFileSync(join(f.worktree, 'feature.txt'), 'utf8'), 'landed');
  f.git(f.checkout, 'worktree', 'unlock', f.worktree);
  assert.match(f.land({ retry: true, cwd: f.checkout }).stdout, /Worktree is gone/);
  assert.equal(existsSync(f.worktree), false);
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
  f.finish({ retry: true });
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
  f.finish({ retry: true });
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
  f.finish({ retry: true });
  assert.equal(f.tip(f.remote, 'refs/heads/main'), advanced);
  assert.equal(f.tip(f.remote, ref), '');
});

test('retiring removes a worktree that will not land and deletes its local branch', (t) => {
  const f = fixture(t);
  writeFileSync(join(f.worktree, 'build.log'), 'build output');
  const result = f.retire();
  assert.match(result.stdout, /Worktree is gone/);
  assert.match(result.stdout, new RegExp(`restore it with git branch ${branch} ${f.checked}`));
  assert.equal(existsSync(f.worktree), false);
  assert.equal(f.tip(f.checkout, ref), '');
  assert.equal(f.tip(f.remote, ref), f.checked, 'the branch on GitHub stays');
  assert.equal(f.tip(f.remote, 'refs/heads/main'), f.base, 'nothing lands');
});

test('retiring leaves a dirty, locked or current worktree alone', (t) => {
  const f = fixture(t);
  const dirty = join(f.worktree, 'local.txt');
  writeFileSync(dirty, 'in progress');
  assert.match(f.retire({ status: 1 }).stderr, /uncommitted files/);
  rmSync(dirty);
  f.git(f.checkout, 'worktree', 'lock', f.worktree);
  assert.match(f.retire({ status: 1 }).stderr, /locked/);
  f.git(f.checkout, 'worktree', 'unlock', f.worktree);
  assert.match(f.retire({ status: 1, cwd: f.worktree }).stderr, /Run retiring from/);
  assert.match(f.retire({ status: 1, folder: f.checkout }).stderr, /isn't a linked worktree/);
  assert.equal(f.tip(f.checkout, ref), f.checked);
  assert.equal(readFileSync(join(f.worktree, 'feature.txt'), 'utf8'), 'landed');
});

test('retiring a detached control checkout keeps every branch', (t) => {
  const f = fixture(t);
  const control = join(f.checkout, '..', 'control');
  f.git(f.checkout, 'worktree', 'add', '--detach', control, 'main');
  assert.match(f.retire({ folder: control }).stdout, /Worktree is gone/);
  assert.equal(existsSync(control), false);
  assert.equal(f.tip(f.checkout, ref), f.checked);
});

// Windows keeps a folder that any process has as its working directory, so git
// drops the worktree and its files but not the folder.
test('a folder a session still has open is named, and retiring clears it once closed',
  { skip: !windows }, async (t) => {
    const f = fixture(t);
    assert.match(f.land({ status: 1 }).stderr, /Worktree cleanup pending/);
    const holder = spawn(process.execPath, ['-e', 'setTimeout(() => {}, 60000)'], {
      cwd: f.worktree, stdio: 'ignore',
    });
    t.after(() => holder.kill());
    await new Promise((done) => setTimeout(done, 500));
    const result = f.land({ status: 1, retry: true, cwd: f.checkout });
    assert.match(result.stderr, /still on disk because something has it open/);
    assert.doesNotMatch(result.stderr, /Retry with --cleanup-only/);
    assert.deepEqual(readdirSync(f.worktree), []);
    holder.kill();
    await once(holder, 'exit');
    assert.match(f.retire().stdout, /Empty leftover folder is gone/);
    assert.equal(existsSync(f.worktree), false);
  });
