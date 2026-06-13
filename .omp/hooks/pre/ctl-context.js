// ctl Control Plane Hook — OMP native extension (thin wrapper around `ctl` binary)
// All control plane logic lives in Rust. This hook calls `ctl hook <subcommand>` for data.
// Installed into .omp/hooks/pre/ by `ctl init`.

import type { HookAPI } from "@oh-my-pi/pi-coding-agent/extensibility/hooks";

const { execSync } = require("child_process");

function ctl(subcommand, args = []) {
  try {
    const result = execSync(`ctl hook ${subcommand} ${args.join(" ")}`, {
      encoding: "utf-8",
      timeout: 5000,
      stdio: ["pipe", "pipe", "pipe"],
    });
    return JSON.parse(result);
  } catch {
    return null;
  }
}

function ctlCheck(path) {
  try {
    const result = execSync(`ctl hook check-write --path "${path.replace(/"/g, '\\"')}"`, {
      encoding: "utf-8",
      timeout: 5000,
      stdio: ["pipe", "pipe", "pipe"],
    });
    return JSON.parse(result);
  } catch {
    return { allowed: true, reason: "hook_error" };
  }
}

function ctlRecord(data) {
  try {
    const json = JSON.stringify(data).replace(/"/g, '\\"');
    execSync(`ctl hook record-decision --data "${json}"`, {
      encoding: "utf-8",
      timeout: 3000,
      stdio: ["pipe", "pipe", "pipe"],
    });
  } catch { /* best effort */ }
}

export default function (pi: HookAPI): void {

  // ═══════════════════════════════════════════
  // 1. CONTEXT — inject session context on every LLM call
  // ═══════════════════════════════════════════
  pi.on("context", async (_event) => {
    const ctx = ctl("context");
    if (!ctx) return;

    const lines = ["# Control Plane Context", ""];
    lines.push(`Binary: ${ctx.binary || "ctl"}`);
    if (ctx.tasks) {
      const t = ctx.tasks;
      lines.push(`Tasks: ${t.total} total` +
        (t.by_phase?.Completed ? `, ${t.by_phase.Completed} completed` : "") +
        (t.by_phase?.InProgress ? `, ${t.by_phase.InProgress} in-progress` : ""));
    }
    lines.push("");
    if (ctx.active_tasks?.length > 0) {
      lines.push("## Active Tasks");
      for (const t of ctx.active_tasks) {
        lines.push(`  - **${t.id}**: ${t.objective}`);
      }
      lines.push("");
    }
    if (ctx.spec_layers?.length > 0) {
      lines.push("## Spec Layers: " + ctx.spec_layers.join(", "));
      lines.push("");
    }
    lines.push("Control plane commands: ctl <subcommand>");

    return {
      messages: [
        { role: "user", content: [{ type: "text", text: lines.join("\n") }], timestamp: Date.now() },
      ],
    };
  });

  // ═══════════════════════════════════════════
  // 2. BEFORE AGENT START — task breadcrumb
  // ═══════════════════════════════════════════
  pi.on("before_agent_start", async (_event) => {
    const bc = ctl("breadcrumb");
    if (!bc || bc === null) return;

    const text = [`Task: ${bc.task_id} | Phase: ${bc.phase}`, `Next: ${bc.next}`];
    if (bc.hold) text.push("⚠️ HELD — resolve before continuing");

    return {
      message: {
        customType: "ctl-state",
        content: text.join("\n"),
        display: text.join("\n"),
        details: { source: "ctl-hook", taskId: bc.task_id },
      },
    };
  });

  // ═══════════════════════════════════════════
  // 3. TOOL CALL — guard out-of-scope writes
  // ═══════════════════════════════════════════
  pi.on("tool_call", async (event) => {
    const writeTools = ["write", "edit"];
    if (!writeTools.includes(event.toolName)) return;

    const targetPath = event.input?.path || event.input?._i;
    if (!targetPath) return;

    const check = ctlCheck(targetPath);
    if (!check.allowed && check.reason === "out_of_scope") {
      ctlRecord({
        signal: "boundary_reject",
        task_id: check.task_id,
        tool: event.toolName,
        path: targetPath,
        action_taken: "blocked",
      });
      return {
        block: true,
        reason: `ctl write guard: '${targetPath}' is outside write_allow for task '${check.task_id}'. Use /ctl-apply instead.`,
      };
    }
  });

  // ═══════════════════════════════════════════
  // 4. TOOL RESULT — audit ctl commands
  // ═══════════════════════════════════════════
  pi.on("tool_result", async (event) => {
    if (event.toolName !== "bash" || event.isError) return;
    const cmd = String(event.input?.command || "");
    if (!cmd.includes("ctl ")) return;

    ctlRecord({
      signal: "ctl_command",
      source_command: cmd.trim(),
      action_taken: "executed",
    });
  });

  // ═══════════════════════════════════════════
  // 5. TURN END — record M5 decision data
  // ═══════════════════════════════════════════
  pi.on("turn_end", async (_event) => {
    const bc = ctl("breadcrumb");
    if (!bc || bc.phase !== "InProgress") return;

    ctlRecord({
      signal: "turn_end",
      task_id: bc.task_id,
      phase_at_decision: bc.phase,
      action_taken: "clean_continue",
    });
  });

  // ═══════════════════════════════════════════
  // 6. AGENT END — check for held tasks
  // ═══════════════════════════════════════════
  pi.on("agent_end", async (_event) => {
    const bc = ctl("breadcrumb");
    if (!bc) return;

    if (bc.hold) {
      ctlRecord({
        signal: "task_held_after_agent",
        task_id: bc.task_id,
        phase: bc.phase,
        action_taken: "needs_attention",
      });
    }
  });

  // ═══════════════════════════════════════════
  // 7. SESSION SHUTDOWN — remind about unfinished tasks
  // ═══════════════════════════════════════════
  pi.on("session_shutdown", async (_event) => {
    const ctx = ctl("context");
    if (!ctx?.tasks?.by_phase) return;

    const inProgress = ctx.tasks.by_phase.InProgress || 0;
    const inReview = ctx.tasks.by_phase.Review || 0;

    if ((inProgress + inReview) > 0 && typeof process.stderr?.write === "function") {
      process.stderr.write(
        `\n⚠️ Unfinished tasks: ${inProgress} in-progress, ${inReview} in review\n` +
        `Run 'ctl task status --id <id>' to check.\n`
      );
    }
  });
}
