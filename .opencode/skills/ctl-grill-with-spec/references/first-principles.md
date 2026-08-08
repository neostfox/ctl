# First Principles Reasoning (ctl-grill-with-spec)

On-demand depth for the grill station. The SKILL.md carries the interview loop
and the alignment-note contract; this file expands the First Principles
framework you reach for when the request is vague, the solution feels
over-engineered, or you are about to add complexity "because everyone does."
Supplementary — the grill stays correct without it.

First Principles is **placed** in grill / design clarification (see the workflow
core). It outputs artifacts (a restated problem, ranked assumptions, a minimum
viable experiment), never a verdict. Challenge inherited assumptions hardest:
*"are we doing this because the domain requires it, or because the existing
architecture/framework suggests it?"*

## Step 1 — Restate the problem

Strip implementation detail to one sentence that names the outcome, not the mechanism.

- Bad: "We need to add Redis caching to the user profile endpoint."
- Good: "User profile data takes too long to load."

If you cannot state it without naming a tool, you have not separated the problem
from a candidate solution.

## Step 2 — List fundamental truths

What is absolutely true (not opinion, not convention)?

| Category | Examples |
|---|---|
| Physical / system constraints | latency ≥ 0; disk I/O has limits; a single writer per file |
| Business rules | "users see only their own data" |
| Technical invariants | "the event ledger is append-only"; "the reducer is pure" |
| User needs | "the response returns within Y seconds" |

These become the floor: every candidate must satisfy all of them.

## Step 3 — Challenge assumptions

For each component of the current or proposed plan:

- **Fact or convention?** "We always use REST" — why? Trace it to a truth or strike it.
- **What if we removed this?** If nothing breaks, it is not load-bearing.
- **Solving the problem or a symptom?** Follow the causal chain to the root.
- **Who benefits from this complexity?** If "nobody can say", simplify.

Separate **observed facts** (you read/ran it — cite the source) from **declared
rules** (a spec/schema states it) from **assumptions** (you are carrying it
unconfirmed). The alignment note keeps these three apart; never promote an
assumption to a fact without evidence.

## Step 4 — Build up from truths

1. Start with the minimum viable mechanism satisfying every truth.
2. Add complexity **only** when a specific truth demands it.
3. Each addition must answer: "which truth requires this?"

If an addition cannot name its truth, it is convention — propose it as an option
to the user, not a default.

## Step 5 — Validate

- Does the solution solve the restated problem (Step 1), not a proxy?
- Which assumptions still need verification? List them as ranked **unknowns**.
- What is the **smallest experiment** that would confirm or refute the direction?
  That becomes the alignment note's *minimum viable experiment*.

## How it feeds the alignment note

- Step 2 truths → **Irreducible constraints** + **Declared rules**.
- Step 3 → **Assumptions** (ranked) + struck conventions.
- Step 4 additions → candidate **approaches** (with the truth each serves).
- Step 5 → **Unknowns** (ranked) + **minimum viable experiment**.

## Anti-patterns

- ❌ Restating the problem with the solution baked in.
- ❌ Treating a framework/library convention as a fundamental truth.
- ❌ Keeping an assumption off the note because you are "sure" — disclose it.
- ❌ Producing a verdict; First Principles proposes, the user + ctl evidence decide.
