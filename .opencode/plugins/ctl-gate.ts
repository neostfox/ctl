/**
 * ctl Control Plane — opencode plugin.
 *
 * The opencode-side shim for the same governance the `.claude` and `.omp` hooks
 * provide: the Rust `ctl` binary computes state + permissions (`ctl hook gate`,
 * `ctl hook context`); this plugin only translates opencode's plugin protocol
 * into `ctl` calls and back.
 *
 *   1. tool.execute.before — gate write/edit/patch/bash/task against the ctl
 *      ledger. Throwing aborts the tool call, so a denied verdict blocks it.
 *   2. experimental.chat.system.transform — inject active task boundaries
 *      (scope, phase, task id) into the system prompt.
 *
 * Fails CLOSED for mutating tools: if `ctl` is missing, times out, or returns
 * unparseable output, mutating tools are blocked rather than silently allowed.
 * Read-only tools are never blocked on ctl errors.
 *
 * Structure: the gate/context decision logic is factored into pure, exported
 * functions (isMutating / extractGateInput / buildGateArgs / buildContextMessage)
 * and the ctl invocation behind a `CtlRunner` seam, so `bun test` can verify the
 * contract without spawning processes or a model. `createHooks(runner)` wires
 * them; the default export injects the real execFile-based runner.
 *
 * Auto-loaded by opencode from `.opencode/plugins/*.ts`.
 */
import type { Plugin } from "@opencode-ai/plugin";
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export interface GateResult {
  allowed: boolean;
  state: string;
  reason: string;
  task_id?: string;
  remedy?: string;
  /** Gate hint: log this verdict even if allowed (e.g. a never-path-scoped
   *  bash_write). Denies are logged regardless of this flag. */
  record?: boolean;
  /** Observe mode: a model-facing nudge on an ALLOWED verdict (out-of-scope
   *  or task-less write, out-of-window commit). Forwarded to stderr and the
   *  decision record; never blocks. */
  warning?: string;
}

export interface ActiveTask {
  id: string;
  objective: string;
  phase?: string;
  boundary?: { write_allow?: string[]; write_deny?: string[]; gates?: string[] };
  next_action?: { action: string; rationale: string };
  drift_level?: string;
  drift_score?: number;
  blocked_by?: string[];
  open_uncertainties?: Array<{ id: string; statement: string }>;
  provenance?: { brainstorm_id: string; convergence_path?: string };
}

export interface FactsDigest {
  total: number;
  categories: Record<string, number>;
  recent: Array<{ fact_id: string; statement: string; category?: string }>;
}

export interface ContextResult {
  active_tasks?: ActiveTask[];
  facts?: FactsDigest;
}

// opencode tool names whose denial/unavailability must fail CLOSED. `bash` is
// included (unlike the Claude hook) because opencode surfaces a clean `command`
// arg, so ctl does not need to parse complex shell on Windows.
export const MUTATING_TOOLS = new Set(["write", "edit", "patch", "bash", "task"]);

export function isMutating(tool: string): boolean {
  return MUTATING_TOOLS.has(tool);
}

// Timeout for `ctl` invocations. Generous ceiling avoids spurious fail-closed
// blocks under load; the async spawn path never hangs. Override via env.
const CTL_TIMEOUT_MS =
  Number.parseInt(process.env.CTL_TIMEOUT_MS ?? "", 10) || 15_000;

/** Map an opencode tool + its args to the ctl gate's logical (tool, path, command, agentType). */
export function extractGateInput(
  tool: string,
  args: Record<string, unknown>,
): { ctlTool: "write" | "bash" | "task" | null; path?: string; command?: string; agentType?: string } {
  switch (tool) {
    case "write":
    case "edit":
    case "patch":
      return { ctlTool: "write", path: (args.filePath as string) || (args.path as string) };
    case "bash":
      return { ctlTool: "bash", command: args.command as string };
    case "task":
      return {
        ctlTool: "task",
        agentType:
          (args.subagent_type as string) ||
          (args.agent as string) ||
          (args.agentType as string) ||
          "task",
      };
    default:
      return { ctlTool: null };
  }
}

/**
 * Build the `ctl hook gate` argv. Pure, and deliberately the ARRAY form (never a
 * shell string), so a path or command containing spaces, quotes, or `$`/`&`
 * stays a single argument and cannot change the command's argument boundaries.
 */
export function buildGateArgs(input: {
  ctlTool: "write" | "bash" | "task";
  path?: string;
  command?: string;
  agentType?: string;
  taskId?: string;
}): string[] {
  const args = ["hook", "gate", "--tool", input.ctlTool];
  if (input.path) args.push("--path", input.path);
  if (input.command) args.push("--command", input.command);
  if (input.agentType) args.push("--agent-type", input.agentType);
  // Forward the dispatch binding so the gate governs this call by the task that
  // dispatched it (resolves multi-active ambiguity).
  if (input.taskId && input.taskId.trim()) args.push("--task", input.taskId.trim());
  return args;
}

/**
 * Whether a verdict belongs in the non-canonical decision log: every DENY, plus
 * any allow the gate explicitly flags (`record === true`, e.g. a bash_write that
 * is never path-scoped against write_allow). Pure, so the policy is unit-tested.
 */
export function shouldRecord(gate: GateResult): boolean {
  return !gate.allowed || gate.record === true;
}

/**
 * Build the `ctl hook record-decision` argv for a verdict. Pure, and the ARRAY
 * form (never a shell string) so a path/command with spaces or quotes stays one
 * argument. The recorded object is labeled non-canonical by `ctl` on write.
 */
export function buildRecordArgs(input: {
  tool: string;
  gate: GateResult;
  path?: string;
  command?: string;
  taskId?: string;
}): string[] {
  const record: Record<string, unknown> = {
    source: "opencode",
    tool: input.tool,
    allowed: input.gate.allowed === true,
    state: input.gate.state,
    reason: input.gate.reason,
  };
  if (input.command) record.command = input.command;
  else if (input.path) record.path = input.path;
  if (input.gate.warning) record.warning = input.gate.warning;
  const task = input.gate.task_id || input.taskId?.trim();
  if (task) record.task_id = task;
  return ["hook", "record-decision", "--data", JSON.stringify(record)];
}

/**
 * Build the `ctl dispatch record` argv for an ALLOWED subagent spawn. Pure +
 * array form. role is a host LABEL (unattested) and adapter is fixed to this
 * platform; ctl records what it was told was dispatched, never verifies it ran.
 */
export function buildDispatchArgs(input: { taskId: string; role: string }): string[] {
  return [
    "dispatch",
    "record",
    "--task",
    input.taskId.trim(),
    "--role",
    input.role,
    "--adapter",
    "opencode",
  ];
}

/**
 * Render the active-task system context — scope, phase, enrichment signals
 * (drift, blockers, open unknowns, provenance) per active task. Returns null
 * when there is no active task, so the plugin never fabricates context out of
 * an empty ledger.
 */
export function buildContextMessage(
  active: ActiveTask[] | undefined,
  facts?: FactsDigest,
): string | null {
  const hasActive = !!active && active.length > 0;
  const hasFacts = !!facts && facts.total > 0;
  if (!hasActive && !hasFacts) return null;

  const lines = (active ?? []).map((t) => {
    const b = t.boundary;
    const scope = b?.write_allow?.length ? b.write_allow.join(", ") : "(no write scope)";
    const deny = b?.write_deny?.length ? `\n  🚫 Deny: ${b.write_deny.join(", ")}` : "";
    const gates = b?.gates?.length ? `\n  🔍 Gates: ${b.gates.join(", ")}` : "";
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
    const prov = t.provenance?.convergence_path
      ? `\n  📎 Derived from: ${t.provenance.convergence_path}`
      : "";
    const phase = t.phase ?? "in_progress";
    return `  📦 ${t.id} [${phase}]: ${t.objective}\n  ✏️ Write: ${scope}${deny}${gates}${drift}${blocked}${unknowns}${prov}`;
  });

  const factsLine = hasFacts
    ? `\n📚 Knowledge base: ${facts!.total} fact(s) [${Object.entries(facts!.categories).map(([k, v]) => `${k}: ${v}`).join(", ")}] | Recent: ${facts!.recent.slice(0, 3).map((r) => `${r.fact_id} (${r.statement.slice(0, 60)})`).join("; ")}\n  Search: ctl spec fact list --search <query>`
    : "";

  const header = hasActive
    ? `📋 Active ctl task boundaries — stay within write scope:`
    : `📋 ctl knowledge base:`;

  return [
    header,
    ...lines,
    `\nTool calls are gated by the ctl state machine: writes outside scope, git commits without a completed task, and pushes are blocked. If the ctl gate is unavailable, mutating tools fail closed (blocked) until it responds.${factsLine}`,
  ].join("\n");
}

/** Seam over the `ctl` invocation so tests can inject verdicts without spawning. */
export interface CtlRunner {
  gate(args: string[]): Promise<GateResult | null>;
  context(): Promise<ContextResult | null>;
  /** Append a decision record (best-effort; never throws). */
  recordDecision(args: string[]): Promise<void>;
  /** Record a subagent dispatch as a canonical event (best-effort; never throws). */
  recordDispatch(args: string[]): Promise<void>;
}

/** Emit a one-line diagnostic so ctl failures are observable, not swallowed. */
function logCtlError(stage: string, args: readonly string[], detail: string): void {
  if (typeof process.stderr?.write !== "function") return;
  process.stderr.write(
    `\n⚠️ ctl ${stage} failed: ${JSON.stringify({ args, detail: detail.slice(0, 500) })}\n`,
  );
}

function platformBinaryName(): string {
  return process.platform === "win32" ? "ctl.exe" : "ctl";
}

/**
 * Resolve the `ctl` executable via the one blessed chain shared by every
 * adapter: CTL_BIN override → ~/.cargo/bin → bare "ctl" (PATH fallback).
 * npm probing was retired with the npm binary distribution (B-lite; see
 * .ctl/spec/alignment/2026-07-04-binary-distribution-shrink.md) so exactly
 * one install location can no longer be shadowed by another. Exported for
 * tests.
 */
export function resolveCtlBin(): string {
  const fromEnv = process.env.CTL_BIN?.trim();
  if (fromEnv) return fromEnv;
  const cargoBin = join(homedir(), ".cargo", "bin", platformBinaryName());
  if (existsSync(cargoBin)) return cargoBin;
  return "ctl";
}

/**
 * Invoke `ctl` via the async libuv spawn path (`execFile`, array argv, no shell):
 * on Windows a sync spawn against the native `ctl.exe` can hang with empty stdio
 * until the timeout kills it. Returns stdout on success, or null on any error.
 */
function runCtl(args: string[], stage: string): Promise<string | null> {
  return new Promise((resolve) => {
    execFile(
      resolveCtlBin(),
      args,
      { encoding: "utf-8", timeout: CTL_TIMEOUT_MS, windowsHide: true },
      (err, stdout, stderr) => {
        if (err) {
          logCtlError(stage, args, `${(err as Error).message}\n${stderr ?? ""}`);
          resolve(null);
          return;
        }
        resolve(stdout);
      },
    );
  });
}

function parseJson<T>(raw: string | null, stage: string, args: readonly string[]): T | null {
  if (raw === null) return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    logCtlError(`${stage}-parse`, args, raw);
    return null;
  }
}

/** Default runner: shells out to the real `ctl` binary. */
export const realCtlRunner: CtlRunner = {
  async gate(args) {
    const parsed = parseJson<Record<string, unknown>>(await runCtl(args, "gate"), "gate", args);
    if (!parsed) return null;
    return {
      allowed: parsed.allowed === true,
      state: (parsed.state as string) ?? "unknown",
      reason: (parsed.reason as string) ?? "",
      task_id: parsed.task_id as string | undefined,
      remedy: parsed.remedy as string | undefined,
      record: parsed.record === true,
      warning: parsed.warning as string | undefined,
    };
  },
  async context() {
    const args = ["hook", "context"];
    return parseJson<ContextResult>(await runCtl(args, "context"), "context", args);
  },
  async recordDecision(args) {
    // Best-effort: runCtl logs and swallows any error (returns null). The
    // advisory decision log must never break the gate it observes.
    await runCtl(args, "record-decision");
  },
  async recordDispatch(args) {
    // Best-effort: a missed dispatch attestation must never break the spawn.
    await runCtl(args, "dispatch-record");
  },
};

/** Wire the opencode hooks over a [`CtlRunner`]. Exported for testing. */
export function createHooks(runner: CtlRunner) {
  return {
    // ── Tool gate: state-machine enforcement ──────────────────────────────
    "tool.execute.before": async (
      input: { tool: string },
      output: { args?: Record<string, unknown> },
    ): Promise<void> => {
      const gi = extractGateInput(input.tool, output.args ?? {});
      if (!gi.ctlTool) return; // not a gated tool

      const gate = await runner.gate(
        buildGateArgs({
          ctlTool: gi.ctlTool,
          path: gi.path,
          command: gi.command,
          agentType: gi.agentType,
          taskId: process.env.CTL_TASK_ID,
        }),
      );

      if (!gate) {
        // ctl unavailable. Fail CLOSED for mutating tools; allow read-only.
        if (isMutating(input.tool)) {
          throw new Error(
            "ctl gate unavailable (binary missing, timeout, or error) — failing closed. " +
              "Mutating tools are blocked until `ctl` responds. Ensure `ctl` is on PATH.",
          );
        }
        return;
      }

      // Record denies + flagged allows (e.g. bash_write) to the non-canonical
      // decision log before acting — a deny throws below, a bash_write allow
      // proceeds. Best-effort: recordDecision never throws.
      if (shouldRecord(gate)) {
        await runner.recordDecision(
          buildRecordArgs({
            tool: input.tool,
            gate,
            path: gi.path,
            command: gi.command,
            taskId: process.env.CTL_TASK_ID,
          }),
        );
      }

      if (!gate.allowed) {
        const remedy = gate.remedy ? `\n💡 ${gate.remedy}` : "";
        throw new Error(`ctl gate [${gate.state}]: ${gate.reason}${remedy}`);
      }

      // Observe mode: an allowed verdict may carry a warning (out-of-scope /
      // task-less write, out-of-window commit). Surface it without blocking —
      // the opencode plugin has no additionalContext channel, so stderr is
      // the honest best-effort forward (also lands in the decision record).
      if (gate.warning) {
        process.stderr.write(`\n⚠️ ctl observe [${gate.state}]: ${gate.warning}\n`);
      }

      // Attestation V1: an ALLOWED subagent dispatch bound to a parent task is
      // recorded as a canonical `subagent_dispatched` event. Best-effort and
      // non-blocking (recordDispatch never throws). Skipped when no parent task
      // is bound (CTL_TASK_ID unset) — there is nothing to attribute it to.
      const dispatchTask = process.env.CTL_TASK_ID?.trim();
      if (gi.ctlTool === "task" && dispatchTask) {
        await runner.recordDispatch(
          buildDispatchArgs({ taskId: dispatchTask, role: gi.agentType ?? "task" }),
        );
      }
    },

    // ── Context injection: active task boundaries on every call ───────────
    "experimental.chat.system.transform": async (
      _input: unknown,
      output: { system: string[] },
    ): Promise<void> => {
      const ctx = await runner.context();
      const msg = buildContextMessage(ctx?.active_tasks, ctx?.facts);
      if (msg) output.system.push(msg);
    },
  };
}

export const CtlGate: Plugin = async () => createHooks(realCtlRunner) as never;

export default CtlGate;
