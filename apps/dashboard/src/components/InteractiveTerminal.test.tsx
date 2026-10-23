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

  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  send() {}

  close() {}
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
        onError={onError}
      />
    );

    await waitFor(() => expect(onError).toHaveBeenCalledWith("session rejected"));
    expect(FakeWebSocket.instances).toHaveLength(0);
  });
});
