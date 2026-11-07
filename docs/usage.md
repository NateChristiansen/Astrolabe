# Using Astrolabe

Astrolabe keeps notes and records for you as your agents work. Your progress, decisions and preferences live together in your own notes repo.

## Start work

Open a session in your project and ask for work as usual:

> Fix the error when an upload fails.

The agent records the work in an **effort**. It keeps that note updated with progress, decisions and what comes next, and adds a short account of what changed to your journal.

A larger piece of work that you plan as several efforts becomes an **initiative**: the agent files its page and all of its efforts in one step, and the board groups them under its name. You can run those efforts one at a time, or ask an agent to orchestrate them through workers.

## Choose how the agent works

Tell the agent how involved you want to be:

- **Approve a plan:** review the approach before it changes files. This is the default.
- **Check each step:** review each part before it moves on.
- **Work autonomously:** let it finish and report back.

You also choose its authority: **read** to investigate and report, **edit** to make changes and local commits, or **push** to share its branch. New efforts default to edit.

Authority applies to project changes and pushes; notes save and sync on their own at every authority level.

For example:

> Fix the broken links. Work autonomously and keep the changes local.

The agent records these choices with the effort, so they carry over when you resume it.

## See where things stand

Astrolabe starts the board automatically. Open your [local board](http://localhost:4747/) or ask an agent to open it for you. The address above is the default.

The board shows active work, what needs your attention and what you could pick up next. Open an effort to read its progress and decisions, or browse your other notes in the tree alongside it. Ideas use a full-width grid that adapts to your screen, with priority first and the latest updates next. When nothing is in motion, the overview moves straight to the work you could pick up.

The journal gives you a short history of what changed. Both the board and journal update as agents record their work, including work recorded on your other machines: an open board pulls their saves about once a minute.

## Pick up an effort

Find the effort's number on the board, then tell an agent:

> Pick up 12.

It reads where the work stands and what's next, along with your project and machine notes. You can resume in a fresh session or on another machine where you've set up Astrolabe.

Use a session in the project to work on its code. From your notes folder, you can discuss any effort or plan new work; just tell the agent which project it's for.

## Add a project

Open an agent session in the project and ask:

> Add this project to Astrolabe as “my-app”.

The agent registers the repo so Astrolabe can connect it with your efforts. Its project note is for context you supply, such as your goals or project-specific preferences.

Keep conventions, build commands and implementation details in the repo's own files. Agents should check those files as they work; a project note can link to them.

## Update your preferences

Ask an agent in your notes folder to help update how you want it to work:

> Keep review comments short, and explain why a change matters. Add that to my preferences.

Your notes hold your working preferences, writing voice, project goals and useful facts about your machines. You can review and edit them as Markdown too. New sessions receive your updated preferences; tell an existing session when you want it to use a change immediately.

## Use another machine

Connect your notes to a private Git remote so Astrolabe can sync them between machines. An agent in your notes folder can help set that up and check that your latest work has synced.

On the other machine, follow [installation](install.md), choosing **existing notes**. Open your project and ask the agent to pick up the effort you want to continue.

[Installation](install.md) · [Troubleshooting](troubleshooting.md) · [Command reference](commands.md) · [Project overview](../README.md)
