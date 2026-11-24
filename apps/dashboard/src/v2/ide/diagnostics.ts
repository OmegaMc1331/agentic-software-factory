import type { Text } from "@codemirror/state";
export function diagnosticFits(text: Text, diagnostic: unknown) {
  if (!diagnostic || typeof diagnostic !== "object") return false;
  const item = diagnostic as {
    message?: unknown;
    source?: unknown;
    range?: {
      start?: { line: number; character: number };
      end?: { line: number; character: number };
    };
  };
  if (
    typeof item.message !== "string" ||
    (item.source !== undefined && typeof item.source !== "string")
  )
    return false;
  const start = item.range?.start,
    end = item.range?.end;
  function valid(position: typeof start) {
    if (
      !position ||
      !Number.isSafeInteger(position.line) ||
      !Number.isSafeInteger(position.character) ||
      position.line < 0 ||
      position.line >= text.lines ||
      position.character < 0
    )
      return false;
    const line = text.line(position.line + 1).text,
      at = position.character;
    if (at > line.length) return false;
    return !(
      at > 0 &&
      at < line.length &&
      line.charCodeAt(at) >= 0xdc00 &&
      line.charCodeAt(at) <= 0xdfff &&
      line.charCodeAt(at - 1) >= 0xd800 &&
      line.charCodeAt(at - 1) <= 0xdbff
    );
  }
  return (
    valid(start) &&
    valid(end) &&
    !!start &&
    !!end &&
    (start.line < end.line || (start.line === end.line && start.character <= end.character))
  );
}
