# First Principles Thinking

Read this guide when you need to derive scope from fundamentals — before proposing boundaries for complex tasks.

## Steps

1. **Restate the problem**: One sentence about what needs to be true when done.
   - Bad: "Add Redis caching"
   - Good: "Profile data loads too slowly when concurrent tasks exceed 10"

2. **List fundamental truths**: Physical constraints, business rules, technical invariants, user needs.

3. **Challenge assumptions**: Fact or convention? What if removed? Solving problem or symptom?

4. **Build up**: Minimum viable scope from truths. Each addition must answer "which truth requires this?"

5. **Validate**: Does it solve the original problem? What's the simplest experiment to confirm?

## When to use

- Complex tasks with ambiguous scope
- Tasks where the user's request may be a symptom, not the problem
- Before proposing `write_allow` for multi-file changes

## When NOT to use

- Trivial fixes (typo, single-line change)
- Clear, well-scoped tasks with obvious boundaries
- User explicitly scoped the task
