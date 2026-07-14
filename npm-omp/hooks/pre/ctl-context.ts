// ctl Control Plane Hook — OMP native extension
// State-machine governance: every tool call is checked against the ctl task ledger.
// Rust side (ctl hook gate) computes state + permissions. This hook is the dispatcher.
//
// Enforcement layers:
//   1. tool_call gate — state machine (write scope, git, deps, subagent spawn)
//   2. subagent timeout — block polling after threshold, force cancel
//   3. context injection — active task boundaries on every LLM call
//   4. spec drift + unfinished reminders on lifecycle events

import type { HookAPI } from "@oh-my-pi/pi-coding-agent/extensibility/hooks";
import { execFile, spawnSync } from "child_process";
import { existsSync, readFileSync } from "fs";
import { homedir } from "os";
import { dirname, join } from "path";

interface GateResult {
  allowed: boolean;
  state: string;
  reason: string;
  task_id?: string;
  remedy?: string;
  /** Gate hint: log this verdict even if allowed (e.g. a never-path-scoped
   *  bash_write). Denies are logged regardless of this flag. */
  record?: boolean;
}

// ── Subagent timeout tracking ──────────────────────────────────────────
// Module-level: persists across tool_call events within the same session.
// Key = subagent label/id, Value = spawn timestamp (ms).
const subagentStartTimes = new Map<string, number>();

// Configurable via env var; default 5 minutes.
const SUBAGENT_TIMEOUT_MS = Number.parseInt(
  process.env.CTL_SUBAGENT_TIMEOUT_MS ?? "",
  10,
) || 300_000;

// Timeout for `ctl` invocations. Default 15s (was 5s) — the async path below
// never hangs, but a generous ceiling avoids spurious fail-closed blocks under
// load. Configurable via env var.
const CTL_TIMEOUT_MS = Number.parseInt(
  process.env.CTL_TIMEOUT_MS ?? "",
  10,
) || 15_000;

/** Emit a one-line diagnostic so ctl call failures are observable instead of
 *  being silently swallowed (see issue #2). */
function logCtlError(
  stage: string,
  args: readonly string[],
  err: NodeJS.ErrnoException & { signal?: string; code?: string | number },
  stderr?: string,
): void {
  if (typeof process.stderr?.write !== "function") return;
  process.stderr.write(
    `\n⚠️ ctl hook ${stage} failed: ${JSON.stringify({
      args,
      code: err?.code,
      signal: err?.signal,
      msg: err?.message,
      stderr: (stderr ?? "").slice(0, 500),
    })}\n`,
  );
}

/**
 * Best-effort: read `CTL_BIN=...` from the nearest `.env` walking up from cwd.
 * `.env` is NOT auto-loaded into a hook subprocess's env; this makes the
 * contract documented in the repo's `.env` comment actually hold. */
function readDotEnvCtlBin(): string | undefined {
  let dir = process.cwd();
  for (let i = 0; i < 8; i++) {
    const p = join(dir, ".env");
    if (existsSync(p)) {
      try {
        for (const line of readFileSync(p, "utf-8").split("\n")) {
          const m = line.match(/^\s*CTL_BIN\s*=\s*(.+?)\s*$/);
          if (m) return m[1].replace(/^["']|["']$/g, "");
        }
      } catch {
        /* unreadable .env — ignore */
      }
      return undefined; // found .env, no CTL_BIN
    }
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return undefined;
}

/**
 * Resolve `ctl` on PATH via the OS lookup tool (`where` on Windows, `which`
 * elsewhere). Node's `execFile` does NOT do PATHEXT resolution on Windows, so
 * a bare "ctl" ENOENTs whenever PATH lacks an exact match — this closes that
 * hole. Sync and cheap. */
function resolveViaPathLookup(): string | undefined {
  try {
    const r = spawnSync(
      process.platform === "win32" ? "where" : "which",
      ["ctl"],
      { encoding: "utf-8", timeout: 3000, windowsHide: true },
    );
    if (r.status === 0) {
      const first = (r.stdout || "").split(/\r?\n/)[0]?.trim();
      if (first && existsSync(first)) return first;
    }
  } catch {
    /* lookup tool unavailable — fall through */
  }
  return undefined;
}

let resolvedCtlBin: string | undefined;

/**
 * Locate the `ctl` binary via a robust candidate chain (most-certain first):
 *   CTL_BIN env → project .env → CARGO_HOME/bin → ~/.cargo/bin → where/which → bare "ctl".
 * Probed once and memoized. The bare-"ctl" tail ENOENTs on Windows without
 * PATHEXT, so `where`/`which` is preferred; if even that fails the error is
 * logged so the failure is observable instead of a silent ENOENT. */
function resolveCtlBin(): string {
  if (resolvedCtlBin !== undefined) return resolvedCtlBin;
  const bin = process.platform === "win32" ? "ctl.exe" : "ctl";

  const fromEnv = process.env.CTL_BIN?.trim();
  if (fromEnv) return (resolvedCtlBin = fromEnv);

  const candidates: string[] = [];
  const fromDotEnv = readDotEnvCtlBin();
  if (fromDotEnv) candidates.push(fromDotEnv);
  const cargoHome = process.env.CARGO_HOME?.trim();
  if (cargoHome) candidates.push(join(cargoHome, "bin", bin));
  candidates.push(join(homedir(), ".cargo", "bin", bin));

  for (const c of candidates) {
    if (existsSync(c)) return (resolvedCtlBin = c);
  }

  const viaPath = resolveViaPathLookup();
  if (viaPath) return (resolvedCtlBin = viaPath);

  logCtlError("resolveCtlBin", [], {
    code: "ENOENT",
    message: `no ctl binary found; tried [${candidates.join(", ")}]. Set CTL_BIN env, or ensure ctl is on PATH.`,
  });
  return (resolvedCtlBin = "ctl"); // last resort — may ENOENT, fail-closed downstream
}

/**
 * Invoke `ctl` via the async libuv spawn path (`execFile`), NOT `execFileSync`.
 * On Windows, `spawnSync` against the native `ctl.exe` intermittently hangs
 * with empty stdio until the timeout kills it (issue #2) — the async path does
 * not. Returns stdout on success, or null on any error (logged, not swallowed).
 */
function ctl(args: string[], stage: string): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    execFile(
      resolveCtlBin(),
      args,
      { encoding: "utf-8", timeout: CTL_TIMEOUT_MS, windowsHide: true },
      (err, stdout, stderr) => {
        if (err) {
          logCtlError(stage, args, err, stderr);
          resolve(null);
          return;
        }
        resolve(stdout);
      },
    );
  });
}

/** Run `ctl hook <subcommand>` asynchronously. */
function ctlHook(subcommand: string): Promise<string | null> {
  const parts = subcommand.split(" ");
  return ctl(["hook", ...parts], parts[0] ?? subcommand);
}

function parseJson(raw: string | null): Record<string, unknown> | null {
  if (!raw) return null;
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}

/** Query the governance gate for a tool action (async — see `ctl` above). */
async function checkGate(
  tool: string,
  path?: string,
  command?: string,
  agentType?: string,
): Promise<GateResult | null> {
  const args = ["hook", "gate", "--tool", tool];
  if (path) args.push("--path", path);
  if (command) args.push("--command", command);
  if (agentType) args.push("--agent-type", agentType);

  const raw = await ctl(args, "gate");
  if (raw === null) return null; // ctl unavailable / errored (already logged)

  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    return {
      allowed: parsed.allowed === true,
      state: (parsed.state as string) ?? "unknown",
      reason: (parsed.reason as string) ?? "",
      task_id: parsed.task_id as string | undefined,
      remedy: parsed.remedy as string | undefined,
      record: parsed.record === true,
    };
  } catch (err) {
    logCtlError("gate-parse", args, err as NodeJS.ErrnoException, raw);
    return null;
  }
}

/**
 * Append a blocked/flagged tool call to the NON-CANONICAL .ctl/decisions.jsonl
 * via `ctl hook record-decision`. Records every DENY and any verdict the gate
 * flags with record=true (e.g. a bash_write ALLOW, never path-scoped). Turns
 * "what the gate blocked/flagged" into auditable evidence.
 *
 * Fire-and-forget and best-effort: this never blocks or delays the tool call,
 * and any failure is swallowed by `ctl()` — an advisory log must not break the
 * gate it observes.
 */
function recordDecision(
  tool: string,
  gate: GateResult,
  path?: string,
  command?: string,
): void {
  const allowed = gate.allowed === true;
  if (allowed && gate.record !== true) return;
  const record: Record<string, unknown> = {
    source: "omp",
    tool,
    allowed,
    state: gate.state,
    reason: gate.reason,
  };
  if (command) record.command = command;
  else if (path) record.path = path;
  const task = gate.task_id || process.env.CTL_TASK_ID?.trim();
  if (task) record.task_id = task;
  void ctl(
    ["hook", "record-decision", "--data", JSON.stringify(record)],
    "record-decision",
  );
}

/**
 * Record an ALLOWED subagent dispatch as a canonical `subagent_dispatched` event
 * (`ctl dispatch record`), bound to the active parent task. role/adapter are host
 * LABELS (unattested); ctl records what it was told was dispatched, never what
 * ran. Fire-and-forget and best-effort — a missed attestation must never block or
 * delay the spawn — and skipped when no parent task is bound (CTL_TASK_ID unset).
 */
function recordDispatch(agentType: string): void {
  const task = process.env.CTL_TASK_ID?.trim();
  if (!task) return;
  void ctl(
    ["dispatch", "record", "--task", task, "--role", agentType || "task", "--adapter", "omp"],
    "dispatch-record",
  );
}

export default function (pi: HookAPI): void {

  // ═══════════════════════════════════════════
  // 1. SESSION START — register context injection
  // ═══════════════════════════════════════════
  pi.on("session_start", async () => {
    pi.on("context", async (event) => {
      const raw = await ctlHook("context");
      const ctx = parseJson(raw);
      if (!ctx) return undefined;

      const active = ctx.active_tasks as Array<{
        id: string;
        objective: string;
        boundary?: {
          write_allow?: string[];
          write_deny?: string[];
          gates?: string[];
        };
        next_action?: { action: string; rationale: string };
        drift_level?: string;
        drift_score?: number;
        blocked_by?: string[];
        open_uncertainties?: Array<{ id: string; statement: string }>;
        provenance?: { brainstorm_id: string; convergence_path?: string };
      }>;
      if (!active || active.length === 0) return undefined;

      const lines = active.map((t) => {
        const b = t.boundary;
        const scope = b?.write_allow?.length
          ? b.write_allow.join(", ")
          : "(no write scope)";
        const deny = b?.write_deny?.length
          ? `\n  🚫 Deny: ${b.write_deny.join(", ")}`
          : "";
        const gates = b?.gates?.length
          ? `\n  🔍 Gates: ${b.gates.join(", ")}`
          : "";
        const drift =
          t.next_action && t.next_action.action !== "pass"
            ? `\n  ⚠️ Drift: ${t.drift_level ?? "?"} (score ${t.drift_score ?? "?"}) → ${t.next_action.action} — ${t.next_action.rationale ?? ""}`
            : "";
        const blocked = t.blocked_by?.length
          ? `\n  🚧 Blocked by: ${t.blocked_by.join(", ")}`
          : "";
        const unknowns = t.open_uncertainties?.length
          ? `\n  ❓ Open unknowns (${t.open_uncertainties.length}): ${t.open_uncertainties.map((u) => `${u.id} (${u.statement})`).join("; ")}`
          : "";
        const prov =
          t.provenance?.convergence_path
            ? `\n  📎 Derived from: ${t.provenance.convergence_path}`
            : "";
        return `  📦 ${t.id}: ${t.objective}\n  ✏️ Write: ${scope}${deny}${gates}${drift}${blocked}${unknowns}${prov}`;
      });

      const facts = ctx.facts as
        | { total: number; categories: Record<string, number>; recent: Array<{ fact_id: string; statement: string }> }
        | undefined;
      const factsLine =
        facts && facts.total > 0
          ? `\n📚 Knowledge base: ${facts.total} fact(s) [${Object.entries(facts.categories).map(([k, v]) => `${k}: ${v}`).join(", ")}] | Recent: ${facts.recent.slice(0, 3).map((r) => `${r.fact_id} (${r.statement.slice(0, 60)})`).join("; ")}\n  Search: ctl spec fact list --search <query>`
          : "";

      const boundary = [
        `📋 Active ctl task boundaries — stay within write scope:`,
        ...lines,
        `\nTool calls are gated by the ctl state machine: writes outside scope, git commits without a completed task, and pushes are blocked. If the ctl gate is unavailable, mutating tools fail closed (blocked) until it responds.${factsLine}`,
      ].join("\n");

      return {
        messages: [
          {
            role: "user" as const,
            content: [{ type: "text" as const, text: boundary }],
            timestamp: Date.now(),
          },
          ...event.messages,
        ],
      };
    });
  });

  // ═══════════════════════════════════════════
  // 2. TOOL CALL — state machine enforcement
  //    + subagent timeout tracking
  // ═══════════════════════════════════════════
  pi.on("tool_call", async (event) => {
    const tool = event.toolName;
    const input = event.input ?? {};

    // ── Subagent timeout: block polling past threshold ──
    if (tool === "job" && Array.isArray(input.poll) && input.poll.length > 0) {
      const now = Date.now();
      const expired: string[] = [];
      for (const id of input.poll as string[]) {
        const start = subagentStartTimes.get(id);
        if (start && now - start > SUBAGENT_TIMEOUT_MS) {
          expired.push(id);
        }
      }
      if (expired.length > 0) {
        const mins = Math.round(SUBAGENT_TIMEOUT_MS / 60_000);
        return {
          block: true,
          reason: `ctl: subagent(s) ${expired.join(", ")} exceeded ${mins}min timeout. Cancel them (job cancel) and handle the work directly or re-spawn with a smaller assignment.`,
        };
      }
      // Not expired — allow poll to proceed
      return;
    }

    // ── Subagent timeout: clean up on cancel ──
    if (tool === "job" && Array.isArray(input.cancel)) {
      for (const id of input.cancel as string[]) {
        subagentStartTimes.delete(id);
      }
      // Allow cancel to proceed
      return;
    }

    // ── Subagent timeout: clean up on list (remove finished) ──
    if (tool === "job" && input.list === true) {
      // Allow list — result handler will clean up
      return;
    }

    // ── Extract tool-specific fields for gate ──
    let path: string | undefined;
    let command: string | undefined;
    let agentType: string | undefined;

    if (tool === "write" || tool === "edit") {
      path = (input.path as string) || (input._i as string);
      if (!path && typeof input.input === "string") {
        const match = input.input.match(/^\[([^\]#]+)#/);
        if (match) path = match[1];
      }
    } else if (tool === "bash") {
      command = input.command as string;
    } else if (tool === "task") {
      agentType =
        (input.agentType as string) ||
        (input.agent as string) ||
        "task";
    }

    // ── Gate check ──
    const gate = await checkGate(tool, path, command, agentType);
    if (!gate) {
      // ctl unavailable (missing binary, timeout, or crash). Fail CLOSED for
      // mutating tools — an unenforceable boundary must never silently allow
      // writes or commands. Read-only tools are unaffected.
      const mutating =
        tool === "write" ||
        tool === "edit" ||
        tool === "multiedit" ||
        tool === "bash" ||
        tool === "task";
      if (mutating) {
        return {
          block: true,
          reason:
            "ctl gate unavailable (binary missing, timeout, or error) — failing closed. Mutating tools are blocked until `ctl` responds. Check that `ctl` is on PATH.",
        };
      }
      return; // read-only / non-mutating — allow
    }

    // Record denies + flagged allows to the non-canonical decision log before
    // acting (a deny returns below; a bash_write allow proceeds). Best-effort.
    recordDecision(tool, gate, path, command);

    if (!gate.allowed) {
      const remedy = gate.remedy ? `\n💡 ${gate.remedy}` : "";
      return {
        block: true,
        reason: `ctl gate [${gate.state}]: ${gate.reason}${remedy}`,
      };
    }

    // ── Post-gate: attest the dispatch + record subagent spawn times ──
    if (tool === "task" && gate.allowed) {
      recordDispatch(agentType ?? "task");
      const tasks = input.tasks as Array<{ id?: string }> | undefined;
      if (Array.isArray(tasks)) {
        const now = Date.now();
        for (const t of tasks) {
          if (t.id) subagentStartTimes.set(t.id, now);
        }
      }
    }
  });

  // ═══════════════════════════════════════════
  // 3. TOOL RESULT — clean up finished subagents
  // ═══════════════════════════════════════════
  pi.on("tool_result", async (event) => {
    if (event.toolName !== "job") return;

    // Parse result text for completed/cancelled/failed job IDs
    const content = event.content ?? [];
    const textParts = content
      .filter((c: { type?: string }) => c.type === "text")
      .map((c: { text?: string }) => c.text ?? "")
      .join("\n");

    // Remove tracked IDs that appear as completed/cancelled/failed
    for (const [id] of subagentStartTimes) {
      // Match patterns like "### <id> [task] — completed/cancelled/failed"
      // or "- Cancelled background job <id>"
      const donePattern = new RegExp(
        `(completed|cancelled|canceled|failed).*${id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`,
        "i",
      );
      const cancelPattern = new RegExp(
        `cancelled.*${id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`,
        "i",
      );
      if (donePattern.test(textParts) || cancelPattern.test(textParts)) {
        subagentStartTimes.delete(id);
      }
    }
  });

  // ═══════════════════════════════════════════
  // 4. AGENT END — spec drift + wrap-up capture reminder
  // ═══════════════════════════════════════════
  pi.on("agent_end", async () => {
    const spec = parseJson(await ctlHook("spec-status"));
    if (
      spec?.drift &&
      typeof process.stderr?.write === "function"
    ) {
      process.stderr.write(
        `\n📝 Specs stale (${spec.source_files} source > ${spec.spec_files} specs). Run /ctl-spec-bootstrap.\n`,
      );
    }
    // Wrap-up reminder (mirrors the .claude Stop hook): a finished task with
    // no knowledge capture afterwards reminds ONCE — ctl's once-guard marks
    // the finish on the pending report, and any capture write self-clears.
    const wrap = parseJson(await ctlHook("wrapup-check"));
    if (
      wrap?.pending === true &&
      typeof process.stderr?.write === "function"
    ) {
      process.stderr.write(
        `\n🧠 ctl wrap-up: task '${wrap.task_id}' finished without a knowledge capture — ` +
          `run /ctl-spec-update (repo lessons → .ctl/spec/, cross-project preferences → ~/.ctl/memory/). ` +
          `Reminds once per finish.\n`,
      );
    }
  });

  // ═══════════════════════════════════════════
  // 5. SESSION SHUTDOWN — unfinished task reminder
  // ═══════════════════════════════════════════
  pi.on("session_shutdown", async () => {
    const raw = await ctlHook("context");
    const ctx = parseJson(raw);
    if (ctx) {
      const tasks = ctx.tasks as {
        by_phase: Record<string, number>;
        total: number;
      };
      const byPhase = tasks?.by_phase;
      const ip = byPhase?.["in_progress"] ?? 0;
      const rv = byPhase?.review ?? 0;
      if (
        (ip + rv) > 0 &&
        typeof process.stderr?.write === "function"
      ) {
        process.stderr.write(
          `\n⚠️ Unfinished: ${ip} in-progress, ${rv} in review. 'ctl task status --id <id>'.\n`,
        );
      }
    }

    const spec = parseJson(await ctlHook("spec-status"));
    if (
      spec?.drift &&
      typeof process.stderr?.write === "function"
    ) {
      process.stderr.write(
        `\n📝 Specs stale. Run /ctl-spec-bootstrap to refresh.\n`,
      );
    }

    const wrap = parseJson(await ctlHook("wrapup-check"));
    if (
      wrap?.pending === true &&
      typeof process.stderr?.write === "function"
    ) {
      process.stderr.write(
        `\n🧠 ctl wrap-up: task '${wrap.task_id}' finished without a knowledge capture — ` +
          `run /ctl-spec-update before closing (repo lessons → .ctl/spec/, ` +
          `cross-project preferences → ~/.ctl/memory/).\n`,
      );
    }
  });
}
