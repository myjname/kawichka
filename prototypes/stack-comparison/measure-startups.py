import argparse
import json
from pathlib import Path
import sys
import time
sys.path.insert(0,str(Path(__file__).resolve().parent))
from bench import Client,summary

ROOT=Path(__file__).resolve().parent
if __name__=="__main__":
    p=argparse.ArgumentParser();p.add_argument("--out",required=True);a=p.parse_args()
    out=Path(a.out).resolve();out.mkdir(parents=True,exist_ok=True)
    variants={"slint":"Quote.Slint.exe","avalonia":"Quote.Avalonia.exe"};results={k:[] for k in variants}
    for i in range(10):
        for k in (list(variants) if i%2==0 else list(reversed(variants))):
            c=Client(ROOT/"artifacts"/k/variants[k],out/f"{i:02d}-{k}")
            try:
                c.command("present",target="pet")
                results[k].append(dict(external_ms=c.startup_ms,internal_ms=c.ready["elapsed_ms"]))
            finally:c.close()
            print(k,i+1,round(c.startup_ms,1),flush=True);time.sleep(.2)
    data=dict(raw=results,summary={k:summary([v["external_ms"] for v in samples]) for k,samples in results.items()})
    (out/"startups.json").write_text(json.dumps(data,ensure_ascii=False,indent=2),encoding="utf-8")
