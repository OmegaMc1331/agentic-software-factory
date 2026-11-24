import { expect, it } from "vitest";
import { language, languageName } from "./languages";
it.each([
  ["main.rs", "Rust"],
  ["main.ts", "TypeScript"],
  ["main.js", "JavaScript"],
  ["main.py", "Python"],
  ["data.json", "JSON"],
  ["index.html", "HTML"],
  ["app.css", "CSS"],
  ["README.md", "Markdown"],
  ["Cargo.toml", "TOML"],
  ["config.yaml", "YAML"],
])("loads bundled syntax support for %s", async (path, name) => {
  expect(languageName(path)).toBe(name);
  expect(await language(path)).toBeTruthy();
  expect(await language(path)).toBe(await language(path));
});
it("retains a plain-text fallback for unknown languages", async () => {
  expect(languageName("source.unrecognized")).toBe("Plain text");
  expect(await language("source.unrecognized")).toEqual([]);
});
