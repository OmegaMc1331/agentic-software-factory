import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { refreshFactorySession } from "../api";
import { InteractiveTerminal } from "./InteractiveTerminal";

vi.mock("../api", () => ({
  agentTerminalSocketUrl: (id: number) => `/api/sessions/${id}/terminal`,
  refreshFactorySession: vi.fn(),
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 100;
    rows = 28;

    loadAddon() {}

    open() {}

    write() {}

    writeln() {}

    dispose() {}

    focus() {}

    onData() {
      return { dispose: vi.fn() };
    }

    onResize() {
      return { dispose: vi.fn() };
    }
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  },
}));

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  readonly url: string;
  onclose: ((event: CloseEvent) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  send() {}

  close() {}

  closeFromServer(code: number, reason = "") {
    this.onclose?.({ code, reason } as CloseEvent);
  }
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.mocked(refreshFactorySession).mockReset();
  FakeWebSocket.instances = [];
});

describe("InteractiveTerminal authentication", () => {
  it("establishes the session before opening the WebSocket", async () => {
    let resolveSession: (token: string) => void = () => {};
    vi.mocked(refreshFactorySession).mockReturnValue(
      new Promise<string>((resolve) => {
        resolveSession = resolve;
      })
    );
    vi.stubGlobal("WebSocket", FakeWebSocket);

    render(
      <InteractiveTerminal
        sessionId={12}
        active
        output=""
        onDisconnect={vi.fn()}
        onBackpressure={vi.fn()}
        onError={vi.fn()}
      />
    );

    await waitFor(() => expect(refreshFactorySession).toHaveBeenCalled());
    expect(FakeWebSocket.instances).toHaveLength(0);

    await act(async () => {
      resolveSession("session-csrf");
      await Promise.resolve();
    });

    await waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    expect(FakeWebSocket.instances[0].url).toBe("/api/sessions/12/terminal");
  });

  it("does not open a WebSocket when authentication fails", async () => {
    vi.mocked(refreshFactorySession).mockRejectedValue(new Error("session rejected"));
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const onError = vi.fn();

    render(
      <InteractiveTerminal
        sessionId={12}
        active
        output=""
        onDisconnect={vi.fn()}
        onBackpressure={vi.fn()}
        onError={onError}
      />
    );

    await waitFor(() => expect(onError).toHaveBeenCalledWith("session rejected"));
    expect(FakeWebSocket.instances).toHaveLength(0);
  });

  it("reports a slow-subscriber close without treating it as normal completion", async () => {
    vi.mocked(refreshFactorySession).mockResolvedValue("session-csrf");
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const onDisconnect = vi.fn();
    const onBackpressure = vi.fn();

    render(
      <InteractiveTerminal
        sessionId={12}
        active
        output=""
        onDisconnect={onDisconnect}
        onBackpressure={onBackpressure}
        onError={vi.fn()}
      />
    );

    await waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    act(() => {
      FakeWebSocket.instances[0].closeFromServer(1013, "terminal output paused");
    });

    expect(onBackpressure).toHaveBeenCalledWith(
      expect.stringContaining("Reconnect to load the retained terminal history")
    );
    expect(onDisconnect).not.toHaveBeenCalled();
    expect(FakeWebSocket.instances).toHaveLength(1);
  });

  it("reports a normal terminal close as a disconnect", async () => {
    vi.mocked(refreshFactorySession).mockResolvedValue("session-csrf");
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const onDisconnect = vi.fn();
    const onBackpressure = vi.fn();

    render(
      <InteractiveTerminal
        sessionId={12}
        active
        output=""
        onDisconnect={onDisconnect}
        onBackpressure={onBackpressure}
        onError={vi.fn()}
      />
    );

    await waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    act(() => {
      FakeWebSocket.instances[0].closeFromServer(1000, "terminal session ended");
    });

    expect(onDisconnect).toHaveBeenCalledOnce();
    expect(onBackpressure).not.toHaveBeenCalled();
  });

  it("opens a fresh WebSocket when the user explicitly reconnects", async () => {
    vi.mocked(refreshFactorySession).mockResolvedValue("session-csrf");
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const { rerender } = render(
      <InteractiveTerminal
        sessionId={12}
        active
        output=""
        reconnectKey={0}
        onDisconnect={vi.fn()}
        onBackpressure={vi.fn()}
        onError={vi.fn()}
      />
    );

    await waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    rerender(
      <InteractiveTerminal
        sessionId={12}
        active
        output=""
        reconnectKey={1}
        onDisconnect={vi.fn()}
        onBackpressure={vi.fn()}
        onError={vi.fn()}
      />
    );

    await waitFor(() => expect(FakeWebSocket.instances).toHaveLength(2));
  });
});
