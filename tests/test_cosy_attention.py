"""Numerical cached-vs-full Qwen regression; tiny random model, no downloads."""
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "backend"))
from kavychka_local.cosy import cached_attention_mask


class CosyAttentionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import torch
        cls.torch = torch
        torch.set_num_threads(4)

    def test_cached_step_matches_full_forward_instead_of_losing_context(self):
        torch = self.torch
        from transformers import Qwen2Config
        from transformers.models.qwen2.modeling_qwen2 import Qwen2Model
        torch.manual_seed(42)
        config = Qwen2Config(vocab_size=32, hidden_size=16, intermediate_size=32,
                             num_hidden_layers=2, num_attention_heads=2, num_key_value_heads=1)
        config._attn_implementation = "sdpa"
        model = Qwen2Model(config).eval()
        xs = torch.randn(1, 8, 16)
        mask = torch.ones(1, 1, 1, dtype=torch.bool)
        with torch.inference_mode():
            expected = model(inputs_embeds=xs, use_cache=False).last_hidden_state[:, -1]
            errors = {}
            for fixed in [False, True]:
                cache = model(inputs_embeds=xs[:, :-1], use_cache=True).past_key_values
                actual_mask = cached_attention_mask(xs[:, -1:], mask, cache) if fixed else mask
                actual = model(inputs_embeds=xs[:, -1:], attention_mask=actual_mask[:, -1],
                               past_key_values=cache, use_cache=True).last_hidden_state[:, -1]
                errors[fixed] = float((actual - expected).abs().max())
        self.assertGreater(errors[False], 0.001, "Fixture must reproduce the old context loss")
        self.assertLess(errors[True], 0.00001)

    def test_prefill_and_complete_masks_remain_unchanged(self):
        torch = self.torch
        xs = torch.randn(1, 2, 4)
        mask = torch.ones(1, 2, 2, dtype=torch.bool).tril()
        self.assertIs(cached_attention_mask(xs, mask, None), mask)
        complete = torch.ones(1, 2, 7, dtype=torch.bool)
        self.assertIs(cached_attention_mask(xs, complete, SimpleNamespace(get_seq_length=lambda: 5)), complete)

    def test_current_causality_and_legacy_cache_are_preserved(self):
        torch = self.torch
        xs = torch.randn(1, 2, 4)
        mask = torch.ones(1, 2, 2, dtype=torch.bool).tril()
        legacy = ((torch.zeros(1, 1, 5, 4), torch.zeros(1, 1, 5, 4)),)
        result = cached_attention_mask(xs, mask, legacy)
        self.assertEqual(tuple(result.shape), (1, 2, 7))
        self.assertTrue(result[:, :, :5].all())
        self.assertTrue(torch.equal(result[:, :, 5:], mask))

    def test_inconsistent_mask_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "attention mask"):
            cached_attention_mask(self.torch.randn(1, 1, 4), self.torch.ones(1, 1, 3),
                                  SimpleNamespace(get_seq_length=lambda: 5))


if __name__ == "__main__":
    unittest.main()
