# Product Thesis

## One Line

Yggterm is a remote-first workspace for long-lived AI terminal work: a place where sessions, machines, notes, and recovery state stay organized as one system instead of collapsing into forgotten tabs, shell history, and broken desktop snapshots.

## The User

The narrow first user is an engineer who:

- runs Codex or similar agent sessions across multiple machines
- keeps several long-lived terminal contexts open for days
- works over SSH constantly
- relies on laptop sleep, reconnect, and crash recovery being safe
- feels real cognitive pain from forgetting which terminal does what

This is not a generic “developer terminal” pitch.

It is for people whose terminal work already behaves like a browser with 120 tabs, except the tabs are machines, shells, AI sessions, deployment work, debugging trails, and half-finished thinking.

## The Pain

Today this work is split across:

- terminal windows
- SSH tabs
- tmux or screen sessions
- AI transcripts
- scratch notes
- half-remembered shell history
- whatever state happened to survive the last crash or reboot

That fragmentation destroys flow.

The biggest loss is not just process state. It is the mental map. You stop knowing:

- what is running
- on which machine
- for which project
- why that session exists
- what the last meaningful conclusion was

Then the laptop sleeps, the client crashes, or the machine reboots, and the whole desktop-shaped memory system is gone.

## The Wedge

The wedge is simple:

**Zen Browser for AI terminal work, with server-owned session persistence.**

That means:

- a vertical, restorable workspace tree
- sessions grouped by project and machine
- preview and terminal as two lenses on the same work item
- remote sessions that survive client loss
- notes and future structured surfaces living beside the terminal, not somewhere else

The first truly valuable promise is not “pretty tabs.”

It is:

**“You can run real work for days across machines, close the laptop, come back, and still know what everything is.”**

## Why This Matters

AI coding made terminals more important, not less.

A single developer can now keep many more active threads alive at once:

- one Codex session exploring
- one patching
- one running on a remote box
- one checking logs
- one shell doing real infra work
- one scratch surface capturing decisions

That is powerful. It is also chaos without a system.

Yggterm is that system.

## What Makes It Different

This project is not trying to be “an editor with a terminal panel.”

The terminal is the center. The rest of the product exists to make terminal work:

- understandable
- recoverable
- searchable
- classifiable
- restart-safe
- remote-safe

The atomic unit is not just a shell tab. It is a small work cluster:

- a project folder
- a Codex session
- a generic terminal
- a paper or scratch surface
- metadata that explains what is going on

That is much closer to how real messy work actually happens.

## The Moat

The moat is not closed source code.

The moat is:

- taste in workflow design
- deep dogfooding
- owning the full remote-session problem
- recovery and persistence that people come to rely on
- a public identity around solving this category well

Open source is part of the strategy, not a sacrifice.

If Yggterm becomes the reference implementation for “serious AI terminal workspaces,” recognition compounds:

- GitHub attention
- user trust
- contributor pull
- adjacent project credibility

That is the financial strategy in plain English.

## What To Build Next

The next product work should stay very focused:

1. Make remote session lifecycle bulletproof.
2. Make preview and terminal feel like the same live object.
3. Make search excellent across tree, preview, and active terminals.
4. Make sluggishness unacceptable and visible through telemetry.
5. Make session metadata generation useful enough that people trust it.
6. Make the workspace feel calm even when the underlying work is messy.

## What To Ignore For Now

Do not dilute the wedge with generic “developer platform” ambitions too early.

Ignore, for now:

- broad IDE aspirations
- fancy automation that hides what the shell is doing
- too many new workspace object types
- trying to serve every kind of developer equally

The first market is narrower and stronger:

people doing multi-machine, AI-heavy terminal work who are tired of losing their mental map.

## The Test

The product is winning if a user says:

“Before this, my AI terminal work felt like scattered survival. Now I can keep long-running work alive, organized, and understandable.” 
