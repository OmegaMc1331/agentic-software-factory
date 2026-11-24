export function sanitizeDocumentation(html: string) {
  const source = document.createElement("template");
  source.innerHTML = html.slice(0, 65536);
  const output = document.createElement("div");
  const allowed = new Set([
    "P",
    "PRE",
    "CODE",
    "EM",
    "STRONG",
    "B",
    "I",
    "BR",
    "UL",
    "OL",
    "LI",
    "BLOCKQUOTE",
    "H1",
    "H2",
    "H3",
    "HR",
  ]);
  const removed = new Set([
    "SCRIPT",
    "STYLE",
    "IFRAME",
    "OBJECT",
    "EMBED",
    "SVG",
    "MATH",
    "TEMPLATE",
  ]);
  let remaining = 2000;
  function copy(node: Node, parent: Node, depth: number) {
    if (--remaining < 0 || depth > 20) return;
    if (node.nodeType === Node.TEXT_NODE) {
      parent.appendChild(document.createTextNode(node.textContent ?? ""));
      return;
    }
    if (!(node instanceof HTMLElement) || removed.has(node.tagName)) return;
    const target = allowed.has(node.tagName)
      ? document.createElement(node.tagName.toLowerCase())
      : null;
    if (target) parent.appendChild(target);
    for (const child of node.childNodes) copy(child, target ?? parent, depth + 1);
  }
  for (const child of source.content.childNodes) copy(child, output, 0);
  return output.innerHTML;
}
