---
name: ctl-brainstorm
description: "Turns a vague request into a concrete, well-scoped ctl task proposal through evidence-first interviewing — one high-value question at a time, each with a recommended answer. Converges on objective, read/write boundaries, gates, and risks. Triggers when: requirements are unclear, there are multiple valid approaches, the user describes a new feature or complex change, or the user runs /ctl-new. Do NOT trigger for: a request that is already unambiguous and ready to scope, code review (ctl-review), or debugging (ctl-diagnose)."
---

# ctl-brainstorm

Planning happens before a ctl task is created. Your job: converge a fuzzy request into a
proposal that control-guard can turn into `ctl task create` — a clear **objective**,
minimal **write_allow**, **read_scope**, **gates**, and known **risks**.

## Non-negotiable: evidence before questions

If a question can be answered by reading the repo, read the repo — do not ask the user.
Inspect code, tests, configs, docs, existing specs, and task history first. Only ask the
user for things the repository cannot answer: product intent, preference, scope boundary,
risk tolerance, or a decision still ambiguous after inspection.

## Non-negotiable: one question at a time

Interview relentlessly but narrowly. Ask the single highest-value open question, then wait.
Each question carries:
- the decision needed,
- why it matters,
- **your recommended answer**,
- the trade-off if they choose otherwise.

Never ask process questions ("should I search the code?") — just do the work. Prefer
offering concrete options over open-ended prompts.

## Flow

1. **Capture** the request and the facts you already know.
2. **Inspect** the codebase; sort what you find into: confirmed facts · intent still
   needed from the user · scope/risk decisions still needed · likely out-of-scope.
3. **Interview** down the decision tree — one question at a time, each with a recommendation
   — until the remaining unknowns are genuinely the user's to decide.
4. **Refine the idea** when the goal is broad: stress-test assumptions, name what's in and
   out, and split a large effort into independently verifiable child tasks with
   **non-overlapping write_allow** (overlap forces sequencing — see control-guard).
5. **Converge** on the task proposal.

## Output: the task proposal

Hand this to control-guard for approval and `ctl task create`:

```
Task Proposal: <id>
  Objective:  <one sentence>
  Read:       <files/dirs to read>
  Write:      <minimal files/dirs to change>
  Deny:       <protected paths, if relevant>
  Gates:      <cargo_check / cargo_test / cargo_fmt_check / cargo_clippy>
  Risks:      <what could go wrong>
  Specs:      <which spec/guide files inform this work>
```

write_allow is **always minimal** — start narrow, widen only with explicit approval.

## Quality bar before proposing

- Objective is one testable sentence, not a paragraph of wishes.
- Every repository-answerable question was answered by inspection, not asked.
- Remaining open items are genuinely user intent / scope / risk.
- write_allow is the smallest set that lets the work happen.
- A large effort is decomposed into child tasks with non-overlapping write scopes.

## Anti-patterns

- ❌ Asking the user something the code already answers.
- ❌ Multiple questions in one message.
- ❌ A question without a recommended answer.
- ❌ Proposing a broad write_allow "to be safe".
- ❌ Jumping to implementation before the proposal is approved.
