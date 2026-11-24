import os
import sys


print(f"P01_PTY_READY isatty={sys.stdin.isatty()} pid={os.getpid()}", flush=True)
for line in sys.stdin:
    text = line.strip()
    if text == "exit":
        print("P01_PTY_EXIT", flush=True)
        break
    if text == "size":
        size = os.get_terminal_size()
        print(f"P01_SIZE={size.columns}x{size.lines}", flush=True)
    elif text.startswith("echo "):
        print("P01_ECHO=" + text[5:], flush=True)
    else:
        print("P01_REFUSED: only echo, size and exit are supported by this trusted PTY probe", flush=True)
