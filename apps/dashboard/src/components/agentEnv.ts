export interface EnvValidationError {
  line: number;
  message: string;
}

export type EnvParseResult =
  | {
      ok: true;
      env: Record<string, string>;
    }
  | {
      ok: false;
      errors: EnvValidationError[];
    };

const ENV_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/;
const LINE_BREAK_PATTERN = /\r\n|\n|\r/;

export function parseAgentEnv(text: string): EnvParseResult {
  const errors: EnvValidationError[] = [];
  const entries: [string, string][] = [];
  const firstLines = new Map<string, number>();

  text.split(LINE_BREAK_PATTERN).forEach((rawLine, index) => {
    const line = index + 1;
    const trimmed = rawLine.trim();
    if (!trimmed) return;

    const separator = trimmed.indexOf("=");
    if (separator < 0) {
      errors.push({ line, message: "Expected KEY=VALUE." });
      return;
    }

    const name = trimmed.slice(0, separator).trim();
    if (!name) {
      errors.push({ line, message: "Environment variable name is missing." });
      return;
    }
    if (!ENV_NAME_PATTERN.test(name)) {
      errors.push({ line, message: "Invalid environment variable name." });
      return;
    }

    const normalizedName = name.toLowerCase();
    const firstLine = firstLines.get(normalizedName);
    if (firstLine !== undefined) {
      errors.push({
        line,
        message: `Duplicate environment variable ${name} (first defined on line ${firstLine}).`,
      });
      return;
    }

    firstLines.set(normalizedName, line);
    entries.push([name, trimmed.slice(separator + 1).trim()]);
  });

  if (errors.length > 0) return { ok: false, errors };
  return { ok: true, env: Object.fromEntries(entries) };
}
