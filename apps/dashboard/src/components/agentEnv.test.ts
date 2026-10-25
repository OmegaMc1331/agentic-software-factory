import { describe, expect, it } from "vitest";
import { parseAgentEnv } from "./agentEnv";

describe("parseAgentEnv", () => {
  it("returns an empty environment for empty input", () => {
    expect(parseAgentEnv("")).toEqual({ ok: true, env: {} });
  });

  it("ignores blank and whitespace-only lines", () => {
    expect(parseAgentEnv("\n\n   \n\t\n")).toEqual({ ok: true, env: {} });
  });

  it("parses one valid variable", () => {
    expect(parseAgentEnv("API_KEY=example")).toEqual({
      ok: true,
      env: { API_KEY: "example" },
    });
  });

  it("parses multiple valid variables", () => {
    expect(parseAgentEnv("API_KEY=example\nDEBUG=true\nMODEL_NAME=gpt-6")).toEqual({
      ok: true,
      env: { API_KEY: "example", DEBUG: "true", MODEL_NAME: "gpt-6" },
    });
  });

  it("rejects a line without an equals sign", () => {
    expect(parseAgentEnv("INVALID_VARIABLE")).toEqual({
      ok: false,
      errors: [{ line: 1, message: "Expected KEY=VALUE." }],
    });
  });

  it("rejects an empty variable name", () => {
    expect(parseAgentEnv("=missing_name")).toEqual({
      ok: false,
      errors: [{ line: 1, message: "Environment variable name is missing." }],
    });
  });

  it("rejects a whitespace-only variable name", () => {
    expect(parseAgentEnv("   =value")).toEqual({
      ok: false,
      errors: [{ line: 1, message: "Environment variable name is missing." }],
    });
  });

  it("rejects unsupported variable name syntax", () => {
    expect(parseAgentEnv("BAD NAME=value")).toEqual({
      ok: false,
      errors: [{ line: 1, message: "Invalid environment variable name." }],
    });
    expect(parseAgentEnv("1BAD=value")).toEqual({
      ok: false,
      errors: [{ line: 1, message: "Invalid environment variable name." }],
    });
    expect(parseAgentEnv("BAD-NAME=value")).toEqual({
      ok: false,
      errors: [{ line: 1, message: "Invalid environment variable name." }],
    });
  });

  it("accepts an empty value", () => {
    expect(parseAgentEnv("EMPTY=")).toEqual({ ok: true, env: { EMPTY: "" } });
  });

  it("preserves equals signs inside values", () => {
    expect(parseAgentEnv("TOKEN=a=b=c")).toEqual({
      ok: true,
      env: { TOKEN: "a=b=c" },
    });
  });

  it("preserves Unicode and internal whitespace in values", () => {
    expect(parseAgentEnv("GREETING=héllo 世界\nMESSAGE=hello world")).toEqual({
      ok: true,
      env: { GREETING: "héllo 世界", MESSAGE: "hello world" },
    });
  });

  it("trims surrounding whitespace from names and values", () => {
    expect(parseAgentEnv("  KEY = value  ")).toEqual({
      ok: true,
      env: { KEY: "value" },
    });
  });

  it("parses Windows CRLF input", () => {
    expect(parseAgentEnv("API_KEY=example\r\nDEBUG=true\r\n")).toEqual({
      ok: true,
      env: { API_KEY: "example", DEBUG: "true" },
    });
  });

  it("parses Linux LF input", () => {
    expect(parseAgentEnv("API_KEY=example\nDEBUG=true\n")).toEqual({
      ok: true,
      env: { API_KEY: "example", DEBUG: "true" },
    });
  });

  it("rejects duplicate variable names", () => {
    expect(parseAgentEnv("DEBUG=true\nAPI_URL=https://example.com\nDEBUG=false")).toEqual({
      ok: false,
      errors: [
        {
          line: 3,
          message: "Duplicate environment variable DEBUG (first defined on line 1).",
        },
      ],
    });
  });

  it("rejects duplicate variable names that differ only by case", () => {
    expect(parseAgentEnv("DEBUG=true\ndebug=false")).toEqual({
      ok: false,
      errors: [
        {
          line: 2,
          message: "Duplicate environment variable debug (first defined on line 1).",
        },
      ],
    });
    expect(parseAgentEnv("debug=true\nDEBUG_MODE=false")).toEqual({
      ok: true,
      env: { debug: "true", DEBUG_MODE: "false" },
    });
  });

  it("reports accurate line numbers after blank lines", () => {
    expect(parseAgentEnv("\n\nINVALID\n\n=oops\nBAD NAME=x")).toEqual({
      ok: false,
      errors: [
        { line: 3, message: "Expected KEY=VALUE." },
        { line: 5, message: "Environment variable name is missing." },
        { line: 6, message: "Invalid environment variable name." },
      ],
    });
  });

  it("does not return partial success when any line is malformed", () => {
    const result = parseAgentEnv("VALID=one\nINVALID");
    expect(result.ok).toBe(false);
    expect("env" in result).toBe(false);
    if (!result.ok) {
      expect(result.errors).toEqual([{ line: 2, message: "Expected KEY=VALUE." }]);
    }
  });

  it("does not expose values in validation errors", () => {
    const secret = "sk-live-super-secret";
    const result = parseAgentEnv(`API_KEY=${secret}\n${secret}\n${secret}=value`);

    expect(result.ok).toBe(false);
    expect(JSON.stringify(result)).not.toContain(secret);
  });

  it("handles prototype-related names without mutating object prototypes", () => {
    const result = parseAgentEnv("__proto__=safe\nconstructor=value\nprototype=other");

    expect(result.ok).toBe(true);
    if (!result.ok) return;

    expect(Object.getPrototypeOf(result.env)).toBe(Object.prototype);
    expect(Object.prototype.hasOwnProperty.call(result.env, "__proto__")).toBe(true);
    expect(result.env["__proto__"]).toBe("safe");
    expect(result.env.constructor).toBe("value");
    expect(result.env.prototype).toBe("other");
    expect(({} as { polluted?: string }).polluted).toBeUndefined();
  });
});
