"""Private stdin/stdout worker: CosyVoice's pinned dependencies stay outside Qwen's interpreter."""
import argparse
import contextlib
import json
import logging
import os
import socket
import sys
import types
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--data", type=Path, required=True)
parser.add_argument("--model", type=Path)
parser.add_argument("--runtime", type=Path)
parser.add_argument("--device", choices=("auto", "cpu", "cuda"), default="auto")
parser.add_argument("--check-only", action="store_true")
parser.add_argument("--site", type=Path, help="Private staged dependency layer, check-only")
args = parser.parse_args()
root = args.data.resolve()
resources = (args.runtime or root).resolve()
if args.site and not args.check_only:
    raise ValueError("A staged dependency layer is only valid for installation checks")
if args.device == "cpu":
    # An empty value removes the native variable on Windows while os.environ
    # still shows it as empty: Python/NVML and CUDA can then disagree. -1 hides
    # GPUs consistently, including when reusing a CUDA-enabled Python runtime.
    os.environ["CUDA_VISIBLE_DEVICES"] = "-1"
os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", HF_HOME=str(root / "hf"),
                  TORCH_HOME=str(root / "torch"), NUMBA_CACHE_DIR=str(root / "numba"), PYTHONDONTWRITEBYTECODE="1")
def offline(*unused, **kwargs):
    raise RuntimeError("CosyVoice uses installed local files only")
socket.socket.connect = offline

with contextlib.redirect_stdout(sys.stderr):
    sys.path[:0] = [str(resources / "cosy/Whisper"), str(args.site or resources / "cosy/site"), str(resources / "cosy/CosyVoice"), str(resources / "cosy/Matcha-TTS")]
    import numpy as np
    import soundfile as sf
    import torch
    import torchaudio
    # Embedded Python's isolated ._pth omits the script directory.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from kavychka_local.cosy import prepare_precision, inference_errors, cached_attention_mask
    from cosyvoice.utils import file_utils
    def load_wav(path, target_sr, min_sr=16000):
        audio, rate = sf.read(path, dtype="float32", always_2d=True)
        speech = torch.from_numpy(audio.T.copy()).mean(dim=0, keepdim=True)
        if rate != target_sr:
            if rate < min_sr:
                raise ValueError("Reference sample rate is too low")
            speech = torchaudio.functional.resample(speech, rate, target_sr)
        return speech
    file_utils.load_wav = load_wav
    # Matcha's package initializer pulls training-only Lightning modules. Inference
    # uses audio.py directly and ordinary single-process logging instead.
    utils = types.ModuleType("matcha.utils")
    utils.__path__ = [str(resources / "cosy/Matcha-TTS/matcha/utils")]
    logger = types.ModuleType("matcha.utils.pylogger"); logger.get_pylogger = logging.getLogger
    sys.modules["matcha.utils"], sys.modules["matcha.utils.pylogger"] = utils, logger
    import cosyvoice.cli.cosyvoice as cli
    from cosyvoice.llm.llm import Qwen2Encoder
    original_step = Qwen2Encoder.forward_one_step
    def cached_step(self, xs, masks, cache=None):
        return original_step(self, xs, cached_attention_mask(xs, masks, cache), cache)
    Qwen2Encoder.forward_one_step = cached_step
    original_yaml = cli.load_hyperpyyaml
    def inference_yaml(stream, overrides=None, **kwargs):
        overrides = dict(overrides or {})
        for name in ("hifigan", "parquet_opener", "tokenize", "filter", "resample", "truncate", "compute_fbank",
                     "compute_f0", "parse_embedding", "shuffle", "sort", "batch", "padding"):
            overrides[name] = None
        configs = original_yaml(stream, overrides=overrides, **kwargs)
        # The trained llm.pt is FP32. Convert before model.load copies it into
        # Qwen's config-default BF16 parameters, preserving the checkpoint bits.
        prepare_precision(configs['llm'], torch.cuda.is_available())
        return configs
    cli.load_hyperpyyaml = inference_yaml
    from cosyvoice.utils.common import set_all_random_seed
    torch.set_num_threads(4)
    if args.device == "cuda" and not torch.cuda.is_available():
        raise RuntimeError("CUDA is unavailable; select CPU or install the NVIDIA runtime")
    if not args.check_only:
        if not args.model or not (args.model / "cosyvoice3.yaml").is_file():
            raise ValueError("Install the CosyVoice 3 model first")
        model = cli.AutoModel(model_dir=str(args.model), fp16=torch.cuda.is_available())
        prepare_precision(model.model.llm, torch.cuda.is_available())
if args.check_only:
    print(json.dumps({"checked": True}), flush=True)
    raise SystemExit(0)
print(json.dumps({"ready": True}), flush=True)
for line in sys.stdin:
    try:
        request = json.loads(line)
        target = Path(request["target"]).resolve()
        if target.parent != (root / "audio").resolve():
            raise ValueError("Audio target must be inside the current data directory")
        with contextlib.redirect_stdout(sys.stderr), torch.inference_mode():
            set_all_random_seed(request["seed"])
            instruction = "You are a helpful assistant. " + request["instruction"] + "<|endofprompt|>"
            with inference_errors():
                chunks = list(model.inference_instruct2(request["text"], instruction, request["reference"],
                                                        stream=False, text_frontend=False))
            audio = torch.cat([chunk["tts_speech"].detach().float().cpu() for chunk in chunks], dim=1).squeeze(0).numpy()
            if not np.isfinite(audio).all() or len(audio) < model.sample_rate // 4:
                raise ValueError("Empty or invalid speech")
            sf.write(target, audio, model.sample_rate, format="WAV", subtype="FLOAT")
        print(json.dumps({"ready": True, "rate": model.sample_rate}), flush=True)
    except Exception as error:
        print(json.dumps({"error": str(error)}), flush=True)
