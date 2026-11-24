import { linter, type Diagnostic } from "@codemirror/lint";
import { keymap } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import type { InstalledExtension } from "./types";
import { language } from "../files/languages";
import { languageId } from "./client";

export function formatText(content: string, operation: string) {
  if (operation === "trim-trailing-whitespace") return content.replace(/[\t ]+(?=\r?$)/gm, "");
  if (operation === "final-newline") return content.endsWith("\n") ? content : content + "\n";
  if (operation === "sort-lines")
    return content
      .split("\n")
      .sort((a, b) => a.localeCompare(b, "en"))
      .join("\n");
  throw new Error("Unsupported declarative formatter.");
}
export async function editorContributions(
  installed: InstalledExtension[],
  path: string,
  command: (extension: InstalledExtension, id: string) => void
): Promise<Extension[]> {
  const extensions: Extension[] = [];
  for (const extension of installed.filter((e) => e.enabled && e.status === "active")) {
    const contributions = extension.manifest.contributions;
    const alias = contributions.languages?.find((alias) => path.endsWith(`.${alias.extension}`));
    if (alias)
      extensions.push(
        await language(
          `extension.${({ rust: "rs", typescript: "ts", javascript: "js", python: "py", markdown: "md" } as Record<string, string>)[alias.syntax] ?? alias.syntax}`
        )
      );
    if (contributions.diagnostics?.length)
      extensions.push(
        linter((view) => {
          const text = view.state.doc.toString();
          const diagnostics: Diagnostic[] = [];
          for (const rule of contributions.diagnostics ?? []) {
            let index = 0;
            while (diagnostics.length < 200 && (index = text.indexOf(rule.needle, index)) !== -1) {
              diagnostics.push({
                from: index,
                to: index + rule.needle.length,
                message: rule.message,
                severity: rule.severity,
                source: extension.manifest.name,
              });
              index += rule.needle.length;
            }
          }
          return diagnostics;
        })
      );
    extensions.push(
      keymap.of(
        (contributions.commands ?? [])
          .filter((command) => command.keybinding)
          .map((binding) => ({
            key: binding.keybinding!,
            run: () => {
              command(extension, binding.id);
              return true;
            },
          }))
      )
    );
  }
  return extensions;
}
export const formatterApplies = (languages: string[], path: string) =>
  !languages.length || languages.includes(languageId(path));
