"""Qualified inference contract. No weights, downloads, audio or user database."""
import contextlib
import importlib.metadata
import json
from pathlib import Path
import sys
from types import SimpleNamespace

root = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / "backend"))
manifest = json.loads((root / "assets/components.json").read_text(encoding="utf-8"))
versions = {}
for name, expected in manifest["runtime_versions"].items():
    actual = importlib.metadata.version(name)
    if actual.split("+")[0] != expected:
        raise RuntimeError(f"Unqualified inference dependency: {name} {actual}; expected {expected}")
    versions[name] = actual
for name in ("qwen-tts", "gradio", "gradio-client", "hf-gradio"):
    try:
        importlib.metadata.version(name)
    except importlib.metadata.PackageNotFoundError:
        continue
    raise RuntimeError(f"Obsolete demo distribution must be removed from inference runtime: {name}")
with contextlib.redirect_stdout(sys.stderr):
    import torch
    import torchaudio
    import qwen_tts
    import soundfile
    import transformers
    import tzdata
    from qwen_tts.core.models.modeling_qwen3_tts import Qwen3TTSPreTrainedModel, Qwen3TTSTalkerTextPreTrainedModel
    from qwen_tts.core.models.modeling_qwen3_tts import Qwen3TTSRotaryEmbedding
    from qwen_tts.core.tokenizer_12hz.modeling_qwen3_tts_tokenizer_v2 import Qwen3TTSTokenizerV2DecoderPreTrainedModel, Qwen3TTSTokenizerV2DecoderRotatoryEmbedding
    # Transformers 5 marks loaded Parameters. Legacy in-place initializers can
    # otherwise silently replace checkpoint weights even after a successful load.
    for cls in (Qwen3TTSPreTrainedModel, Qwen3TTSTalkerTextPreTrainedModel):
        linear = torch.nn.Linear(3, 2)
        before = [p.detach().clone() for p in linear.parameters()]
        for param in linear.parameters():
            param._is_hf_initialized = True
        cls._init_weights(SimpleNamespace(config=SimpleNamespace(initializer_range=0.02)), linear)
        if not all(torch.equal(a, b) for a, b in zip(before, linear.parameters())):
            raise RuntimeError("Inference initializer overwrote loaded checkpoint parameters")
    config = SimpleNamespace(rope_scaling=None, max_position_embeddings=128, rope_theta=10000,
                             hidden_size=32, num_attention_heads=4, head_dim=8)
    for model_cls, rope_cls in ((Qwen3TTSPreTrainedModel, Qwen3TTSRotaryEmbedding),
                                (Qwen3TTSTokenizerV2DecoderPreTrainedModel, Qwen3TTSTokenizerV2DecoderRotatoryEmbedding)):
        rotary = rope_cls(config)
        expected = rotary.inv_freq.clone()
        rotary.register_buffer("inv_freq", torch.zeros_like(expected), persistent=False)
        rotary.original_inv_freq = torch.zeros_like(expected)
        model_cls._init_weights(SimpleNamespace(config=SimpleNamespace(initializer_range=0.02)), rotary)
        if not torch.equal(rotary.inv_freq, expected) or not torch.equal(rotary.original_inv_freq, expected):
            raise RuntimeError("Inference initializer did not restore non-persistent rotary buffers")
    if "--gpu" in sys.argv and not torch.cuda.is_available():
        raise RuntimeError("The installed CUDA runtime cannot use the current NVIDIA driver; select CPU")
print(json.dumps(dict(torch=torch.__version__, transformers=transformers.__version__, versions=versions,
                      loaded_weights_protected=True)))
