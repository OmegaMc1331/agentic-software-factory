# Development terminal

P06 adds a **development terminal**, separate from V1's configured **agent console**. Both reuse Factory Runtime's portable-pty/ConPTY, incremental reader and bounded subscriber implementation. No agent session is converted into an unrestricted shell endpoint.

## Authorization and identity

A user approves a `kind: "shell"` profile with an installed executable and fixed arguments. Starting a terminal selects that profile and an authorized P05 root, with explicit confirmation. The backend never accepts an unchecked executable or command string in a start request. Managed agent worktrees cannot host a manual development shell.

Identity is `(project_id, root_id, terminal_id)`; terminal IDs are opaque strings, distinct from numeric agent session IDs. Each terminal retains its selected root/profile/PID. Project switches and renamed tabs do not change process ownership. A shell has the user's OS permissions and may leave its initial working directory. This is **not a filesystem or network sandbox**.

## Environment and native runtime behavior

The service clears inherited environment and allows known OS, locale, home, PATH and toolchain location variables. Provider credentials and runtime injection variables are excluded. Profile metadata contains no environment values. On Windows, canonical path verification and process path presentation are separate: trusted extended paths are converted to the ordinary Windows form before launching runtimes such as Windows PowerShell.

ConPTY may issue a cursor-position query during startup. xterm responds over the same authorized input channel; raw native test clients also implement that handshake. Startup tests must verify an executed command or file result, not just an echoed input marker.

## Streaming and lifecycle

| Event | Behavior |
| --- | --- |
| Hide, move or split a view | Preserve the process; split views subscribe to the same terminal identity |
| New terminal | Explicitly start another approved process, subject to project/global limits |
| WebSocket disconnect/reload | Preserve the process; reconnect replays bounded history |
| Terminate shell | Explicit process-tree termination; unrelated terminals/agents are unaffected |
| Shell exits | Report its exit code; release the application terminal permit |
| Backend restart | Processes are cleaned up; saved terminal views explain that explicit restart is needed |
| Project close | Refuse while a live terminal or other IDE owner remains |

The listener retains the existing authenticated WebSocket, Host/Origin policy and session rules. Each subscription is project-bound. The reader retains at most 1,000,000 bytes; subscriber queues are bounded to 256 chunks/2 MiB, with at most eight subscribers. Slow subscribers are evicted and do not block PTY draining. A socket send has a five-second deadline. Input messages are bounded to 64 KiB; resize dimensions are bounded. A replay is explicitly a bounded window, not a full transcript or a durable shell session.

xterm provides selection, copy/paste, scrollback, search and clear-view operations. The panel adds profile selection, names, reconnect, split views and explicit termination. Clearing the view does not erase server history. Audit metadata records sensitive lifecycle attempts without storing command input or output globally.

The input writer admits one write at a time with a nonblocking lock. Competing writes fail rather than accumulating blocking tasks behind a stalled PTY. A timed-out write can remain blocked until the process/pipe is terminated, but cannot create an unbounded queue of waiting writers. Explicit Stop has an independent API control lane and does not wait for that writer lock.

The native 512 KiB burst qualification evicted the initial subscriber in all three samples, with `overflowed: true`. Reconnection recovered the bounded VT history and completion marker; the command was not repeated. This validates the protection and recovery path, not uninterrupted delivery of arbitrary bursts. Live VT bytes, replay bytes and requested console characters are distinct measurements. The benchmark emits its completion marker on a fresh line because ConPTY can insert wrapping/cursor sequences inside a marker placed at the right edge.

Windows qualification uses actual native shells/ConPTY. Unix signal handling, process groups, Bash/Zsh availability and native PTY behavior require their own platform qualification. No new workflow suspension behavior is introduced.
