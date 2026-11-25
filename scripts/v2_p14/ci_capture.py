"""Bounded, private capture for CI commands on native runners."""

import ctypes
from dataclasses import dataclass
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import threading
import time


MAX_LOG = 16 * 1024 * 1024
RUNNER_CHANNELS = {
    "GITHUB_ENV", "GITHUB_OUTPUT", "GITHUB_PATH", "GITHUB_STATE", "GITHUB_STEP_SUMMARY",
    "GITHUB_TOKEN", "GH_TOKEN", "NODE_AUTH_TOKEN", "ACTIONS_RUNTIME_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN", "ACTIONS_ID_TOKEN_REQUEST_URL",
    "ACTIONS_RUNTIME_URL", "ACTIONS_RESULTS_URL", "ACTIONS_CACHE_URL",
    "OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY",
    "CODEX_HOME", "CLAUDE_CONFIG_DIR",
}


@dataclass(frozen=True)
class Captured:
    exit: int
    seconds: float
    problem: str | None = None


class WindowsJob:
    """Close a kill-on-close Job Object before any caller can publish results."""

    def __init__(self):
        from ctypes import wintypes

        class Limits(ctypes.Structure):
            _fields_ = [
                ("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64), ("flags", wintypes.DWORD),
                ("min_working_set", ctypes.c_size_t), ("max_working_set", ctypes.c_size_t),
                ("active_processes", wintypes.DWORD), ("affinity", ctypes.c_size_t),
                ("priority", wintypes.DWORD), ("scheduling", wintypes.DWORD),
            ]

        class ExtendedLimits(ctypes.Structure):
            _fields_ = [("limits", Limits), ("io", ctypes.c_uint64 * 6), ("memory", ctypes.c_size_t * 4)]

        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateJobObjectW.argtypes = (ctypes.c_void_p, wintypes.LPCWSTR)
        kernel.CreateJobObjectW.restype = wintypes.HANDLE
        kernel.SetInformationJobObject.argtypes = (wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD)
        kernel.AssignProcessToJobObject.argtypes = (wintypes.HANDLE, wintypes.HANDLE)
        kernel.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = (wintypes.HANDLE,)
        self.kernel = kernel
        self.handle = kernel.CreateJobObjectW(None, None)
        limits = ExtendedLimits()
        limits.limits.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.handle or not kernel.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            self.close()
            raise OSError("Process containment unavailable")

    def attach(self, process):
        handle = self.kernel.OpenProcess(0x101, False, process.pid)  # PROCESS_SET_QUOTA | PROCESS_TERMINATE
        try:
            if (not handle or not self.kernel.AssignProcessToJobObject(self.handle, handle)) and process.poll() is None:
                raise OSError("Process containment unavailable")
        finally:
            if handle:
                self.kernel.CloseHandle(handle)

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


def child_environment(environment):
    return {key: value for key, value in environment.items() if key.upper() not in RUNNER_CHANNELS}


def command_args(argv):
    if os.name == "nt" and argv[0] in {"npm", "npx"}:
        # Only checkout-owned recipes reach cmd.exe. Reject shell syntax before
        # starting a batch entry point and quote the executable via PATH.
        if any(not re.fullmatch(r"[a-zA-Z0-9._@=:/\-]+", arg) for arg in argv[1:]):
            raise ValueError("Unsupported package manager argument")
        batch = argv[0] + ".cmd"
        if not shutil.which(batch):
            raise FileNotFoundError("Package manager entry point unavailable")
        return [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", batch, *argv[1:]]
    return argv


def capture(argv, log: Path, *, cwd, environment, timeout, limit=MAX_LOG, supervise=True):
    started = time.monotonic()
    errors = []
    finished = threading.Event()
    process = None
    reader = None
    job = None
    problem = None
    try:
        if supervise and os.name == "nt":
            job = WindowsJob()
        kwargs = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {"start_new_session": supervise}
        with log.open("xb") as private:
            process = subprocess.Popen(
                command_args(argv), cwd=cwd, env=child_environment(environment),
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kwargs,
            )
            try:
                if job:
                    job.attach(process)

                def drain():
                    try:
                        remaining = limit
                        for chunk in iter(lambda: process.stdout.read1(65536), b""):
                            if len(chunk) > remaining:
                                errors.append("OUTPUT_LIMIT")
                                return
                            private.write(chunk)
                            remaining -= len(chunk)
                    except (OSError, ValueError):
                        errors.append("SCREENING_ERROR")
                    finally:
                        finished.set()

                reader = threading.Thread(target=drain, daemon=True)
                reader.start()
                while not (finished.is_set() and process.poll() is not None):
                    if errors:
                        break
                    if time.monotonic() - started >= timeout:
                        problem = "TIMEOUT"
                        break
                    time.sleep(0.025)
                problem = problem or (errors[0] if errors else None)
            except KeyboardInterrupt:
                problem = "INTERRUPTED"
            finally:
                if job:
                    job.close()
                elif supervise:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if process.poll() is None and (problem or not finished.is_set()):
                    process.kill()
                process.wait(timeout=5)
                if reader:
                    reader.join(timeout=5)
                    if reader.is_alive():
                        problem = problem or "SCREENING_ERROR"
                if not reader or not reader.is_alive():
                    process.stdout.close()
    except (OSError, ValueError, subprocess.SubprocessError):
        problem = "SCREENING_ERROR"
    finally:
        if job:
            job.close()
    actual_exit = process.returncode if process and process.returncode is not None else 1
    return Captured(actual_exit, round(time.monotonic() - started, 3), problem)
