import { randomUUID } from "node:crypto";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export const AFK_DIR = ".pi/afk";
export const ACTIVE_RESULTS_REL = `${AFK_DIR}/active-results.json`;
export const RESULTS_REL = `${AFK_DIR}/results`;

export type Phase = "implement" | "quality" | "verify";
export type Role = "implementer" | "quality" | "verifier";

export type RoleResult = {
	status: "pass" | "needs-info";
	summary: string;
	reason?: string;
};

export type VerifyResult = {
	status: "pass" | "fail" | "needs-info";
	summary: string;
	feedback: string;
	commands_run: string[];
	commit: string;
};

export type AfkResultKind = "role" | "verify";

export type ActiveResultToken = {
	kind: AfkResultKind;
	issue: number;
	role: Role;
	phase: Phase;
	cycle: number;
	createdAt: string;
};

type ActiveResults = {
	tokens: Record<string, ActiveResultToken>;
};

type WrappedResult<T> = ActiveResultToken & {
	token: string;
	completedAt: string;
	result: T;
};

function nowIso() {
	return new Date().toISOString();
}

async function readJson<T>(filePath: string): Promise<T> {
	return JSON.parse(await fs.readFile(filePath, "utf8")) as T;
}

async function writeJson(filePath: string, value: unknown) {
	await fs.mkdir(path.dirname(filePath), { recursive: true });
	await fs.writeFile(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

async function exists(filePath: string) {
	try {
		await fs.access(filePath);
		return true;
	} catch {
		return false;
	}
}

async function unlinkIfExists(filePath: string) {
	try {
		await fs.unlink(filePath);
	} catch (err) {
		if ((err as NodeJS.ErrnoException).code !== "ENOENT") throw err;
	}
}

async function writeJsonAtomicNoOverwrite(filePath: string, value: unknown) {
	await fs.mkdir(path.dirname(filePath), { recursive: true });
	const tmpPath = `${filePath}.tmp-${process.pid}-${randomUUID()}`;
	await fs.writeFile(tmpPath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
	try {
		await fs.link(tmpPath, filePath);
	} catch (err) {
		if ((err as NodeJS.ErrnoException).code === "EEXIST") {
			throw new Error(`AFK result already exists for token: ${path.basename(filePath, ".json")}`);
		}
		throw err;
	} finally {
		await unlinkIfExists(tmpPath);
	}
}

async function loadActiveResults(cwd: string): Promise<ActiveResults> {
	const registryPath = path.join(cwd, ACTIVE_RESULTS_REL);
	if (!(await exists(registryPath))) return { tokens: {} };
	const parsed = await readJson<Partial<ActiveResults>>(registryPath);
	return { tokens: parsed.tokens ?? {} };
}

async function saveActiveResults(cwd: string, registry: ActiveResults) {
	await writeJson(path.join(cwd, ACTIVE_RESULTS_REL), registry);
}

export async function registerResultToken(cwd: string, token: string, meta: ActiveResultToken) {
	const registry = await loadActiveResults(cwd);
	registry.tokens[token] = meta;
	await saveActiveResults(cwd, registry);
}

export async function removeResultToken(cwd: string, token: string) {
	const registry = await loadActiveResults(cwd);
	delete registry.tokens[token];
	await saveActiveResults(cwd, registry);
}

function resultFilePath(cwd: string, token: string) {
	return path.join(cwd, RESULTS_REL, `${token}.json`);
}

function assertObject(value: unknown, label: string): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object.`);
	return value as Record<string, unknown>;
}

function nonEmptyString(value: unknown, label: string): string {
	if (typeof value !== "string" || value.trim().length === 0) throw new Error(`${label} must be a non-empty string.`);
	return value;
}

function optionalString(value: unknown, label: string): string | undefined {
	if (value === undefined) return undefined;
	if (typeof value !== "string") throw new Error(`${label} must be a string when present.`);
	return value;
}

function validateWrappedResult<T>(value: unknown, expected: ActiveResultToken & { token: string }): WrappedResult<T> {
	const wrapper = assertObject(value, "AFK result wrapper") as Partial<WrappedResult<T>>;
	if (wrapper.token !== expected.token) throw new Error("AFK result token mismatch.");
	if (wrapper.kind !== expected.kind) throw new Error("AFK result kind mismatch.");
	if (wrapper.issue !== expected.issue) throw new Error("AFK result issue mismatch.");
	if (wrapper.role !== expected.role) throw new Error("AFK result role mismatch.");
	if (wrapper.phase !== expected.phase) throw new Error("AFK result phase mismatch.");
	if (wrapper.cycle !== expected.cycle) throw new Error("AFK result cycle mismatch.");
	if (typeof wrapper.completedAt !== "string") throw new Error("AFK result missing completedAt.");
	if (wrapper.result === undefined) throw new Error("AFK result missing result payload.");
	return wrapper as WrappedResult<T>;
}

export function validateRoleResult(value: unknown): RoleResult {
	const parsed = assertObject(value, "AFK role result") as Partial<RoleResult>;
	if (parsed.status !== "pass" && parsed.status !== "needs-info") throw new Error(`Invalid role status: ${String(parsed.status)}`);
	return {
		status: parsed.status,
		summary: nonEmptyString(parsed.summary, "Role summary"),
		reason: optionalString(parsed.reason, "Role reason"),
	};
}

export function validateVerifyResult(value: unknown): VerifyResult {
	const parsed = assertObject(value, "AFK verifier result") as Partial<VerifyResult>;
	if (parsed.status !== "pass" && parsed.status !== "fail" && parsed.status !== "needs-info") {
		throw new Error(`Invalid verifier status: ${String(parsed.status)}`);
	}
	if (!Array.isArray(parsed.commands_run)) throw new Error("Verifier commands_run must be an array.");
	const commit = typeof parsed.commit === "string" ? parsed.commit : "";
	if (parsed.status === "pass" && !/^[0-9a-f]{7,40}$/i.test(commit)) throw new Error(`Verifier pass requires a commit hash: ${commit}`);
	if (parsed.status !== "pass" && commit !== "") throw new Error("Verifier commit must be empty unless status is pass.");
	if (typeof parsed.feedback !== "string") throw new Error("Verifier feedback must be a string.");
	return {
		status: parsed.status,
		summary: nonEmptyString(parsed.summary, "Verifier summary"),
		feedback: parsed.feedback,
		commands_run: parsed.commands_run.map(String),
		commit,
	};
}

export async function consumeStructuredResult<T>(cwd: string, token: string, expected: ActiveResultToken, validate: (value: unknown) => T): Promise<T> {
	const filePath = resultFilePath(cwd, token);
	if (!(await exists(filePath))) {
		throw new Error(`AFK ${expected.role} did not submit structured result via ${expected.kind === "role" ? "afk_role_result" : "afk_verify_result"} token ${token.slice(0, 8)}.`);
	}
	let wrapper: WrappedResult<T>;
	try {
		wrapper = validateWrappedResult<T>(await readJson<unknown>(filePath), { ...expected, token });
		const result = validate(wrapper.result);
		await unlinkIfExists(filePath);
		return result;
	} catch (err) {
		throw new Error(`Invalid AFK structured result at ${path.relative(cwd, filePath)}: ${err instanceof Error ? err.message : String(err)}`);
	}
}

async function submitAfkResult<T>(cwd: string, token: string, kind: AfkResultKind, result: T) {
	const registry = await loadActiveResults(cwd);
	const meta = registry.tokens[token];
	if (!meta) throw new Error("Unknown or inactive AFK result token.");
	if (meta.kind !== kind) throw new Error(`AFK token expects ${meta.kind} result, not ${kind}.`);
	const wrapped: WrappedResult<T> = {
		...meta,
		token,
		completedAt: nowIso(),
		result,
	};
	await writeJsonAtomicNoOverwrite(resultFilePath(cwd, token), wrapped);
}

export function registerAfkResultTools(pi: ExtensionAPI) {
	pi.registerTool({
		name: "afk_role_result",
		label: "AFK Role Result",
		description: "AFK internal result submission. Only use when an AFK prompt gives you a valid token.",
		promptSnippet: "AFK internal result submission for implementer/quality roles with a valid token",
		parameters: {
			type: "object",
			additionalProperties: false,
			required: ["token", "status", "summary"],
			properties: {
				token: { type: "string", description: "AFK result token provided in the role prompt" },
				status: { type: "string", enum: ["pass", "needs-info"] },
				summary: { type: "string", description: "Human-readable phase completion summary" },
				reason: { type: "string", description: "Required when status is needs-info" },
			},
		},
		async execute(_toolCallId: string, params: Record<string, unknown>, _signal: unknown, _onUpdate: unknown, ctx: any) {
			const token = nonEmptyString(params.token, "AFK result token");
			const result = validateRoleResult({
				status: params.status,
				summary: params.summary,
				reason: params.reason,
			});
			await submitAfkResult(ctx.cwd, token, "role", result);
			return {
				content: [{ type: "text", text: "AFK role result received." }],
				details: result,
				terminate: true,
			};
		},
	});

	pi.registerTool({
		name: "afk_verify_result",
		label: "AFK Verify Result",
		description: "AFK internal verifier result submission. Only use when an AFK prompt gives you a valid token.",
		promptSnippet: "AFK internal result submission for verifier roles with a valid token",
		parameters: {
			type: "object",
			additionalProperties: false,
			required: ["token", "status", "summary", "feedback", "commands_run", "commit"],
			properties: {
				token: { type: "string", description: "AFK result token provided in the verifier prompt" },
				status: { type: "string", enum: ["pass", "fail", "needs-info"] },
				summary: { type: "string", description: "Human-readable verification summary" },
				feedback: { type: "string", description: "Exact implementer feedback for fail/needs-info, or pass notes" },
				commands_run: { type: "array", items: { type: "string" } },
				commit: { type: "string", description: "Commit hash on pass, empty string otherwise" },
			},
		},
		async execute(_toolCallId: string, params: Record<string, unknown>, _signal: unknown, _onUpdate: unknown, ctx: any) {
			const token = nonEmptyString(params.token, "AFK result token");
			const result = validateVerifyResult({
				status: params.status,
				summary: params.summary,
				feedback: params.feedback,
				commands_run: params.commands_run,
				commit: params.commit,
			});
			await submitAfkResult(ctx.cwd, token, "verify", result);
			return {
				content: [{ type: "text", text: "AFK verifier result received." }],
				details: result,
				terminate: true,
			};
		},
	});
}
