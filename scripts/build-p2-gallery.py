"""Verify synthetic Slint P2 renders and build a local, data-free review gallery.

python -B scripts/build-p2-gallery.py --frames <render-directory> --output <new-gallery>
Only the reviewed JSON fixture and explicitly supplied render folder are read.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import struct

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = {"welcome", "first-schedule", "first-reminder", "first-models", "migration-review",
            "appearance", "activity", "fullscreen", "audio", "audio-missing", "animation",
            "generation", "manual-batch", "storage", "move-review", "move-progress", "move-failed",
            "storage-offline", "storage-space", "archive-export", "restore-review", "restore-conflicts",
            "archive-invalid", "diagnostics", "issues", "unsaved", "battery", "install-review",
            "erase-data", "download-paused", "library-empty", "library-loading", "library-partial"}


def ratio(a, b):
    def luminance(rgb):
        values = [int(rgb[i:i+2], 16) / 255 for i in (0, 2, 4)]
        values = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055)**2.4 for v in values]
        return sum(v * w for v, w in zip(values, (0.2126, 0.7152, 0.0722)))
    x, y = sorted((luminance(a), luminance(b)))
    return (y + 0.05) / (x + 0.05)


HTML = r'''<!doctype html><html lang="ru"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Кавычка · P2</title>
<style>
*{box-sizing:border-box}body{margin:0;background:#f5f6fa;color:#242837;font:15px system-ui,sans-serif}header{padding:18px 24px;background:white;border-bottom:1px solid #d6dae5}h1{font-size:22px;margin:0 0 8px}p{line-height:1.5;margin:8px 0}.muted{color:#596579}main{display:grid;grid-template-columns:255px 1fr;gap:20px;padding:20px}nav{max-height:85vh;overflow:auto}button,select,a{font:inherit}button,select{padding:10px 13px;border:1px solid #9da6bb;border-radius:8px;background:white;color:#242837}button{cursor:pointer}button:disabled{cursor:default;color:#596579;background:#edf0fc}button:focus-visible,select:focus-visible,a:focus-visible{outline:3px solid #4358b3;outline-offset:3px}nav button{display:block;width:100%;text-align:left;margin:0 0 5px;font-size:13px}nav button[aria-current=page]{background:#edf0fc;border-color:#4358b3;color:#4358b3}.toolbar,.actions{display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin-bottom:14px}figure{margin:0}img{width:100%;height:auto;display:block;border:1px solid #d6dae5;border-radius:8px}figure.full img{width:auto;max-width:none}section{min-width:0;overflow:auto}.actions{margin-top:16px}.actions button:first-child{background:#4d60bd;color:white}#status{min-height:28px}#requirements{font-size:13px;color:#596579}.banner{max-width:1100px}.preview-note{background:#edf0fc;padding:12px 16px;border-radius:8px}a{color:#4358b3}@media(max-width:850px){main{grid-template-columns:1fr}nav{max-height:180px}header{padding:15px}main{padding:12px}}
</style>
<header><h1>Кавычка · комплект макетов P2</h1><p class="banner">Это демонстрация будущих экранов, а не работа с приложением. Кнопки под макетом открывают пример следующего состояния. Записи, настройки и модели не изменяются.</p><p class="muted">Выбери экран, язык и тему. Для длинных форм доступен вид снизу; для контрольных экранов — узкий размер и DPI 150/200%. Полная проверка Windows и экранного диктора остаётся отдельным этапом.</p></header>
<main><nav id="screens" aria-label="Экраны макетов"></nav><section aria-label="Просмотр выбранного макета">
<div class="toolbar"><label>Язык <select id="language"><option value="ru">Русский</option><option value="en">English</option></select></label><label>Тема <select id="theme"><option value="light">Светлая</option><option value="dark">Тёмная</option></select></label><label>Размер <select id="viewport"></select></label><label><input id="bottom" type="checkbox"> Низ формы и фокус</label><button id="size">Исходный размер</button></div>
<h2 id="heading"></h2><p id="requirements"></p><figure id="figure"><img id="preview" alt=""></figure><div id="actions" class="actions"></div><p id="status" role="status" aria-live="polite"></p><p class="preview-note">Поля на изображении демонстрационные. Навигация галереи работает с клавиатуры; это не доказательство готовности Tab-навигации и Narrator в будущем приложении.</p>
</section></main><script>
const DATA=__DATA__;
const $=id=>document.getElementById(id), cases=DATA.cases, frames=DATA.frames;
let selected=location.hash.slice(1);if(!cases.some(c=>c.id===selected))selected='welcome';
function show(){const c=cases.find(c=>c.id===selected),lang=$('language').value,n=lang==='ru'?0:1,dark=$('theme').value==='dark';
 $('screens').replaceChildren(...cases.map(item=>{const b=document.createElement('button');b.textContent=item.heading[n];b.dataset.case=item.id;b.setAttribute('aria-current',item.id===selected?'page':'false');b.onclick=()=>{selected=item.id;history.replaceState(null,'','#'+selected);show()};return b}));
 const views=[...new Set(frames.filter(f=>f.case===selected).map(f=>f.viewport))],old=$('viewport').value;$('viewport').replaceChildren(...views.map(v=>{const o=document.createElement('option');o.value=v;o.textContent=v==='desktop'?'1100 × 780':v==='compact'?'720 × 480':v==='dpi150'?'150% · 1100 × 780':'200% · 720 × 480';return o}));if(views.includes(old))$('viewport').value=old;
 let f=frames.find(f=>f.case===selected&&f.language===lang&&f.dark===dark&&f.viewport===$('viewport').value&&f.bottom===$('bottom').checked);if(!f){$('bottom').checked=false;f=frames.find(f=>f.case===selected&&f.language===lang&&f.dark===dark&&f.viewport===$('viewport').value&&!f.bottom)}
 $('heading').textContent=c.heading[n];$('requirements').textContent=c.requirements.join(' · ')+' · '+f.width+' × '+f.height+' px';$('preview').src='frames/'+f.file;$('preview').alt=c.heading[n]+' — '+lang+' — '+(dark?'dark':'light')+' — '+f.viewport; $('actions').replaceChildren();
 ['primary','secondary','tertiary'].forEach((key,i)=>{if(!c[key][n])return;const b=document.createElement('button');b.textContent=c[key][n];b.disabled=i===0&&c.primary_enabled===false;b.onclick=()=>{const previous=c.heading[n];selected=(c.actions||[c.next,'today','today'])[i];history.replaceState(null,'','#'+selected);show();$('status').textContent='Демонстрационный переход: '+previous+' → '+cases.find(x=>x.id===selected).heading[n]};$('actions').append(b)});}
['language','theme','viewport','bottom'].forEach(id=>$(id).onchange=show);$('size').onclick=()=>{$('figure').classList.toggle('full');$('size').textContent=$('figure').classList.contains('full')?'Вписать в окно':'Исходный размер'};show();
</script></html>'''


def build(frames_path, output):
    review = json.loads((ROOT / "next/crates/design-preview/review.json").read_text(encoding="utf-8"))
    result = json.loads((frames_path / "render-results.json").read_text(encoding="utf-8"))
    assert result["keyboard_primary_enter"] and result["synthetic"] and not result["real_data_accessed"]
    cases = review["cases"]
    ids = {c["id"] for c in cases}
    assert len(ids) == len(cases) and REQUIRED <= ids
    requirements = {x for c in cases for x in c["requirements"]}
    assert {f"S{i:02}" for i in range(1, 21)} <= requirements
    assert {"B02", "B03", "B04", "B05", "B06", "B07", "B08", "B09", "B10", "B18"} <= requirements
    for c in cases:
        assert re.fullmatch(r"[a-z0-9-]+", c["id"])
        assert all(target in ids for target in c["actions"])
        for key in ("section", "heading", "subtitle", "notice", "notice_body", "primary", "secondary", "tertiary", "sample"):
            assert len(c[key]) == 2 and all(isinstance(v, str) for v in c[key])
        assert all(c[key][0] and c[key][1] for key in ("heading", "notice", "notice_body", "primary"))
        for row in c["rows"]:
            assert row["kind"] in {"note", "select", "input", "toggle"}
            assert all(len(row[key]) == 2 for key in ("label", "value", "help"))
            assert all(row["label"])
        for lang in ("ru", "en"):
            for dark in (False, True):
                assert any(f["case"] == c["id"] and f["language"] == lang and f["dark"] == dark and f["viewport"] == "desktop" and not f["bottom"] for f in result["frames"])
    assert next(c for c in cases if c["id"] == "erase-data")["primary_enabled"] is False
    for identifier in review["stress_cases"]:
        for v in review["viewports"]:
            assert any(f["case"] == identifier and f["viewport"] == v["id"] and f["bottom"] for f in result["frames"])
    for frame in result["frames"]:
        path = frames_path / frame["file"]
        assert path.parent == frames_path and path.suffix == ".png"
        with path.open("rb") as stream:
            h = stream.read(24)
        assert h[:8] == b"\x89PNG\r\n\x1a\n" and struct.unpack(">II", h[16:24]) == (frame["width"], frame["height"])
        assert not frame["footer_clipped"] and frame["body_height"] >= 100
    slint = (ROOT / "next/crates/design-preview/ui/review.slint").read_text(encoding="utf-8")
    colors = {name: [a.lstrip("#"), "ffffff" if b == "white" else b.lstrip("#")]
              for name, a, b in re.findall(r"out property <color> ([\w-]+): dark \? (#[0-9a-f]+) : (#[0-9a-f]+|white);", slint)}
    pairs = [("ink", "surface"), ("muted", "surface"), ("muted", "canvas"), ("accent", "soft"), ("button-ink", "button"), ("warning", "soft")]
    ratios = [ratio(colors[a][i], colors[b][i]) for a, b in pairs for i in (0, 1)]
    assert min(ratios) >= 4.5, ratios
    borders = [ratio(colors["control-border"][i], colors[bg][i]) for i in (0, 1) for bg in ("surface", "canvas")]
    assert min(borders) >= 3, borders
    assert not output.exists(), "choose a new output folder"
    (output / "frames").mkdir(parents=True)
    for frame in result["frames"]:
        shutil.copyfile(frames_path / frame["file"], output / "frames" / frame["file"])
    data = json.dumps(dict(cases=cases, frames=result["frames"]), ensure_ascii=False).replace("</", "<\\/")
    (output / "index.html").write_text(HTML.replace("__DATA__", data), encoding="utf-8")
    verification = dict(format=1, cases=len(cases), frames=len(result["frames"]), scenarios=sorted(x for x in requirements if x.startswith("S")),
                        languages=["ru", "en"], themes=["light", "dark"], stress_cases=review["stress_cases"],
                        min_text_contrast=round(min(ratios), 2), min_control_contrast=round(min(borders), 2), footer_and_body_checks=True, keyboard_primary_enter=True,
                        windows_screen_reader_tested=False, physical_dpi_tested=False, user_approval=False,
                        fixture_sha256=hashlib.sha256((ROOT / "next/crates/design-preview/review.json").read_bytes()).hexdigest(),
                        gallery_sha256=hashlib.sha256((output / "index.html").read_bytes()).hexdigest())
    (output / "verification.json").write_text(json.dumps(verification, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    print(json.dumps(verification, ensure_ascii=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frames", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    build(args.frames.resolve(), args.output.resolve())
