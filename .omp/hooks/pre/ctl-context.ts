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
import { execFile } from "child_process";

interface GateResult {
  allowed: boolean;
  state: string;
  reason: string;
  task_id?: string;
  remedy?: string;
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
 * Invoke `ctl` via the async libuv spawn path (`execFile`), NOT `execFileSync`.
 * On Windows, `spawnSync` against the native `ctl.exe` intermittently hangs
 * with empty stdio until the timeout kills it (issue #2) — the async path does
 * not. Returns stdout on success, or null on any error (logged, not swallowed).
 */
function ctl(args: string[], stage: string): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    execFile(
      "ctl",
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
    };
  } catch (err) {
    logCtlError("gate-parse", args, err as NodeJS.ErrnoException, raw);
    return null;
  }
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
        return `  📦 ${t.id}: ${t.objective}\n  ✏️ Write: ${scope}${deny}${gates}`;
      });

      const boundary = [
        `📋 Active ctl task boundaries — stay within write scope:`,
        ...lines,
        `\nTool calls are gated by the ctl state machine: writes outside scope, git commits without a completed task, and pushes are blocked. If the ctl gate is unavailable, mutating tools fail closed (blocked) until it responds.`,
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

    if (!gate.allowed) {
      const remedy = gate.remedy ? `\n💡 ${gate.remedy}` : "";
      return {
        block: true,
        reason: `ctl gate [${gate.state}]: ${gate.reason}${remedy}`,
      };
    }

    // ── Post-gate: record subagent spawn times ──
    if (tool === "task" && gate.allowed) {
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
  // 4. AGENT END — spec drift detection
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
  });
}
