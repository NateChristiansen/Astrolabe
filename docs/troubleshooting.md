# Troubleshooting

Find the symptom below. For first-time setup, follow the [installation guide](install.md).

## The installed program can't be found

Repeat the installation and version check in [step 1](install.md#1-download-and-install-the-program). Use the full installed path, including `&` before the quoted path in PowerShell. Hooks expect the `astrolabe` filename. If an older plugin still calls `astrolabe-trial`, [update the plugin](install.md#update-astrolabe) rather than renaming files by hand.

Commands such as `astrolabe effort save` are instructions the **agent** runs through the plugin. They aren't terminal setup commands.

## The installer refuses an existing file

The installer replaces only a program it installed, as recorded in its receipt. An unreceipted `astrolabe`, a symlink, or a managed file whose bytes changed is left alone. An `astrolabe-trial` file that isn't the installer's own copy is also left in place and named in the output; nothing calls it. Check what is installed before moving it aside; don't remove a conflict blindly. The installer prints the path it refused.

On Windows, close Astrolabe sessions and its board process if a file is locked, then retry. The installer does not stop processes itself. If it reports an interrupted installation, follow [recovery](releases.md#recover-an-interrupted-installation) using the named backup and marker.

## Update targets a repo build or warns about PATH

Newer updaters verify the installed copy before downloading and leave repo build outputs alone. A build-copy invocation uses the verified user-local installation; an installed copy in a custom folder keeps its location. If no verified installation exists, install Astrolabe first. Do not delete the build output or manufacture an installation receipt to get past a refusal.

With an older updater, run the installed copy directly: `~/.local/bin/astrolabe update` on macOS/Linux, or `& "$HOME/.local/bin/astrolabe.exe" update` in PowerShell. If that reports a missing, damaged or interrupted installation, use the recovery guidance above.

Older shell installers may warn that `astrolabe` is shadowed immediately after staging a download in a temporary folder. That staging warning does not mean installation failed; read the final result. New installers check PATH against the final destination instead and name any conflicting command. Adjust PATH or use the installed program's full path when a real conflict is reported.

## The download or verification fails

Check that the selected tag exists on the [releases page](https://github.com/NateChristiansen/Astrolabe/releases). An unpublished candidate has no working download URLs yet. A checksum mismatch stops installation; download the installer again from that release rather than bypassing verification. The shell installer needs `sha256sum` or `shasum`. Unsupported architectures and older Linux glibc versions require a supported system or a source build.

## Notes initialization fails

If Git reports a missing name or email, configure your Git commit identity and retry. If the folder already contains notes, use `repo add agent` instead. If Astrolabe says a notes checkout is already registered, you can replace that registration with `repo add agent` pointing at the intended checkout.

## The agent doesn't receive Astrolabe context

Run the installed program with `status` in your terminal. If it says off, complete [step 4](install.md#4-turn-it-on). Check that the plugin is installed and enabled in the app, review hook trust in Codex, and start a fresh session. Ask the agent to run `astrolabe activate --status`; machine status alone doesn't prove that the app runs the hooks.

## The app stops running the hooks partway through a session

The session loaded an effort and saved earlier, then its commands start printing "the plugin's hook didn't take this command": a save says **NOT recorded**, and `astrolabe activate --status` says **No effort loaded**. The effort is still bound in Astrolabe's local state; the app has stopped passing that session's commands to the plugin, so nothing is written and the guard isn't checking them either. This has been seen after a session was resumed with its earlier context summarized. The cause isn't known.

Check that the command ran alone, exactly as written, and not from a subagent. Run the installed program with `status` in your terminal to confirm the machine switch is still on, and check that the plugin is still enabled and its hooks still trusted in the app. If those hold and `astrolabe activate --status` still isn't taken, retrying won't help: start a fresh session and load the effort again. Work left in the project folder is still there; anything the old session hadn't saved needs saving from the new one.

## The agent doesn't recognize my project

[Register the project](usage.md#add-a-project) and start a fresh session there. Project repos without a remote need registration on each machine.

## An effort won't load

Check the property named in the error at the top of the effort note. Its repo must match the current project, unless you are working from the notes folder. [Involvement and authority](usage.md#choose-how-the-agent-works) must have valid values. New efforts get defaults; you don't need to create these properties by hand during installation.

## Notes haven't reached another machine

An open board pulls other machines' saves about once a minute; a save that still hasn't shown up may not have been pushed from the machine that made it. Ask the agent to check whether your notes have synced, or use the [notes sync commands](commands.md#board-and-notes). If a save reports that the effort changed elsewhere, the agent needs to read the returned state and reconcile its work before saving again.

## A sandboxed session can't write a handoff or commit

Follow the [Codex workspace sandbox setup](codex.md) for the notes write grant, command approvals and dispatcher fallback. Briefs and reports belong under `.handoffs/`; a writable notes checkout doesn't make project Git metadata writable.

Lesson closing and notes sync run through Astrolabe's hook and need no extra writable folder. A GitHub read may separately need host approval.


[Installation](install.md) · [Using Astrolabe](usage.md) · [Command reference](commands.md)
