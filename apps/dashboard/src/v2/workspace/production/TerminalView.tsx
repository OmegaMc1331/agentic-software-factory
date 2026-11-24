import { useEffect, useRef, useState } from "react";
import { refreshFactorySession } from "../../../api";
import type { AgentSession } from "../../../types";
import type { TerminalInfo } from "../../ide/types";
import { useTheme } from "../../design-system/themes/theme";
import { Button, Input } from "../../design-system/primitives/Controls";
import { projectPath } from "../../projects/projectApi";
import "@xterm/xterm/css/xterm.css";

export default function TerminalView({
  projectId,
  session,
  development,
}: { projectId: string } & (
  { session: AgentSession; development?: never } | { session?: never; development: TerminalInfo }
)) {
  const host = useRef<HTMLDivElement>(null),
    appearance = useRef<(theme: Record<string, string>) => void>();
  const [error, setError] = useState(""),
    [retry, setRetry] = useState(0);
  const { resolved } = useTheme();
  const active = development
    ? development.exit_code === null
    : !!session && ["running", "active"].includes(session.status);
  const sessionId = development?.id ?? session?.id;
  const isDevelopment = !!development;
  const output = useRef((session?.stdout ?? "") + (session?.stderr ?? ""));
  const tools = useRef<{
    clear(): void;
    find(text: string): void;
    copy(): Promise<void>;
    paste(): Promise<void>;
  }>();
  const [search, setSearch] = useState("");
  const [connected, setConnected] = useState(false);
  const palette = () => {
    const style = getComputedStyle(document.documentElement);
    return {
      background: style.getPropertyValue("--color-canvas").trim(),
      foreground: style.getPropertyValue("--color-text").trim(),
      cursor: style.getPropertyValue("--color-accent").trim(),
      selectionBackground: style.getPropertyValue("--color-selection").trim(),
    };
  };
  useEffect(() => {
    appearance.current?.(palette());
  }, [resolved]);
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let disposed = false,
      cleanup = () => {};
    setConnected(false);
    void Promise.all([
      import("@xterm/xterm"),
      import("@xterm/addon-fit"),
      import("@xterm/addon-search"),
    ])
      .then(async ([{ Terminal }, { FitAddon }, { SearchAddon }]) => {
        if (disposed) return;
        const terminal = new Terminal({
          convertEol: true,
          fontSize: 12,
          fontFamily: getComputedStyle(element).getPropertyValue("--font-code"),
          scrollback: 3000,
          theme: palette(),
          allowProposedApi: false,
          disableStdin: true,
          screenReaderMode: true,
        });
        const fit = new FitAddon();
        terminal.loadAddon(fit);
        const finder = new SearchAddon();
        terminal.loadAddon(finder);
        terminal.open(element);
        appearance.current = (theme) => {
          terminal.options.theme = theme;
        };
        const resize = () => {
          if (element.clientWidth && element.clientHeight) fit.fit();
        };
        resize();
        const observer = new ResizeObserver(resize);
        observer.observe(element);
        let socket: WebSocket | undefined;
        let data: { dispose(): void } | undefined, size: { dispose(): void } | undefined;
        tools.current = {
          clear: () => terminal.clear(),
          find: (text) => {
            finder.findNext(text);
          },
          copy: () => navigator.clipboard.writeText(terminal.getSelection()),
          paste: async () => {
            const text = await navigator.clipboard.readText();
            terminal.paste(text);
          },
        };
        cleanup = () => {
          appearance.current = undefined;
          tools.current = undefined;
          observer.disconnect();
          data?.dispose();
          size?.dispose();
          if (socket) {
            socket.onclose = null;
            socket.onmessage = null;
            socket.onerror = null;
            socket.close();
          }
          terminal.dispose();
        };
        if (!active && !isDevelopment) {
          terminal.write(output.current || "No terminal output was retained.\r\n");
          return;
        }
        try {
          await refreshFactorySession();
          if (disposed) return;
          const protocol = location.protocol === "https:" ? "wss" : "ws";
          socket = new WebSocket(
            `${protocol}://${location.host}/api${projectPath(projectId, isDevelopment ? `ide/terminals/${sessionId}/socket` : `sessions/${sessionId}/terminal`)}`
          );
          socket.binaryType = "arraybuffer";
          const send = (value: object) => {
            if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(value));
          };
          socket.onopen = () => {
            terminal.options.disableStdin = !active;
            setConnected(true);
            resize();
            send({ type: "resize", cols: terminal.cols, rows: terminal.rows });
            setError("");
          };
          socket.onmessage = (event) => {
            if (event.data instanceof ArrayBuffer) terminal.write(new Uint8Array(event.data));
            else if (typeof event.data === "string") {
              if (isDevelopment) {
                try {
                  const status = JSON.parse(event.data) as {
                    type: string;
                    overflowed?: boolean;
                    terminal?: TerminalInfo;
                  };
                  if (status.type === "closed")
                    setError(
                      status.overflowed
                        ? "Output subscriber overflowed. Reconnect to bounded history."
                        : `Terminal exited (${status.terminal?.exit_code ?? "pending"}).`
                    );
                } catch {
                  setError("Invalid terminal status message.");
                }
              } else terminal.writeln(event.data);
            }
          };
          socket.onclose = (event) => {
            terminal.options.disableStdin = true;
            setConnected(false);
            setError(
              event.code === 1013
                ? "Output paused because this view fell behind. Reconnect to retained history."
                : "Terminal connection closed. Closing a view does not terminate its process."
            );
          };
          socket.onerror = () =>
            setError("Terminal connection failed. Check the project runtime, then reconnect.");
          data = terminal.onData((value) => send({ type: "input", data: value }));
          size = terminal.onResize(({ cols, rows }) => send({ type: "resize", cols, rows }));
        } catch (reason) {
          if (!disposed)
            setError(reason instanceof Error ? reason.message : "Terminal authentication failed.");
        }
      })
      .catch((reason) => {
        if (!disposed) setError(String(reason));
      });
    return () => {
      disposed = true;
      cleanup();
    };
  }, [projectId, sessionId, active, retry, isDevelopment]);
  return (
    <div className="ws-terminal" data-connection={connected ? "open" : "closed"}>
      <span className="ide-status" role="status">
        {connected ? "Terminal connected" : "Terminal disconnected or connecting"}
      </span>
      <div className="files-toolbar">
        <Input
          label="Find terminal output"
          hideLabel
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") tools.current?.find(search);
          }}
        />
        <Button tone="quiet" onClick={() => tools.current?.find(search)}>
          Find output
        </Button>
        <Button tone="quiet" onClick={() => tools.current?.clear()}>
          Clear
        </Button>
        <Button
          tone="quiet"
          onClick={() => void tools.current?.copy().catch((error) => setError(String(error)))}
        >
          Copy selection
        </Button>
        <Button
          tone="quiet"
          disabled={!active}
          onClick={() => void tools.current?.paste().catch((error) => setError(String(error)))}
        >
          Paste
        </Button>
      </div>
      <div
        className="ws-terminal-host"
        ref={host}
        role="region"
        aria-label={`${development ? "Development" : "Agent"} terminal ${projectId} session ${sessionId}`}
      />
      {error && (
        <div className="ws-inline-error" role="alert">
          {error}
          <Button onClick={() => setRetry((v) => v + 1)}>Reconnect terminal</Button>
        </div>
      )}
    </div>
  );
}
