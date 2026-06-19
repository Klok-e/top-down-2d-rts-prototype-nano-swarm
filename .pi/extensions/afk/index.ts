import { randomUUID } from "node:crypto";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const AFK_DIR = ".pi/afk";
const STATE_REL = `${AFK_DIR}/state.json`;
const CONFIG_REL = `${AFK_DIR}/config.json`;
const PROMPTS_REL = `${AFK_DIR}/prompts`;
const READY_LABEL = "ready-for-agent";
const NEEDS_INFO_LABEL = "needs-info";
const EXCLUDED_LABELS = new Set([NEEDS_INFO_LABEL, "ready-for-human", "wontfix"]);

type Phase = "implement" | "quality" | "verify";
type Role = "implementer" | "quality" | "verifier";

type AfkConfig = {
	maxCycles: number;
	roles: Record<Role, {
		agentType: string;
		model?: string;
		description: string;
		maxTurns?: number;
	}>;
};

type AfkState = {
	version: 1;
	issue: number;
	phase: Phase;
	cycle: number;
	activeAgentId?: string;
	feedback: string;
	startedAt: string;
	updatedAt: string;
};

type Issue = {
	number: number;
	title: string;
	body: string;
	labels: Array<{ name: string }>;
	updatedAt?: string;
};

type RoleResult = {
	status: "pass" | "needs-info";
	reason?: string;
};

type VerifyResult = {
	status: "pass" | "fail" | "needs-info";
	summary: string;
	feedback: string;
	commands_run: string[];
	commit: string;
};

type RpcReply<T = unknown> = { success: true; data?: T } | { success: false; error: string };

type SubagentDoneEvent = {
	id: string;
	type: string;
	description: string;
	result?: string;
	error?: string;
	status?: string;
	tokens?: { input: number; output: number; total: number };
	toolUses?: number;
	durationMs?: number;
};

function nowIso() {
	return new Date().toISOString();
}

function usage() {
	return "Usage: /afk run [--all | --issue N] | /afk resume | /afk status | /afk stop";
}

function splitArgs(args: string): string[] {
	return args.trim().length === 0 ? [] : args.trim().split(/\s+/g);
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

async function loadConfig(cwd: string): Promise<AfkConfig> {
	const configPath = path.join(cwd, CONFIG_REL);
	return readJson<AfkConfig>(configPath);
}

async function loadState(cwd: string): Promise<AfkState | null> {
	const statePath = path.join(cwd, STATE_REL);
	if (!(await exists(statePath))) return null;
	return readJson<AfkState>(statePath);
}

async function saveState(cwd: string, state: AfkState) {
	state.updatedAt = nowIso();
	await writeJson(path.join(cwd, STATE_REL), state);
}

async function clearState(cwd: string) {
	const statePath = path.join(cwd, STATE_REL);
	if (await exists(statePath)) await fs.unlink(statePath);
}

async function readPrompt(cwd: string, role: Role): Promise<string> {
	return fs.readFile(path.join(cwd, PROMPTS_REL, `${role}.md`), "utf8");
}

function fillTemplate(template: string, values: Record<string, string | number>) {
	return template.replace(/\{([A-Za-z0-9_]+)\}/g, (match, key) => {
		const value = values[key];
		return value === undefined ? match : String(value);
	});
}

async function exec(pi: ExtensionAPI, cwd: string, command: string, timeout = 120_000) {
	const result = await pi.exec("bash", ["-lc", command], { cwd, timeout });
	if (result.code !== 0) {
		throw new Error(result.stderr.trim() || result.stdout.trim() || `Command failed: ${command}`);
	}
	return result.stdout;
}

async function gh(pi: ExtensionAPI, cwd: string, command: string, timeout = 120_000) {
	return exec(pi, cwd, `source .env 2>/dev/null || true; ${command}`, timeout);
}

function shellQuote(value: unknown) {
	return `'${String(value).replace(/'/g, `'\\''`)}'`;
}

async function listCandidateIssues(pi: ExtensionAPI, cwd: string): Promise<Issue[]> {
	const stdout = await gh(
		pi,
		cwd,
		`gh issue list --state open --label ${shellQuote(READY_LABEL)} --limit 1000 --json number,title,body,labels,updatedAt`,
	);
	return JSON.parse(stdout) as Issue[];
}

async function viewIssue(pi: ExtensionAPI, cwd: string, issueNumber: number): Promise<Issue> {
	const stdout = await gh(pi, cwd, `gh issue view ${issueNumber} --json number,title,body,labels,updatedAt`);
	return JSON.parse(stdout) as Issue;
}

async function issueState(pi: ExtensionAPI, cwd: string, issueNumber: number): Promise<string> {
	const stdout = await gh(pi, cwd, `gh issue view ${issueNumber} --json state --jq .state`);
	return stdout.trim().toUpperCase();
}

function labelNames(issue: Issue) {
	return new Set((issue.labels ?? []).map((l) => l.name));
}

function blockerNumbers(body: string): number[] {
	const lines = body.split(/\r?\n/);
	const blockers: number[] = [];
	let inBlockedBy = false;
	for (const line of lines) {
		if (/^##\s+Blocked by\s*$/i.test(line.trim())) {
			inBlockedBy = true;
			continue;
		}
		if (inBlockedBy && /^##\s+/.test(line)) break;
		if (!inBlockedBy) continue;
		for (const match of line.matchAll(/#(\d+)/g)) blockers.push(Number(match[1]));
	}
	return Array.from(new Set(blockers)).sort((a, b) => a - b);
}

async function isRunnableIssue(pi: ExtensionAPI, cwd: string, issue: Issue): Promise<boolean> {
	const labels = labelNames(issue);
	if (!labels.has(READY_LABEL)) return false;
	for (const label of EXCLUDED_LABELS) if (labels.has(label)) return false;
	for (const blocker of blockerNumbers(issue.body ?? "")) {
		if ((await issueState(pi, cwd, blocker)) !== "CLOSED") return false;
	}
	return true;
}

async function selectIssue(pi: ExtensionAPI, cwd: string): Promise<Issue | null> {
	const candidates = (await listCandidateIssues(pi, cwd)).sort((a, b) => a.number - b.number);
	for (const issue of candidates) {
		if (await isRunnableIssue(pi, cwd, issue)) return issue;
	}
	return null;
}

async function ensureCleanForStart(pi: ExtensionAPI, cwd: string) {
	const stdout = await exec(pi, cwd, "git status --porcelain", 30_000);
	const dirty = stdout
		.split(/\r?\n/)
		.map((l) => l.trimEnd())
		.filter(Boolean)
		.filter((line) => {
			const file = line.slice(3).replace(/^\"|\"$/g, "");
			return !file.startsWith(`${AFK_DIR}/`);
		});
	if (dirty.length > 0) {
		throw new Error(`Refusing to start AFK with dirty worktree outside ${AFK_DIR}/:\n${dirty.join("\n")}`);
	}
}

async function ensurePassCommit(pi: ExtensionAPI, cwd: string, commit: string) {
	if (!/^[0-9a-f]{7,40}$/i.test(commit)) throw new Error(`Verifier returned invalid commit hash: ${commit}`);
	await exec(pi, cwd, `git cat-file -e ${commit}^{commit}`, 30_000);
	const stdout = await exec(pi, cwd, "git status --porcelain", 30_000);
	const dirty = stdout
		.split(/\r?\n/)
		.map((l) => l.trimEnd())
		.filter(Boolean)
		.filter((line) => {
			const file = line.slice(3).replace(/^\"|\"$/g, "");
			return !file.startsWith(`${AFK_DIR}/`);
		});
	if (dirty.length > 0) throw new Error(`Verifier pass left dirty worktree:\n${dirty.join("\n")}`);
}

function finalJsonLine(text: string): unknown {
	const lines = text.trim().split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
	for (let i = lines.length - 1; i >= 0; i--) {
		try {
			return JSON.parse(lines[i]);
		} catch {
			// keep scanning upward
		}
	}
	throw new Error("Could not parse final JSON line from subagent result.");
}

function parseRoleResult(text: string): RoleResult {
	const parsed = finalJsonLine(text) as Partial<RoleResult>;
	if (parsed.status !== "pass" && parsed.status !== "needs-info") {
		throw new Error(`Invalid role status: ${String(parsed.status)}`);
	}
	return { status: parsed.status, reason: String(parsed.reason ?? "") };
}

function parseVerifyResult(text: string): VerifyResult {
	const parsed = finalJsonLine(text) as Partial<VerifyResult>;
	if (parsed.status !== "pass" && parsed.status !== "fail" && parsed.status !== "needs-info") {
		throw new Error(`Invalid verifier status: ${String(parsed.status)}`);
	}
	if (typeof parsed.summary !== "string" || typeof parsed.feedback !== "string" || !Array.isArray(parsed.commands_run)) {
		throw new Error("Verifier JSON missing summary, feedback, or commands_run.");
	}
	return {
		status: parsed.status,
		summary: parsed.summary,
		feedback: parsed.feedback,
		commands_run: parsed.commands_run.map(String),
		commit: String(parsed.commit ?? ""),
	};
}

function nextPhase(phase: Phase): Phase {
	if (phase === "implement") return "quality";
	if (phase === "quality") return "verify";
	return "implement";
}

async function rpc<T>(pi: ExtensionAPI, channel: string, payload: Record<string, unknown>, timeoutMs = 30_000): Promise<T> {
	const requestId = randomUUID();
	const replyChannel = `${channel}:reply:${requestId}`;
	return new Promise<T>((resolve, reject) => {
		const timer = setTimeout(() => {
			unsub();
			reject(new Error(`Timed out waiting for ${channel} reply.`));
		}, timeoutMs);
		const unsub = pi.events.on(replyChannel, (reply: unknown) => {
			clearTimeout(timer);
			unsub();
			const envelope = reply as RpcReply<T>;
			if (!envelope.success) reject(new Error(envelope.error));
			else resolve(envelope.data as T);
		});
		pi.events.emit(channel, { requestId, ...payload });
	});
}

async function requireSubagents(pi: ExtensionAPI) {
	const pong = await rpc<{ version: number }>(pi, "subagents:rpc:ping", {});
	if (!pong?.version) throw new Error("@tintinweb/pi-subagents RPC did not report protocol version.");
}

async function spawnSubagent(pi: ExtensionAPI, cwd: string, config: AfkConfig, role: Role, prompt: string, issue: Issue, state: AfkState): Promise<string> {
	const roleConfig = config.roles[role];
	const description = `${roleConfig.description} #${issue.number}`;
	const data = await rpc<{ id: string }>(pi, "subagents:rpc:spawn", {
		type: roleConfig.agentType,
		prompt,
		options: {
			description,
			model: roleConfig.model,
			maxTurns: roleConfig.maxTurns,
			isBackground: true,
			run_in_background: true,
			cwd,
		},
	});
	if (!data?.id) throw new Error("Subagent spawn did not return an id.");
	state.activeAgentId = data.id;
	await saveState(cwd, state);
	return data.id;
}

async function waitForSubagent(pi: ExtensionAPI, id: string): Promise<SubagentDoneEvent> {
	return new Promise((resolve, reject) => {
		const cleanup = () => {
			unsubCompleted();
			unsubFailed();
		};
		const unsubCompleted = pi.events.on("subagents:completed", (event: unknown) => {
			const done = event as SubagentDoneEvent;
			if (done.id !== id) return;
			cleanup();
			resolve(done);
		});
		const unsubFailed = pi.events.on("subagents:failed", (event: unknown) => {
			const done = event as SubagentDoneEvent;
			if (done.id !== id) return;
			cleanup();
			reject(new Error(done.error || done.result || done.status || `Subagent ${id} failed.`));
		});
	});
}

async function runRole(pi: ExtensionAPI, cwd: string, config: AfkConfig, state: AfkState, issue: Issue, role: Role) {
	const promptTemplate = await readPrompt(cwd, role);
	const prompt = fillTemplate(promptTemplate, {
		issueNumber: issue.number,
		issueTitle: issue.title,
		issueBody: issue.body ?? "",
		feedback: state.feedback || "(none)",
		cycle: state.cycle,
	});
	const id = await spawnSubagent(pi, cwd, config, role, prompt, issue, state);
	const done = await waitForSubagent(pi, id);
	state.activeAgentId = undefined;
	await saveState(cwd, state);
	return done.result ?? "";
}

async function commentIssue(pi: ExtensionAPI, cwd: string, issue: number, body: string) {
	await gh(pi, cwd, `gh issue comment ${issue} --body ${shellQuote(body)}`);
}

async function markNeedsInfo(pi: ExtensionAPI, cwd: string, issue: Issue, heading: string, body: string) {
	await commentIssue(pi, cwd, issue.number, `### ${heading}\n\n${body}`);
	await gh(
		pi,
		cwd,
		`gh issue edit ${issue.number} --remove-label ${shellQuote(READY_LABEL)} --add-label ${shellQuote(NEEDS_INFO_LABEL)}`,
	);
}

async function markCompleted(pi: ExtensionAPI, cwd: string, issue: Issue, verify: VerifyResult) {
	const commands = verify.commands_run.length > 0 ? verify.commands_run.map((c) => `- \`${c}\``).join("\n") : "- (none reported)";
	await commentIssue(
		pi,
		cwd,
		issue.number,
		`### AFK completed\n\n${verify.summary}\n\nCommit: ${verify.commit}\n\nCommands run:\n${commands}`,
	);
	await gh(pi, cwd, `gh issue edit ${issue.number} --remove-label ${shellQuote(READY_LABEL)}`);
	await gh(pi, cwd, `gh issue close ${issue.number} --comment ${shellQuote(`AFK completed in ${verify.commit}.`)}`);
}

async function runOne(pi: ExtensionAPI, ctx: any, config: AfkConfig, initialIssue: Issue, existingState?: AfkState): Promise<"completed" | "needs-info" | "paused"> {
	const cwd = ctx.cwd;
	const issue = await viewIssue(pi, cwd, initialIssue.number);
	let state = existingState ?? {
		version: 1 as const,
		issue: issue.number,
		phase: "implement" as Phase,
		cycle: 1,
		feedback: "",
		startedAt: nowIso(),
		updatedAt: nowIso(),
	};
	await saveState(cwd, state);

	while (true) {
		ctx.ui.setStatus("afk", `#${issue.number} ${state.phase} ${state.cycle}/${config.maxCycles}`);
		ctx.ui.setWidget("afk", [
			`AFK #${issue.number}: ${issue.title}`,
			`phase: ${state.phase}`,
			`cycle: ${state.cycle}/${config.maxCycles}`,
			state.feedback ? `feedback: ${state.feedback.slice(0, 160)}` : "feedback: none",
		]);

		if (state.phase === "implement") {
			let parsed: RoleResult;
			try {
				parsed = parseRoleResult(await runRole(pi, cwd, config, state, issue, "implementer"));
			} catch (err) {
				ctx.ui.notify(`AFK paused: ${err instanceof Error ? err.message : String(err)}`, "error");
				return "paused";
			}
			if (parsed.status === "needs-info") {
				await markNeedsInfo(pi, cwd, issue, "AFK needs info", parsed.reason || "Implementer requested more information.");
				await clearState(cwd);
				return "needs-info";
			}
			state.phase = nextPhase(state.phase);
			state.feedback = "";
			await saveState(cwd, state);
			continue;
		}

		if (state.phase === "quality") {
			let parsed: RoleResult;
			try {
				parsed = parseRoleResult(await runRole(pi, cwd, config, state, issue, "quality"));
			} catch (err) {
				ctx.ui.notify(`AFK paused: ${err instanceof Error ? err.message : String(err)}`, "error");
				return "paused";
			}
			if (parsed.status === "needs-info") {
				await markNeedsInfo(pi, cwd, issue, "AFK needs info", parsed.reason || "Quality pass requested more information.");
				await clearState(cwd);
				return "needs-info";
			}
			state.phase = nextPhase(state.phase);
			await saveState(cwd, state);
			continue;
		}

		let verify: VerifyResult;
		try {
			verify = parseVerifyResult(await runRole(pi, cwd, config, state, issue, "verifier"));
		} catch (err) {
			ctx.ui.notify(`AFK paused: ${err instanceof Error ? err.message : String(err)}`, "error");
			return "paused";
		}

		if (verify.status === "pass") {
			try {
				await ensurePassCommit(pi, cwd, verify.commit);
			} catch (err) {
				state.feedback = `Verifier reported pass but commit validation failed: ${err instanceof Error ? err.message : String(err)}`;
				await saveState(cwd, state);
				ctx.ui.notify("AFK paused: verifier pass failed commit validation.", "error");
				return "paused";
			}
			await markCompleted(pi, cwd, issue, verify);
			await clearState(cwd);
			return "completed";
		}

		if (verify.status === "needs-info") {
			await markNeedsInfo(pi, cwd, issue, "AFK needs info", verify.feedback || verify.summary);
			await clearState(cwd);
			return "needs-info";
		}

		if (state.cycle >= config.maxCycles) {
			await markNeedsInfo(pi, cwd, issue, `AFK needs info after ${config.maxCycles} cycles`, verify.feedback || verify.summary);
			await clearState(cwd);
			return "needs-info";
		}

		state = {
			...state,
			phase: "implement",
			cycle: state.cycle + 1,
			feedback: verify.feedback || verify.summary,
			activeAgentId: undefined,
		};
		await saveState(cwd, state);
	}
}

async function handleRun(pi: ExtensionAPI, ctx: any, argv: string[]) {
	let runAll = false;
	let issueNumber: number | undefined;
	for (let i = 0; i < argv.length; i++) {
		const arg = argv[i];
		if (arg === "--all") runAll = true;
		else if (arg === "--issue") {
			const next = argv[++i];
			if (!next || !/^\d+$/.test(next)) throw new Error("--issue requires an issue number.");
			issueNumber = Number(next);
		} else throw new Error(usage());
	}
	if (runAll && issueNumber !== undefined) throw new Error("--all cannot be combined with --issue.");
	if (await loadState(ctx.cwd)) throw new Error(`AFK state already exists. Use /afk resume or /afk stop. State: ${STATE_REL}`);
	await ensureCleanForStart(pi, ctx.cwd);
	await requireSubagents(pi);
	const config = await loadConfig(ctx.cwd);
	const completed: number[] = [];
	const needsInfo: number[] = [];

	while (true) {
		const issue = issueNumber !== undefined ? await viewIssue(pi, ctx.cwd, issueNumber) : await selectIssue(pi, ctx.cwd);
		if (!issue) break;
		if (!(await isRunnableIssue(pi, ctx.cwd, issue))) throw new Error(`#${issue.number} is not runnable.`);
		ctx.ui.notify(`AFK starting #${issue.number}: ${issue.title}`, "info");
		const result = await runOne(pi, ctx, config, issue);
		if (result === "paused") return;
		if (result === "completed") completed.push(issue.number);
		if (result === "needs-info") needsInfo.push(issue.number);
		if (!runAll) break;
		issueNumber = undefined;
	}

	ctx.ui.setStatus("afk", undefined);
	ctx.ui.setWidget("afk", undefined);
	ctx.ui.notify(
		`AFK finished\ncompleted: ${completed.length ? completed.map((n) => `#${n}`).join(", ") : "none"}\nneeds-info: ${needsInfo.length ? needsInfo.map((n) => `#${n}`).join(", ") : "none"}`,
		needsInfo.length > 0 ? "warning" : "info",
	);
}

async function handleResume(pi: ExtensionAPI, ctx: any) {
	const state = await loadState(ctx.cwd);
	if (!state) throw new Error(`No AFK state found at ${STATE_REL}.`);
	await requireSubagents(pi);
	const config = await loadConfig(ctx.cwd);
	const issue = await viewIssue(pi, ctx.cwd, state.issue);
	state.activeAgentId = undefined;
	await saveState(ctx.cwd, state);
	const result = await runOne(pi, ctx, config, issue, state);
	if (result !== "paused") {
		ctx.ui.setStatus("afk", undefined);
		ctx.ui.setWidget("afk", undefined);
	}
}

async function handleStatus(ctx: any) {
	const state = await loadState(ctx.cwd);
	if (!state) {
		ctx.ui.notify("AFK idle.", "info");
		return;
	}
	ctx.ui.notify(
		`AFK #${state.issue}\nphase: ${state.phase}\ncycle: ${state.cycle}\nactiveAgentId: ${state.activeAgentId ?? "none"}\nstate: ${STATE_REL}`,
		"info",
	);
}

async function handleStop(pi: ExtensionAPI, ctx: any) {
	const state = await loadState(ctx.cwd);
	if (!state) {
		ctx.ui.notify("AFK idle.", "info");
		return;
	}
	if (state.activeAgentId) {
		await rpc(pi, "subagents:rpc:stop", { agentId: state.activeAgentId });
		state.activeAgentId = undefined;
		await saveState(ctx.cwd, state);
	}
	ctx.ui.setStatus("afk", undefined);
	ctx.ui.setWidget("afk", undefined);
	ctx.ui.notify(`AFK stopped locally. Resume with /afk resume. State kept at ${STATE_REL}.`, "warning");
}

export default function afkExtension(pi: ExtensionAPI) {
	pi.registerCommand("afk", {
		description: "Run AFK GitHub issue implementation loop",
		getArgumentCompletions(prefix: string) {
			return ["run", "resume", "status", "stop", "--all", "--issue"]
				.filter((item) => item.startsWith(prefix))
				.map((value) => ({ value, label: value }));
		},
		handler: async (args: string, ctx: any) => {
			try {
				const argv = splitArgs(args);
				const command = argv.shift();
				if (command === "run") await handleRun(pi, ctx, argv);
				else if (command === "resume") await handleResume(pi, ctx);
				else if (command === "status") await handleStatus(ctx);
				else if (command === "stop") await handleStop(pi, ctx);
				else throw new Error(usage());
			} catch (err) {
				ctx.ui.notify(err instanceof Error ? err.message : String(err), "error");
			}
		},
	});
}
