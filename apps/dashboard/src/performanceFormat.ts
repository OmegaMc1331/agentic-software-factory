import type { DurationStats, RateStats } from "./types";

/** Display labels for the deterministic language keys the backend derives
 * from changed-file extensions. */
export const LANGUAGE_LABELS: Record<string, string> = {
  rust: "Rust",
  typescript: "TypeScript",
  javascript: "JavaScript",
  python: "Python",
  go: "Go",
  java: "Java",
  kotlin: "Kotlin",
  swift: "Swift",
  c: "C",
  cpp: "C++",
  csharp: "C#",
  ruby: "Ruby",
  php: "PHP",
  scala: "Scala",
  objective_c: "Objective-C",
  shell: "Shell",
  powershell: "PowerShell",
  sql: "SQL",
  vue: "Vue",
  svelte: "Svelte",
  html: "HTML",
  css: "CSS",
  markdown: "Markdown",
};

export function languageLabel(key: string): string {
  return LANGUAGE_LABELS[key] ?? key;
}

/**
 * A percentage only when the sample is reliable; tiny samples render as
 * "Insufficient data" with their count instead of pretending certainty.
 */
export function formatRate(rate: RateStats): string {
  if (rate.total === 0) return "No data";
  if (!rate.reliable) return `Insufficient data (n=${rate.total})`;
  return `${Math.round((rate.rate ?? 0) * 100)}%`;
}

/** Percentage with the Wilson 95% interval, for detail views. */
export function formatRateWithInterval(rate: RateStats): string {
  if (rate.total === 0) return "No data";
  if (!rate.reliable) return `Insufficient data (n=${rate.total})`;
  const low = Math.round((rate.intervalLow ?? 0) * 100);
  const high = Math.round((rate.intervalHigh ?? 0) * 100);
  return `${Math.round((rate.rate ?? 0) * 100)}% (${low}–${high}%)`;
}

export function formatPercent(rate: RateStats): string {
  if (rate.total === 0 || !rate.reliable) return "–";
  return `${Math.round((rate.rate ?? 0) * 100)}%`;
}

export function formatDurationMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return "–";
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${String(seconds % 60).padStart(2, "0")}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${String(minutes % 60).padStart(2, "0")}m`;
}

export function formatDurationStats(duration: DurationStats): string {
  if (duration.samples === 0) return "No data";
  return formatDurationMs(duration.medianMs);
}

export function formatAttempts(value: number | null | undefined): string {
  if (value === null || value === undefined) return "–";
  return value.toFixed(2);
}

export function formatSignedPp(value: number | null | undefined): string {
  if (value === null || value === undefined) return "–";
  const rounded = Math.round(value);
  return `${rounded >= 0 ? "+" : ""}${rounded} pp`;
}
