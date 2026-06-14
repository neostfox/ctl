# Code Decay Risk Reference (R1–R6)

Six patterns that cause software to degrade. Apply the **Iron Law** to every finding:

```
NEVER suggest fixes before completing risk diagnosis.
EVERY finding MUST follow: Symptom → Source → Consequence → Remedy.
```

A finding without all four fields is noise, not a diagnosis.

---

## R1: Cognitive Overload

**Diagnostic question:** How much mental effort does a human need to understand this?

### Symptoms

- Function > 20 lines mixing multiple abstraction levels
- Nesting depth > 3
- Parameter list > 4
- Magic numbers or unexplained constants
- Names that require reading implementation (`d`, `tmp2`, `flag`)
- Boolean expressions with 3+ conditions
- Train-wreck chains: `a.getB().getC().doD()`
- Code names don't match business terminology
- Flag Arguments: boolean parameter making function do two different things
- Primitive Obsession: domain concepts as raw types (`String email`, `int orderId`)
- Shallow module: interface complexity > functionality provided

### Severity

- 🔴 Critical: function > 50 lines, nesting > 5, no meaningful names
- 🟡 Warning: function 20–50 lines, nesting 4–5, some unclear names
- 🟢 Suggestion: minor naming issues, 1–2 magic numbers, isolated train-wrecks

### What NOT to Flag

- Linear code with clear names and guard clauses
- Internal detail hidden behind a deep module boundary
- Domain terminology that matches how experts speak

---

## R2: Change Propagation

**Diagnostic question:** How many unrelated things break when you change one thing?

### Symptoms

- One feature change touches > 3 files in unrelated modules
- One class changes for multiple business reasons
- Method uses more data from another class than its own
- Two classes know each other's internal state
- Changing one module forces recompiling/retesting many unrelated ones
- Hyrum's Law: every observable behavior becomes implicit contract
- Orthogonality violation: adding payment type requires touching logging/caching/notifications
- Information Leakage: design decision encoded in multiple modules

### Severity

- 🔴 Critical: one change touches > 5 files, or domain depends on infrastructure
- 🟡 Warning: one change touches 3–5 files, mild coupling
- 🟢 Suggestion: minor coupling, easily isolatable

### What NOT to Flag

- Composition root wiring dependencies
- Stable public API with intentionally supported behavior
- Coordinated edits within one bounded context

---

## R3: Knowledge Duplication

**Diagnostic question:** Is the same decision expressed in more than one place?

### Symptoms

- Same logic copy-pasted across files
- Same concept named differently (`user`/`account`/`member`/`customer`)
- Parallel class hierarchies that must change in sync
- Config values repeated as literals
- Two modules implementing same algorithm independently

### Severity

- 🔴 Critical: core business logic duplicated, or same concept named 3+ ways
- 🟡 Warning: utility code duplicated, naming inconsistent within subsystem
- 🟢 Suggestion: minor literal duplication, single naming inconsistency

### What NOT to Flag

- Repetition across separate bounded contexts
- Temporary duplication during active extraction/migration
- Shared protocol constants at explicit boundaries with local ownership

---

## R4: Accidental Complexity

**Diagnostic question:** Is the code more complex than the problem it solves?

### Symptoms

- Abstractions built "for future use" with no current consumer
- Classes that barely justify existence (wrap a single call)
- Pure middle-men delegating without adding behavior
- Second system significantly more elaborate than first
- Switch statements signaling missing polymorphism
- Config options never changed from defaults
- Framework code larger than application it powers
- Accumulated tactical shortcuts: every new feature fights the structure

### Severity

- 🔴 Critical: entire subsystem for speculative requirement, or framework overhead dominates
- 🟡 Warning: several unnecessary abstractions or wrappers
- 🟢 Suggestion: one or two lazy classes in non-critical paths

### What NOT to Flag

- Switch over external protocol, wire format, or closed enum
- Thin wrappers absorbing vendor churn
- Larger second version with legitimate present needs

---

## R5: Dependency Disorder

**Diagnostic question:** Do dependencies flow in a consistent, predictable direction?

### Symptoms

- Circular dependencies between modules
- High-level business logic imports from low-level infrastructure
- Stable components depend on unstable ones
- Abstract components depending on concrete implementations
- Law of Demeter violations: `order.getCustomer().getAddress().getCity()`
- Module fan-out > 5
- Fat interface forcing unwanted dependencies (ISP violation)
- Incompatible architectural patterns with no rule for which to use
- Diamond dependency: upgrading one library requires coordinating multiple repos

### Severity

- 🔴 Critical: dependency cycles, or domain layer depends on infrastructure
- 🟡 Warning: several SDP/DIP violations but no cycles
- 🟢 Suggestion: minor Demeter violations, slightly elevated fan-out

### What NOT to Flag

- High fan-out in orchestration layer or composition root
- Adapter modules depending on both domain and infrastructure (translation boundary)
- Stable facade over many leaf dependencies with clear policy

---

## R6: Domain Model Distortion

**Diagnostic question:** Does the code faithfully represent the problem it is solving?

### Symptoms

- Business logic in service layers while domain objects are getters/setters only (anemic model)
- Code names don't match business stakeholder language
- Class that only holds data with no behavior (pure data bag)
- Subclass that ignores/overrides most parent behavior
- Bounded context boundaries crossed without translation layer
- Methods more interested in another class's data than their own
- Value Objects treated as Entities with mutable ID and lifecycle

### Severity

- 🔴 Critical: domain logic entirely in service layer, domain objects are pure data bags
- 🟡 Warning: partial anemia, naming inconsistency between code and domain
- 🟢 Suggestion: minor naming drift in non-core areas, isolated Feature Envy

### What NOT to Flag

- CRUD workflows legitimately using transaction scripts
- DTOs, persistence records, API payloads allowed to be data-only
- Shared infrastructure language with simple business model
