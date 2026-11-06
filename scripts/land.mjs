import { spawnSync } from 'node:child_process';

function git(args, allowedStatuses = [0]) {
  const result = spawnSync('git', ['--no-replace-objects', ...args], { encoding: 'utf8' });
  if (result.error) throw result.error;
  if (!allowedStatuses.includes(result.status)) {
    const detail = result.stderr.trim() || result.stdout.trim() || result.signal;
    throw new Error(`git ${args[0]} failed${detail ? `\n${detail}` : ''}`);
  }
  return { status: result.status, output: result.stdout.trim() };
}

function remoteTip(ref) {
  const result = git(['ls-remote', '--exit-code', '--refs', 'origin', ref], [0, 2]);
  for (const line of result.output.split('\n')) {
    const [oid, name] = line.split('\t');
    if (name === ref) return oid;
  }
  return '';
}

function isAncestor(commit, tip) {
  return git(['merge-base', '--is-ancestor', commit, tip], [0, 1]).status === 0;
}

function push(...args) {
  git(['-c', 'remote.origin.mirror=false', '-c', 'push.followTags=false',
    'push', '--recurse-submodules=no', ...args]);
}

function removeLocal(branch, ref, checked, landedMain) {
  const tip = git(['rev-parse', '--verify', '--quiet', ref], [0, 1]).output;
  if (!tip) return;
  if (tip !== checked) throw new Error('The local branch tip changed.');

  const current = git(['symbolic-ref', '--quiet', 'HEAD'], [0, 1]).output;
  if (current === ref) {
    if (git(['status', '--porcelain=v1', '--untracked-files=all', '--ignore-submodules=none']).output) {
      throw new Error('The branch worktree has uncommitted files.');
    }
    git(['switch', '--detach', '--no-overwrite-ignore', checked]);
  }

  // Git checks worktree occupancy and uses the branch upstream or HEAD to
  // decide if deletion is safe. Only use bases already on GitHub's main.
  const upstream = git(['for-each-ref', '--format=%(upstream)', ref]).output;
  const base = upstream && git(['rev-parse', '--verify', '--quiet', upstream], [0, 1]).output;
  if (!isAncestor(base || 'HEAD', landedMain)) {
    throw new Error('Retry from the finished worktree or an up-to-date main checkout.');
  }
  if (git(['rev-parse', '--verify', ref]).output !== checked) {
    throw new Error('The local branch tip changed.');
  }
  git(['branch', '-d', '--', branch]);
}

function worktrees() {
  return git(['worktree', 'list', '--porcelain']).output.split('\n\n').map((block) => {
    const entry = {};
    for (const line of block.split('\n')) {
      const [key, ...value] = line.split(' ');
      entry[key] = value.join(' ');
    }
    return entry;
  });
}

// Removes the finished worktree once its branch is gone: the linked worktree
// landing ran from, or, from the main checkout, the one detached at the
// checked commit. A locked or dirty worktree stays; ignored build output goes.
function removeWorktree(checked) {
  const [mainCheckout, ...linked] = worktrees();
  const gitDir = git(['rev-parse', '--path-format=absolute', '--git-dir']).output;
  const commonDir = git(['rev-parse', '--path-format=absolute', '--git-common-dir']).output;
  let target;
  if (gitDir !== commonDir) {
    if (git(['symbolic-ref', '--quiet', 'HEAD'], [0, 1]).output) {
      throw new Error('The worktree is on a branch.');
    }
    if (git(['rev-parse', 'HEAD']).output !== checked) {
      throw new Error('The worktree moved past the checked commit.');
    }
    if (process.platform === 'win32') {
      throw new Error(`Windows can't remove the folder a shell is in. Retry from ${mainCheckout.worktree}.`);
    }
    target = git(['rev-parse', '--show-toplevel']).output;
  } else {
    const finished = linked.filter((entry) => 'detached' in entry && entry.HEAD === checked);
    if (finished.length === 0) return '';
    if (finished.length > 1) throw new Error('More than one worktree is at the checked commit.');
    target = finished[0].worktree;
  }
  process.chdir(mainCheckout.worktree);
  git(['worktree', 'remove', target]);
  return mainCheckout.worktree;
}

function verifyLanded(main, checked) {
  const landedMain = remoteTip(main);
  if (!landedMain) throw new Error('Origin has no main branch.');
  if (git(['cat-file', '-e', landedMain], [0, 1, 128]).status !== 0) {
    git(['fetch', '--no-tags', '--no-prune', '--no-recurse-submodules', '--no-write-fetch-head',
      'origin', main]);
  }
  if (!isAncestor(checked, landedMain)) throw new Error('The checked commit is missing from origin/main.');
  return landedMain;
}

function land(args) {
  const cleanupOnly = args[0] === '--cleanup-only';
  const [branch, checked, extra] = cleanupOnly ? args.slice(1) : args;
  if (!branch || branch.startsWith('-') || branch.toLowerCase() === 'main'
    || !/^[a-f0-9]{40}$/.test(checked ?? '') || extra !== undefined) {
    throw new Error('Usage: node scripts/land.mjs [--cleanup-only] <branch> <full-checked-sha>');
  }

  const ref = `refs/heads/${branch}`;
  const main = 'refs/heads/main';
  git(['check-ref-format', ref]);
  const fetchUrl = git(['remote', 'get-url', 'origin']).output;
  const pushUrls = git(['remote', 'get-url', '--push', '--all', 'origin']).output;
  if (pushUrls !== fetchUrl) throw new Error('Origin must have one matching fetch and push URL.');
  if (git(['cat-file', '-t', checked]).output !== 'commit') {
    throw new Error('The checked SHA must name a commit.');
  }

  if (!cleanupOnly) {
    const current = git(['symbolic-ref', '--quiet', 'HEAD'], [0, 1]).output;
    if (current !== ref) throw new Error('Run landing from the finished branch worktree.');
    if (git(['rev-parse', '--verify', ref]).output !== checked || remoteTip(ref) !== checked) {
      throw new Error('The local and remote branch tips must both match the checked commit.');
    }
    push('origin', `${checked}:${main}`);
  }

  let landedMain;
  try {
    landedMain = verifyLanded(main, checked);
  } catch (error) {
    const state = cleanupOnly ? 'Cleanup skipped.' : 'Main push accepted, but cleanup could not verify the landing.';
    throw new Error(`${state} ${error.message}`);
  }
  console.log('Landed on origin/main.');

  const cleanup = [
    ['GitHub', () => {
      if (remoteTip(ref)) {
        // The lease applies to this deletion alone. It cannot force an update to main.
        push(`--force-with-lease=${ref}:${checked}`, 'origin', `:${ref}`);
      }
    }],
    ['Local', () => removeLocal(branch, ref, checked, landedMain)],
  ];
  let pending = false;
  for (const [where, remove] of cleanup) {
    try {
      remove();
      console.log(`${where} branch is gone.`);
    } catch (error) {
      pending = true;
      console.error(`${where} cleanup pending: ${error.message}`);
    }
  }
  if (!pending) {
    try {
      const mainCheckout = removeWorktree(checked);
      if (mainCheckout) console.log(`Worktree is gone. Continue from ${mainCheckout}.`);
    } catch (error) {
      pending = true;
      console.error(`Worktree cleanup pending: ${error.message}`);
    }
  }
  if (pending) {
    console.error('Retry with --cleanup-only and the same branch and checked SHA.');
    process.exitCode = 1;
  }
}

try {
  land(process.argv.slice(2));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
