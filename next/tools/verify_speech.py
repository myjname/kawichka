"""Offline release qualification: decode audio AND compare recognized speech to text.

Run with an installed model-worker Python. --components supplies the qualified
Cosy Whisper source/site; --asr-model is a separately cached official Whisper
base checkpoint. This tool never downloads models or writes application data.
Cases JSON: [{"audio":"absolute path", "text":"expected phrase", "language":"ru"}].
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re

BASE_SHA256 = "ed3a0b6b1c0edf879ad9b11b1af5a0e6ab5db9205f891f668f8b0e6c6326e34e"


def normalize(text):
    return re.sub(r"[^a-zа-я0-9]+", " ", text.lower().replace("ё", "е")).strip()


def character_error_rate(recognized, expected):
    actual, target = normalize(recognized), normalize(expected)
    if not target:
        raise ValueError("Expected speech must contain text")
    row = list(range(len(target) + 1))
    for i, char in enumerate(actual, 1):
        current = [i]
        for j, expected_char in enumerate(target, 1):
            current.append(min(current[-1] + 1, row[j] + 1,
                               row[j - 1] + (char != expected_char)))
        row = current
    return row[-1] / len(target)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--components", type=Path, required=True)
    parser.add_argument("--asr-model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-cer", type=float, default=0.35)
    args = parser.parse_args()
    if not 0 <= args.max_cer <= 1 or args.output.exists():
        raise ValueError("Use a new report path and max CER from 0 to 1")
    cases = json.loads(args.cases.read_text(encoding="utf-8-sig"))
    if not isinstance(cases, list) or not 1 <= len(cases) <= 32:
        raise ValueError("Expected 1-32 speech cases")
    for case in cases:
        if (not isinstance(case, dict) or case.get("language") not in ("ru", "en")
                or not isinstance(case.get("text"), str) or len(normalize(case["text"])) < 15):
            raise ValueError("Each case needs nontrivial expected text and ru/en language")
    with args.asr_model.open("rb") as file:
        if hashlib.file_digest(file, "sha256").hexdigest() != BASE_SHA256:
            raise ValueError("Expected the official SHA256-verified Whisper base checkpoint")
    import sys
    import socket
    os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", PYTHONDONTWRITEBYTECODE="1",
                      NUMBA_CACHE_DIR=str(args.output.parent / "numba"))
    sys.path[:0] = [str(args.components / "cosy/Whisper"), str(args.components / "cosy/site")]
    import numpy as np
    import soundfile as sf
    import torch
    import torchaudio
    import whisper
    def offline(*unused, **kwargs):
        raise RuntimeError("Speech qualification is offline")
    socket.socket.connect = offline
    torch.set_num_threads(4)
    model = whisper.load_model(str(args.asr_model), device="cpu")
    results = []
    for case in cases:
        audio, rate = sf.read(case["audio"], dtype="float32", always_2d=True)
        if len(audio) < rate // 4 or not np.isfinite(audio).all():
            raise ValueError("Empty or non-finite audio")
        rms = float(np.sqrt(np.mean(audio ** 2)))
        wave = torch.from_numpy(audio.mean(axis=1).copy())
        if rate != 16000:
            wave = torchaudio.functional.resample(wave, rate, 16000)
        text = model.transcribe(wave.numpy(), language=case["language"], fp16=False,
                                temperature=0.0, condition_on_previous_text=False, verbose=False)["text"]
        error = character_error_rate(text, case["text"])
        results.append(dict(audio=case["audio"], expected=case["text"], recognized=text,
                            seconds=len(audio) / rate, rms=rms, character_error_rate=error,
                            passed=rms > 0.00001 and error <= args.max_cer))
    report = dict(passed=all(row["passed"] for row in results), model="Whisper base",
                  asr_model_sha256=BASE_SHA256, max_character_error_rate=args.max_cer,
                  network=False, audio_played=False, results=results)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(dict(passed=report["passed"],cases=len(results),report=str(args.output))))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
