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
  createHooks,
  type CtlRunner,
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

const allow: CtlRunner = {
  gate: async () => ({ allowed: true, state: "in_progress", reason: "" }),
  context: async () => null,
};
const deny: CtlRunner = {
  gate: async () => ({
    allowed: false,
    state: "in_progress",
    reason: "outside write_allow",
    remedy: "ctl task revise --id ...",
  }),
  context: async () => null,
};
const down: CtlRunner = { gate: async () => null, context: async () => null };

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

test("REAL exported plugin hook fails closed when ctl is unreachable", async () => {
  const hooks = (await CtlGate({} as never)) as ReturnType<typeof createHooks>;
  const saved = process.env.PATH;
  process.env.PATH = "C:\\__no_ctl_here__"; // make `ctl` unresolvable
  try {
    const errWrite = await caught(() => hooks["tool.execute.before"]({ tool: "write" }, { args: { filePath: ".opencode/x" } }));
    const errRead = await caught(() => hooks["tool.execute.before"]({ tool: "read" }, { args: { filePath: ".opencode/x" } }));
    expect(errWrite).not.toBeNull(); // mutating → blocked
    expect(errRead).toBeNull(); // read-only → allowed
  } finally {
    process.env.PATH = saved;
  }
});
