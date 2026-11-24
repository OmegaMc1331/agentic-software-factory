import type { Extension } from "@codemirror/state";
import { StreamLanguage } from "@codemirror/language";
export function languageName(path: string) {
  const ext = path.split(".").at(-1)?.toLowerCase() ?? "";
  return (
    (
      {
        rs: "Rust",
        ts: "TypeScript",
        tsx: "TSX",
        js: "JavaScript",
        jsx: "JSX",
        mjs: "JavaScript",
        cjs: "JavaScript",
        py: "Python",
        json: "JSON",
        jsonc: "JSON",
        html: "HTML",
        htm: "HTML",
        css: "CSS",
        md: "Markdown",
        markdown: "Markdown",
        toml: "TOML",
        yaml: "YAML",
        yml: "YAML",
      } as Record<string, string>
    )[ext] ?? "Plain text"
  );
}
const cache = new Map<string, Promise<Extension>>();
export function language(path: string): Promise<Extension> {
  const name = languageName(path);
  let result = cache.get(name);
  if (result) return result;
  result = (async () => {
    switch (name) {
      case "Rust":
        return (await import("@codemirror/lang-rust")).rust();
      case "TypeScript":
      case "TSX":
      case "JavaScript":
      case "JSX":
        return (await import("@codemirror/lang-javascript")).javascript({
          typescript: name === "TypeScript" || name === "TSX",
          jsx: name === "TSX" || name === "JSX",
        });
      case "Python":
        return (await import("@codemirror/lang-python")).python();
      case "JSON":
        return (await import("@codemirror/lang-json")).json();
      case "HTML":
        return (await import("@codemirror/lang-html")).html();
      case "CSS":
        return (await import("@codemirror/lang-css")).css();
      case "Markdown":
        return (await import("@codemirror/lang-markdown")).markdown();
      case "TOML":
        return StreamLanguage.define((await import("@codemirror/legacy-modes/mode/toml")).toml);
      case "YAML":
        return StreamLanguage.define((await import("@codemirror/legacy-modes/mode/yaml")).yaml);
      default:
        return [];
    }
  })();
  cache.set(name, result);
  return result;
}
