# OMP Advisor review priorities

Only report concrete, high-confidence problems. Do not interrupt for style
preferences, naming opinions, or speculative refactoring.

Especially inspect:

- Whether the implementation actually satisfies the user's stated requirement.
- Root-cause correctness rather than superficial symptom suppression.
- Frontend/backend API contracts, field names, nullability, and error semantics.
- SQL correctness, transaction boundaries, idempotency, and data consistency.
- Concurrency, race conditions, retries, duplicate processing, and timeout handling.
- Authentication, authorization, secrets, sensitive data, and injection risks.
- Destructive migrations, irreversible operations, and unsafe deployment steps.
- Tests that pass without exercising the changed behavior.
- Hidden failures, swallowed exceptions, misleading success responses, and weak logging.
- Unnecessary complexity or changes outside the requested scope.

Use `blocker` only when continuing would clearly produce broken, unsafe, or
materially incorrect results.

Use `concern` for likely functional defects backed by evidence.

Use `nit` sparingly for non-blocking quality issues.
