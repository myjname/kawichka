"""Run sequentially: compiling or running both stacks together biases results."""
import argparse
import json
from pathlib import Path
import time
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent))
from bench import Client, describe_machine, full, summary

ROOT=Path(__file__).resolve().parent

if __name__=="__main__":
    p=argparse.ArgumentParser();p.add_argument("--out",required=True);p.add_argument("--seconds",type=float,default=30);p.add_argument("--startups",type=int,default=10);p.add_argument("--cycles",type=int,default=200)
    a=p.parse_args();out=Path(a.out).resolve();out.mkdir(parents=True,exist_ok=True)
    (out/"machine.json").write_text(json.dumps(describe_machine(),ensure_ascii=False,indent=2),encoding="utf-8")
    exes={"slint":ROOT/"artifacts/slint/Quote.Slint.exe","avalonia":ROOT/"artifacts/avalonia/Quote.Avalonia.exe"}
    startup={key:[] for key in exes}
    for i in range(a.startups):
        order=list(exes) if i%2==0 else list(reversed(exes))
        for key in order:
            c=Client(exes[key],out/f"startup-{i:02d}-{key}")
            try:
                c.command("present",target="pet")
                startup[key].append(dict(external_ms=c.startup_ms,internal_ms=c.ready["elapsed_ms"],ready=c.ready))
            finally:c.close()
            print(f"startup {i+1}/{a.startups} {key}: {startup[key][-1]['external_ms']:.1f} ms",flush=True)
            time.sleep(.3)
    (out/"startups.json").write_text(json.dumps(startup,ensure_ascii=False,indent=2),encoding="utf-8")
    # Alternate the order in repeated runs to expose background-machine noise.
    for repeat in range(2):
        for key in (list(exes) if repeat==0 else list(reversed(exes))):
            print(f"full run {repeat+1} {key}",flush=True)
            full(exes[key],out/f"run-{repeat+1}-{key}",a.seconds,a.cycles)
            time.sleep(1)
    results=dict(startup={key:summary([v["external_ms"] for v in items]) for key,items in startup.items()},runs={})
    for key in exes:
        results["runs"][key]=[json.loads((out/f"run-{repeat+1}-{key}/result.json").read_text(encoding="utf-8")) for repeat in range(2)]
    (out/"comparison.json").write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding="utf-8")
    print("PASS complete comparison",flush=True)
