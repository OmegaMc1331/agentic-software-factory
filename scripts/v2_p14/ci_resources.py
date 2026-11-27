"""Bounded numeric samples for release build heartbeats."""

import ctypes
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


def memory_sample():
    if os.name == "nt":
        class MemoryStatus(ctypes.Structure):
            _fields_ = [("length", ctypes.c_uint32), ("load", ctypes.c_uint32)] + [
                (name, ctypes.c_uint64) for name in (
                    "total_phys", "avail_phys", "total_page", "avail_page",
                    "total_virtual", "avail_virtual", "avail_extended_virtual",
                )
            ]

        state = MemoryStatus()
        state.length = ctypes.sizeof(state)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(state)):
            return {"availableMemoryBytes": state.avail_phys, "commitHeadroomBytes": state.avail_page}
    if sys.platform.startswith("linux"):
        with open("/proc/meminfo", "rb") as source:
            output = source.read(4096)
        if amount := re.search(rb"(?m)^MemAvailable:\s+([0-9]{1,12}) kB$", output):
            return {"availableMemoryBytes": int(amount[1]) * 1024}
    if sys.platform == "darwin":
        output = subprocess.check_output(["/usr/bin/vm_stat"], timeout=2, stderr=subprocess.DEVNULL)
        page_size = re.search(rb"page size of ([0-9]{1,8}) bytes", output)
        free = re.search(rb"(?m)^Pages free:\s+([0-9]{1,12})\.", output)
        speculative = re.search(rb"(?m)^Pages speculative:\s+([0-9]{1,12})\.", output)
        if page_size and free and speculative:
            return {"freeOrSpeculativeMemoryBytes": int(page_size[1]) * (int(free[1]) + int(speculative[1]))}
    return {}


def sample(root, environment):
    path = (root / Path(environment.get("CARGO_TARGET_DIR", "target"))).resolve()
    while not path.exists() and path != path.parent:
        path = path.parent
    try:
        disk = shutil.disk_usage(path).free
    except OSError:
        disk = None
    try:
        memory = memory_sample()
    except (OSError, AttributeError, subprocess.SubprocessError):
        memory = {}
    values = {"cpuConcurrency": getattr(os, "process_cpu_count", os.cpu_count)(),
              "availableDiskBytes": disk, **memory}
    return {name: value for name, value in values.items() if type(value) is int and 0 <= value <= 2**60}
