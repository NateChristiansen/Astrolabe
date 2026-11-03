---
name: effort
description: Switch this session on for one Astrolabe effort. Only when the user asks by name, as in "/astrolabe:effort 70" or "$effort 70".
argument-hint: <effort number>
disable-model-invocation: true
allowed-tools: Bash(astrolabe activate:*)
---

Switch this session on for Astrolabe effort $ARGUMENTS. On Codex, the number is in the user's message.

1. Run this shell command alone, with the number filled in and nothing before or after it, because the plugin's hook binds only a plain command to this session: `astrolabe activate --effort <n>`
2. If it says "NOT switched on", tell the user that line and stop. Don't retry another way, because a workaround would skip the binding, and claim no protection.
3. Otherwise, take what it printed as this session's context. Then run the probe command it names, alone. The hook denying it is the expected result.
4. Run `astrolabe activate --status`. Tell the user, in a line or two, the effort and what it says about the guard.
