# Preparing a release

Releases contain prebuilt programs, platform-selecting installers, and the plugin at the same version. The preparation workflow builds artifacts for review. It has read-only repository permissions and never creates a release or pushes a tag.

## Build the candidate

1. Align the version in the workspace `Cargo.toml`, `Cargo.lock`, and plugin manifest. Keep installation examples on that release tag. Run `scripts/check` before committing.
2. Run **Prepare release** on the exact reviewed branch revision. It also runs when release-related files are pushed, so a new workflow can be checked before it lands on the default branch. It checks and builds on Apple silicon and Intel macOS, x64 Windows, and x64 Linux, then exercises each native installer through a temporary HTTP mirror. Rust commands are blocked during installer tests. These tests use disposable notes and install folders, not the runner's real app configuration.
3. Download `astrolabe-release-for-review` from the successful run. Check its `release-files.json` inventory and `SHA256SUMS`. All four archives must be present; a locally generated single-target preview is not a complete release.

The pinned cargo-dist tool is downloaded by `scripts/setup-dist.py`, with platform-specific SHA-256 hashes checked before extraction. Python 3.13 and Node.js are development dependencies only. End users need neither.

For a local preview, use the executable path printed by the setup script in place of `dist` below:

```sh
python3 scripts/setup-dist.py /tmp/astrolabe-release-tools
dist build --artifacts=local --target aarch64-apple-darwin
dist build --artifacts=global
python3 scripts/package-release.py --target aarch64-apple-darwin
python3 scripts/smoke-release.py --target aarch64-apple-darwin
```

Choose your native target from `Cargo.toml`. The preview's other platforms are deliberately rejected. The release assembly runs `package-release.py` without `--target`, after collecting every archive.

## What ships

- One `.tar.gz` archive for each macOS architecture and Linux, and a Windows `.zip`, each with the program and license.
- `install.sh` and `install.ps1`, which embed the expected archive hashes, stage the download, and run the staged program's `install` command.
- `astrolabe-plugin.zip`, with the plugin manifest, hooks, skill, and license.
- `SHA256SUMS` and `release-files.json`.

Do not publish cargo-dist's unwrapped `astrolabe-installer.*` files. Those bypass Astrolabe's adoption checks, backups, and compatibility copy. The wrapper generator checks its integration points and refuses an unexpected cargo-dist template; review those changes when updating cargo-dist.

The wrappers preserve cargo-dist's license header. They require hash verification before archive extraction; the shell wrapper supports both `sha256sum` and macOS `shasum`. They stage into a temporary directory using unmanaged mode, so cargo-dist does not change shell profiles, registry PATH, or its own install receipt. The final installer writes its receipt and backups beside the program. It never edits notes, trust settings, or app configuration.

## Review before publication

1. Require green checks and native installer smoke tests on the exact revision for every advertised target. Verify both command names report the release version and failed downloads preserve the previous program.
2. Test the downloaded archives under normal OS protections. The initial pipeline does not sign or notarize executables. Resolve any Gatekeeper, SmartScreen, script-policy, or prerequisite failures before advertising ordinary-user support; do not solve these by disabling OS protections in the installer.
3. Test the hosted marketplace from the candidate tag in each supported app, including migration from a local marketplace, removal and re-registration at a new tag, hook review, and a fresh session's guard proof. Manifest validation alone does not establish host support. Use disposable app profiles or explicitly approved machines.
4. Review the installer, archive hashes, versioned catalogs, release notes, and rollback steps. Obtain authorization to push the candidate and run remote CI when needed; obtain authorization to publish the final release. A local preview does not authorize either action.

Publication is manual after review: create the version tag on the tested revision, publish a release for that tag, and upload exactly the files in `release-files.json`. Check that all documented asset URLs work before directing users to them. Avoid publishing a tag that points at an untested revision or rebuilding different bytes after review. No package-manager or public-directory submission is part of this workflow.

## Updates and rollback

The program and plugin share a version, but the installer does not modify app configuration. Users update the program, replace the marketplace registration with the new tag, install the matching plugin, review changed hooks, and start a fresh session. The compatibility copy keeps old trial-named hooks callable during this transition; it is not permission to run arbitrary older plugins with newer executables indefinitely.

The install receipt records a format version, release version, and binary hash. Future updates refuse a managed executable whose bytes no longer match. Adopting an unreceipted installation requires `--migrate-trial` and checks both existing programs with `--version`. Symlinks and unrelated files are refused. A successful update retains old files in the printed `.astrolabe-backup-*` folder. Notes and the machine state remain outside this transaction.

For a normal rollback, rerun the previous release's installer and register its matching plugin. For the first trial migration, restore the backed-up executable names and previous plugin checkout instead. Keep the backup until the fresh-session check succeeds.

## Recover an interrupted installation

Close Astrolabe sessions and its board before recovery. If `.astrolabe-install-pending` exists in the install folder, read it: it names the unique backup folder and which files existed before the attempt. The installer refuses another attempt until this is resolved.

The managed files are `astrolabe`, `astrolabe-trial`, and `.astrolabe-install.json` (the executable names end in `.exe` on Windows). The backup's top level contains originals that were moved; its `new` folder contains any replacements not yet moved. Do not restore files from `new` as originals.

For each managed file, restore the top-level backup copy when present. If the marker says the file originally existed and there is no backup copy, its original was never moved; leave the destination in place. If the marker says it did not exist, remove only that managed destination file if the failed attempt created it. Once all originals are restored, remove only `.astrolabe-install-pending` and retry. Keep the backup folder until recovery and the fresh-session check succeed. Do not delete notes or the machine-state folder.

Ordinary replacement failures attempt this rollback automatically. If rollback itself fails, the error names the recovery files instead of claiming the old installation was restored.

## Marketplace distribution

The repository root contains catalogs for hosted installation; `plugin/` retains the local catalogs for developers. Both point to the same plugin. Use a version tag in the marketplace URL so the app installs the plugin matching the executable.

The [OpenAI packaging guide](https://developers.openai.com/plugins/build/plugins) supports custom marketplaces and manually installed desktop lifecycle hooks, but excludes lifecycle hooks from its public directory. Astrolabe therefore uses the custom marketplace route.

The [Claude publishing guide](https://code.claude.com/docs/en/plugins/publish) documents hosted marketplaces and directory submission. Directory submission is separate from an official-marketplace listing and does not guarantee acceptance. See the [draft listing](claude-distribution.md) when preparing a submission. A listing on several surfaces does not prove that Astrolabe's required hooks and local program work on each one.
