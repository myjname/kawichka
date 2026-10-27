import gc
import hashlib
import json
import re
import secrets
import time
from pathlib import Path
from .downloads import checkpoint
from .store import cache_key
from .character import APPROACHES, messages as character_messages, starts_with_pora
from .delivery import fixed, select, options, catalog
from .diversity import too_similar, same_opening, normalize, opening_pressure, opening_hint
from .profiles import Profiles

SWEAR = re.compile(r"(?iu)(?:\b(?:бля\w*|хуй\w*|хуя\w*|хуе\w*|хуё\w*|нах\w*|нихуя\w*|пизд\w*|еб\w*|ёб\w*|заеб\w*|заёб\w*|выеб\w*|выёб\w*|сука\w*|суч\w*|мудак\w*|мудил\w*|долбо\w*|охуе\w*|охуё\w*))")
TOPICS = {"break": r"перерыв|отдох|разомн|разм[яи]|вста|отойд|монитор|отлож|компьютер|работ|пройдись", "breakfast": r"завтрак|позавтрак", "lunch": r"обед|пообед", "dinner": r"ужин|поужин", "exercise": r"зарядк|разомн|разм[яи]|упражн|потян|мышц|плеч"}
TOPICS_EN = {"break": r"break|rest|stand|stretch|screen|computer|step away|walk|pause", "breakfast": r"breakfast|morning meal", "lunch": r"lunch|midday meal", "dinner": r"dinner|evening meal|supper", "exercise": r"stretch|exercise|shoulder|muscle|move|stand"}
SWEAR_EN = re.compile(r"\b(?:fuck\w*|shit\w*|bitch\w*|asshole\w*)\b", re.I)

class Engine:
    def __init__(self, app_root, store, components):
        self.app_root, self.store, self.components = Path(app_root), store, components
        self.model = self.tokenizer = self.prompt = None
        self.loaded = None
        self.profiles = Profiles(self.app_root, self.store.root)
        self.refresh_voice()

    def refresh_voice(self):
        refs = self.profiles.voice(self.store.settings().get("voice_profile", ""))
        self.references = {ref["hash"]: ref for ref in refs}
        self.reference = refs[0]["path"]
        self.profile = {"transcript": refs[0]["transcript"]}
        self.profile_hash = hashlib.sha256(json.dumps([(ref["hash"], ref["transcript"]) for ref in refs], ensure_ascii=False).encode()).hexdigest()

    def unload(self):
        if self.loaded and self.components.manifest["models"][self.loaded].get("format") in ("gguf", "cosyvoice3") and self.model is not None:
            self.model.close()
        self.model = self.tokenizer = self.prompt = None
        self.loaded = None
        gc.collect()
        import sys
        torch = sys.modules.get("torch")
        if torch and torch.cuda.is_available():
            torch.cuda.empty_cache()

    def load(self, key, event, progress):
        checkpoint(event)
        if self.loaded == key:
            return
        self.unload()
        if not self.components.ready(key):
            raise ValueError("Сначала установите компонент «" + key + "» на вкладке «Модели»")
        spec = self.components.manifest["models"][key]
        if spec.get("format") != "gguf":
            # Cosy's local YAML/PT files and HF configs are executable inputs,
            # even with offline mode / weights_only. Do not trust size alone.
            self.components.verify_inference(key, event, progress)
        if spec.get("format") == "gguf":
            from .llama import Llama
            executable = self.components.llama_executable()
            device = self.components.llama_device()
            if not self.components.llama_ready(gpu=device == "cuda"):
                raise ValueError("Сначала установите «llama.cpp · генератор текста» на вкладке «Модели»")
            progress("Загрузка " + spec["name"], 0, 0)
            self.model = Llama(executable, self.components.path(key) / spec["files"][0]["path"], self.store.root,
                               device, event, progress)
            self.loaded = key
            return
        if spec.get("format") == "cosyvoice3":
            from .cosy import Cosy
            if not self.components.cosy_ready():
                raise ValueError("Сначала установите «Среда CosyVoice» на вкладке «Модели»")
            progress("Загрузка CosyVoice 3 · управляемая подача", 0, 0)
            self.model = Cosy(self.app_root, self.store.root, self.components.path(key),
                              self.components.voice_device(), event, runtime=self.components.cosy_root() if hasattr(self.components, "cosy_root") else self.components.root)
            self.loaded = key
            return
        import torch
        torch.set_num_threads(4)
        chosen = self.components.voice_device()
        if chosen == "cuda" and not torch.cuda.is_available():
            raise ValueError("CUDA недоступна. Выберите CPU или установите среду NVIDIA.")
        self.device = "cuda:0" if chosen != "cpu" and torch.cuda.is_available() else "cpu"
        dtype = (torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float16) if self.device != "cpu" else torch.float32
        progress("Загрузка " + key + " · " + self.device, 0, 0)
        folder = str(self.components.path(key))
        if key.startswith("text"):
            from transformers import AutoModelForCausalLM, AutoTokenizer
            self.tokenizer = AutoTokenizer.from_pretrained(folder, local_files_only=True, trust_remote_code=False)
            self.model = AutoModelForCausalLM.from_pretrained(folder, torch_dtype=dtype, device_map=self.device,
                                                             attn_implementation="sdpa", local_files_only=True,
                                                             trust_remote_code=False, use_safetensors=True)
        else:
            from qwen_tts import Qwen3TTSModel
            import soundfile as sf
            self.model = Qwen3TTSModel.from_pretrained(folder, dtype=dtype, device_map=self.device,
                                                      attn_implementation="sdpa", local_files_only=True,
                                                      trust_remote_code=False, use_safetensors=True)
            audio, rate = sf.read(self.reference, dtype="float32")
            transcript = self.profile["transcript"].strip()
            self.prompt = self.model.create_voice_clone_prompt(ref_audio=(audio, rate), ref_text=transcript or None, x_vector_only_mode=not bool(transcript))
        self.loaded = key
        checkpoint(event)

    def stopping(self, event):
        from transformers import StoppingCriteria, StoppingCriteriaList
        class Stop(StoppingCriteria):
            def __call__(self, input_ids, scores, **kwargs):
                return event.is_set()
        return StoppingCriteriaList([Stop()])

    def text(self, category, event, progress, model=None, followup=False, style_retries=None):
        settings = self.store.settings()
        if style_retries is None:
            style_retries = not settings.get("runtime", False)
        self.load(model or settings["text_model"], event, progress)
        variant = secrets.randbelow(len(APPROACHES))
        rejected = []
        reserve = None
        for attempt in range(6):
            checkpoint(event)
            messages = character_messages(category, settings, variant + attempt, followup)
            recent = self.store.recent(8)
            english = settings["speech_language"] == "en"
            if recent:
                messages[-1]["content"] = (("Previously spoken lines, not examples to copy:\n" if english else "Уже произнесённые реплики, не образцы для копирования:\n") + "\n".join(recent)
                                           + ("\nDo not reuse their openings, turns or bargains. Follow the new task below.\n\n" if english else "\nНе повторяй их начала, обороты и сделки. Ниже — новое задание; сохрани именно его тему.\n\n")
                                           + messages[-1]["content"])
            hint = opening_hint(recent, settings["speech_language"])
            if hint:
                messages[-1]["content"] += "\n\n" + hint
            if rejected:
                messages[-1]["content"] += ("\n\nPrevious rejected candidates for this task:\n" if english else "\n\nПредыдущие неудачные варианты этого задания:\n") + "\n".join(rejected[-3:]) + ("\nCorrect the stated issue, change the opening and image, keep the requested action. Return only the new spoken line." if english else "\nИсправь указанную причину. Измени начало и образ, сохрани нужное действие. Верни только новую реплику.")
            try:
                if self.components.manifest["models"][self.loaded].get("format") == "gguf":
                    text = self.model.generate(messages, event)
                else:
                    import torch
                    prompt = self.tokenizer.apply_chat_template(messages, tokenize=False, add_generation_prompt=True, enable_thinking=False)
                    inputs = self.tokenizer([prompt], return_tensors="pt").to(self.device)
                    with torch.inference_mode():
                        result = self.model.generate(**inputs, max_new_tokens=140, do_sample=True, temperature=0.7,
                                                     top_p=0.8, repetition_penalty=1.05, stopping_criteria=self.stopping(event))
                    text = self.tokenizer.decode(result[0][inputs.input_ids.shape[1]:], skip_special_tokens=True)
            except (OSError, RuntimeError, ValueError):
                checkpoint(event)
                if reserve is None:
                    raise
                return reserve[1]
            checkpoint(event)
            text = text.strip().strip('"«»')
            text = re.sub(r"<think>.*?</think>", "", text, flags=re.S).strip()
            text = re.sub(r"[\U00010000-\U0010ffff\u2600-\u27ff]", "", text).strip()
            reason = None
            if not 8 <= len(text) <= 320:
                reason = "Нужна короткая законченная реплика длиной от 8 до 320 символов."
            elif not re.search((TOPICS_EN if settings["speech_language"] == "en" else TOPICS)[category], text, re.I):
                reason = "Не названо требуемое действие: " + character_messages(category, settings, variant)[-1]["content"]
            elif re.search(r"[А-Яа-яЁё\u3400-\u9fff]" if settings["speech_language"] == "en" else r"[A-Za-z]{3,}|[\u3400-\u9fff]", text):
                reason = "Нужна русская речь без иностранных слов и пояснений."
            elif starts_with_pora(text):
                reason = "Начало «Пора» запрещено; начни совсем иначе."
            elif not settings["swear"] and (SWEAR.search(text) or SWEAR_EN.search(text)):
                reason = "Мат выключен; сформулируй без мата."
            elif self.store.known(text):
                history = self.store.recent(500)
                conflict = next((old for i, old in enumerate(history) if normalize(text) == normalize(old) or too_similar(text, old) or (i < 8 and same_opening(text, old))), None)
                reason = "Повторяется прежняя формулировка или начало."
                if conflict:
                    reason += " Уже было: «" + conflict + "». Нужна другая конструкция, а не замена отдельных слов."
            if reason is None:
                pressure = opening_pressure(text, recent)
                # Runtime keeps the history-aware prompt but must not wait for
                # extra model calls once a valid unique line is available.
                if not pressure or not style_retries:
                    return text
                # Keep a valid unique line if all six attempts only miss the
                # stylistic preference. Hard validation/duplicate checks above
                # can never put an invalid line into this reserve.
                if reserve is None or pressure < reserve[0]:
                    reserve = (pressure, text)
                reason = ("Choose a different opening; this word family is overused." if english
                          else "Выбери другое начало: это слово или семейство обращений встречается слишком часто.")
            rejected.append("«" + text + "» — " + reason)
            progress("Повтор генерации: " + reason, attempt + 1, 6)
        checkpoint(event)
        if reserve is not None:
            return reserve[1]
        raise ValueError("Не удалось создать новую реплику за 6 попыток. Последняя причина: " + reason)

    def direction(self, text, category, event, progress, text_model=None, use_text_model=True):
        settings = self.store.settings()
        result = fixed(settings)
        if result:
            return result
        with self.store.lock:
            recent = [r[0] for r in self.store.db.execute("SELECT voice_style FROM phrases WHERE model='tts-cosyvoice3' AND voice_style<>'' ORDER BY rowid DESC LIMIT 6")]
        styles = catalog(settings)
        recent = [style for style in recent if style in styles]
        if not use_text_model:
            return select("", category, recent, text, styles)
        self.load(text_model or settings["text_model"], event, progress)
        progress("Подбор интонации CosyVoice к новой фразе", 0, 0)
        allowed = options(recent, styles)
        reply = self.model.generate([
            {"role": "system", "content": "You direct an actor's delivery of a " + ("Russian" if settings["speech_language"] == "ru" else "English") + " reminder. Choose by the sentence's emotional intent, not the character's dominant personality. Prefer variety when several deliveries fit. Return exactly ONE allowed label, no explanation. Available deliveries (label: name; acting instruction):\n" + "\n".join(item["id"] + ": " + item["name"] + "; " + item["instruction"] for item in settings["delivery_catalog"]) + "\nCharacter: " + settings["character_description"]},
            {"role": "user", "content": "Recent deliveries (newest first): " + ", ".join(recent) + "\nAllowed labels: " + ", ".join(allowed) + "\nChoose the delivery for this sentence:\n" + text}], event, max_tokens=96)
        return select(reply, category, recent, text, styles)

    def speech(self, row, model, event, progress, seed=None):
        import numpy as np
        import soundfile as sf
        revision = self.components.manifest["models"][model]["revision"]
        old_profile = self.profile_hash
        self.refresh_voice()
        if old_profile != self.profile_hash:
            self.unload()
        force_direction = seed is not None
        seed = row.get("seed", 42) if seed is None else seed
        delivery = None
        reference = self.reference
        profile_hash = self.profile_hash
        if model == "tts-cosyvoice3":
            names = tuple(self.references)
            chosen = self.store.assign_reference(row["id"], names)
            reference = self.references[chosen]["path"]
            profile_hash = chosen
            if self.store.settings()["cosy_style"] == "auto" and row.get("voice_instruction") and not force_direction:
                delivery = {name: row[name] for name in ("voice_style", "voice_instruction")}
            else:
                delivery = self.direction(row["text"], row["category"], event, progress)
        key = cache_key(row["text"], model, profile_hash, revision, seed,
                        "cosy-instruct2-v1:" + delivery["voice_instruction"] if delivery else "", language=self.store.settings()["speech_language"])
        target = self.store.root / "audio" / (key + ".flac")
        if not target.exists():
            self.load(model, event, progress)
            started = time.monotonic()
            progress("Озвучка: " + row["text"][:100], 0, 0)
            if model == "tts-cosyvoice3":
                raw = target.with_suffix(".cosy.wav")
                try:
                    self.model.generate(row["text"], delivery["voice_instruction"], reference, raw, seed, event)
                    wave, rate = sf.read(raw, dtype="float32")
                except BaseException:
                    self.unload()  # Release the writer and its GPU before removing a cancelled temporary file.
                    raise
                finally:
                    raw.unlink(missing_ok=True)
            else:
                import torch
                torch.manual_seed(seed)
                with torch.inference_mode():
                    waves, rate = self.model.generate_voice_clone(text=row["text"], language="Russian" if self.store.settings()["speech_language"] == "ru" else "English", voice_clone_prompt=self.prompt,
                                                                 max_new_tokens=768, stopping_criteria=self.stopping(event))
                wave = np.asarray(waves[0], dtype=np.float32)
            checkpoint(event)
            rms = float(np.sqrt(np.mean(wave * wave)))
            if not np.isfinite(wave).all() or len(wave) < rate // 4 or rms < 1e-6:
                raise ValueError("Модель вернула пустое или повреждённое аудио")
            wave *= min(0.1 / rms, 0.97 / max(float(np.max(np.abs(wave))), 1e-6))
            temp = target.with_suffix(".tmp")
            sf.write(temp, wave, rate, format="FLAC", subtype="PCM_16")
            checkpoint(event)
            temp.replace(target)
            progress("Готово за %.1f с · %.1f с аудио" % (time.monotonic() - started, len(wave) / rate), 1, 1)
        return self.store.audio(row["id"], model, key, target.name, seed, delivery)
