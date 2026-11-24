export type Document = { path: string; uri: string; content: string; revision: string; view?: unknown };
export type Project = { id: string; root: string; uri: string };
export type EditorHandle = {
  text(): string;
  replace(value: string): void;
  focus(): void;
  state(): unknown;
  restore(value: unknown): void;
  diagnostics(): unknown[];
  hover(): Promise<unknown>;
  destroy(): void;
};
export type Connection = { id: string; pid: number; socket: WebSocket; messages: any[] };
export type DiffHandle = { changes(): number; snapshot(): { original: string; modified: string }; focusModified(): void; destroy(): void };
export type Adapter = {
  mount(element: HTMLElement, doc: Document, changed: (text: string) => void): EditorHandle;
  diff(element: HTMLElement, original: string, modified: string): DiffHandle;
  lsp(project: Project, connection: Connection): Promise<void>;
  plugin(active: boolean): Promise<unknown>;
  pluginEvidence(): unknown;
};

declare global { const __CANDIDATE__: string; interface Window { p01: any } }

let csrf = "";
export async function api(path: string, method = "GET", body?: unknown): Promise<any> {
  const response = await fetch(`/api${path}`, {
    method, headers: { "Content-Type": "application/json", ...(csrf ? { "X-Factory-CSRF": csrf } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const result = await response.json();
  if (!response.ok) throw Object.assign(new Error(result.error ?? response.statusText), { status: response.status, body: result });
  return result;
}
export async function authenticate() { csrf = (await api("/auth/session", "POST", {})).csrfToken; }
export async function connect(project: Project, kind: string): Promise<Connection> {
  const lease = await api(`/projects/${project.id}/protocol/${kind}`, "POST", {});
  const socket = new WebSocket(`${location.origin.replace("http", "ws")}/api/projects/${project.id}/transport/${lease.id}`);
  const messages: any[] = [];
  socket.addEventListener("message", event => {
    const message = JSON.parse(event.data);
    messages.push(message);
    if (messages.length > 1000) messages.shift();
    if (["transport-ended", "policy-error"].includes(message.type)) {
      window.dispatchEvent(new CustomEvent("p01-error", { detail: message.error }));
    }
  });
  await new Promise<void>((resolve, reject) => { socket.onopen = () => resolve(); socket.onerror = () => reject(new Error("protocol socket failed")); });
  return { ...lease, socket, messages };
}
export async function waitFor<T>(read: () => T | undefined | null | false, timeout = 20000): Promise<T> {
  const end = performance.now() + timeout;
  while (performance.now() < end) { const value = read(); if (value) return value; await new Promise(resolve => setTimeout(resolve, 30)); }
  throw new Error("timed out waiting for real protocol evidence");
}
