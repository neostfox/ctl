import { expect, test } from "bun:test";

import guardrails from "../hooks/pre/guardrails";

function createHarness() {
	const handlers: Record<string, (event: any, ctx?: any) => any> = {};
	const commands: Record<string, { handler: (args: string, ctx?: any) => any }> = {};
	const entries: Array<{ customType: string; data: any }> = [];
	const messages: string[] = [];
	const notices: Array<{ message: string; type: string }> = [];
	const ctx = {
		ui: {
			notify: (message: string, type: string) => notices.push({ message, type }),
		},
	};

	guardrails({
		on: (event, handler) => {
			handlers[event] = handler;
		},
		registerCommand: (name, command) => {
			commands[name] = command;
		},
		appendEntry: (customType, data) => {
			entries.push({ customType, data });
		},
		sendUserMessage: message => {
			messages.push(message);
		},
	});

	return { commands, ctx, entries, handlers, messages, notices };
}

test("requires a human-scoped implementation contract for writes", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);

	expect(harness.handlers.tool_call({ toolName: "write", input: { path: "src/domain/task.rs" } })).toMatchObject({
		block: true,
	});

	await harness.commands["guardrails-implement"].handler("implement reducer invariants | src/domain/**", harness.ctx);

	expect(harness.handlers.tool_call({ toolName: "write", input: { path: "src/domain/task.rs" } })).toBeUndefined();
	expect(harness.handlers.tool_call({ toolName: "write", input: { path: "src/cli/main.rs" } })).toMatchObject({
		block: true,
	});
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo check" } })).toMatchObject({
		block: true,
	});
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo check --offline" } })).toMatchObject({
		block: true,
	});
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo check --locked --offline" } })).toBeUndefined();

	await harness.handlers.session_start({}, harness.ctx);
	expect(harness.handlers.tool_call({ toolName: "write", input: { path: "src/domain/task.rs" } })).toMatchObject({
		block: true,
	});
});

test("enforces scope for real OMP edit input and fails closed for unknown shapes", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);
	await harness.commands["guardrails-implement"].handler("implement reducer | src/domain/**", harness.ctx);

	const allowedEdit = { toolName: "edit", input: { input: "¶src/domain/task.rs#ABCD\nreplace 1..1:\n+updated\n" } };
	expect(harness.handlers.tool_call(allowedEdit)).toBeUndefined();
	await harness.handlers.tool_result({ ...allowedEdit, isError: false }, harness.ctx);

	expect(harness.handlers.tool_call({
		toolName: "edit",
		input: { input: "¶src/cli/mod.rs#ABCD\nreplace 1..1:\n+updated\n" },
	})).toMatchObject({ block: true });

	expect(harness.handlers.tool_call({
		toolName: "edit",
		input: { input: "¶Cargo.toml#ABCD\nreplace 1..1:\n+updated\n" },
	})).toMatchObject({ block: true });

	expect(harness.handlers.tool_call({
		toolName: "edit",
		input: { input: "replace 1..1:\n+updated\n" },
	})).toMatchObject({ block: true });
});

test("enforces scope for ast_edit paths arrays", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);
	await harness.commands["guardrails-implement"].handler("codemod reducer | src/domain/**", harness.ctx);

	expect(harness.handlers.tool_call({
		toolName: "ast_edit",
		input: { ops: [], paths: ["src/domain/task.rs"] },
	})).toBeUndefined();

	expect(harness.handlers.tool_call({
		toolName: "ast_edit",
		input: { ops: [], paths: ["src/domain/task.rs", "src/cli/mod.rs"] },
	})).toMatchObject({ block: true });

	expect(harness.handlers.tool_call({
		toolName: "ast_edit",
		input: { ops: [], paths: ["Cargo.toml"] },
	})).toMatchObject({ block: true });

	expect(harness.handlers.tool_call({
		toolName: "ast_edit",
		input: { ops: [] },
	})).toMatchObject({ block: true });
});

test("allows audit verification commands in explore mode without opening arbitrary shell", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);

	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo fmt --check" } })).toBeUndefined();
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo check --locked --offline" } })).toBeUndefined();
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo test --locked --offline" } })).toBeUndefined();
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo clippy --locked --offline -- -D warnings" } })).toBeUndefined();
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo run --locked --offline -- architecture check" } })).toBeUndefined();
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "git status --short" } })).toBeUndefined();
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "git diff --stat" } })).toBeUndefined();
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo test --offline" } })).toMatchObject({
		block: true,
	});
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo test" } })).toMatchObject({
		block: true,
	});
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "ls -R" } })).toMatchObject({
		block: true,
	});
	expect(harness.handlers.tool_call({ toolName: "bash", input: { command: "cargo test --locked --offline && cargo clippy --locked --offline" } })).toMatchObject({
		block: true,
	});
});

test("rejects shell arguments that can write files or escape the workspace", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);

	expect(harness.handlers.tool_call({
		toolName: "bash",
		input: { command: "git diff --output=src/domain/task.rs --no-index a b" },
	})).toMatchObject({ block: true });
	expect(harness.handlers.tool_call({
		toolName: "bash",
		input: { command: "cargo run --locked --offline --manifest-path C:\\outside\\Cargo.toml" },
	})).toMatchObject({ block: true });
	expect(harness.handlers.tool_call({
		toolName: "bash",
		input: { command: "cargo build --locked --offline --target-dir src/domain/generated" },
	})).toMatchObject({ block: true });
	expect(harness.handlers.tool_call({
		toolName: "bash",
		input: { command: "cargo test --locked --offline --manifest-path C:\\outside\\Cargo.toml" },
	})).toMatchObject({ block: true });
	expect(harness.handlers.tool_call({
		toolName: "bash",
		input: { command: "\"C:\\outside\\cargo.exe\" check --locked --offline" },
	})).toMatchObject({ block: true });
});

test("triggers audits for review boundaries and mutation batches", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);
	await harness.commands["guardrails-implement"].handler("review schema | schemas/**", harness.ctx);
	expect(harness.notices.at(-1)?.message).toContain("REVIEW 边界需要明确批准");
	await harness.commands["guardrails-review-approve"].handler("review schema | schemas/**", harness.ctx);

	const schemaEvent = { toolName: "write", input: { path: "schemas/control.task.v1.schema.json" } };
	expect(harness.handlers.tool_call(schemaEvent)).toBeUndefined();
	await harness.handlers.tool_result({ ...schemaEvent, isError: false }, harness.ctx);
	expect(harness.handlers.before_agent_start({}).message.content).toContain("mode=audit_hold");
	expect(harness.handlers.tool_call(schemaEvent)).toMatchObject({ block: true });
	await harness.commands["guardrails-implement"].handler("bypass audit | src/domain/**", harness.ctx);
	expect(harness.notices.at(-1)?.message).toContain("audit_hold 已生效");
	await harness.commands["guardrails-resume"].handler("", harness.ctx);

	await harness.commands["guardrails-implement"].handler("implement reducer | src/domain/**", harness.ctx);
	for (let index = 0; index < 5; index += 1) {
		const editEvent = { toolName: "edit", input: { path: "src/domain/task.rs" } };
		expect(harness.handlers.tool_call(editEvent)).toBeUndefined();
		await harness.handlers.tool_result({ ...editEvent, isError: false }, harness.ctx);
	}
	expect(harness.handlers.before_agent_start({}).message.content).toContain("mode=audit_hold");
	expect(harness.handlers.tool_call({ toolName: "edit", input: { path: "src/domain/task.rs" } })).toMatchObject({ block: true });

	const auditTriggers = harness.entries
		.filter(entry => entry.customType === "control-plane-audit-trigger")
		.map(entry => entry.data.trigger);
	expect(auditTriggers).toEqual([
		"检测到 REVIEW 边界修改：schemas/control.task.v1.schema.json",
		"已完成 5 次成功的修改工具调用",
	]);
	expect(harness.messages).toHaveLength(2);
});

test("requires a separate human REVIEW approval for dependency files", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);

	await harness.commands["guardrails-implement"].handler("add dependency | Cargo.toml", harness.ctx);
	expect(harness.notices.at(-1)?.message).toContain("REVIEW 边界需要明确批准");
	expect(harness.handlers.tool_call({ toolName: "write", input: { path: "Cargo.toml" } })).toMatchObject({ block: true });

	await harness.commands["guardrails-review-approve"].handler("add dependency | Cargo.toml", harness.ctx);
	const dependencyEdit = {
		toolName: "edit",
		input: { input: "¶Cargo.toml#ABCD\nreplace 1..1:\n+updated\n" },
	};
	expect(harness.handlers.tool_call(dependencyEdit)).toBeUndefined();
	await harness.handlers.tool_result({ ...dependencyEdit, isError: false }, harness.ctx);
	expect(harness.handlers.before_agent_start({}).message.content).toContain("mode=audit_hold");
	expect(harness.entries.at(-1)).toMatchObject({
		customType: "control-plane-audit-trigger",
		data: {
			observed_files: ["cargo.toml"],
		},
	});
});

test("rejects protected scopes and unknown tools", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);

	await harness.commands["guardrails-implement"].handler("bad scope | .omp/**", harness.ctx);
	expect(harness.notices.at(-1)).toMatchObject({
		message: "实现范围不安全或受保护：.omp/**",
		type: "error",
	});

	await harness.commands["guardrails-implement"].handler("implement reducer | src/domain/**", harness.ctx);
	expect(harness.handlers.tool_call({ toolName: "mcp__node_repl_js", input: {} })).toMatchObject({
		block: true,
	});
});

test("injects the active mode and scope before every agent run", async () => {
	const harness = createHarness();
	await harness.handlers.session_start({}, harness.ctx);

	expect(harness.handlers.before_agent_start({})).toMatchObject({
		message: {
			customType: "control-plane-guardrail-state",
			content: expect.stringContaining("mode=explore"),
			display: false,
		},
	});

	await harness.commands["guardrails-implement"].handler("fix clippy | src/domain/**, src/cli/mod.rs", harness.ctx);
	const implementState = harness.handlers.before_agent_start({});
	expect(implementState.message.content).toContain("mode=implement");
	expect(implementState.message.content).toContain("capability=fix clippy");
	expect(implementState.message.content).toContain("allowed_scopes=src/domain/**, src/cli/mod.rs");
	expect(implementState.message.content).toContain("不要推断、扩大或重试被拦截的 scope");
	expect(implementState.message.content).toContain("不要要求用户手动执行被围栏拦截的 shell 命令来绕过限制");
});
