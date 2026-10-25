"""Native integration regression checks, always restricted to the prototype HWNDs."""
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
import time
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent))
from bench import Client, U32, window_hit_test

U32.SendMessageW.argtypes=[W.HWND,W.UINT,W.WPARAM,W.LPARAM]
U32.SendMessageW.restype=W.LPARAM
U32.GetForegroundWindow.restype=W.HWND

if __name__=="__main__":
    p=argparse.ArgumentParser();p.add_argument("--exe",required=True);p.add_argument("--out",required=True);a=p.parse_args()
    out=Path(a.out).resolve();c=Client(a.exe,out);checks=[]
    try:
        for i in range(30):
            before=U32.GetForegroundWindow()
            c.command("visibility",show=0)
            c.command("visibility",show=1)
            c.command("present",target="pet")
            n=c.command("native")[0]
            hit=window_hit_test(n)
            checks.append(dict(cycle=i+1,native=n,hit=hit,
                mouse_activate_result=U32.SendMessageW(n["hwnd"],0x21,0,0),foreground_unchanged=before==U32.GetForegroundWindow()))
            assert all(n[k] for k in ("no_activate","tool_window","topmost","region_present","transparent_corner_excluded","solid_part_included")),n
            assert hit["corner_passes_to_other_window"] and hit["solid_resolves_to_mascot"],hit
            assert checks[-1]["mouse_activate_result"]==3,checks[-1]
        # Verify that an exited main process does not leave its active worker alive.
        c.command("load");time.sleep(.4)
    finally:
        c.close()
    from bench import process_metrics
    report=dict(passed=True,cycles=checks,worker_after_main_exit=process_metrics(c.worker_pid),exit_code=c.proc.returncode)
    assert report["worker_after_main_exit"] is None,report
    (out/"native.json").write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding="utf-8")
    print(json.dumps(dict(passed=True,cycles=len(checks),worker_cleaned=True)),flush=True)
