const blockedWritePrefixes = [
	".git/",
	".omp/",
	".trellis/control/",
];

const blockedWriteExact = ["agents.md", "architecture_guardrails.md"];

const blockedWriteSuffixes = ["/events.jsonl", ".env"];

const reviewBoundaryExact = ["cargo.toml", "cargo.lock"];

const reviewBoundaryPrefixes = ["schemas/"];

const exploreAllowedTools = new Set([
	"ask",
	"ast_grep",
	"bash",
	"find",
	"lsp",
	"read",
	"resolve",
	"search",
	"todo_write",
]);

const implementAllowedTools = new Set([
	...exploreAllowedTools,
	"ast_edit",
	"bash",
	"edit",
	"write",
]);

const blockedShellPatterns: Array<{ pattern: RegExp; reason: string }> = [
	{ pattern: /\bgit\b[^\r\n;&|]*\b(add|branch|checkout|clean|commit|merge|mv|push|rebase|reset|restore|rm|switch|tag)\b/i, reason: "禁止通过 OMP 工具调用修改 Git 状态。" },
	{ pattern: /\bgit\s+reset\s+--hard\b/i, reason: "禁止执行破坏性的 Git reset。" },
	{ pattern: /\bgit\s+checkout\s+--\b/i, reason: "禁止执行破坏性的 Git checkout。" },
	{
		pattern: /\b(cargo\s+(add|install|update)|pip(?:3)?\s+install|uv\s+add|npm\s+install|pnpm\s+(add|install)|yarn\s+(add|install)|bun\s+(add|install)|winget\s+install|rustup\s+(install|update|toolchain|component))\b/i,
		reason: "安装依赖或修改工具链需要用户明确批准。",
	},
	{
		pattern: /\b(curl|wget|irm|iwr|invoke-webrequest|start-bitstransfer)\b/i,
		reason: "通过 shell 访问网络需要用户明确批准。",
	},
	{
		pattern: /\b(rm\s+-rf|remove-item\b[^\n]*\s-recurse\b|del\s+\/s\b|rmdir\s+\/s\b)\b/i,
		reason: "禁止执行递归破坏性文件命令。",
	},
	{
		pattern: /(?:^|\s)(?:["'][^"'\r\n]*[/\\])?cargo(?:\.exe)?["']?\s+(check|test|clippy|run|build)\b(?![^\r\n;&|]*\s--offline\b)/i,
		reason: "OMP 中的 Cargo 构建命令必须使用 --offline，防止隐式下载依赖。",
	},
	{
		pattern: /(?:^|\s)(?:["'][^"'\r\n]*[/\\])?cargo(?:\.exe)?["']?\s+(check|test|clippy|run|build|tree)\b(?![^\r\n;&|]*\s--locked\b)/i,
		reason: "OMP 中的 Cargo 审计命令必须使用 --locked，防止依赖解析时修改 lockfile。",
	},
];

const allowedShellPatterns = [
	/^\s*cargo(?:\.exe)?\s+fmt\s+--check\s*$/i,
	/^\s*cargo(?:\.exe)?\s+check\s+--locked\s+--offline\s*$/i,
	/^\s*cargo(?:\.exe)?\s+test\s+--locked\s+--offline\s*$/i,
	/^\s*cargo(?:\.exe)?\s+clippy\s+--locked\s+--offline\s+--\s+-D\s+warnings\s*$/i,
	/^\s*cargo(?:\.exe)?\s+run\s+--locked\s+--offline\s+--\s+architecture\s+check\s*$/i,
	/^\s*git\s+status\s+--short\s*$/i,
	/^\s*git\s+diff(?:\s+--(?:stat|name-only|name-status|check))?\s*$/i,
];

function normalizePath(value: string): string {
	return value.replaceAll("\\", "/").replace(/^\.\/+/, "").toLowerCase();
}

function hasUnsafePathShape(value: string): boolean {
	const normalized = normalizePath(value);
	return (
		normalized.startsWith("/") ||
		normalized.startsWith("//") ||
		/^[a-z]:\//i.test(normalized) ||
		normalized.split("/").includes("..") ||
		normalized.includes("\0")
	);
}

function collectPaths(value: unknown, key = "", result: string[] = []): string[] {
	if (typeof value === "string") {
		if (/^(path|paths|file|filepath|file_path|filename|target|destination|dest|oldpath|newpath)$/i.test(key)) {
			result.push(value);
		}
		if (/^input$/i.test(key)) {
			const embeddedPath = value.match(/^¶([^#\r\n]+)(?:#[^\r\n]*)?(?:\r?\n|$)/)?.[1];
			if (embeddedPath) result.push(embeddedPath);
		}
		return result;
	}
	if (Array.isArray(value)) {
		for (const item of value) collectPaths(item, key, result);
		return result;
	}
	if (value && typeof value === "object") {
		for (const [childKey, childValue] of Object.entries(value)) {
			collectPaths(childValue, childKey, result);
		}
	}
	return result;
}

function isBlockedWritePath(value: string): boolean {
	const normalized = normalizePath(value);
	return (
		hasUnsafePathShape(value) ||
		blockedWriteExact.includes(normalized) ||
		blockedWritePrefixes.some(prefix => normalized === prefix.slice(0, -1) || normalized.startsWith(prefix)) ||
		blockedWriteSuffixes.some(suffix => normalized === suffix.slice(1) || normalized.endsWith(suffix)) ||
		(normalized.startsWith(".trellis/tasks/") && (normalized.endsWith("/task.json") || normalized.endsWith("/control.json")))
	);
}

function protectedPathMentioned(command: string): boolean {
	const normalized = normalizePath(command);
	return (
		blockedWriteExact.some(path => normalized.includes(path)) ||
		blockedWritePrefixes.some(prefix => normalized.includes(prefix)) ||
		blockedWriteSuffixes.some(suffix => normalized.includes(suffix)) ||
		(normalized.includes(".trellis/tasks/") && (normalized.includes("/task.json") || normalized.includes("/control.json")))
	);
}

function unsafePathMentioned(command: string): boolean {
	const normalized = normalizePath(command);
	return /(^|\s|["'])(\/|\/\/|[a-z]:\/|\.\.\/)/i.test(normalized);
}

function isReviewBoundaryPath(value: string): boolean {
	const normalized = normalizePath(value);
	return (
		reviewBoundaryExact.includes(normalized) ||
		reviewBoundaryPrefixes.some(prefix => normalized === prefix.slice(0, -1) || normalized.startsWith(prefix))
	);
}

function reviewBoundaryMentioned(command: string): boolean {
	const normalized = normalizePath(command);
	return reviewBoundaryExact.some(path => normalized.includes(path)) || reviewBoundaryPrefixes.some(prefix => normalized.includes(prefix));
}

function parseImplementationContract(args: string): { capability: string; scopes: string[] } | { error: string } {
	const [capabilityPart, scopePart, ...extra] = args.split("|");
	const capability = capabilityPart?.trim();
	const rawScopes = scopePart?.split(",").map(scope => scope.trim()).filter(Boolean) ?? [];

	if (!capability || rawScopes.length === 0 || extra.length > 0) {
		return { error: "用法：/guardrails-implement <能力> | <相对文件或目录/**>, ..." };
	}

	const scopes: string[] = [];
	for (const rawScope of rawScopes) {
		const normalized = normalizePath(rawScope);
		const withoutGlob = normalized.endsWith("/**") ? normalized.slice(0, -3) : normalized;
		if (!withoutGlob || hasUnsafePathShape(rawScope) || isBlockedWritePath(withoutGlob)) {
			return { error: `实现范围不安全或受保护：${rawScope}` };
		}
		scopes.push(normalized);
	}

	return { capability, scopes };
}

function isPathWithinScope(value: string, scopes: string[]): boolean {
	const normalized = normalizePath(value);
	return scopes.some(scope => {
		if (scope.endsWith("/**")) {
			const prefix = scope.slice(0, -3);
			return normalized === prefix || normalized.startsWith(`${prefix}/`);
		}
		return normalized === scope;
	});
}

function mutatesFiles(command: string): boolean {
	return /\b(remove-item|set-content|add-content|out-file|move-item|copy-item|rm|mv|cp|del|erase|mkdir|new-item|touch|truncate|tee|sed\s+-i|perl\s+-pi|git\s+(restore|checkout))\b|>>?|2>>?/i.test(command);
}

function isAllowedShellCommand(command: string): boolean {
	return allowedShellPatterns.some(pattern => pattern.test(command));
}

function isMutatingToolCall(event: any): boolean {
	if (["write", "edit", "ast_edit"].includes(event.toolName)) return true;
	return event.toolName === "bash" && mutatesFiles(typeof event.input?.command === "string" ? event.input.command : "");
}

type GuardrailAPI = {
	on: (event: string, handler: (event: any, ctx?: any) => unknown) => void;
	appendEntry?: (customType: string, data: unknown) => unknown | Promise<unknown>;
	registerCommand?: (name: string, command: { description: string; handler: (args: string, ctx?: any) => unknown | Promise<unknown> }) => void;
	sendUserMessage?: (content: string, options?: { deliverAs?: "steer" | "followUp" | "nextTurn" }) => unknown | Promise<unknown>;
};

export default function guardrails(pi: GuardrailAPI) {
	let mode: "explore" | "implement" | "audit_hold" = "explore";
	let capability = "";
	let allowedScopes: string[] = [];
	let reviewApprovedScopes: string[] = [];
	let successfulMutations = 0;
	const touchedFiles = new Set<string>();

	async function persistMode(reason: string) {
		await pi.appendEntry?.("control-plane-mode", {
			schema: "control.omp-mode.v1",
			mode,
			capability,
			allowed_scopes: allowedScopes,
			review_approved_scopes: reviewApprovedScopes,
			reason,
			canonical: false,
		});
	}

	function resetBatch() {
		successfulMutations = 0;
		touchedFiles.clear();
	}

	function activeStateMessage(): string {
		const scopeText = allowedScopes.length > 0 ? allowedScopes.join(", ") : "[无]";
		return [
			"[OMP 围栏状态]",
			`mode=${mode}`,
			`capability=${capability || "[无]"}`,
			`allowed_scopes=${scopeText}`,
			mode === "explore"
				? "当前禁止写入和任意 shell 执行。M0 shell allowlist 中的只读审计命令仍可使用。开始实现前，必须由用户运行 /guardrails-implement。"
				: mode === "audit_hold"
					? "强制只读 audit_hold 已生效。禁止写入、扩大 scope 和批准 REVIEW 边界。运行允许的 --locked --offline 离线审计 gate，报告 STOP / ASK / ALLOW，然后等待用户执行 /guardrails-resume 或 /guardrails-explore。"
					: "只能写入已声明的 scope。不要推断、扩大或重试被拦截的 scope。不要要求用户手动执行被围栏拦截的 shell 命令来绕过限制。REVIEW scope 需要 /guardrails-review-approve。",
		].join("\n");
	}

	async function requestAudit(trigger: string, ctx?: any) {
		const observedFiles = [...touchedFiles].sort();
		resetBatch();
		mode = "audit_hold";
		await persistMode(`进入强制 audit_hold：${trigger}`);

		await pi.appendEntry?.("control-plane-audit-trigger", {
			schema: "control.omp-audit-trigger.v1",
			trigger,
			observed_files: observedFiles,
			canonical: false,
		});

		const observed = observedFiles.length > 0 ? observedFiles.join(", ") : "[bash 修改：检查实际 diff]";
		const message = [
			"[OMP 围栏审计请求]",
			`触发原因：${trigger}。`,
			`本次实现批次观测到的文件：${observed}。`,
			"强制 audit_hold 已生效。立即执行只读围栏审计，对比冻结的实施合同与实际变更，使用文件和行号证据报告 STOP / ASK / ALLOW，然后等待用户执行 /guardrails-resume 或 /guardrails-explore。",
		].join("\n");

		if (pi.sendUserMessage) {
			await pi.sendUserMessage(message, { deliverAs: "steer" });
		} else {
			ctx?.ui?.notify?.(message, "warning");
		}
	}

	pi.registerCommand?.("guardrails-implement", {
		description: "进入限定 scope 的实现模式：/guardrails-implement <能力> | <相对文件或目录/**>, ...",
		handler: async (args, ctx) => {
			if (mode === "audit_hold") {
				ctx?.ui?.notify?.("围栏：audit_hold 已生效。请先执行只读审计，然后使用 /guardrails-resume 或 /guardrails-explore。", "error");
				return;
			}
			const contract = parseImplementationContract(args);
			if ("error" in contract) {
				ctx?.ui?.notify?.(contract.error, "error");
				return;
			}
			const reviewScope = contract.scopes.find(isReviewBoundaryPath);
			if (reviewScope) {
				ctx?.ui?.notify?.(`REVIEW 边界需要明确批准：${reviewScope}。请使用 /guardrails-review-approve <能力> | <scope>。`, "error");
				return;
			}

			mode = "implement";
			capability = contract.capability;
			allowedScopes = contract.scopes;
			reviewApprovedScopes = [];
			resetBatch();
			await persistMode("已启用明确限定 scope 的实施合同。");
			ctx?.ui?.notify?.(`围栏：已为 ${allowedScopes.join(", ")} 启用 implement 模式。`, "info");
		},
	});

	pi.registerCommand?.("guardrails-review-approve", {
		description: "批准精确的 REVIEW 边界实施合同：/guardrails-review-approve <能力> | <相对文件或目录/**>, ...",
		handler: async (args, ctx) => {
			if (mode === "audit_hold") {
				ctx?.ui?.notify?.("围栏：audit_hold 已生效。REVIEW 批准不能绕过强制审计。", "error");
				return;
			}
			const contract = parseImplementationContract(args);
			if ("error" in contract) {
				ctx?.ui?.notify?.(contract.error, "error");
				return;
			}
			const nonReviewScope = contract.scopes.find(scope => !isReviewBoundaryPath(scope));
			if (nonReviewScope) {
				ctx?.ui?.notify?.(`REVIEW 批准只能包含 REVIEW 路径：${nonReviewScope}。普通文件请单独使用 /guardrails-implement。`, "error");
				return;
			}

			mode = "implement";
			capability = contract.capability;
			allowedScopes = contract.scopes;
			reviewApprovedScopes = [...contract.scopes];
			resetBatch();
			await persistMode("已启用明确的 REVIEW 边界实施合同。");
			ctx?.ui?.notify?.(`围栏：已批准 ${allowedScopes.join(", ")} 的 REVIEW 实现。`, "warning");
		},
	});

	pi.registerCommand?.("guardrails-resume", {
		description: "用户审阅 audit_hold 报告后，恢复已冻结的 scope 合同",
		handler: async (_args, ctx) => {
			if (mode !== "audit_hold") {
				ctx?.ui?.notify?.("围栏：当前没有生效中的 audit_hold。", "error");
				return;
			}
			mode = "implement";
			resetBatch();
			await persistMode("用户在 audit_hold 后恢复已冻结的 scope 实施合同。");
			ctx?.ui?.notify?.(`围栏：已为 ${allowedScopes.join(", ")} 恢复 implement 模式。`, "info");
		},
	});

	pi.registerCommand?.("guardrails-explore", {
		description: "返回只读探索模式",
		handler: async (_args, ctx) => {
			mode = "explore";
			capability = "";
			allowedScopes = [];
			reviewApprovedScopes = [];
			resetBatch();
			await persistMode("已启用探索模式。");
			ctx?.ui?.notify?.("围栏：已启用 explore 模式。当前禁止写入和执行。", "info");
		},
	});

	pi.registerCommand?.("guardrails-status", {
		description: "显示当前项目围栏模式和实现 scope",
		handler: async (_args, ctx) => {
			const scopeText = allowedScopes.length > 0 ? allowedScopes.join(", ") : "[无]";
			const reviewText = reviewApprovedScopes.length > 0 ? reviewApprovedScopes.join(", ") : "[无]";
			ctx?.ui?.notify?.(`围栏 mode=${mode}; capability=${capability || "[无]"}; scopes=${scopeText}; review_scopes=${reviewText}`, "info");
		},
	});

	pi.on("session_start", async (_event, ctx) => {
		mode = "explore";
		capability = "";
		allowedScopes = [];
		reviewApprovedScopes = [];
		resetBatch();
		await persistMode("新的 OMP 会话默认进入只读探索模式。");
		ctx?.ui?.notify?.("围栏：explore 模式已生效。编辑前请使用 /guardrails-implement。", "info");
	});

	pi.on("before_agent_start", () => ({
		message: {
			customType: "control-plane-guardrail-state",
			content: activeStateMessage(),
			display: false,
			details: {
				schema: "control.omp-mode-context.v1",
				mode,
				capability,
				allowed_scopes: allowedScopes,
				review_approved_scopes: reviewApprovedScopes,
				canonical: false,
			},
		},
	}));

	pi.on("tool_call", event => {
		const allowedTools = mode === "implement" ? implementAllowedTools : exploreAllowedTools;
		if (!allowedTools.has(event.toolName)) {
			return {
				block: true,
				reason: mode === "explore"
					? "架构围栏：只读 explore 模式已生效。开始实现前，必须由用户运行 /guardrails-implement <能力> | <相对文件或目录/**>, ...。"
					: `架构围栏：implement 模式下已拦截工具 '${event.toolName}'。项目 hook 使用 fail-closed 工具 allowlist。`,
			};
		}

		if (["write", "edit", "ast_edit"].includes(event.toolName)) {
			const paths = collectPaths(event.input);
			if (paths.length === 0) {
				return {
					block: true,
					reason: `架构围栏：无法从工具 '${event.toolName}' 的输入中提取目标路径。为避免 scope 绕过，已按 fail-closed 策略拦截。`,
				};
			}
			const blockedPath = paths.find(isBlockedWritePath);
			if (blockedPath) {
				return {
					block: true,
					reason: `架构围栏：已拦截对受保护路径的写入：${blockedPath}`,
				};
			}
			const outOfScopePath = paths.find(path => !isPathWithinScope(path, allowedScopes));
			if (outOfScopePath) {
				return {
					block: true,
					reason: `架构围栏：已拦截声明 scope 之外的写入：${outOfScopePath}`,
				};
			}
			const unapprovedReviewPath = paths.find(path => isReviewBoundaryPath(path) && !isPathWithinScope(path, reviewApprovedScopes));
			if (unapprovedReviewPath) {
				return {
					block: true,
					reason: `架构围栏：已拦截未经批准的 REVIEW 边界写入：${unapprovedReviewPath}`,
				};
			}
		}

		if (event.toolName === "bash") {
			const command = typeof event.input?.command === "string" ? event.input.command : "";
			for (const { pattern, reason } of blockedShellPatterns) {
				if (pattern.test(command)) {
					return { block: true, reason: `架构围栏：${reason}` };
				}
			}
			if (protectedPathMentioned(command) && mutatesFiles(command)) {
				return {
					block: true,
					reason: "架构围栏：已拦截 shell 对受保护路径的修改。",
				};
			}
			if (unsafePathMentioned(command) && mutatesFiles(command)) {
				return {
					block: true,
					reason: "架构围栏：已拦截使用绝对路径、UNC 路径或父目录路径的 shell 修改。",
				};
			}
			if (mutatesFiles(command)) {
				return {
					block: true,
					reason: "架构围栏：已拦截 shell 直接修改文件。请使用受 scope 限制的 write/edit 工具，以便执行已声明的实施范围。",
				};
			}
			if (!isAllowedShellCommand(command)) {
				return {
					block: true,
					reason: "架构围栏：已拦截 M0 allowlist 之外的 shell 命令。请使用 read/find/search 工具，或允许的离线 Cargo、git status、git diff 模板。",
				};
			}
		}
	});

	pi.on("tool_result", async (event, ctx) => {
		if (event.isError || !isMutatingToolCall(event)) return;

		successfulMutations += 1;
		const paths = collectPaths(event.input);
		for (const path of paths) touchedFiles.add(normalizePath(path));

		const reviewBoundaryPaths = paths.filter(isReviewBoundaryPath);
		const command = typeof event.input?.command === "string" ? event.input.command : "";
		if (reviewBoundaryPaths.length > 0 || reviewBoundaryMentioned(command)) {
			const resources = reviewBoundaryPaths.length > 0 ? reviewBoundaryPaths.join(", ") : "bash 中的依赖或 schema 路径";
			await requestAudit(`检测到 REVIEW 边界修改：${resources}`, ctx);
			return;
		}

		if (successfulMutations >= 5) {
			await requestAudit("已完成 5 次成功的修改工具调用", ctx);
		}
	});
}
