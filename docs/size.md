# Size

Measured on macOS at the end of phase 1a. Lines are every line in the file, from `wc -l`, blanks and comments included. No budgets yet.

| What | Lines |
| --- | --- |
| Rust source, total, including phase 0 | 8,478 |
| Core, `crates/astrolabe/src` | 7,856 |
| Protocol, `crates/protocol/src` | 339 |
| Desktop, `app/src-tauri`, of which 200 are the test control, in no normal build | 283 |
| Rust tests and examples | 11,123 |
| Page, styles and scripts | 1,555 |

Largest modules in the core: `acp` 1,214, `shell` 637, `store` 636, `guard` 600, `doctor` 573, `engine` 494, `driver/codex` 411, `layer` 317, `trust` 280, `hook` 278, `model` 260, `worktree` 253, `launch` 229, `protect` 216, `bridge` 178, `driver/claude` 168. The first command below lists them all.

269 tests take about 34 seconds.

## Rerun

From the repo root:

```bash
count() { git ls-files "$@" | grep -v '^app/src/protocol/' | xargs wc -l | tail -1; }
for dir in crates/astrolabe/src crates/protocol/src app/src-tauri/src; do wc -l $(git ls-files "$dir" | grep '\.rs$') | sort -n; done
count 'crates/*/src/*.rs' 'crates/*/src/**/*.rs' 'app/src-tauri/src/*.rs' 'app/src-tauri/build.rs'
count 'crates/*/tests/*.rs' 'crates/*/tests/**/*.rs' 'crates/*/examples/*.rs'
count 'app/src/*.ts' 'app/src/*.tsx' 'app/src/*.css' 'app/src-tauri/src/*.js' 'app/scripts/*.mjs' 'scripts/*'
time cargo test --locked
```

`app/src/protocol/` is generated, so it isn't counted. The test time is for all tests, no agent login needed, on an Apple silicon Mac with the code already built.
