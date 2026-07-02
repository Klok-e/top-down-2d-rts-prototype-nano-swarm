import * as fs from "node:fs/promises";
import * as path from "node:path";
import type { Phase, Role } from "./result-tools.ts";

const AFK_DIR = ".pi/afk";
const TRANSCRIPTS_REL = `${AFK_DIR}/transcripts`;
const MAX_TEXT_CHARS = 4096;

export type TranscriptMeta = {
	issue: number;
	role: Role;
	phase: Phase;
	cycle: number;
};

export type TranscriptWriter = {
	path: string;
	write(event: Record<string, unknown>): Promise<void>;
	close(): Promise<void>;
};

function nowIso() {
	return new Date().toISOString();
}

function byteLength(value: string) {
	return Buffer.byteLength(value, "utf8");
}

function truncateHead(value: string) {
	if (value.length <= MAX_TEXT_CHARS) return { value, truncated: false, originalBytes: byteLength(value) };
	return {
		value: value.slice(0, MAX_TEXT_CHARS),
		truncated: true,
		originalBytes: byteLength(value),
	};
}

function truncateHeadTail(value: string) {
	if (value.length <= MAX_TEXT_CHARS) return { value, truncated: false, originalBytes: byteLength(value) };
	const headSize = Math.floor(MAX_TEXT_CHARS / 2);
	const tailSize = MAX_TEXT_CHARS - headSize;
	return {
		value: `${value.slice(0, headSize)}\n…[truncated ${byteLength(value)} bytes total]…\n${value.slice(-tailSize)}`,
		truncated: true,
		originalBytes: byteLength(value),
	};
}

function applyTextCap(event: Record<string, unknown>, mode: "head" | "headTail") {
	const capped: Record<string, unknown> = {};
	for (const [key, value] of Object.entries(event)) {
		if (typeof value !== "string") {
			capped[key] = value;
			continue;
		}
		const next = mode === "headTail" ? truncateHeadTail(value) : truncateHead(value);
		capped[key] = next.value;
		if (next.truncated) {
			capped[`${key}_truncated`] = true;
			capped[`${key}_originalBytes`] = next.originalBytes;
		}
	}
	return capped;
}

export function capTextFields(event: Record<string, unknown>, mode: "head" | "headTail" = "head") {
	return applyTextCap(event, mode);
}

export async function appendTranscriptEvent(filePath: string, event: Record<string, unknown>) {
	const mode = event.type === "tool_result" ? "headTail" : "head";
	const line = JSON.stringify(capTextFields({ ts: nowIso(), ...event }, mode));
	await fs.appendFile(filePath, `${line}\n`, "utf8");
}

export function makeTranscriptPath(cwd: string, meta: TranscriptMeta) {
	return path.join(cwd, TRANSCRIPTS_REL, `issue-${meta.issue}`, `cycle-${meta.cycle}-${meta.role}.jsonl`);
}

export async function createTranscriptWriter(cwd: string, meta: TranscriptMeta): Promise<TranscriptWriter> {
	const filePath = makeTranscriptPath(cwd, meta);
	await fs.mkdir(path.dirname(filePath), { recursive: true });
	await fs.writeFile(filePath, "", "utf8");
	let closed = false;
	return {
		path: filePath,
		async write(event: Record<string, unknown>) {
			if (closed) return;
			await appendTranscriptEvent(filePath, event);
		},
		async close() {
			closed = true;
		},
	};
}
