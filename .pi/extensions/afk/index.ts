import { randomUUID } from "node:crypto";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Box, Text, truncateToWidth } from "@earendil-works/pi-tui";
import {
	consumeStructuredResult,
	registerAfkResultTools,
	registerResultToken,
	removeResultToken,
	validateRoleResult,
	validateVerifyResult,
	type ActiveResultToken,
	type AfkResultKind,
	type Phase,
	type Role,
	type RoleResult,
	type VerifyResult,
} from "./result-tools.ts";
import { startRoleSession, type RoleSessionRun } from "./role-session-runner.ts";
import { appendTranscriptEvent } from "./transcript.ts";

const AFK_DIR = ".pi/afk";
const STATE_REL = `${AFK_DIR}/state.json`;
const HISTORY_REL = `${AFK_DIR}/history.json`;
const CONFIG_REL = `${AFK_DIR}/config.json`;
const PROMPTS_REL = `${AFK_DIR}/prompts`;
const READY_LABEL = "ready-for-agent";
const NEEDS_INFO_LABEL = "needs-info";
const EXCLUDED_LABELS = new Set([NEEDS_INFO_LABEL, "ready-for-human", "wontfix"]);

type AfkConfig = {
	maxCycles: number;
	roles: Record<Role, {
		model?: string;
	}>;
};

type AfkRunStatus = "running" | "paused";

type AfkLastResult = {
	role: Role;
	status: string;
	summary: string;
	at: string;
};

type AfkState = {
	version: 1;
	status: AfkRunStatus;
	issue: number;
	phase: Phase;
	cycle: number;
	activeRoleSessionId?: string;
	activeTranscriptPath?: string;
	feedback: string;
	lastResult?: AfkLastResult;
	startedAt: string;
	updatedAt: string;
};

type AfkHistoryEntry = {
	issue: number;
	result: "completed" | "needs-info" | "paused";
	summary: string;
	commit?: string;
	at: string;
};

type Issue = {
	number: number;
	title: string;
	body: string;
	labels: Array<{ name: string }>;
	updatedAt?: string;
};

type ActiveRun = {
	promise: Promise<void>;
	stopRequested: boolean;
	activeRoleSession?: RoleSessionRun;
};

type AfkLiveActivity = {
	spinnerIndex: number;
	model: string | undefined;
	activeTool?: string;
	lastTool?: string;
	lastText?: string;
	turnCount?: number;
	tokenTotal: number;
	toolUses: number;
	updatedAt: string;
};

type ToolActivity = {
	type: "start" | "end";
	toolName: string;
};

type Theme = {
	fg(color: string, text: string): string;
	bg(color: string, text: string): string;
	bold(text: string): string;
};

let activeRun: ActiveRun | undefined;
let liveActivity = freshLiveActivity();
let widgetTimer: ReturnType<typeof setInterval> | undefined;
let widgetRegistered = false;
let widgetTui: any;
let currentWidgetCtx: any;
let currentWidgetIssue: Issue | undefined;
let currentWidgetState: AfkState | undefined;
let currentWidgetConfig: AfkConfig | undefined;

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
	const state = await readJson<AfkState & { activeAgentId?: string }>(statePath);
	const { activeAgentId, ...rest } = state;
	return {
		...rest,
		status: state.status ?? "paused",
		activeRoleSessionId: state.activeRoleSessionId ?? activeAgentId,
	};
}

async function loadHistory(cwd: string): Promise<AfkHistoryEntry[]> {
	const historyPath = path.join(cwd, HISTORY_REL);
	if (!(await exists(historyPath))) return [];
	return readJson<AfkHistoryEntry[]>(historyPath);
}

async function appendHistory(cwd: string, entry: AfkHistoryEntry) {
	const history = [entry, ...(await loadHistory(cwd))].slice(0, 5);
	await writeJson(path.join(cwd, HISTORY_REL), history);
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
	// Do not use `gh issue list --label` here. Some repos return zero rows even
	// when `gh issue view` shows the label on matching issues. List open issues
	// and apply the runnable label rules locally instead.
	const stdout = await gh(
		pi,
		cwd,
		"gh issue list --state open --limit 1000 --json number,title,body,labels,updatedAt",
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
	if (/^PRD:/i.test(issue.title.trim())) return false;
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

function nextPhase(phase: Phase): Phase {
	if (phase === "implement") return "quality";
	if (phase === "quality") return "verify";
	return "implement";
}

function freshLiveActivity(): AfkLiveActivity {
	return {
		spinnerIndex: 0,
		model: undefined,
		tokenTotal: 0,
		toolUses: 0,
		updatedAt: nowIso(),
	};
}

function oneLine(text: string, max = 160) {
	const line = text.replace(/\s+/g, " ").trim();
	return line.length > max ? `${line.slice(0, max)}…` : line;
}

function modelLabel(model: any) {
	if (!model?.provider || !model?.id) return undefined;
	return `${model.provider}/${model.id}`;
}

function roleModelLabel(ctx: any, modelSpec?: string) {
	return modelSpec || modelLabel(ctx.model);
}

function sanitizeAgentText(text: string) {
	const withoutReminder = text.split("<system-reminder>")[0] ?? text;
	return withoutReminder
		.replace(/<task-notification>[\s\S]*?<\/task-notification>/g, " ")
		.replace(/<[^>]+>/g, " ")
		.replace(/\s+/g, " ")
		.trim();
}

function lastTextLine(text: string) {
	const lines = text
		.split(/\r?\n/g)
		.map(sanitizeAgentText)
		.filter(Boolean);
	return lines.at(-1) ?? "";
}

function truncateLine(text: string, width: number) {
	return truncateToWidth(text, width);
}

function phaseStatus(state: AfkState) {
	if (!state.activeRoleSessionId) return "transitioning…";
	return "Working…";
}

function toolLabel(toolName: string) {
	const labels: Record<string, string> = {
		bash: "running command",
		read: "reading file",
		edit: "editing file",
		write: "writing file",
		grep: "searching",
		find: "finding files",
		ls: "listing files",
	};
	return labels[toolName] ?? toolName;
}

function currentActivity(state: AfkState) {
	if (!state.activeRoleSessionId) return `starting ${state.phase}…`;
	if (liveActivity.activeTool) return liveActivity.activeTool;
	return "thinking…";
}

function lastOutput(state: AfkState) {
	return liveActivity.lastText
		|| state.lastResult?.summary
		|| state.feedback
		|| state.activeTranscriptPath
		|| "waiting for role session output";
}

function renderAfkPanel(tui: any, theme: Theme): string[] {
	const issue = currentWidgetIssue;
	const state = currentWidgetState;
	const config = currentWidgetConfig;
	if (!issue || !state || !config) return [];

	const width = Math.max(20, tui?.terminal?.columns ?? 100);
	const spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][liveActivity.spinnerIndex % 10];
	const icon = state.activeRoleSessionId ? theme.fg("accent", spinner) : theme.fg("dim", "•");
	const status = state.activeRoleSessionId ? theme.fg("accent", phaseStatus(state)) : theme.fg("dim", phaseStatus(state));
	const stats: string[] = [];
	if (liveActivity.turnCount) stats.push(`↻${liveActivity.turnCount}`);
	if (liveActivity.toolUses) stats.push(`${liveActivity.toolUses} tools`);
	if (liveActivity.tokenTotal) stats.push(`${(liveActivity.tokenTotal / 1000).toFixed(1)}k tok`);
	const statSuffix = stats.length ? ` ${theme.fg("dim", `· ${stats.join(" · ")}`)}` : "";
	const roleSession = state.activeRoleSessionId ? `${state.activeRoleSessionId.slice(0, 8)}…` : "none";

	return [
		`${icon} ${theme.bold(`AFK #${issue.number}`)} ${theme.fg("dim", "·")} ${state.phase} ${state.cycle}/${config.maxCycles} ${theme.fg("dim", "·")} ${status}${statSuffix}`,
		`${theme.fg("dim", "issue:")} ${issue.title}`,
		`${theme.fg("dim", "role session:")} ${roleSession}`,
		`${theme.fg("dim", "model:")} ${liveActivity.model ?? "unknown"}`,
		`${theme.fg("dim", "activity:")} ${currentActivity(state)}`,
		`${theme.fg("dim", "last:")} ${oneLine(lastOutput(state), 220)}`,
	].map((line) => truncateLine(line, width));
}

type AfkResultMessageDetails = {
	issue: number;
	title: string;
	role: Role;
	phase: Phase;
	cycle: number;
	status: string;
	summary: string;
	reason?: string;
	feedback?: string;
	commands_run?: string[];
	commit?: string;
};

function afkResultContent(details: AfkResultMessageDetails) {
	return `AFK ${details.role} #${details.issue}: ${details.status} — ${oneLine(details.summary)}`;
}

function sendAfkResultMessage(pi: ExtensionAPI, issue: Issue, state: AfkState, role: Role, result: RoleResult | VerifyResult) {
	const isVerify = "feedback" in result;
	const details: AfkResultMessageDetails = {
		issue: issue.number,
		title: issue.title,
		role,
		phase: state.phase,
		cycle: state.cycle,
		status: result.status,
		summary: result.summary,
		...("reason" in result && result.reason ? { reason: result.reason } : {}),
		...(isVerify ? {
			feedback: result.feedback,
			commands_run: result.commands_run,
			commit: result.commit,
		} : {}),
	};
	try {
		pi.sendMessage({
			customType: "afk-result",
			content: afkResultContent(details),
			display: true,
			details,
		});
	} catch {
		// Result messages are for transcript visibility only; never break the AFK loop.
	}
}

function afkStatusText(issue: Issue, state: AfkState, config: AfkConfig) {
	const modelSuffix = liveActivity.model ? ` · ${liveActivity.model}` : "";
	return `#${issue.number} ${state.phase} ${state.cycle}/${config.maxCycles}${modelSuffix}`;
}

function updateAfkStatus() {
	if (!currentWidgetCtx || !currentWidgetIssue || !currentWidgetState || !currentWidgetConfig) return;
	currentWidgetCtx.ui.setStatus("afk", afkStatusText(currentWidgetIssue, currentWidgetState, currentWidgetConfig));
}

function renderAfkWidgetNow() {
	if (!currentWidgetCtx) return;
	updateAfkStatus();
	if (widgetRegistered) {
		widgetTui?.requestRender?.();
		return;
	}
	currentWidgetCtx.ui.setWidget("afk", (tui: any, theme: Theme) => {
		widgetTui = tui;
		return {
			render: () => renderAfkPanel(tui, theme),
			invalidate: () => {
				widgetRegistered = false;
				widgetTui = undefined;
			},
		};
	}, { placement: "aboveEditor" });
	widgetRegistered = true;
}

function startAfkWidgetTimer() {
	if (widgetTimer) return;
	widgetTimer = setInterval(() => {
		if (currentWidgetState?.activeRoleSessionId) liveActivity.spinnerIndex++;
		renderAfkWidgetNow();
	}, 160);
	widgetTimer.unref?.();
}

function setAfkWidget(ctx: any, issue: Issue, state: AfkState, config: AfkConfig) {
	currentWidgetCtx = ctx;
	currentWidgetIssue = issue;
	currentWidgetState = { ...state };
	currentWidgetConfig = config;
	updateAfkStatus();
	startAfkWidgetTimer();
	renderAfkWidgetNow();
}

function clearAfkWidget(ctx: any) {
	if (widgetTimer) {
		clearInterval(widgetTimer);
		widgetTimer = undefined;
	}
	ctx.ui.setStatus("afk", undefined);
	ctx.ui.setWidget("afk", undefined);
	widgetRegistered = false;
	widgetTui = undefined;
	currentWidgetCtx = undefined;
	currentWidgetIssue = undefined;
	currentWidgetState = undefined;
	currentWidgetConfig = undefined;
}

async function pauseState(cwd: string, summary?: string) {
	const state = await loadState(cwd);
	if (!state) return;
	state.status = "paused";
	state.activeRoleSessionId = undefined;
	if (summary) state.feedback = summary;
	await saveState(cwd, state);
}

async function stopActiveRoleSession(cwd: string) {
	const roleSession = activeRun?.activeRoleSession;
	if (roleSession) {
		try {
			await roleSession.abort();
		} catch {
			// Best effort: the role session may already have completed.
		}
	}
	const state = await loadState(cwd);
	if (state) {
		state.activeRoleSessionId = undefined;
		await saveState(cwd, state);
	}
	if (activeRun?.activeRoleSession === roleSession) activeRun.activeRoleSession = undefined;
}

function structuredResultInstructions(kind: AfkResultKind, token: string) {
	if (kind === "role") {
		return `

# AFK structured result protocol

When your work is complete, do not write a final prose response and do not print JSON manually. As your final action, call the \`afk_role_result\` tool with this token: \`${token}\`.

Use status \`pass\` when the phase is complete. Use status \`needs-info\` only when you cannot proceed safely. Put the human-readable completion summary in \`summary\`; for \`needs-info\`, also put the blocking reason in \`reason\`.`;
	}
	return `

# AFK structured result protocol

When verification is complete, do not write a final prose response and do not print JSON manually. As your final action, call the \`afk_verify_result\` tool with this token: \`${token}\`.

Include status, summary, feedback, commands_run, and commit. Use status \`pass\` only after committing the verified changes and put that commit hash in \`commit\`. For \`fail\` or \`needs-info\`, do not commit and set \`commit\` to an empty string.`;
}

async function runStructuredRole<T>(
	ctx: any,
	cwd: string,
	config: AfkConfig,
	state: AfkState,
	issue: Issue,
	role: Role,
	kind: AfkResultKind,
	validate: (value: unknown) => T,
): Promise<T> {
	const token = randomUUID();
	const meta: ActiveResultToken = {
		kind,
		issue: issue.number,
		role,
		phase: state.phase,
		cycle: state.cycle,
		createdAt: nowIso(),
	};
	await registerResultToken(cwd, token, meta);
	let roleSession: RoleSessionRun | undefined;
	try {
		const promptTemplate = await readPrompt(cwd, role);
		const prompt = fillTemplate(promptTemplate, {
			issueNumber: issue.number,
			issueTitle: issue.title,
			issueBody: issue.body ?? "",
			feedback: state.feedback || "(none)",
			cycle: state.cycle,
		}) + structuredResultInstructions(kind, token);

		liveActivity = freshLiveActivity();
		liveActivity.lastText = state.lastResult?.summary || state.feedback || undefined;
		const roleConfig = config.roles[role];
		liveActivity.model = roleModelLabel(ctx, roleConfig.model);
		renderAfkWidgetNow();
		roleSession = await startRoleSession({
			cwd,
			ctx,
			issue: issue.number,
			role,
			phase: state.phase,
			cycle: state.cycle,
			prompt,
			model: roleConfig.model,
			onTextDelta: (_delta: string, fullText: string) => {
				const line = lastTextLine(fullText);
				if (line) liveActivity.lastText = line;
				liveActivity.updatedAt = nowIso();
				renderAfkWidgetNow();
			},
			onToolActivity: (activity: ToolActivity) => {
				if (activity.type === "start") liveActivity.activeTool = toolLabel(activity.toolName);
				else {
					liveActivity.lastTool = liveActivity.activeTool || toolLabel(activity.toolName);
					liveActivity.activeTool = undefined;
					liveActivity.toolUses++;
				}
				liveActivity.updatedAt = nowIso();
				renderAfkWidgetNow();
			},
			onTurnEnd: (turnCount: number) => {
				liveActivity.turnCount = turnCount;
				liveActivity.updatedAt = nowIso();
				renderAfkWidgetNow();
			},
			onAssistantUsage: (usage: { input?: number; output?: number; cacheWrite?: number }) => {
				liveActivity.tokenTotal += (usage.input ?? 0) + (usage.output ?? 0) + (usage.cacheWrite ?? 0);
				liveActivity.updatedAt = nowIso();
				renderAfkWidgetNow();
			},
		});
		state.activeRoleSessionId = roleSession.id;
		state.activeTranscriptPath = roleSession.transcriptPath;
		liveActivity.model = roleSession.model ?? liveActivity.model;
		if (activeRun) activeRun.activeRoleSession = roleSession;
		currentWidgetState = { ...state };
		renderAfkWidgetNow();
		await saveState(cwd, state);
		if (activeRun?.stopRequested) await stopActiveRoleSession(cwd);
		await roleSession.done;
	} finally {
		await removeResultToken(cwd, token);
		state.activeRoleSessionId = undefined;
		if (activeRun?.activeRoleSession === roleSession) activeRun.activeRoleSession = undefined;
		currentWidgetState = { ...state };
		renderAfkWidgetNow();
		await saveState(cwd, state);
	}
	const result = await consumeStructuredResult(cwd, token, meta, validate);
	if (roleSession) {
		try {
			await appendTranscriptEvent(roleSession.transcriptPath, {
				type: "afk_result",
				status: (result as any).status,
				summary: (result as any).summary,
				feedback: (result as any).feedback,
				reason: (result as any).reason,
				commit: (result as any).commit,
			});
		} catch {
			// Transcript writes must not affect AFK phase outcomes.
		}
	}
	return result;
}

async function runRoleResult(ctx: any, cwd: string, config: AfkConfig, state: AfkState, issue: Issue, role: "implementer" | "quality") {
	return runStructuredRole(ctx, cwd, config, state, issue, role, "role", validateRoleResult);
}

async function runVerifyResult(ctx: any, cwd: string, config: AfkConfig, state: AfkState, issue: Issue) {
	return runStructuredRole(ctx, cwd, config, state, issue, "verifier", "verify", validateVerifyResult);
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
		status: "running" as AfkRunStatus,
		issue: issue.number,
		phase: "implement" as Phase,
		cycle: 1,
		feedback: "",
		startedAt: nowIso(),
		updatedAt: nowIso(),
	};
	state.status = "running";
	await saveState(cwd, state);

	while (true) {
		if (activeRun?.stopRequested) {
			state.status = "paused";
			state.activeRoleSessionId = undefined;
			await saveState(cwd, state);
			await appendHistory(cwd, {
				issue: issue.number,
				result: "paused",
				summary: `Stopped during ${state.phase} phase.`,
				at: nowIso(),
			});
			return "paused";
		}
		setAfkWidget(ctx, issue, state, config);

		if (state.phase === "implement") {
			let parsed: RoleResult;
			try {
				parsed = await runRoleResult(ctx, cwd, config, state, issue, "implementer");
				sendAfkResultMessage(pi, issue, state, "implementer", parsed);
			} catch (err) {
				state.status = "paused";
				state.activeRoleSessionId = undefined;
				state.feedback = err instanceof Error ? err.message : String(err);
				await saveState(cwd, state);
				await appendHistory(cwd, { issue: issue.number, result: "paused", summary: state.feedback, at: nowIso() });
				ctx.ui.notify(`AFK paused: ${state.feedback}`, "error");
				return "paused";
			}
			state.lastResult = {
				role: "implementer",
				status: parsed.status,
				summary: parsed.summary,
				at: nowIso(),
			};
			if (parsed.status === "needs-info") {
				const summary = parsed.reason || parsed.summary;
				await markNeedsInfo(pi, cwd, issue, "AFK needs info", summary);
				await appendHistory(cwd, { issue: issue.number, result: "needs-info", summary, at: nowIso() });
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
				parsed = await runRoleResult(ctx, cwd, config, state, issue, "quality");
				sendAfkResultMessage(pi, issue, state, "quality", parsed);
			} catch (err) {
				state.status = "paused";
				state.activeRoleSessionId = undefined;
				state.feedback = err instanceof Error ? err.message : String(err);
				await saveState(cwd, state);
				await appendHistory(cwd, { issue: issue.number, result: "paused", summary: state.feedback, at: nowIso() });
				ctx.ui.notify(`AFK paused: ${state.feedback}`, "error");
				return "paused";
			}
			state.lastResult = {
				role: "quality",
				status: parsed.status,
				summary: parsed.summary,
				at: nowIso(),
			};
			if (parsed.status === "needs-info") {
				const summary = parsed.reason || parsed.summary;
				await markNeedsInfo(pi, cwd, issue, "AFK needs info", summary);
				await appendHistory(cwd, { issue: issue.number, result: "needs-info", summary, at: nowIso() });
				await clearState(cwd);
				return "needs-info";
			}
			state.phase = nextPhase(state.phase);
			await saveState(cwd, state);
			continue;
		}

		let verify: VerifyResult;
		try {
			verify = await runVerifyResult(ctx, cwd, config, state, issue);
			sendAfkResultMessage(pi, issue, state, "verifier", verify);
		} catch (err) {
			state.status = "paused";
			state.activeRoleSessionId = undefined;
			state.feedback = err instanceof Error ? err.message : String(err);
			await saveState(cwd, state);
			await appendHistory(cwd, { issue: issue.number, result: "paused", summary: state.feedback, at: nowIso() });
			ctx.ui.notify(`AFK paused: ${state.feedback}`, "error");
			return "paused";
		}
		state.lastResult = {
			role: "verifier",
			status: verify.status,
			summary: verify.summary || verify.feedback,
			at: nowIso(),
		};

		if (verify.status === "pass") {
			try {
				await ensurePassCommit(pi, cwd, verify.commit);
			} catch (err) {
				state.status = "paused";
				state.feedback = `Verifier reported pass but commit validation failed: ${err instanceof Error ? err.message : String(err)}`;
				await saveState(cwd, state);
				await appendHistory(cwd, { issue: issue.number, result: "paused", summary: state.feedback, at: nowIso() });
				ctx.ui.notify("AFK paused: verifier pass failed commit validation.", "error");
				return "paused";
			}
			await markCompleted(pi, cwd, issue, verify);
			await appendHistory(cwd, { issue: issue.number, result: "completed", summary: verify.summary, commit: verify.commit, at: nowIso() });
			await clearState(cwd);
			return "completed";
		}

		if (verify.status === "needs-info") {
			const summary = verify.feedback || verify.summary;
			await markNeedsInfo(pi, cwd, issue, "AFK needs info", summary);
			await appendHistory(cwd, { issue: issue.number, result: "needs-info", summary, at: nowIso() });
			await clearState(cwd);
			return "needs-info";
		}

		if (state.cycle >= config.maxCycles) {
			const summary = verify.feedback || verify.summary;
			await markNeedsInfo(pi, cwd, issue, `AFK needs info after ${config.maxCycles} cycles`, summary);
			await appendHistory(cwd, { issue: issue.number, result: "needs-info", summary, at: nowIso() });
			await clearState(cwd);
			return "needs-info";
		}

		state = {
			...state,
			phase: "implement",
			cycle: state.cycle + 1,
			feedback: verify.feedback || verify.summary,
			activeRoleSessionId: undefined,
		};
		await saveState(cwd, state);
	}
}

function ensureNoActiveRun() {
	if (activeRun) throw new Error("AFK already running. Use /afk status or /afk stop.");
}

function startDetachedRun(ctx: any, run: () => Promise<void>) {
	ensureNoActiveRun();
	const handle: ActiveRun = {
		stopRequested: false,
		promise: Promise.resolve(),
	};
	activeRun = handle;
	handle.promise = run()
		.catch(async (err) => {
			const message = err instanceof Error ? err.message : String(err);
			await pauseState(ctx.cwd, message);
			clearAfkWidget(ctx);
			ctx.ui.notify(`AFK paused: ${message}`, "error");
		})
		.finally(() => {
			if (activeRun === handle) activeRun = undefined;
		});
}

async function runIssueLoop(pi: ExtensionAPI, ctx: any, config: AfkConfig, runAll: boolean, issueNumber?: number) {
	const completed: number[] = [];
	const needsInfo: number[] = [];

	while (true) {
		if (activeRun?.stopRequested) break;
		const issue = issueNumber !== undefined ? await viewIssue(pi, ctx.cwd, issueNumber) : await selectIssue(pi, ctx.cwd);
		if (!issue) break;
		if (!(await isRunnableIssue(pi, ctx.cwd, issue))) throw new Error(`#${issue.number} is not runnable.`);
		ctx.ui.notify(`AFK starting #${issue.number}: ${issue.title}`, "info");
		const result = await runOne(pi, ctx, config, issue);
		if (result === "paused") {
			clearAfkWidget(ctx);
			return;
		}
		if (result === "completed") completed.push(issue.number);
		if (result === "needs-info") needsInfo.push(issue.number);
		if (!runAll) break;
		issueNumber = undefined;
	}

	clearAfkWidget(ctx);
	if (activeRun?.stopRequested) return;
	ctx.ui.notify(
		`AFK finished\ncompleted: ${completed.length ? completed.map((n) => `#${n}`).join(", ") : "none"}\nneeds-info: ${needsInfo.length ? needsInfo.map((n) => `#${n}`).join(", ") : "none"}`,
		needsInfo.length > 0 ? "warning" : "info",
	);
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
	ensureNoActiveRun();
	if (await loadState(ctx.cwd)) throw new Error(`AFK state already exists. Use /afk resume or /afk stop. State: ${STATE_REL}`);
	await ensureCleanForStart(pi, ctx.cwd);
	const config = await loadConfig(ctx.cwd);
	startDetachedRun(ctx, () => runIssueLoop(pi, ctx, config, runAll, issueNumber));
	ctx.ui.notify("AFK started. Live output: /afk status shows transcript path. Control: /afk status | /afk stop.", "info");
}

async function handleResume(pi: ExtensionAPI, ctx: any) {
	ensureNoActiveRun();
	const state = await loadState(ctx.cwd);
	if (!state) throw new Error(`No AFK state found at ${STATE_REL}.`);
	const config = await loadConfig(ctx.cwd);
	state.status = "running";
	state.activeRoleSessionId = undefined;
	await saveState(ctx.cwd, state);
	startDetachedRun(ctx, async () => {
		const issue = await viewIssue(pi, ctx.cwd, state.issue);
		await runOne(pi, ctx, config, issue, state);
		clearAfkWidget(ctx);
	});
	ctx.ui.notify("AFK resumed. Live output: /afk status shows transcript path. Control: /afk status | /afk stop.", "info");
}

async function handleStatus(ctx: any) {
	const state = await loadState(ctx.cwd);
	if (!state && activeRun) {
		ctx.ui.notify("AFK active; selecting or starting issue.", "info");
		return;
	}
	if (!state) {
		const history = await loadHistory(ctx.cwd);
		const last = history[0];
		ctx.ui.notify(
			last
				? `AFK idle.\nlast: #${last.issue} ${last.result} — ${oneLine(last.summary)}${last.commit ? `\ncommit: ${last.commit}` : ""}`
				: "AFK idle.",
			"info",
		);
		return;
	}
	ctx.ui.notify(
		[
			`AFK #${state.issue}`,
			`status: ${activeRun ? activeRun.stopRequested ? "stopping" : "active" : state.status}`,
			`phase: ${state.phase}`,
			`cycle: ${state.cycle}`,
			`model: ${liveActivity.model ?? "unknown"}`,
			`activeRoleSessionId: ${state.activeRoleSessionId ?? "none"}`,
			`transcript: ${state.activeTranscriptPath ?? "none"}`,
			`activity: ${currentActivity(state)}`,
			`last: ${oneLine(lastOutput(state))}`,
			state.feedback ? `feedback: ${oneLine(state.feedback)}` : "feedback: none",
			`state: ${STATE_REL}`,
		].join("\n"),
		"info",
	);
}

async function handleStop(ctx: any) {
	const state = await loadState(ctx.cwd);
	if (!state && !activeRun) {
		ctx.ui.notify("AFK idle.", "info");
		return;
	}
	if (activeRun) activeRun.stopRequested = true;
	await stopActiveRoleSession(ctx.cwd);
	await pauseState(ctx.cwd, "Stopped by user. Resume restarts the current phase.");
	clearAfkWidget(ctx);
	ctx.ui.notify(`AFK stopped locally. Resume with /afk resume. State kept at ${STATE_REL}.`, "warning");
}

export default function afkExtension(pi: ExtensionAPI) {
	registerAfkResultTools(pi);

	pi.registerMessageRenderer("afk-result", (message: any, { expanded }: { expanded: boolean }, theme: any) => {
		const details = message.details as AfkResultMessageDetails | undefined;
		const status = details?.status ?? "unknown";
		const color = status === "pass" ? "success" : status === "fail" ? "error" : "warning";
		let text = `${theme.fg(color, `[AFK ${status.toUpperCase()}]`)} ${message.content}`;
		if (expanded && details) {
			const lines = [
				`role: ${details.role}`,
				`phase: ${details.phase} cycle ${details.cycle}`,
			];
			if (details.commit) lines.push(`commit: ${details.commit}`);
			if (details.reason) lines.push(`reason: ${details.reason}`);
			if (details.feedback) lines.push(`feedback: ${details.feedback}`);
			if (details.commands_run?.length) lines.push(`commands:\n${details.commands_run.map((command) => `  - ${command}`).join("\n")}`);
			text += `\n${theme.fg("dim", lines.join("\n"))}`;
		}
		const box = new Box(1, 1, (value: string) => theme.bg("customMessageBg", value));
		box.addChild(new Text(text, 0, 0));
		return box;
	});

	pi.on("session_start", async (_event, ctx: any) => {
		if (activeRun) return;
		const state = await loadState(ctx.cwd);
		if (state?.status === "running") {
			state.status = "paused";
			state.activeRoleSessionId = undefined;
			state.feedback = state.feedback || "Paused because Pi restarted or reloaded while AFK was running.";
			await saveState(ctx.cwd, state);
		}
	});

	pi.on("before_agent_start", async (event) => {
		if (!activeRun) return;
		return {
			systemPrompt: `${event.systemPrompt}\n\nAFK run active. AFK and role-session completion notifications are progress signals only. When one arrives, do NOT inspect, read, edit, test, verify, commit, log, or continue the work, even if the result says "pass" and even if tests look broken — those belong to the AFK pipeline, not you. Emit at most a one-line acknowledgement and end your turn immediately. Do not call tools. The AFK widget and transcript already display the result; you add nothing. Act only if the user gives an explicit instruction this turn.`,
		};
	});

	pi.on("session_shutdown", async (_event, ctx: any) => {
		if (!activeRun) return;
		activeRun.stopRequested = true;
		await stopActiveRoleSession(ctx.cwd);
		await pauseState(ctx.cwd, "Paused because Pi session shut down while AFK was running.");
		clearAfkWidget(ctx);
	});

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
				else if (command === "stop") await handleStop(ctx);
				else throw new Error(usage());
			} catch (err) {
				ctx.ui.notify(err instanceof Error ? err.message : String(err), "error");
			}
		},
	});
}
