// ctl Control Plane Hook — OMP native extension
// Three concerns: write guard, spec drift, unfinished task reminder.
// All logic in Rust (ctl hook). This is a thin async wrapper.

import type { HookAPI } from "@oh-my-pi/pi-coding-agent/extensibility/hooks";
import { execFile } from "child_process";

function ctlAsync(subcommand: string): Promise<string | null> {
  return new Promise((resolve) => {
    execFile("ctl", ["hook", ...subcommand.split(" ")], {
      timeout: 5000,
      stdio: ["pipe", "pipe", "pipe"],
    }, (err, stdout) => {
      if (err) { resolve(null); return; }
      resolve(stdout);
    });
  });
}

function ctlSync(subcommand: string): string | null {
  try {
    const { execFileSync } = require("child_process");
    return execFileSync("ctl", ["hook", ...subcommand.split(" ")], {
      encoding: "utf-8", timeout: 5000, stdio: ["pipe", "pipe", "pipe"],
    });
  } catch { return null; }
}

export default function (pi: HookAPI): void {

  // ═══════════════════════════════════════════
  // 1. TOOL CALL — write guard (blocking)
  // ═══════════════════════════════════════════
  pi.on("tool_call", async (event) => {
    if (!["write", "edit"].includes(event.toolName)) return;

    const targetPath = event.input?.path || event.input?._i;
    if (!targetPath) return;

    // Must be sync — tool_call handlers can block
    const raw = ctlSync(`check-write --path "${targetPath.replace(/"/g, '\\"')}"`);
    if (!raw) return;

    let check: any;
    try { check = JSON.parse(raw); } catch { return; }

    if (!check.allowed && check.reason === "out_of_scope") {
      return {
        block: true,
        reason: `ctl: '${targetPath}' is outside write_allow for task '${check.task_id}'. Use /ctl-apply.`,
      };
    }
  });

  // ═══════════════════════════════════════════
  // 2. AGENT END — spec drift detection
  // ═══════════════════════════════════════════
  pi.on("agent_end", async () => {
    const raw = await ctlAsync("spec-status");
    if (!raw) return;

    let spec: any;
    try { spec = JSON.parse(raw); } catch { return; }

    if (spec.drift && typeof process.stderr?.write === "function") {
      process.stderr.write(
        `\n📝 Specs stale (${spec.source_files} source > ${spec.spec_files} specs). Run /ctl-spec-bootstrap.\n`
      );
    }
  });

  // ═══════════════════════════════════════════
  // 3. SESSION SHUTDOWN — unfinished task reminder
  // ═══════════════════════════════════════════
  pi.on("session_shutdown", async () => {
    // Check unfinished tasks
    const raw = await ctlAsync("context");
    if (raw) {
      try {
        const ctx = JSON.parse(raw);
        const ip = ctx.tasks?.by_phase?.InProgress || 0;
        const rv = ctx.tasks?.by_phase?.Review || 0;
        if ((ip + rv) > 0 && typeof process.stderr?.write === "function") {
          process.stderr.write(`\n⚠️ Unfinished: ${ip} in-progress, ${rv} in review. 'ctl task status --id <id>'.\n`);
        }
      } catch { /* ignore */ }
    }

    // Check spec staleness
    const specRaw = await ctlAsync("spec-status");
    if (specRaw) {
      try {
        const spec = JSON.parse(specRaw);
        if (spec.drift && typeof process.stderr?.write === "function") {
          process.stderr.write(`\n📝 Specs stale. Run /ctl-spec-bootstrap to refresh.\n`);
        }
      } catch { /* ignore */ }
    }
  });
}
