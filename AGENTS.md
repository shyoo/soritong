# soritong — agent guide

How to work in this codebase. Read [`HANDOFF.md`](HANDOFF.md) first: it is the only file that
carries current state.

## Before you start

- Detected stack: not detected.
- TODO: anything an agent cannot infer from the code — a service that must be running, a credential
  that must be present, a directory that is generated and must not be hand-edited.

## Checks

Run these and make them pass before you report done:

- _None declared yet._ Add them in Project → Settings → Verification.

## When you commit

- Work on a branch, never directly on `main`.
- One coherent commit per task where it is safe to squash; explain *why*, not *what*.
- ⛔ Never commit generated output, dependency directories, credentials, or anything gitignored.
- Update [`HANDOFF.md`](HANDOFF.md) in the same commit: what changed, and what is next.

## House rules

- TODO: the conventions this project actually holds — naming, error handling, test placement.
- TODO: the things that have bitten somebody here before. An entry earns its place by having cost
  a real debugging session; delete it when the pitfall becomes impossible.
