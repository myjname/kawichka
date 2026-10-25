"""Identical isolated synthetic inference substitute for BOTH UI prototypes.

No model files, user settings, network, or application database are opened.
Two CPU threads hash one shared 32 MiB buffer. This is not an AI benchmark.
"""
import hashlib
import json
import os
import sys
import threading
import time

stop = threading.Event()
buffer = b"Kavychka load fixture\0" * (32 * 1024 * 1024 // 21)


def cpu():
    while not stop.is_set():
        hashlib.sha256(buffer).digest()


def input_loop():
    for line in sys.stdin:
        if line.strip() == "stop":
            stop.set()
            return
    stop.set()


threading.Thread(target=input_loop, daemon=True).start()
threads = [threading.Thread(target=cpu) for _ in range(2)]
for thread in threads:
    thread.start()
started = time.monotonic()
print(json.dumps({"event": "ready", "pid": os.getpid(), "bytes": len(buffer)}), flush=True)
while not stop.wait(0.1) and time.monotonic() - started < 120:
    print(json.dumps({"event": "progress", "value": int((time.monotonic() - started) * 5) % 101}), flush=True)
stop.set()
for thread in threads:
    thread.join()
print(json.dumps({"event": "stopped"}), flush=True)
