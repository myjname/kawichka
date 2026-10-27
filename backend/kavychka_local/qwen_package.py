"""Reproducible, attributed inference-only repack of the pinned Qwen wheel.

No upstream demo/CLI, Gradio requirement, remote build backend or edited installed
metadata. The derived distribution has its own name and dependency contract.
"""
import base64
import csv
import hashlib
import io
from pathlib import Path
import zipfile

SOURCE_SHA256 = "11a290d8dabc7ef91a90c54478c8ab19b3edb1d85c0882313721892bdc4af15d"
VERSION = "0.1.1.post3"
NAME = "kavychka_qwen_tts_inference"
FILENAME = f"{NAME}-{VERSION}-py3-none-any.whl"
INFO = f"{NAME}-{VERSION}.dist-info"
REQUIRES = ("transformers==5.10.0", "accelerate==1.15.0", "torch>=2.13,<2.15",
            "torchaudio>=2.11,<2.12", "librosa", "soundfile", "sox", "onnxruntime", "einops")


def adapt(name, data):
    """Only exact, reviewed changes to the SHA-pinned upstream source."""
    counts = {"qwen_tts/core/models/modeling_qwen3_tts.py": 2,
              "qwen_tts/core/tokenizer_12hz/modeling_qwen3_tts_tokenizer_v2.py": 1}
    if name in counts:
        text = data.decode("utf-8")
        before = "self.rope_init_fn = ROPE_INIT_FUNCTIONS[self.rope_type]"
        if text.count(before) != counts[name]:
            raise ValueError("Unexpected upstream RoPE source")
        text = text.replace(before, 'self.rope_init_fn = default_rope_parameters if self.rope_type == "default" else ROPE_INIT_FUNCTIONS[self.rope_type]')
        text = text.replace("logger = logging.get_logger(__name__)",
                            "from qwen_tts._kavychka_rope import default_rope_parameters, initialize_rope\n\nlogger = logging.get_logger(__name__)")
        if text.count("from transformers.masking_utils import") != 1:
            raise ValueError("Unexpected upstream mask imports")
        text = text.replace("from transformers.masking_utils import", "from qwen_tts._kavychka_masks import")
        if "tokenizer_12hz" in name:
            header = "class Qwen3TTSTokenizerV2DecoderPreTrainedModel(PreTrainedModel):\n"
            if text.count(header) != 1:
                raise ValueError("Unexpected upstream tokenizer initializer")
            text = text.replace(header, header + '''    def _init_weights(self, module):
        if not initialize_rope(module):
            super()._init_weights(module)

''')
            if text.count("@check_model_inputs()") != 1:
                raise ValueError("Unexpected upstream model-input decorator")
            text = text.replace("from transformers.utils.generic import check_model_inputs", "from transformers.utils.generic import merge_with_config_defaults")
            text = text.replace("@check_model_inputs()", "@merge_with_config_defaults")
            signature = "    ) -> BaseModelOutputWithPast:\n        if input_ids is not None:"
            if text.count(signature) != 1:
                raise ValueError("Unexpected upstream tokenizer forward")
            text = text.replace(signature, '    ) -> BaseModelOutputWithPast:\n        r"""\n        cache_position (`torch.LongTensor`, *optional*):\n            Absolute positions of the current decoder tokens in the cache.\n        """\n        if input_ids is not None:')
        else:
            # 5.x marks loaded Parameters, rather than entire nn.Modules.
            # Old `.data.normal_()` overrides bypass that contract and overwrite
            # checkpoint tensors during missing-buffer initialization.
            initializers = {
                "module.weight.data.normal_(mean=0.0, std=std)": (4, "init.normal_(module.weight, mean=0.0, std=std)"),
                "module.bias.data.zero_()": (3, "init.zeros_(module.bias)"),
                "module.weight.data.fill_(1.0)": (2, "init.ones_(module.weight)"),
                "if module.padding_idx is not None:": (2, 'if module.padding_idx is not None and not getattr(module.weight, "_is_hf_initialized", False):'),
            }
            for before, (expected, after) in initializers.items():
                if text.count(before) != expected:
                    raise ValueError("Unexpected upstream weight initialization")
                text = text.replace(before, after)
            header = "    def _init_weights(self, module):\n"
            if text.count(header) != 2:
                raise ValueError("Unexpected upstream initialization hooks")
            text = text.replace(header, header + "        if initialize_rope(module):\n            return\n")
            text = text.replace("from qwen_tts._kavychka_rope import default_rope_parameters", "from transformers import initialization as init\nfrom qwen_tts._kavychka_rope import default_rope_parameters")
            if text.count("config.pad_token_id") != 2:
                raise ValueError("Unexpected upstream embedding padding")
            text = text.replace("config.pad_token_id", 'getattr(config, "pad_token_id", None)')
            position_block = "        if attention_mask is not None:\n            if (\n                cache_position is None"
            if text.count(position_block) != 1:
                raise ValueError("Unexpected upstream generation cache positions")
            text = text.replace(position_block, '''        # Transformers 5 no longer supplies cache_position during generation.
        # Reconstruct the same absolute positions used by the 4.x generation
        # loop before computing multimodal RoPE, including cached token steps.
        if cache_position is None:
            past_seen = past_key_values.get_seq_length() if past_key_values is not None else 0
            cache_position = torch.arange(past_seen, past_seen + inputs_embeds.shape[1], device=inputs_embeds.device)
        if attention_mask is not None:
            if (
                cache_position is None''')
        return text.encode("utf-8")
    return data


def build(source, directory):
    source = Path(source)
    if source.stat().st_size > 2_000_000 or hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError("Qwen upstream wheel integrity check failed")
    files = {}
    with zipfile.ZipFile(source) as archive:
        for name in archive.namelist():
            if name == "qwen_tts/__init__.py" or name.startswith(("qwen_tts/core/", "qwen_tts/inference/")):
                if name.endswith("/"):
                    continue
                files[name] = adapt(name, archive.read(name))
        files[f"{INFO}/licenses/LICENSE"] = archive.read("qwen_tts-0.1.1.dist-info/licenses/LICENSE")
    if not files.get("qwen_tts/inference/qwen3_tts_model.py"):
        raise ValueError("Pinned upstream inference package is incomplete")
    # Transformers 5 moved the unscaled RoPE initializer into individual model
    # classes. Keep Qwen's original, unscaled equation local to this package;
    # do not modify Transformers' global registry or bypass input validation.
    files["qwen_tts/_kavychka_rope.py"] = b'''# Kavychka adaptation for the catalog-pinned Qwen-TTS models. Apache-2.0.
import torch
from transformers import initialization as init

def default_rope_parameters(config, device=None, seq_len=None, **unused):
    head_dim = getattr(config, "head_dim", None) or config.hidden_size // config.num_attention_heads
    dim = int(head_dim * getattr(config, "partial_rotary_factor", 1.0))
    inverse = 1.0 / (config.rope_theta ** (torch.arange(0, dim, 2, dtype=torch.int64).float().to(device) / dim))
    return inverse, 1.0

def initialize_rope(module):
    # Non-persistent buffers are materialized empty by the 5.x loader. They are
    # absent from checkpoints and must be regenerated after weight loading.
    if not hasattr(module, "rope_init_fn") or not hasattr(module, "inv_freq"):
        return False
    inverse, module.attention_scaling = module.rope_init_fn(module.config, device=module.inv_freq.device)
    init.copy_(module.inv_freq, inverse)
    module.original_inv_freq = module.inv_freq
    return True
'''
    files["qwen_tts/_kavychka_masks.py"] = b'''# Qwen 4.x call contract -> Transformers 5.10 masking API. Apache-2.0.
from transformers import masking_utils

def _mask(function, config, input_embeds=None, inputs_embeds=None, cache_position=None, **kwargs):
    # The new API derives cache offsets from the KV cache and position_ids.
    # All Qwen call sites provide position_ids; preserve the old fallback too.
    if kwargs.get("position_ids") is None and cache_position is not None:
        kwargs["position_ids"] = cache_position.unsqueeze(0)
    embeds = inputs_embeds if inputs_embeds is not None else input_embeds
    return function(config=config, inputs_embeds=embeds, **kwargs)

def create_causal_mask(**kwargs):
    return _mask(masking_utils.create_causal_mask, **kwargs)

def create_sliding_window_causal_mask(**kwargs):
    return _mask(masking_utils.create_sliding_window_causal_mask, **kwargs)
'''
    metadata = ("Metadata-Version: 2.4\nName: kavychka-qwen-tts-inference\n"
                f"Version: {VERSION}\nSummary: Kavychka inference adaptation of Qwen-TTS 0.1.1\n"
                "Author: Alibaba Qwen Team\nRequires-Python: >=3.11\nLicense: Apache-2.0\n"
                "License-File: licenses/LICENSE\n"
                "Project-URL: Upstream, https://github.com/QwenLM/Qwen3-TTS\n"
                + "".join(f"Requires-Dist: {r}\n" for r in REQUIRES) + "\n"
                "Downstream inference-only distribution. Upstream demo/CLI and its Gradio dependency are omitted.\n")
    files[f"{INFO}/METADATA"] = metadata.encode()
    files[f"{INFO}/WHEEL"] = b"Wheel-Version: 1.0\nGenerator: Kavychka pinned inference builder\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
    files[f"{INFO}/top_level.txt"] = b"qwen_tts\n"
    files[f"{INFO}/NOTICE"] = ("Qwen-TTS 0.1.1, Copyright 2026 The Alibaba Qwen team. Apache-2.0.\n"
        "Modified for Kavychka: inference-only packaging, dependency metadata, RoPE/mask/cache APIs and loaded-weight-preserving initialization for Transformers 5.10.\n"
        "Upstream: https://github.com/QwenLM/Qwen3-TTS\n"
        f"Upstream PyPI wheel SHA-256: {SOURCE_SHA256}\n").encode()
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\n")
    for name, data in sorted(files.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        writer.writerow([name, "sha256=" + digest, str(len(data))])
    writer.writerow([f"{INFO}/RECORD", "", ""])
    files[f"{INFO}/RECORD"] = record.getvalue().encode()
    payload = io.BytesIO()
    with zipfile.ZipFile(payload, "w", compression=zipfile.ZIP_STORED) as wheel:
        for name, data in sorted(files.items()):
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            wheel.writestr(info, data)
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / FILENAME
    data = payload.getvalue()
    if target.exists():
        if target.read_bytes() != data:
            raise ValueError("Existing derived Qwen wheel does not match this builder")
    else:
        with target.open("xb") as output:
            output.write(data)
    return target
