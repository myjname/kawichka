"""Own-process benchmark only. Does not send keyboard/mouse input to the desktop."""
from __future__ import annotations
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import math
import os
from pathlib import Path
import platform
import queue
import statistics
import subprocess
import sys
import threading
import time
import winreg

ROOT = Path(__file__).resolve().parent
COMMON = ROOT / "common"
K32, PSAPI, U32 = C.WinDLL("kernel32", use_last_error=True), C.WinDLL("psapi", use_last_error=True), C.WinDLL("user32", use_last_error=True)
K32.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]
K32.OpenProcess.restype = W.HANDLE
K32.CloseHandle.argtypes = [W.HANDLE]
K32.GetProcessTimes.argtypes = [W.HANDLE, C.POINTER(W.FILETIME), C.POINTER(W.FILETIME), C.POINTER(W.FILETIME), C.POINTER(W.FILETIME)]
U32.GetGuiResources.argtypes = [W.HANDLE, W.DWORD]


class Memory(C.Structure):
    _fields_ = [("cb", W.DWORD), ("PageFaultCount", W.DWORD)] + [(s, C.c_size_t) for s in (
        "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage",
        "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage")]


PSAPI.GetProcessMemoryInfo.argtypes = [W.HANDLE, C.POINTER(Memory), W.DWORD]
K32.GetProcessHandleCount.argtypes = [W.HANDLE, C.POINTER(W.DWORD)]
K32.SetProcessAffinityMask.argtypes = [W.HANDLE, C.c_size_t]
K32.GetProcessAffinityMask.argtypes = [W.HANDLE, C.POINTER(C.c_size_t), C.POINTER(C.c_size_t)]


def restrict_cores(pid, cores=4):
    handle=K32.OpenProcess(0x0200|0x0400,False,pid)
    if not handle:raise C.WinError(C.get_last_error())
    try:
        allowed,system=C.c_size_t(),C.c_size_t()
        if not K32.GetProcessAffinityMask(handle,C.byref(allowed),C.byref(system)):raise C.WinError(C.get_last_error())
        bits=[i for i in range(C.sizeof(C.c_size_t)*8) if allowed.value&(1<<i)][:cores]
        mask=sum(1<<i for i in bits)
        if not K32.SetProcessAffinityMask(handle,mask):raise C.WinError(C.get_last_error())
        return len(bits),mask
    finally:K32.CloseHandle(handle)


class PhysicalMemory(C.Structure):
    _fields_ = [("length", W.DWORD), ("load", W.DWORD)] + [(s, C.c_ulonglong) for s in (
        "total", "available", "total_page", "available_page", "total_virtual", "available_virtual", "extended")]


class Point(C.Structure):
    _fields_=[("x",W.LONG),("y",W.LONG)]


class Rect(C.Structure):
    _fields_=[("left",W.LONG),("top",W.LONG),("right",W.LONG),("bottom",W.LONG)]


U32.WindowFromPoint.argtypes=[Point]
U32.WindowFromPoint.restype=W.HWND
U32.GetWindowRect.argtypes=[W.HWND,C.POINTER(Rect)]


def window_hit_test(native):
    hwnd=native["hwnd"];r=Rect()
    if not U32.GetWindowRect(hwnd,C.byref(r)):raise C.WinError(C.get_last_error())
    corner=U32.WindowFromPoint(Point(r.left+1,r.top+1))
    solid=U32.WindowFromPoint(Point(r.left+native["width"]*42//150,r.top+native["height"]*60//170))
    return dict(corner_passes_to_other_window=corner!=hwnd,solid_resolves_to_mascot=solid==hwnd,
                method="WindowFromPoint on own window; no mouse movement or click injection")


def process_metrics(pid):
    handle = K32.OpenProcess(0x0400 | 0x0010, False, pid)
    if not handle:
        return None
    try:
        m = Memory(); m.cb = C.sizeof(m)
        if not PSAPI.GetProcessMemoryInfo(handle, C.byref(m), m.cb):
            return None
        created, exited, kernel, user = (W.FILETIME() for _ in range(4))
        K32.GetProcessTimes(handle, C.byref(created), C.byref(exited), C.byref(kernel), C.byref(user))
        handles = W.DWORD(); K32.GetProcessHandleCount(handle, C.byref(handles))
        ticks = lambda t: (t.dwHighDateTime << 32) | t.dwLowDateTime
        return dict(private_bytes=m.PrivateUsage, working_set=m.WorkingSetSize,
                    cpu_seconds=(ticks(kernel) + ticks(user)) / 1e7,
                    handles=handles.value, gdi=U32.GetGuiResources(handle, 0), user=U32.GetGuiResources(handle, 1))
    finally:
        K32.CloseHandle(handle)


def describe_machine():
    mem = PhysicalMemory(); mem.length = C.sizeof(mem)
    K32.GlobalMemoryStatusEx(C.byref(mem))
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0") as key:
            cpu = winreg.QueryValueEx(key, "ProcessorNameString")[0].strip()
    except OSError:
        cpu = platform.processor()
    return dict(os=platform.platform(), python=sys.version, logical_cpus=os.cpu_count(), cpu=cpu,
                ram_bytes=mem.total, available_ram_bytes=mem.available,
                warning="This is the actual host, not an emulated 8 GB laptop.")


def percentile(values, p=95):
    if not values:
        return None
    v = sorted(values); return v[max(0, math.ceil(len(v) * p / 100) - 1)]


def summary(values):
    return dict(n=len(values), median=statistics.median(values), p95=percentile(values), max=max(values)) if values else {}


class Client:
    def __init__(self, exe, out, extra=()):
        self.exe, self.out = Path(exe).resolve(), Path(out)
        self.out.mkdir(parents=True, exist_ok=True)
        self.log = (self.out / "stderr.log").open("w", encoding="utf-8")
        self.messages, self.serial, self.worker_pid = queue.Queue(), 0, None
        env = dict(os.environ, SLINT_STYLE="fluent", DOTNET_CLI_TELEMETRY_OPTOUT="1", PYTHONDONTWRITEBYTECODE="1")
        started = time.perf_counter()
        self.proc = subprocess.Popen([str(self.exe), "--bench", "--common", str(COMMON), "--python", sys.executable, *extra],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True, encoding="utf-8", bufsize=1,
            env=env, creationflags=subprocess.CREATE_NO_WINDOW, cwd=self.exe.parent)
        self.cores,self.affinity_mask=restrict_cores(self.proc.pid)
        def reader():
            for line in self.proc.stdout:
                try: self.messages.put(json.loads(line))
                except json.JSONDecodeError: pass
            self.messages.put({"eof": self.proc.poll()})
        threading.Thread(target=reader, daemon=True).start()
        try:
            self.ready = self.messages.get(timeout=30)
            self.startup_ms = (time.perf_counter() - started) * 1000
            if self.ready.get("event_name") != "ready":
                raise RuntimeError(f"No ready message: {self.ready}, see {self.out / 'stderr.log'}")
        except Exception:
            if self.proc.poll() is None: self.proc.kill()
            self.proc.wait(timeout=5);self.log.close()
            raise
        self.raw = (self.out / "commands.jsonl").open("w", encoding="utf-8")
        self.raw.write(json.dumps(dict(startup_ms=self.startup_ms, ready=self.ready), ensure_ascii=False) + "\n")

    def command(self, command, **params):
        self.serial += 1; ident = self.serial
        started = time.perf_counter()
        self.proc.stdin.write(json.dumps(dict(id=ident, command=command, **params), ensure_ascii=False) + "\n")
        self.proc.stdin.flush()
        response = self.messages.get(timeout=30)
        duration = (time.perf_counter() - started) * 1000
        if response.get("id") != ident or not response.get("ok"):
            raise RuntimeError(f"{command}: {response}")
        self.raw.write(json.dumps(dict(command=command, **params, duration_ms=duration, response=response), ensure_ascii=False) + "\n")
        self.raw.flush()
        if command == "load": self.worker_pid = response["result"]["pid"]
        return response["result"], duration, response.get("queue_ms", 0)

    def sample(self, seconds, interval=0.25):
        samples = []; started = time.perf_counter()
        while time.perf_counter() - started < seconds:
            main = process_metrics(self.proc.pid)
            child = process_metrics(self.worker_pid) if self.worker_pid else None
            samples.append(dict(at=time.perf_counter(), main=main, worker=child))
            time.sleep(interval)
        cpus = self.cores
        cpu = 0;tree_cpu=0
        if len(samples) > 1:
            cpu = (samples[-1]["main"]["cpu_seconds"] - samples[0]["main"]["cpu_seconds"]) / (samples[-1]["at"] - samples[0]["at"]) / cpus * 100
            totals=lambda s:sum(v["cpu_seconds"] for v in (s["main"],s["worker"]) if v)
            tree_cpu=(totals(samples[-1])-totals(samples[0]))/(samples[-1]["at"]-samples[0]["at"])/cpus*100
        return dict(cpu_percent_allocated_cores=cpu,tree_cpu_percent_allocated_cores=tree_cpu,affinity_cores=self.cores,
            private_bytes=summary([s["main"]["private_bytes"] for s in samples]),
            working_set=summary([s["main"]["working_set"] for s in samples]),
            tree_private_bytes=summary([sum(v["private_bytes"] for v in (s["main"], s["worker"]) if v) for s in samples]),
            last=samples[-1], samples=samples)

    def action_frame(self, command, target="settings", **params):
        started=time.perf_counter()
        result,_,queue_ms=self.command(command,**params)
        self.command("present",target=target)
        return result,(time.perf_counter()-started)*1000,queue_ms

    def close(self):
        if self.proc.poll() is None:
            try: self.command("quit"); self.proc.wait(timeout=8)
            except Exception: self.proc.kill(); self.proc.wait(timeout=5)
        if hasattr(self, "raw"): self.raw.close()
        self.log.close()


def smoke(exe, out, extra=()):
    c = Client(exe, out, extra)
    try:
        print(json.dumps(dict(startup_ms=c.startup_ms, ready=c.ready), ensure_ascii=False), flush=True)
        c.action_frame("settings"); time.sleep(0.4)
        c.command("theme", theme="dark"); c.command("message"); time.sleep(0.3)
        c.command("snapshot", path=str(Path(out).resolve() / "dark"))
        c.command("library"); c.command("page", page=1); time.sleep(0.3)
        c.command("theme", theme="light");time.sleep(0.3)
        c.command("snapshot", path=str(Path(out).resolve() / "light"))
        c.command("load");time.sleep(0.6)
        c.command("cancel");time.sleep(0.5)
        result=c.command("worker")[0]
        if result["running"]: raise RuntimeError("Worker did not stop")
        print("PASS smoke", flush=True)
    finally: c.close()


def full(exe, out, seconds=15, cycles=200, extra=()):
    c = Client(exe, out, extra); results = dict(ready=c.ready,startup_ms=c.startup_ms,affinity_cores=c.cores,affinity_mask=c.affinity_mask)
    try:
        native=c.command("native")[0]
        for key in ("no_activate", "tool_window", "topmost", "region_present", "transparent_corner_excluded", "solid_part_included"):
            if not native.get(key): raise RuntimeError(f"Native prerequisite failed: {key}: {native}")
        results["native"]=native
        results["hit_test"]=window_hit_test(native)
        if not all(results["hit_test"][key] for key in ("corner_passes_to_other_window","solid_resolves_to_mascot")):
            raise RuntimeError(f"Window hit test failed (could also be another topmost window): {results['hit_test']}")
        time.sleep(1)
        results["idle_pet"]=c.sample(seconds)
        c.command("animate", enabled=0); time.sleep(.5)
        results["static_pet"]=c.sample(max(3, seconds/3))
        c.command("animate", enabled=1)
        _,duration,_=c.action_frame("settings");results["first_settings_frame_ms"]=duration
        time.sleep(1)
        results["settings_idle"]=c.sample(seconds)
        _,duration,_=c.action_frame("library");results["load_10000_rows_frame_ms"]=duration
        time.sleep(.5)
        results["library_idle"]=c.sample(max(3,seconds/3))
        actions,queues=[],[]
        for i in range(100):
            _,ms,queue_ms=c.action_frame("page",page=i%4);actions.append(ms);queues.append(queue_ms);time.sleep(.02)
        results["page_frame_ms"]=summary(actions);results["ui_queue_idle_ms"]=summary(queues)
        baseline=process_metrics(c.proc.pid)
        actions=[];checkpoints=[]
        for i in range(cycles):
            _,ms,_=c.command("settings",show=0)
            _,ms,_=c.action_frame("settings",show=1);actions.append(ms)
            c.command("message");c.command("confirm")
            if i%20==0: checkpoints.append(dict(cycle=i+1,metrics=process_metrics(c.proc.pid)))
            if i%10==0: time.sleep(.05)
        time.sleep(2)
        after=process_metrics(c.proc.pid)
        results["cycles"]=dict(count=cycles,open_frame_ms=summary(actions),before=baseline,after=after,checkpoints=checkpoints)
        c.command("page",page=0);c.command("load");time.sleep(1)
        results["under_load"]=c.sample(seconds)
        actions,queues=[],[]
        for i in range(150):
            _,ms,queue_ms=c.action_frame("page",page=i%4);actions.append(ms);queues.append(queue_ms);time.sleep(.02)
        results["page_frame_load_ms"]=summary(actions);results["ui_queue_load_ms"]=summary(queues)
        t=time.perf_counter();c.command("cancel")
        while c.command("worker")[0]["running"]:
            if time.perf_counter()-t>5:raise RuntimeError("Cancellation exceeded 5 seconds")
            time.sleep(.01)
        results["cancel_until_exit_ms"]=(time.perf_counter()-t)*1000
        results["worker_pid"]=c.worker_pid
        c.command("page",page=0);c.command("theme",theme="dark");c.command("message");time.sleep(.3)
        c.command("snapshot",path=str(Path(out).resolve()/"dark"))
        c.command("theme",theme="light");time.sleep(.3)
        c.command("snapshot",path=str(Path(out).resolve()/"light"))
        results["stats"]=c.command("stats")[0]
        results["passed"]=True
    finally:
        c.close()
        results["exit_code"]=c.proc.returncode
        results["worker_after_exit"]=process_metrics(c.worker_pid) if c.worker_pid else None
        (Path(out)/"result.json").write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding="utf-8")
    print(json.dumps(dict(out=str(out),passed=results.get("passed"),startup_ms=c.startup_ms),ensure_ascii=False),flush=True)


if __name__ == "__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("mode",choices=("smoke","full","machine"))
    parser.add_argument("--exe");parser.add_argument("--out",default=str(ROOT/"artifacts"/"run"))
    parser.add_argument("--seconds",type=float,default=15);parser.add_argument("--cycles",type=int,default=200)
    parser.add_argument("--renderer")
    args=parser.parse_args(); extra=["--renderer",args.renderer] if args.renderer else []
    if args.mode=="machine":print(json.dumps(describe_machine(),ensure_ascii=False,indent=2))
    elif args.mode=="smoke":smoke(args.exe,args.out,extra)
    else:full(args.exe,args.out,args.seconds,args.cycles,extra)
