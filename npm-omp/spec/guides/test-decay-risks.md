# Test Decay Risk Reference (T1–T6)

Six patterns that cause test suites to degrade. Apply the **Iron Law** to every finding:

```
EVERY finding MUST follow: Symptom → Source → Consequence → Remedy.
```

---

## T1: Test Obscurity

**Diagnostic question:** How much effort to understand what this test verifies?

### Symptoms

- Assertion Roulette: multiple assertions with no message — can't tell which failed
- Mystery Guest: test depends on external state not visible in test body
- Test names don't express scenario + expected outcome (`test1`, `shouldWork`)
- General Fixture: oversized setUp shared by unrelated tests
- Test body requires reading production code to understand what's verified

### Severity

- 🔴 Critical: no test name describes behavior; all assertions lack messages
- 🟡 Warning: multiple Mystery Guests; several ambiguous names
- 🟢 Suggestion: minor naming issues; isolated General Fixture

### What NOT to Flag

- Multiple assertions describing one coherent behavior with clear failure story
- Shared setup where every value is relevant to nearly every test
- Concise names if scenario and outcome are still obvious

---

## T2: Test Brittleness

**Diagnostic question:** Do tests break when you refactor without changing behavior?

### Symptoms

- Assertions on private methods, internal state, or implementation details
- Eager Test: one test verifying multiple unrelated behaviors
- Over-specified: mock call order or exact parameter values irrelevant to behavior
- Renaming/extracting a method causes 5+ test failures with no behavior change
- Erratic Test: different results across runs (race conditions, time, shared mutable state)

### Severity

- 🔴 Critical: refactoring with no behavior change causes failures; 5+ tests coupled to one detail
- 🟡 Warning: Eager Tests common; moderate implementation-detail assertions
- 🟢 Suggestion: isolated over-specification in non-critical tests

### What NOT to Flag

- Verifying externally observable event or emitted command (not implementation coupling)
- One test with several assertions all supporting one behavior claim
- Fake/in-memory adapter where test still asserts behavior not wiring

---

## T3: Test Duplication

**Diagnostic question:** Is the same test scenario expressed in more than one place?

### Symptoms

- Same setup/assertion logic copy-pasted without extraction
- Lazy Test: multiple tests verifying identical behavior with no input differentiation
- Same boundary condition tested identically at unit, integration, and E2E
- Test helpers/fixtures duplicated across files instead of shared

### Severity

- 🔴 Critical: core scenario fully duplicated across all three test layers
- 🟡 Warning: common setup repeated in 5+ tests without extraction
- 🟢 Suggestion: minor helper duplication; isolated Lazy Tests

### What NOT to Flag

- Same scenario at unit and integration when each verifies distinct risk
- Small local setup clearer than over-abstracted fixture maze
- Similar assertions against different domain rules with different business intent

---

## T4: Mock Abuse

**Diagnostic question:** Is the test more complex than the behavior it tests?

### Symptoms

- Mock setup longer than test logic itself
- Primary assertion is `expect(mock).toHaveBeenCalledWith(...)` — verifies mock, not behavior
- Test-only methods added to production classes
- Single test uses > 3 mocks
- Incomplete Mock: missing fields downstream code will access
- Hard-Coded Test Data with no resemblance to real data shapes

### Severity

- 🔴 Critical: mock setup > 50% of test code; production methods only called from tests
- 🟡 Warning: mocks consistently > 3 per test; primary assertions are mock verifications
- 🟢 Suggestion: isolated Incomplete Mocks; minor Hard-Coded Data

### What NOT to Flag

- Small number of mocks around nondeterministic dependencies with behavior assertions
- Fakes and spies observing state transitions
- One interaction assertion when the interaction IS the behavior under test

---

## T5: Coverage Illusion

**Diagnostic question:** Does the test suite protect against the failures that matter?

### Symptoms

- High line coverage but error-handling, boundary, and exception paths untested
- Happy-path only: no sad paths, null/empty/zero, concurrency edge cases
- Legacy code actively modified with no tests
- Coverage % as sign-off while critical paths untested
- Assertions on return values but not side effects (DB writes, events, state transitions)

### Severity

- 🔴 Critical: legacy code actively modified with no tests; error paths entirely absent
- 🟡 Warning: coverage > 80% but edge/exception paths systematically absent
- 🟢 Suggestion: a few non-critical paths missing sad-path tests

### What NOT to Flag

- High line coverage paired with branch, boundary, and change-path coverage
- New module with limited coverage still private and low-risk
- Side-effect assertions in integration tests rather than unit tests

---

## T6: Architecture Mismatch

**Diagnostic question:** Does the test suite structure reflect the system's actual risk profile?

### Symptoms

- Inverted pyramid: E2E/integration count exceeds unit test count
- Legacy code with no seam points (no interfaces, no DI)
- Legacy areas modified without Characterization Tests
- Full suite execution > 10 minutes
- High-risk and low-risk paths tested at identical density

### Severity

- 🔴 Critical: legacy code modified with no seams and no characterization tests; pyramid fully inverted
- 🟡 Warning: suite > 10 minutes; integration/E2E count exceeds unit tests
- 🟢 Suggestion: localized pyramid deviation; few legacy areas missing characterization tests

### What NOT to Flag

- Deviating from 70:20:10 justified by platform constraints or product risk
- Integration-heavy suite that's still fast and purposefully layered
- Small number of critical-path E2E tests (desirable, not a smell)
