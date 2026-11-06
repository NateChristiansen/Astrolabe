# Troubleshooting

Find the symptom below. For first-time setup, follow the [installation guide](../plugin/README.md).

## The installed program can't be found

Repeat the installation and version check in [step 1](../plugin/README.md#1-download-and-install-the-program). Use the full installed path, including `&` before the quoted path in PowerShell. New hooks expect the `astrolabe` filename. If an older plugin still calls `astrolabe-trial`, complete the [migration](../plugin/README.md#migrate-an-existing-installation) rather than renaming files by hand.

Commands such as `astrolabe effort save` are instructions the **agent** runs through the plugin. They aren't terminal setup commands.

## The installer refuses an existing file

An unreceipted Astrolabe build needs the explicit migration option. An unrelated executable, symlink, or managed file whose bytes changed is left alone. Check what is installed before moving it aside; don't remove a conflict blindly. The installer prints the path it refused.

On Windows, close Astrolabe sessions and its board process if a file is locked, then retry. The installer does not stop processes itself. If it reports an interrupted installation, follow [recovery](releases.md#recover-an-interrupted-installation) using the named backup and marker.

## The download or verification fails

Check that the selected tag exists on the [releases page](https://github.com/NateChristiansen/Astrolabe/releases). An unpublished candidate has no working download URLs yet. A checksum mismatch stops installation; download the installer again from that release rather than bypassing verification. The shell installer needs `sha256sum` or `shasum`. Unsupported architectures and older Linux glibc versions require a supported system or a source build.

## Notes initialization fails

If Git reports a missing name or email, configure your Git commit identity and retry. If the folder already contains notes, use `repo add agent` instead. If Astrolabe says a notes checkout is already registered, you can replace that registration with `repo add agent` pointing at the intended checkout.

## The agent doesn't receive Astrolabe context

Run the installed program with `status` in your terminal. If it says off, complete [step 4](../plugin/README.md#4-turn-it-on). Check that the plugin is installed and enabled in the app, review hook trust in Codex, and start a fresh session. Ask the agent to run `astrolabe activate --status`; machine status alone doesn't prove that the app runs the hooks.

## The agent doesn't recognize my project

[Register the project](usage.md#add-a-project) and start a fresh session there. Project repos without a remote need registration on each machine.

## An effort won't load

Check the property named in the error at the top of the effort note. Its repo must match the current project, unless you are working from the notes folder. [Involvement and authority](usage.md#choose-how-the-agent-works) must have valid values. New efforts get defaults; you don't need to create these properties by hand during installation.

## Notes haven't reached another machine

Use the [notes sync checks](usage.md#share-notes-between-machines). If a save reports that the effort changed elsewhere, the agent needs to read the returned state and reconcile its work before saving again.


[Installation](../plugin/README.md) · [Using Astrolabe](usage.md) · [Command reference](commands.md)
