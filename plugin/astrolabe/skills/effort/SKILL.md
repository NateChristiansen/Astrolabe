---
name: effort
description: Set this session's Astrolabe effort when the owner's message names one by number, as in "pick up 70", "work on effort 70", "switch to effort 70" or "/astrolabe:effort 70". Only when the owner names the number in this message; never on a guess, and never to choose between efforts yourself.
argument-hint: <effort number>
allowed-tools: Bash(astrolabe activate:*)
---

Set this session's Astrolabe effort to the number the owner named: $ARGUMENTS, or the number in their message. If they named no number, ask which effort, because picking one is their call.

1. Run this shell command alone, with the number filled in and nothing before or after it, because the plugin's hook binds only a plain command to this session: `astrolabe activate --effort <n>`
2. If it doesn't begin "Astrolabe is on", tell the owner what it printed and stop. Don't retry another way, because a workaround would skip the binding, and claim no protection.
3. Otherwise, take what it printed as this session's context. Then run the probe command it names, alone. The hook denying it is the expected result.
4. Run `astrolabe activate --status`. Tell the owner, in a line or two, the effort and what it says about the guard.
