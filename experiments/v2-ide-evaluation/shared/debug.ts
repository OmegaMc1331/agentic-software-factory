import { api, connect, waitFor, type Connection, type Project } from "./bridge";

export class DebugSession {
  sequence = 1;
  events: any[] = [];
  pending = new Map<number, { resolve: (value: any) => void; reject: (reason: Error) => void }>();
  constructor(public connection: Connection, public project: Project) {
    connection.socket.addEventListener("message", event => {
      const message = JSON.parse(event.data);
      if (message.type === "response") {
        const pending = this.pending.get(message.request_seq);
        this.pending.delete(message.request_seq);
        if (message.success) pending?.resolve(message.body); else pending?.reject(new Error(message.message ?? "DAP request failed"));
      } else if (message.type === "event") this.events.push(message);
    });
  }
  request(command: string, args: unknown = {}): Promise<any> {
    const seq = this.sequence++;
    const result = new Promise((resolve, reject) => {
      this.pending.set(seq, { resolve, reject });
      setTimeout(() => { if (this.pending.delete(seq)) reject(new Error(`DAP timeout: ${command}`)); }, 20000);
    });
    this.connection.socket.send(JSON.stringify({ seq, type: "request", command, arguments: args }));
    return result;
  }
  async stop() { this.connection.socket.close(); await api(`/projects/${this.project.id}/transport/${this.connection.id}`, "DELETE"); }
}

export async function breakpointSession(project: Project) {
  const start = performance.now();
  const connection = await connect(project, "debugpy");
  const session = new DebugSession(connection, project);
  const capabilities = await session.request("initialize", { clientID: "asf-p01", adapterID: "debugpy", pathFormat: "path", linesStartAt1: true, columnsStartAt1: true, supportsRunInTerminalRequest: false });
  const adapterReadyMs = performance.now() - start;
  const launch = session.request("launch", {});
  await waitFor(() => session.events.find(event => event.event === "initialized"));
  const separator = project.root.includes("\\") ? "\\" : "/";
  const breakpoints = await session.request("setBreakpoints", { source: { path: project.root + separator + "debug.py" }, breakpoints: [{ line: 2 }] });
  await session.request("configurationDone");
  await launch;
  const stopped = await waitFor(() => session.events.find(event => event.event === "stopped"));
  const stack = await session.request("stackTrace", { threadId: stopped.body.threadId });
  const scopes = await session.request("scopes", { frameId: stack.stackFrames[0].id });
  const variables = await session.request("variables", { variablesReference: scopes.scopes[0].variablesReference });
  return { session, evidence: { adapterReadyMs, breakpointMs: performance.now() - start, capabilities, breakpoints, stopped, stack, variables } };
}
