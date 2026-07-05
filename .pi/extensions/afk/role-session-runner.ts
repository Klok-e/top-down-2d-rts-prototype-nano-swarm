import * as path from "node:path";
import {
	createAgentSession,
	createCodingTools,
	createReadOnlyTools,
	DefaultResourceLoader,
	getAgentDir,
	SessionManager,
} from "@earendil-works/pi-coding-agent";
import { registerAfkResultTools, type Phase, type Role } from "./result-tools.ts";
import { createTranscriptWriter, type TranscriptWriter } from "./transcript.ts";

type ToolActivity = {
	type: "start" | "end";
	toolName: string;
};

type RoleSessionCallbacks = {
	onTextDelta?: (delta: string, fullText: string) => void;
	onToolActivity?: (activity: ToolActivity) => void;
	onTurnEnd?: (turnCount: number) => void;
	onAssistantUsage?: (usage: { input?: number; output?: number; cacheWrite?: number }) => void;
};

export type RoleSessionDiagnostics = {
	assistantMessages: number;
	assistantTextSeen: boolean;
	assistantError?: string;
};

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";

export type StartRoleSessionOptions = RoleSessionCallbacks & {
	cwd: string;
	ctx: any;
	issue: number;
	role: Role;
	phase: Phase;
	cycle: number;
	prompt: string;
	model: string;
	thinkingLevel: ThinkingLevel;
};

export type RoleSessionRun = {
	id: string;
	transcriptPath: string;
	model: string;
	thinkingLevel: ThinkingLevel;
	done: Promise<void>;
	diagnostics(): RoleSessionDiagnostics;
	abort(): Promise<void>;
};

function nowIso() {
	return new Date().toISOString();
}

function extensionCanonicalName(extPath: string): string {
	const base = path.basename(extPath);
	const name = base === "index.ts" || base === "index.js"
		? path.basename(path.dirname(extPath))
		: base.replace(/\.(ts|js)$/, "");
	return name.toLowerCase();
}

function isFullAfkExtension(extPath: string, cwd: string) {
	const resolved = path.resolve(extPath);
	const afkDir = path.resolve(cwd, ".pi/extensions/afk");
	return extensionCanonicalName(resolved) === "afk" && (resolved === path.join(afkDir, "index.ts") || resolved.startsWith(`${afkDir}${path.sep}`));
}

function resolveRoleModel(ctx: any, modelSpec: string) {
	const slash = modelSpec.indexOf("/");
	if (slash <= 0 || slash === modelSpec.length - 1) {
		throw new Error(`AFK role model must be exact provider/modelId, got: ${modelSpec}`);
	}
	const provider = modelSpec.slice(0, slash);
	const modelId = modelSpec.slice(slash + 1);
	const model = ctx.modelRegistry?.find?.(provider, modelId);
	if (!model) throw new Error(`AFK role model not found: ${modelSpec}`);
	return model;
}

function modelWithThinkingLabel(modelLabel: string, thinkingLevel: ThinkingLevel) {
	return `${modelLabel}:${thinkingLevel}`;
}

function collectAllToolNames(loader: DefaultResourceLoader, cwd: string) {
	const builtins = [...new Set([...createCodingTools(cwd), ...createReadOnlyTools(cwd)].map((tool: any) => tool.name))];
	const extensionTools: string[] = [];
	for (const extension of loader.getExtensions().extensions) {
		for (const toolName of extension.tools.keys()) extensionTools.push(toolName);
	}
	return [...new Set([...builtins, ...extensionTools, "afk_role_result", "afk_verify_result"])];
}

function sanitizeForPreview(value: unknown, depth = 0): unknown {
	if (typeof value === "string") return value.length > 8192 ? value.slice(0, 8192) : value;
	if (value === null || typeof value !== "object") return value;
	if (depth >= 3) return `[${Array.isArray(value) ? "array" : "object"}]`;
	if (Array.isArray(value)) return value.slice(0, 12).map((item) => sanitizeForPreview(item, depth + 1));
	const out: Record<string, unknown> = {};
	for (const [key, item] of Object.entries(value as Record<string, unknown>).slice(0, 24)) {
		out[key] = sanitizeForPreview(item, depth + 1);
	}
	return out;
}

function preview(value: unknown) {
	if (value === undefined) return "";
	if (typeof value === "string") return value;
	try {
		return JSON.stringify(sanitizeForPreview(value));
	} catch {
		return String(value);
	}
}

function messageText(message: any) {
	const parts = message?.content;
	if (!Array.isArray(parts)) return "";
	return parts
		.filter((part: any) => part?.type === "text" && typeof part.text === "string")
		.map((part: any) => part.text)
		.join("\n");
}

function assistantErrorText(message: any) {
	const parts: string[] = [];
	if (typeof message?.errorMessage === "string") parts.push(message.errorMessage);
	for (const diagnostic of Array.isArray(message?.diagnostics) ? message.diagnostics : []) {
		if (typeof diagnostic?.error?.message === "string") parts.push(diagnostic.error.message);
		if (diagnostic?.error?.code !== undefined) parts.push(String(diagnostic.error.code));
	}
	const text = messageText(message);
	if (text) parts.push(text);
	return [...new Set(parts.map((part) => part.trim()).filter(Boolean))].join("\n");
}

function usageFromMessage(message: any) {
	const usage = message?.usage;
	if (!usage) return undefined;
	return {
		input: usage.input ?? 0,
		output: usage.output ?? 0,
		cacheWrite: usage.cacheWrite ?? 0,
		cacheRead: usage.cacheRead ?? 0,
		totalTokens: usage.totalTokens ?? undefined,
	};
}

function usageTotal(usage: { input?: number; output?: number; cacheWrite?: number } | undefined) {
	if (!usage) return 0;
	return (usage.input ?? 0) + (usage.output ?? 0) + (usage.cacheWrite ?? 0);
}

function toolName(event: any) {
	return event.toolName ?? event.name ?? event.toolCall?.name ?? "unknown";
}

function toolInput(event: any) {
	return event.input ?? event.arguments ?? event.args ?? event.toolCall?.arguments;
}

function toolOutput(event: any) {
	return event.result ?? event.output ?? event.content ?? event.message ?? event.error;
}

function enqueueWriter(writer: TranscriptWriter) {
	let chain = Promise.resolve();
	const write = (event: Record<string, unknown>) => {
		chain = chain.then(() => writer.write(event)).catch(() => {});
	};
	const flush = async () => {
		await chain;
	};
	return { write, flush };
}

export async function startRoleSession(options: StartRoleSessionOptions): Promise<RoleSessionRun> {
	const { cwd, ctx, issue, role, phase, cycle, prompt, model: modelSpec, thinkingLevel } = options;
	const agentDir = getAgentDir();
	const writer = await createTranscriptWriter(cwd, { issue, role, phase, cycle });
	const { write, flush } = enqueueWriter(writer);
	let discoveredNames: string[] = [];

	const loader = new DefaultResourceLoader({
		cwd,
		agentDir,
		extensionFactories: [(pi: any) => registerAfkResultTools(pi)],
		extensionsOverride: (base: any) => {
			discoveredNames = base.extensions.map((extension: any) => extension.path);
			return {
				...base,
				extensions: base.extensions.filter((extension: any) => !isFullAfkExtension(extension.path, cwd)),
			};
		},
	});
	await loader.reload();

	const model = resolveRoleModel(ctx, modelSpec);
	const modelLabel = `${model.provider}/${model.id}`;
	const displayModelLabel = modelWithThinkingLabel(modelLabel, thinkingLevel);
	const tools = collectAllToolNames(loader, cwd);
	const { session } = await createAgentSession({
		cwd,
		agentDir,
		resourceLoader: loader,
		sessionManager: SessionManager.inMemory(cwd),
		settingsManager: ctx.settingsManager,
		modelRegistry: ctx.modelRegistry,
		model,
		thinkingLevel,
		tools,
	});

	const id = session.sessionId;
	let turnCount = 0;
	let toolUses = 0;
	let tokenTotal = 0;
	let streamedText = "";
	let completed = false;
	let assistantMessages = 0;
	let assistantTextSeen = false;
	let assistantError: string | undefined;

	write({
		type: "start",
		issue,
		role,
		phase,
		cycle,
		sessionId: id,
		model: modelLabel,
		thinkingLevel,
		displayModel: displayModelLabel,
		toolCount: tools.length,
		excludedExtensions: discoveredNames.filter((name) => isFullAfkExtension(name, cwd)),
	});

	const unsubscribe = session.subscribe((event: any) => {
		try {
			if (event.type === "message_update" && event.assistantMessageEvent?.type === "text_delta") {
				const delta = String(event.assistantMessageEvent.delta ?? "");
				streamedText += delta;
				options.onTextDelta?.(delta, streamedText);
				return;
			}

			if (event.type === "tool_execution_start") {
				const name = toolName(event);
				options.onToolActivity?.({ type: "start", toolName: name });
				write({ type: "tool_call", tool: name, args_preview: preview(toolInput(event)) });
				return;
			}

			if (event.type === "tool_execution_end") {
				const name = toolName(event);
				toolUses++;
				options.onToolActivity?.({ type: "end", toolName: name });
				write({
					type: "tool_result",
					tool: name,
					ok: event.isError === undefined ? undefined : !event.isError,
					summary: event.isError ? "error" : "completed",
					output_excerpt: preview(toolOutput(event)),
				});
				return;
			}

			if (event.type === "message_end") {
				const message = event.message;
				if (message?.role === "assistant") {
					assistantMessages++;
					const text = messageText(message);
					if (text) {
						assistantTextSeen = true;
						write({ type: "assistant_text", text });
					}
					if (message.stopReason === "error") {
						assistantError = assistantErrorText(message) || "assistant message ended with error";
						write({ type: "assistant_error", message: assistantError, stopReason: message.stopReason });
					}
					const usage = usageFromMessage(message);
					if (usage) {
						tokenTotal += usageTotal(usage);
						options.onAssistantUsage?.(usage);
						write({ type: "usage", usage, tokensTotal: tokenTotal });
					}
				}
				return;
			}

			if (event.type === "turn_end") {
				turnCount = typeof event.turnIndex === "number" ? event.turnIndex + 1 : turnCount + 1;
				options.onTurnEnd?.(turnCount);
				write({
					type: "turn_end",
					turn: turnCount,
					toolResults: Array.isArray(event.toolResults) ? event.toolResults.length : undefined,
				});
				return;
			}

			if (event.type === "agent_end") {
				write({ type: "agent_end", messageCount: Array.isArray(event.messages) ? event.messages.length : undefined });
			}
		} catch {
			// Transcript and live-panel updates must not affect role execution.
		}
	});

	const done = (async () => {
		try {
			await session.prompt(prompt);
			completed = true;
			write({ type: "end", status: "completed", turns: turnCount, toolUses, tokensTotal: tokenTotal, completedAt: nowIso() });
		} catch (err) {
			write({
				type: "error",
				message: err instanceof Error ? err.message : String(err),
				turns: turnCount,
				toolUses,
				tokensTotal: tokenTotal,
			});
			write({ type: "end", status: "error", turns: turnCount, toolUses, tokensTotal: tokenTotal, completedAt: nowIso() });
			throw err;
		} finally {
			unsubscribe();
			await flush();
			await writer.close();
			session.dispose();
		}
	})();

	return {
		id,
		transcriptPath: writer.path,
		model: displayModelLabel,
		thinkingLevel,
		done,
		diagnostics() {
			return { assistantMessages, assistantTextSeen, assistantError };
		},
		async abort() {
			if (completed) return;
			await session.abort();
		},
	};
}
