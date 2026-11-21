"""Independent portable personalization. Presets never contain models or schedules."""
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import uuid
import zipfile
from pathlib import Path, PureWindowsPath

KINDS = ("voice", "character", "visual")
MAX_PACKAGE = 200 * 1024 * 1024
MAX_MANIFEST = 150_000


def read_json(path):
    with Path(path).open("rb") as source:
        raw = source.read(MAX_MANIFEST + 1)
    if len(raw) > MAX_MANIFEST:
        raise ValueError("Preset manifest exceeds 150 KB")
    item = json.loads(raw.decode("utf-8-sig"))
    if not isinstance(item, dict):
        raise ValueError("Preset manifest must be an object")
    return item


def inside(root, name):
    if (not isinstance(name, str) or not name or Path(name).is_absolute()
            or PureWindowsPath(name).drive or any(c in name for c in '\\:<>"|?*')
            or any(ord(c) < 32 or ord(c) == 127 for c in name)):
        raise ValueError("Invalid preset path")
    reserved = {"CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"}
    reserved.update(prefix + str(n) for prefix in ("COM", "LPT") for n in range(1, 10))
    if any(not part or part in (".", "..") or part.endswith((".", " "))
           or part.split(".")[0].upper() in reserved for part in name.split("/")):
        raise ValueError("Invalid preset path")
    result = (root / name).resolve()
    if root.resolve() not in result.parents:
        raise ValueError("Invalid preset path")
    return result


class Profiles:
    def __init__(self, app, data):
        self.app, self.data = Path(app), Path(data)
        self.root = self.data / "presets"
        self.root.mkdir(parents=True, exist_ok=True)

    def list(self):
        result = []
        for path in sorted(self.root.glob("*/preset.json")):
            try:
                item = read_json(path)
                if item["kind"] in KINDS:
                    result.append({"id": path.parent.name, "name": item["name"], "kind": item["kind"]})
            except (OSError, ValueError, KeyError):
                continue
        return result

    def get(self, ident, kind):
        if kind not in KINDS or not isinstance(ident, str) or not ident.isalnum():
            raise ValueError("Invalid preset")
        folder = inside(self.root, ident)
        item = self.clean(read_json(folder / "preset.json"), kind)
        if item.get("kind") != kind:
            raise ValueError("Preset has a different kind")
        return folder, item

    @staticmethod
    def clean(item, kind):
        if not isinstance(item, dict) or item.get("kind") != kind or item.get("version") != 1:
            raise ValueError("Incompatible preset")
        keys = {"version", "kind", "name", {"voice": "references", "character": "description", "visual": "file"}[kind]}
        if kind == "visual":
            keys.add("layout")
        item = {key: value for key, value in item.items() if key in keys}
        if kind == "voice":
            refs = item.get("references", [])
            if not isinstance(refs, list) or not 1 <= len(refs) <= 8 or any(not isinstance(ref, dict) for ref in refs):
                raise ValueError("A voice needs 1–8 references")
            item["references"] = [{"file": ref["file"], "transcript": ref.get("transcript", "")} for ref in refs]
        if kind == "visual":
            item.setdefault("layout", "auto")
            if item["layout"] not in ("auto", "image", "codex", "pixel", "smooth"):
                raise ValueError("Invalid visual layout")
        return item

    def voice(self, ident):
        if not ident:
            folder = self.app / "assets/voice"
            item = read_json(folder / "profile.json")
            refs = item["references"] if "references" in item else [{"file": item["file"], "transcript": item.get("transcript", "")}]
        else:
            # Complete characters keep voice/visual media in the same portable
            # folder. Independently selected legacy voice presets still work.
            if not isinstance(ident, str) or not ident.isalnum():
                raise ValueError("Invalid preset")
            folder = inside(self.root, ident)
            item = read_json(folder / "preset.json")
            if item.get("kind") == "character_pack":
                item = self.clean(dict(version=item.get("version"), kind="voice",
                                       name=item.get("name", ""), references=item.get("references", [])), "voice")
            else:
                folder, item = self.get(ident, "voice")
            refs = item["references"]
        result = []
        for ref in refs:
            path = inside(folder, ref["file"])
            if not path.is_file():
                raise ValueError("Voice reference is missing")
            result.append({"path": path, "transcript": ref.get("transcript", ""), "hash": hashlib.sha256(path.read_bytes()).hexdigest()})
        if not 1 <= len(result) <= 8:
            raise ValueError("A voice needs 1–8 references")
        return result

    def import_file(self, kind, source, name="", transcript=""):
        if kind not in KINDS:
            raise ValueError("Unknown preset kind")
        source = Path(source)
        if not source.is_file() or source.stat().st_size > MAX_PACKAGE:
            raise ValueError("Choose a file up to 200 MiB")
        ident = uuid.uuid4().hex
        with tempfile.TemporaryDirectory(dir=self.root) as temporary:
            stage = Path(temporary)
            if source.suffix.casefold() == ".kavychka":
                with zipfile.ZipFile(source) as archive:
                    files = archive.infolist()
                    if len(files) > 150 or sum(f.file_size for f in files) > MAX_PACKAGE:
                        raise ValueError("Preset archive is too large")
                    seen = set()
                    actual_total = 0
                    for entry in files:
                        inside(stage, entry.filename.rstrip("/"))
                        if entry.is_dir():
                            continue
                        if entry.filename.casefold() in seen:
                            raise ValueError("Duplicate preset entry")
                        seen.add(entry.filename.casefold())
                        path = inside(stage, entry.filename)
                        if path.suffix.casefold() not in (".json", ".flac", ".wav", ".png", ".webp", ".gif", ".jpg", ".jpeg"):
                            raise ValueError("Unexpected preset file")
                        bound = min(entry.file_size, MAX_MANIFEST if entry.filename == "preset.json" else MAX_PACKAGE)
                        if entry.file_size > bound:
                            raise ValueError("Preset manifest exceeds 150 KB")
                        path.parent.mkdir(parents=True, exist_ok=True)
                        with archive.open(entry) as src, path.open("wb") as dst:
                            copied = 0
                            while True:
                                chunk = src.read(min(65536, bound - copied + 1))
                                if not chunk:
                                    break
                                copied += len(chunk)
                                actual_total += len(chunk)
                                if copied > bound or actual_total > MAX_PACKAGE:
                                    raise ValueError("Preset expanded beyond its declared size or limit")
                                dst.write(chunk)
                            if copied != entry.file_size:
                                raise ValueError("Preset entry size mismatch")
                item = read_json(stage / "preset.json")
                item = self.clean(item, kind)
                if kind == "voice":
                    # Imported inputs may already use our normalized names.
                    # Decode every reference before replacing any input file.
                    with tempfile.TemporaryDirectory(dir=stage) as normalized_dir:
                        normalized_root = Path(normalized_dir)
                        for index, ref in enumerate(item["references"]):
                            self.normalize_audio(inside(stage, ref["file"]), normalized_root / ("normalized%d.flac" % index))
                        for index, ref in enumerate(item["references"]):
                            normalized = "normalized%d.flac" % index
                            (normalized_root / normalized).replace(stage / normalized)
                            ref["file"] = normalized
            else:
                item = {"version": 1, "kind": kind, "name": (name or source.stem)[:80]}
                if kind == "voice":
                    self.normalize_audio(source, stage / "reference.flac")
                    item["references"] = [{"file": "reference.flac", "transcript": transcript[:4000]}]
                elif kind == "character":
                    with source.open("rb") as file:
                        raw = file.read(16_004)
                    if len(raw) > 16_003:
                        raise ValueError("Character description needs 1–4000 characters")
                    text = raw.decode("utf-8-sig").strip()
                    if not 1 <= len(text) <= 4000:
                        raise ValueError("Character description needs 1–4000 characters")
                    item["description"] = text
                else:
                    if source.suffix.casefold() not in (".png", ".webp", ".gif", ".jpg", ".jpeg"):
                        raise ValueError("Use PNG, WebP, GIF or JPEG")
                    target = "visual" + source.suffix.casefold()
                    shutil.copy2(source, stage / target)
                    item["file"] = target
            item = self.clean(item, kind)
            self.validate(stage, item)
            (stage / "preset.json").write_text(json.dumps(item, ensure_ascii=False, indent=2), encoding="utf-8")
            # Carry only the declared component files into the library.
            target = self.root / ident
            target.mkdir()
            names = ["preset.json"] + ([ref["file"] for ref in item["references"]] if kind == "voice" else [item["file"]] if kind == "visual" else [])
            try:
                for name in names:
                    destination = inside(target, name)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(inside(stage, name), destination)
            except BaseException:
                if target.name == ident and self.root.resolve() in target.resolve().parents and not target.is_symlink():
                    shutil.rmtree(target)
                raise
        return {"id": ident, "kind": kind, "name": item["name"]}

    def validate(self, folder, item):
        if not isinstance(item.get("name"), str) or not 1 <= len(item["name"]) <= 80:
            raise ValueError("Invalid preset name")
        if item["kind"] == "voice":
            import soundfile as sf
            import numpy as np
            refs = item.get("references", [])
            if not 1 <= len(refs) <= 8:
                raise ValueError("A voice needs 1–8 references")
            for ref in refs:
                path = inside(folder, ref["file"])
                info = sf.info(path)
                if info.samplerate < 16000 or info.frames / info.samplerate < 1 or info.frames / info.samplerate > 30:
                    raise ValueError("Reference must contain 1–30 seconds of speech, 16 kHz or higher")
                samples, unused = sf.read(path, dtype="float32")
                if not np.isfinite(samples).all() or float(np.max(np.abs(samples))) < 0.00001:
                    raise ValueError("Reference audio is empty or invalid")
                if not isinstance(ref.get("transcript", ""), str) or len(ref.get("transcript", "")) > 4000:
                    raise ValueError("Invalid reference transcript")
        elif item["kind"] == "character":
            if not isinstance(item.get("description"), str) or not 1 <= len(item["description"].strip()) <= 4000:
                raise ValueError("Character description needs 1–4000 characters")
        else:
            from PIL import Image
            with Image.open(inside(folder, item["file"])) as image:
                if image.width * image.height > 40000000:
                    raise ValueError("Visual image is too large")
                if item.get("layout") == "codex" and image.size != (1536, 1872):
                    raise ValueError("Codex sprite sheet must be 1536 x 1872")
                image.verify()

    @staticmethod
    def normalize_audio(source, target):
        import soundfile as sf
        try:
            import imageio_ffmpeg
            ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()
        except ImportError:
            ffmpeg = shutil.which("ffmpeg")
        if ffmpeg:
            result = subprocess.run([ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "error", "-protocol_whitelist", "file,pipe", "-y", "-i", str(source),
                                     "-t", "31", "-vn", "-ac", "1", "-ar", "24000", str(target)],
                                    capture_output=True, timeout=90, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
            if result.returncode:
                raise ValueError("Cannot decode this audio file")
        else:
            # Check metadata before decoding: a small compressed input can
            # otherwise allocate hours of audio. Downmix in bounded blocks.
            with sf.SoundFile(source) as audio:
                if audio.samplerate <= 0 or audio.channels < 1 or not 1 <= audio.frames / audio.samplerate <= 30:
                    raise ValueError("Reference must contain 1–30 seconds of speech")
                with sf.SoundFile(target, "w", samplerate=audio.samplerate, channels=1, format="FLAC") as output:
                    for block in audio.blocks(blocksize=max(1, 16384 // audio.channels), dtype="float32", always_2d=True):
                        output.write(block.mean(axis=1))

    def export(self, ident, kind, target):
        target = Path(target).resolve()
        if target == self.root.resolve() or self.root.resolve() in target.parents:
            raise ValueError("Export outside the preset library")
        folder, item = self.get(ident, kind)
        self.validate(folder, item)
        names = ["preset.json"]
        if kind == "voice":
            names += [ref["file"] for ref in item["references"]]
        elif kind == "visual":
            names += [item["file"]]
        partial = target.with_name(target.name + ".part")
        try:
            with zipfile.ZipFile(partial, "w", zipfile.ZIP_DEFLATED) as archive:
                archive.writestr("preset.json", json.dumps(item, ensure_ascii=False, indent=2))
                for name in names[1:]:
                    archive.write(inside(folder, name), name)
            os.replace(partial, target)
        finally:
            partial.unlink(missing_ok=True)
        return {"file": str(target)}
