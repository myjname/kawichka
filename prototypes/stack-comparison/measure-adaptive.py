"""Test an idle animation policy through the existing prototype commands.

Policy only: 1 second animated / 9 seconds still. The GUI renderer and worker
implementations are unchanged. The external controller is not part of UI CPU.
"""
import argparse
import json
from pathlib import Path
import threading
import time
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent))
from bench import Client

ROOT=Path(__file__).resolve().parent
if __name__=="__main__":
    p=argparse.ArgumentParser();p.add_argument("--out",required=True);p.add_argument("--seconds",type=float,default=60);p.add_argument("--warmup",type=float,default=2);p.add_argument("--reverse",action="store_true");a=p.parse_args()
    out=Path(a.out).resolve();out.mkdir(parents=True,exist_ok=True)
    variants=[("slint","Quote.Slint.exe"),("avalonia","Quote.Avalonia.exe")]
    if a.reverse:variants.reverse()
    for stack,exe in variants:
        c=Client(ROOT/"artifacts"/stack/exe,out/stack)
        try:
            time.sleep(a.warmup);c.command("animate",enabled=0);time.sleep(2)
            result={}
            sampling=threading.Thread(target=lambda:result.update(c.sample(a.seconds)))
            sampling.start();start=time.perf_counter()
            while time.perf_counter()-start<a.seconds:
                c.command("animate",enabled=1);time.sleep(1)
                c.command("animate",enabled=0);time.sleep(min(9,max(0,a.seconds-(time.perf_counter()-start))))
            sampling.join()
            report=dict(stack=stack,policy="1 second at 30 FPS + 9 seconds static",duration_seconds=a.seconds,metrics=result)
            (out/f"{stack}.json").write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding="utf-8")
            print(json.dumps(dict(stack=stack,cpu_percent=result["cpu_percent_allocated_cores"])),flush=True)
        finally:c.close()
