/**
 * Contract tests for the ctl opencode gate plugin. Run with `bun test`.
 *
 * Most tests exercise the plugin's pure logic and its hooks over an injected
 * fake `CtlRunner`, so they are deterministic and need neither a `ctl` binary
 * nor a model. The final test drives the REAL exported plugin hook (with `ctl`
 * made unreachable) to prove the wiring — not just the extracted helpers —
 * fails closed.
 */
import { test, expect } from "bun:test";
import {
  isMutating,
  extractGateInput,
  buildGateArgs,
  buildContextMessage,
  shouldRecord,
  buildRecordArgs,
  buildDispatchArgs,
  createHooks,
  resolveCtlBin,
  type CtlRunner,
  type GateResult,
} from "./ctl-gate.ts";
import CtlGate from "./ctl-gate.ts";

async function caught(fn: () => Promise<unknown>): Promise<Error | null> {
  try {
    await fn();
    return null;
  } catch (e) {
    return e as Error;
  }
}

const noRecord = async () => {};

const allow: CtlRunner = {
  gate: async () => ({ allowed: true, state: "in_progress", reason: "" }),
  context: async () => null,
  recordDecision: noRecord,
  recordDispatch: noRecord,
};
const deny: CtlRunner = {
  gate: async () => ({
    allowed: false,
    state: "in_progress",
    reason: "outside write_allow",
    remedy: "ctl task revise --id ...",
  }),
  context: async () => null,
  recordDecision: noRecord,
  recordDispatch: noRecord,
};
const down: CtlRunner = {
  gate: async () => null,
  context: async () => null,
  recordDecision: noRecord,
  recordDispatch: noRecord,
};

/** A runner returning a fixed verdict that captures recordDecision + recordDispatch argvs. */
function recordingRunner(gate: GateResult | null): {
  runner: CtlRunner;
  calls: string[][];
  dispatchCalls: string[][];
} {
  const calls: string[][] = [];
  const dispatchCalls: string[][] = [];
  return {
    calls,
    dispatchCalls,
    runner: {
      gate: async () => gate,
      context: async () => null,
      recordDecision: async (args) => {
        calls.push(args);
      },
      recordDispatch: async (args) => {
        dispatchCalls.push(args);
      },
    },
  };
}

test("isMutating: write/edit/patch/bash/task are mutating; read-only is not", () => {
  for (const t of ["write", "edit", "patch", "bash", "task"]) expect(isMutating(t)).toBe(true);
  for (const t of ["read", "grep", "glob", "list", "webfetch"]) expect(isMutating(t)).toBe(false);
});

test("extractGateInput maps opencode tools to ctl gate inputs", () => {
  expect(extractGateInput("write", { filePath: "a.txt" })).toMatchObject({ ctlTool: "write", path: "a.txt" });
  expect(extractGateInput("edit", { filePath: "b.rs" })).toMatchObject({ ctlTool: "write", path: "b.rs" });
  expect(extractGateInput("patch", { filePath: "c" })).toMatchObject({ ctlTool: "write" });
  expect(extractGateInput("bash", { command: "ls -la" })).toMatchObject({ ctlTool: "bash", command: "ls -la" });
  expect(extractGateInput("task", { subagent_type: "explore" })).toMatchObject({ ctlTool: "task", agentType: "explore" });
  expect(extractGateInput("read", { filePath: "x" }).ctlTool).toBeNull();
});

test("buildGateArgs is array form; a path with spaces/quotes/$ stays ONE argument", () => {
  const weird = 'a b/c$d "e" & f.txt';
  const args = buildGateArgs({ ctlTool: "write", path: weird });
  expect(Array.isArray(args)).toBe(true);
  expect(args).toEqual(["hook", "gate", "--tool", "write", "--path", weird]);
  // the path is exactly one element — no shell splitting on space/&/$/quote
  expect(args[args.indexOf("--path") + 1]).toBe(weird);
});

test("buildGateArgs forwards --task only when a non-blank taskId is present", () => {
  expect(buildGateArgs({ ctlTool: "bash", command: "ls", taskId: "t1" })).toContain("--task");
  expect(buildGateArgs({ ctlTool: "bash", command: "ls" })).not.toContain("--task");
  expect(buildGateArgs({ ctlTool: "bash", command: "ls", taskId: "   " })).not.toContain("--task");
});

test("ctl allow → tool is not blocked", async () => {
  const h = createHooks(allow);
  expect(await caught(() => h["tool.execute.before"]({ tool: "write" }, { args: { filePath: ".opencode/x" } }))).toBeNull();
});

test("ctl deny → throws, preserving reason and remedy for diagnosis", async () => {
  const h = createHooks(deny);
  const err = await caught(() => h["tool.execute.before"]({ tool: "write" }, { args: { filePath: "src/x" } }));
  expect(err).not.toBeNull();
  expect(err!.message).toContain("outside write_allow");
  expect(err!.message).toContain("ctl task revise");
});

test("ctl unavailable → every mutating tool fails closed", async () => {
  const h = createHooks(down);
  const cases: Array<[string, Record<string, unknown>]> = [
    ["write", { filePath: "src/x" }],
    ["edit", { filePath: "src/x" }],
    ["patch", { filePath: "src/x" }],
    ["bash", { command: "ls" }],
    ["task", { subagent_type: "x" }],
  ];
  for (const [tool, args] of cases) {
    const err = await caught(() => h["tool.execute.before"]({ tool }, { args }));
    expect(err).not.toBeNull();
  }
});

test("ctl unavailable → read-only tools are still allowed", async () => {
  const h = createHooks(down);
  expect(await caught(() => h["tool.execute.before"]({ tool: "read" }, { args: { filePath: "src/x" } }))).toBeNull();
});

test("buildContextMessage includes scope, phase, and task id; null when no active task", () => {
  expect(buildContextMessage([])).toBeNull();
  expect(buildContextMessage(undefined)).toBeNull();
  const msg = buildContextMessage([
    { id: "t-42", objective: "do x", phase: "in_progress", boundary: { write_allow: ["src/foo"], gates: ["cargo_check"] } },
  ]);
  expect(msg).toContain("t-42"); // task id
  expect(msg).toContain("in_progress"); // phase
  expect(msg).toContain("src/foo"); // scope
});

test("context hook injects nothing when the ledger has no active task", async () => {
  const h = createHooks({ gate: async () => null, context: async () => ({ active_tasks: [] }) });
  const out = { system: [] as string[] };
  await h["experimental.chat.system.transform"]({}, out);
  expect(out.system.length).toBe(0);
});

test("shouldRecord: denies and record-flagged allows are logged; plain allows are not", () => {
  const base = { state: "in_progress", reason: "" };
  expect(shouldRecord({ allowed: false, ...base })).toBe(true); // any deny
  expect(shouldRecord({ allowed: true, record: true, ...base })).toBe(true); // bash_write allow
  expect(shouldRecord({ allowed: true, ...base })).toBe(false); // ordinary allow
  expect(shouldRecord({ allowed: true, record: false, ...base })).toBe(false);
});

test("buildRecordArgs: array form, source=opencode, carries path/command + task_id", () => {
  const denyArgs = buildRecordArgs({
    tool: "write",
    gate: { allowed: false, state: "in_progress", reason: "outside write_allow", task_id: "t-9" },
    path: "src/x.rs",
  });
  expect(denyArgs.slice(0, 3)).toEqual(["hook", "record-decision", "--data"]);
  const payload = JSON.parse(denyArgs[3]);
  expect(payload).toMatchObject({
    source: "opencode",
    tool: "write",
    allowed: false,
    state: "in_progress",
    reason: "outside write_allow",
    path: "src/x.rs",
    task_id: "t-9",
  });

  // bash records `command`, not `path`; taskId falls back to the dispatch arg.
  const bashArgs = buildRecordArgs({
    tool: "bash",
    gate: { allowed: true, record: true, state: "in_progress", reason: "bash write allowed" },
    command: "echo hi > src/x.rs",
    taskId: "t-env",
  });
  const bashPayload = JSON.parse(bashArgs[3]);
  expect(bashPayload.command).toBe("echo hi > src/x.rs");
  expect(bashPayload.path).toBeUndefined();
  expect(bashPayload.task_id).toBe("t-env");

  // Observe mode: an allowed verdict's warning text travels into the record.
  const observeArgs = buildRecordArgs({
    tool: "write",
    gate: {
      allowed: true,
      record: true,
      state: "idle",
      reason: "no active in_progress task (observe mode)",
      warning: "ungoverned write recorded",
    },
    path: "notes.md",
  });
  expect(JSON.parse(observeArgs[3]).warning).toBe("ungoverned write recorded");
});

test("hook records a DENY before throwing", async () => {
  const { runner, calls } = recordingRunner({
    allowed: false,
    state: "in_progress",
    reason: "outside write_allow",
  });
  const h = createHooks(runner);
  const err = await caught(() =>
    h["tool.execute.before"]({ tool: "write" }, { args: { filePath: "src/x" } }),
  );
  expect(err).not.toBeNull(); // still blocks
  expect(calls.length).toBe(1);
  expect(calls[0].slice(0, 2)).toEqual(["hook", "record-decision"]);
  expect(JSON.parse(calls[0][3])).toMatchObject({ allowed: false, tool: "write", path: "src/x" });
});

test("hook records a bash_write ALLOW (record=true) and still allows it", async () => {
  const { runner, calls } = recordingRunner({
    allowed: true,
    record: true,
    state: "in_progress",
    reason: "bash write allowed under active task — NOT path-scope-checked",
  });
  const h = createHooks(runner);
  const err = await caught(() =>
    h["tool.execute.before"]({ tool: "bash" }, { args: { command: "echo hi > src/x.rs" } }),
  );
  expect(err).toBeNull(); // allowed
  expect(calls.length).toBe(1);
  expect(JSON.parse(calls[0][3])).toMatchObject({ allowed: true, tool: "bash", command: "echo hi > src/x.rs" });
});

test("hook does NOT record an ordinary allow", async () => {
  const { runner, calls } = recordingRunner({ allowed: true, state: "in_progress", reason: "within write_allow" });
  const h = createHooks(runner);
  await h["tool.execute.before"]({ tool: "write" }, { args: { filePath: ".opencode/x" } });
  expect(calls.length).toBe(0);
});

/** Run `fn` with CTL_TASK_ID set to `id` (or unset when undefined), then restore. */
async function withTaskId(id: string | undefined, fn: () => Promise<void>): Promise<void> {
  const saved = process.env.CTL_TASK_ID;
  if (id === undefined) delete process.env.CTL_TASK_ID;
  else process.env.CTL_TASK_ID = id;
  try {
    await fn();
  } finally {
    if (saved === undefined) delete process.env.CTL_TASK_ID;
    else process.env.CTL_TASK_ID = saved;
  }
}

test("buildDispatchArgs: array form — ctl dispatch record with trimmed task, role, adapter", () => {
  expect(buildDispatchArgs({ taskId: "  t-7  ", role: "designer" })).toEqual([
    "dispatch",
    "record",
    "--task",
    "t-7",
    "--role",
    "designer",
    "--adapter",
    "opencode",
  ]);
});

test("hook records a dispatch on an ALLOWED task spawn bound to a parent task", async () => {
  const { runner, dispatchCalls } = recordingRunner({
    allowed: true,
    state: "in_progress",
    reason: "",
  });
  const h = createHooks(runner);
  await withTaskId("t-42", () =>
    h["tool.execute.before"]({ tool: "task" }, { args: { subagent_type: "designer" } }),
  );
  expect(dispatchCalls.length).toBe(1);
  expect(dispatchCalls[0]).toEqual([
    "dispatch",
    "record",
    "--task",
    "t-42",
    "--role",
    "designer",
    "--adapter",
    "opencode",
  ]);
});

test("hook does NOT record a dispatch when no parent task is bound", async () => {
  const { runner, dispatchCalls } = recordingRunner({
    allowed: true,
    state: "in_progress",
    reason: "",
  });
  const h = createHooks(runner);
  await withTaskId(undefined, () =>
    h["tool.execute.before"]({ tool: "task" }, { args: { subagent_type: "designer" } }),
  );
  expect(dispatchCalls.length).toBe(0);
});

test("hook does NOT record a dispatch for a non-task tool", async () => {
  const { runner, dispatchCalls } = recordingRunner({
    allowed: true,
    state: "in_progress",
    reason: "",
  });
  const h = createHooks(runner);
  await withTaskId("t-42", () =>
    h["tool.execute.before"]({ tool: "write" }, { args: { filePath: ".opencode/x" } }),
  );
  expect(dispatchCalls.length).toBe(0);
});

test("hook does NOT record a dispatch when the task spawn is denied", async () => {
  const { runner, dispatchCalls } = recordingRunner({
    allowed: false,
    state: "idle",
    reason: "no active task",
  });
  const h = createHooks(runner);
  await withTaskId("t-42", async () => {
    const err = await caught(() =>
      h["tool.execute.before"]({ tool: "task" }, { args: { subagent_type: "designer" } }),
    );
    expect(err).not.toBeNull(); // denied → throws before recording a dispatch
  });
  expect(dispatchCalls.length).toBe(0);
});

test("resolveCtlBin: CTL_BIN override is returned verbatim (resolution priority #1)", () => {
  const saved = process.env.CTL_BIN;
  process.env.CTL_BIN = "/custom/path/to/ctl.exe";
  try {
    expect(resolveCtlBin()).toBe("/custom/path/to/ctl.exe");
  } finally {
    if (saved === undefined) delete process.env.CTL_BIN;
    else process.env.CTL_BIN = saved;
  }
});

test("REAL exported plugin hook fails closed when ctl is unreachable", async () => {
  const hooks = (await CtlGate({} as never)) as ReturnType<typeof createHooks>;
  // Point CTL_BIN at a bogus binary: the resolver now finds a globally-installed
  // ctl regardless of PATH, so breaking PATH alone no longer makes it unreachable —
  // CTL_BIN (priority #1) is the deterministic way to force an unrunnable binary.
  const saved = process.env.CTL_BIN;
  process.env.CTL_BIN = "C:\\__no_ctl_here__\\ctl.exe";
  try {
    const errWrite = await caught(() => hooks["tool.execute.before"]({ tool: "write" }, { args: { filePath: ".opencode/x" } }));
    const errRead = await caught(() => hooks["tool.execute.before"]({ tool: "read" }, { args: { filePath: ".opencode/x" } }));
    expect(errWrite).not.toBeNull(); // mutating → blocked
    expect(errRead).toBeNull(); // read-only → allowed
  } finally {
    if (saved === undefined) delete process.env.CTL_BIN;
    else process.env.CTL_BIN = saved;
  }
});
