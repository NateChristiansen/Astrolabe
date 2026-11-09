# Codex sessions with a workspace sandbox

Astrolabe's effort settings describe what an agent may do. Codex's sandbox separately controls which files a command can write. An effort with edit authority still needs host permission for a local Git commit.

## Start a session

Create the branch worktree before starting the worker. Start Codex in that worktree and grant the notes checkout as an additional writable directory:

```sh
codex --sandbox workspace-write --ask-for-approval on-request --cd /path/to/worktree --add-dir /path/to/notes
```

Replace both paths with your own. The same flags work in PowerShell with Windows paths. Review and trust the installed Astrolabe hooks as described in [installation](install.md#2-install-the-plugin-in-your-agent-app).

For an unattended CLI session whose eligible approval requests should go through automatic review:

```sh
codex exec --approve-for-me --cd /path/to/worktree --add-dir /path/to/notes "<task>"
```

`--approve-for-me` selects workspace-write and automatic approval review; don't combine it with `--sandbox`. Run it with closed stdin when dispatching from a shell tool (`< /dev/null` in Bash or zsh). Automatic review may deny an action and uses additional model calls. It does not extend the effort's authority.

When resuming, invoke Codex from the same worktree, with the same notes grant and approval settings. A different working directory can leave the worker's files outside its writable workspace.

## Briefs, reports and Git

Write briefs and reports at the requested location under the notes' `.handoffs/<initiative>/` folder. The notes grant covers these files. Use Astrolabe commands for effort records and the journal; their hook already saves and syncs notes without a writable notes grant. Workers still save only their own effort and leave the journal and closing to their dispatcher. The dispatcher also commits and syncs directly written handoff files in the notes checkout, with host approval when needed.

Git metadata stays protected even in a writable checkout. For an authorized branch, commit or local integration, request host approval for that command when needed. Making the notes folder writable doesn't grant project Git access. Adding `.git` as a writable root isn't the workflow.

If approvals are unavailable or denied, finish the allowed edits and checks. Report the worktree, branch, changed files, checks and the exact Git step left for the dispatcher. Save built and checked work in review waiting for that step; use blocked when the restriction prevents the build or checks themselves. Don't mark a worker done or claim a commit or integration that didn't happen. An ordinary autonomous effort can finish only when its required work is actually complete.

If the handoff folder isn't writable, return the report in chat with its intended destination for the dispatcher to file. Don't silently move it to the notes root. The dispatcher must have permission to perform any remaining step; dispatch itself doesn't authorize a push or landing.

## Check the setup

Before dispatching real work, use a disposable repo and notes checkout. Confirm that a session can write a brief and report under `.handoffs`, commit an authorized change through host approval, and integrate a prepared worker branch. Check the files and Git history from outside the session. A run with approvals disabled tests the fallback, not autonomous Git completion.

The unattended workflow was checked with Codex CLI 0.166.0 on macOS. Other hosts may apply additional restrictions. See the official [CLI reference](https://learn.chatgpt.com/docs/cli/reference) and [approvals and security guide](https://learn.chatgpt.com/docs/agent-approvals-security).
