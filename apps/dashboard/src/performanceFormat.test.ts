import { describe, expect, it } from "vitest";
import {
  formatAttempts,
  formatDurationMs,
  formatRate,
  formatRateWithInterval,
  formatSignedPp,
  languageLabel,
} from "./performanceFormat";
import type { RateStats } from "./types";

function rate(successes: number, total: number, reliable: boolean): RateStats {
  return {
    successes,
    total,
    rate: total === 0 ? null : successes / total,
    intervalLow: total === 0 ? null : 0.8,
    intervalHigh: total === 0 ? null : 0.98,
    reliable,
  };
}

describe("performance formatting", () => {
  it("shows percentages only for reliable samples", () => {
    expect(formatRate(rate(80, 86, true))).toBe("93%");
    expect(formatRate(rate(2, 2, false))).toBe("Insufficient data (n=2)");
    expect(formatRate(rate(0, 0, false))).toBe("No data");
  });

  it("appends the Wilson interval for detail views", () => {
    expect(formatRateWithInterval(rate(80, 86, true))).toBe("93% (80–98%)");
    expect(formatRateWithInterval(rate(1, 2, false))).toBe("Insufficient data (n=2)");
  });

  it("formats durations compactly", () => {
    expect(formatDurationMs(94_000)).toBe("1m 34s");
    expect(formatDurationMs(45_000)).toBe("45s");
    expect(formatDurationMs(3_780_000)).toBe("1h 03m");
    expect(formatDurationMs(null)).toBe("–");
  });

  it("formats attempt averages and trend deltas", () => {
    expect(formatAttempts(1.1046)).toBe("1.10");
    expect(formatAttempts(null)).toBe("–");
    expect(formatSignedPp(12.4)).toBe("+12 pp");
    expect(formatSignedPp(-7.6)).toBe("-8 pp");
    expect(formatSignedPp(null)).toBe("–");
  });

  it("maps language keys to labels", () => {
    expect(languageLabel("cpp")).toBe("C++");
    expect(languageLabel("objective_c")).toBe("Objective-C");
    expect(languageLabel("custom")).toBe("custom");
  });
});
