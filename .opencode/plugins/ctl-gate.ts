/**
 * ctl Control Plane — opencode plugin.
 *
 * The opencode-side shim for the same governance the `.claude` and `.omp` hooks
 * provide: the Rust `ctl` binary computes state + permissions (`ctl hook gate`,
 * `ctl hook context`); this plugin only translates opencode's plugin protocol
 * into `ctl` calls and back.
 *
 *   1. tool.execute.before — gate write/edit/bash/task against the ctl ledger.
 *      Throwing aborts the tool call, so a denied verdict blocks the action.
 *   2. experimental.chat.system.transform — inject active task boundaries into
 *      the system prompt on every call (opencode's analog of SessionStart
 *      context injection).
 *
 * Fails CLOSED for mutating tools: if `ctl` is missing, times out, or returns
 * unparseable output, mutating tools are blocked rather than silently allowed —
 * an unenforceable boundary must never wave writes through. Read-only tools are
 * never blocked on ctl errors.
 *
 * Auto-loaded by opencode from `.opencode/plugins/*.ts` (the brace glob
 * `{plugin,plugins}` also matches the singular form on current versions).
 */
import type { Plugin } from "@opencode-ai/plugin";
import { execFile } from "node:child_process";

interface GateResult {
  allowed: boolean;
  state: string;
  reason: string;
  task_id?: string;
  remedy?: string;
}

// Timeout for `ctl` invocations. Generous ceiling avoids spurious fail-closed
// blocks under load; the async spawn path below never hangs. Override via env.
const CTL_TIMEOUT_MS =
  Number.parseInt(process.env.CTL_TIMEOUT_MS ?? "", 10) || 15_000;

// opencode tool names whose denial/unavailability must fail CLOSED.
// `bash` is included (unlike the Claude hook) because opencode surfaces a clean
// `command` arg, so ctl does not need to parse complex shell on Windows.
const FAIL_CLOSED_TOOLS = new Set(["write", "edit", "patch", "bash", "task"]);

/** Emit a one-line diagnostic so ctl failures are observable, not swallowed. */
function logCtlError(stage: string, args: readonly string[], detail: string): void {
  if (typeof process.stderr?.write !== "function") return;
  process.stderr.write(
    `\n⚠️ ctl ${stage} failed: ${JSON.stringify({ args, detail: detail.slice(0, 500) })}\n`,
  );
}

/**
 * Invoke `ctl` via the async libuv spawn path (`execFile`), NOT a sync spawn:
 * on Windows `spawnSync` against the native `ctl.exe` can hang with empty stdio
 * until the timeout kills it. Returns stdout on success, or null on any error.
 */
function ctl(args: string[], stage: string): Promise<string | null> {
  return new Promise((resolve) => {
    execFile(
      "ctl",
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

/** Query the governance gate for a tool action. */
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

  // Forward the dispatch binding so the gate governs this call by the task that
  // dispatched it (resolves multi-active ambiguity). ctl also reads CTL_TASK_ID
  // from its own env; this is the explicit, audited seam.
  const task = (process.env.CTL_TASK_ID ?? "").trim();
  if (task) args.push("--task", task);

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
  } catch {
    logCtlError("gate-parse", args, raw);
    return null;
  }
}

/** Map an opencode tool + its args to the ctl gate's (tool, path, command, agentType). */
function extractGateInput(tool: string, args: Record<string, unknown>): {
  ctlTool: "write" | "bash" | "task" | null;
  path?: string;
  command?: string;
  agentType?: string;
} {
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

export const CtlGate: Plugin = async () => {
  return {
    // ── Tool gate: state-machine enforcement ──────────────────────────────
    "tool.execute.before": async (input, output) => {
      const { ctlTool, path, command, agentType } = extractGateInput(
        input.tool,
        (output.args ?? {}) as Record<string, unknown>,
      );
      if (!ctlTool) return; // not a gated tool

      const gate = await checkGate(ctlTool, path, command, agentType);

      if (!gate) {
        // ctl unavailable. Fail CLOSED for mutating tools; allow read-only.
        if (FAIL_CLOSED_TOOLS.has(input.tool)) {
          throw new Error(
            "ctl gate unavailable (binary missing, timeout, or error) — failing closed. " +
              "Mutating tools are blocked until `ctl` responds. Ensure `ctl` is on PATH.",
          );
        }
        return;
      }

      if (!gate.allowed) {
        const remedy = gate.remedy ? `\n💡 ${gate.remedy}` : "";
        throw new Error(`ctl gate [${gate.state}]: ${gate.reason}${remedy}`);
      }
    },

    // ── Context injection: active task boundaries on every call ───────────
    "experimental.chat.system.transform": async (_input, output) => {
      const raw = await ctl(["hook", "context"], "context");
      if (!raw) return;
      let ctx: Record<string, unknown>;
      try {
        ctx = JSON.parse(raw) as Record<string, unknown>;
      } catch {
        return;
      }

      const active = (ctx.active_tasks ?? []) as Array<{
        id: string;
        objective: string;
        boundary?: { write_allow?: string[]; write_deny?: string[]; gates?: string[] };
      }>;
      if (!active.length) return;

      const lines = active.map((t) => {
        const b = t.boundary;
        const scope = b?.write_allow?.length ? b.write_allow.join(", ") : "(no write scope)";
        const deny = b?.write_deny?.length ? `\n  🚫 Deny: ${b.write_deny.join(", ")}` : "";
        const gates = b?.gates?.length ? `\n  🔍 Gates: ${b.gates.join(", ")}` : "";
        return `  📦 ${t.id}: ${t.objective}\n  ✏️ Write: ${scope}${deny}${gates}`;
      });

      output.system.push(
        [
          `📋 Active ctl task boundaries — stay within write scope:`,
          ...lines,
          `\nTool calls are gated by the ctl state machine: writes outside scope, git commits without a completed task, and pushes are blocked. If the ctl gate is unavailable, mutating tools fail closed (blocked) until it responds.`,
        ].join("\n"),
      );
    },
  };
};

export default CtlGate;
