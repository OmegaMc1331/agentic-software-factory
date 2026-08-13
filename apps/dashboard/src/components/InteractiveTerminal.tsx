import { useEffect, useRef } from "react";
import { agentTerminalSocketUrl } from "../api";

export function InteractiveTerminal({
  sessionId,
  active,
  output,
  onDisconnect,
  onError,
}: {
  sessionId: number;
  active: boolean;
  output: string;
  onDisconnect: () => void;
  onError: (message: string) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    let disposed = false;
    let cleanup = () => {};
    void Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit")])
      .then(([{ Terminal }, { FitAddon }]) => {
        if (disposed) return;
        const terminal = new Terminal({
          convertEol: true,
          cursorBlink: active,
          cursorStyle: "bar",
          fontFamily: '"Cascadia Mono", "SFMono-Regular", Consolas, monospace',
          fontSize: 12,
          lineHeight: 1.35,
          scrollback: 5000,
          theme: {
            background: "#11151b",
            foreground: "#d8dce4",
            cursor: "#d8dce4",
            selectionBackground: "#344159",
            black: "#11151b",
            red: "#d96b63",
            green: "#65b883",
            yellow: "#d6a85f",
            blue: "#78a5e8",
            magenta: "#b49ad9",
            cyan: "#69b5bc",
            white: "#d8dce4",
          },
        });
        const fit = new FitAddon();
        terminal.loadAddon(fit);
        terminal.open(container);
        fit.fit();

        if (!active) {
          terminal.write(output || "No terminal output was recorded.\r\n");
          cleanup = () => terminal.dispose();
          return;
        }

        const socket = new WebSocket(agentTerminalSocketUrl(sessionId));
        socket.binaryType = "arraybuffer";
        socket.onopen = () => {
          fit.fit();
          socket.send(JSON.stringify({ type: "resize", cols: terminal.cols, rows: terminal.rows }));
          terminal.focus();
        };
        socket.onmessage = (event) => {
          if (event.data instanceof ArrayBuffer) {
            terminal.write(new Uint8Array(event.data));
          } else if (event.data instanceof Blob) {
            void event.data.arrayBuffer().then((data) => terminal.write(new Uint8Array(data)));
          } else {
            terminal.writeln(`\r\n${String(event.data)}`);
          }
        };
        socket.onclose = onDisconnect;
        socket.onerror = () => {
          terminal.writeln("\r\nTerminal connection lost.");
          onError("The interactive terminal connection failed.");
        };

        const input = terminal.onData((data) => {
          if (socket.readyState === WebSocket.OPEN) {
            socket.send(JSON.stringify({ type: "input", data }));
          }
        });
        const resize = terminal.onResize(({ cols, rows }) => {
          if (socket.readyState === WebSocket.OPEN) {
            socket.send(JSON.stringify({ type: "resize", cols, rows }));
          }
        });
        const observer =
          typeof ResizeObserver === "undefined"
            ? null
            : new ResizeObserver(() => {
                try {
                  fit.fit();
                } catch {
                  // The panel may be between layout states while it closes.
                }
              });
        observer?.observe(container);

        cleanup = () => {
          observer?.disconnect();
          input.dispose();
          resize.dispose();
          socket.onclose = null;
          socket.close();
          terminal.dispose();
        };
      })
      .catch((reason: unknown) => {
        if (!disposed) {
          onError(
            reason instanceof Error ? reason.message : "The terminal renderer could not load."
          );
        }
      });

    return () => {
      disposed = true;
      cleanup();
    };
  }, [active, onDisconnect, onError, output, sessionId]);

  return (
    <div className="interactive-terminal" ref={containerRef} aria-label="Interactive terminal" />
  );
}
