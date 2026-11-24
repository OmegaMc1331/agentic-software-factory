import json
import sys


def send(value):
    data = json.dumps(value).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    sys.stdout.buffer.flush()


mode = sys.argv[1]
while True:
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1])
    message = json.loads(sys.stdin.buffer.read(length))
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        if mode == "oversized":
            sys.stdout.buffer.write(b"Content-Length: 2097153\r\n\r\n")
            sys.stdout.buffer.flush()
            continue
        if mode == "malformed":
            sys.stdout.buffer.write(b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}")
            sys.stdout.buffer.flush()
            continue
        send({"jsonrpc": "2.0", "id": request_id, "result": {"capabilities": {"textDocumentSync": 1, "workspaceSymbolProvider": True}}})
    elif method == "workspace/symbol":
        if mode == "crash":
            sys.exit(17)
        if mode == "hang":
            continue
        if mode == "flood":
            for index in range(160):
                send({"jsonrpc": "2.0", "method": "fixture/event", "params": {"index": index}})
        if mode == "server-edit":
            send({"jsonrpc": "2.0", "id": "fixture-edit", "method": "workspace/applyEdit", "params": {"edit": {"changes": {"file:///C:/outside.txt": []}}}})
        send({"jsonrpc": "2.0", "id": request_id, "result": []})
    elif method == "$/cancelRequest":
        send({"jsonrpc": "2.0", "method": "fixture/cancelled", "params": message["params"]})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": request_id, "result": None})
    elif method == "exit":
        sys.exit(0)
    elif request_id == "fixture-edit":
        send({"jsonrpc": "2.0", "method": "fixture/edit-refused", "params": {"applied": message["result"]["applied"]}})
