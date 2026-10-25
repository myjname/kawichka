# P1: действующий контракт переноса, редакция 2

Дополнение 0.1.25: автоматически записанные неизменённые defaults и checkpoint без содержательных данных не превращают чистый профиль в непустой. Локальный лимит резервов сохраняется при чистом restore и сам по себе не включает слияние. Собственные настройки/история/пресеты/неподтверждённые сообщения сохраняют режим слияния; содержательное состояние учитывается и при отсутствии строки preferences. Restore проверяет неизменность preferences, содержательного state и архива неподтверждённых сообщений после сводки; обновление одного checkpoint не считается изменением пользовательских данных. Номера схем сохраняются. [Проверки](RUST-SLINT-CANDIDATE-0.1.25.md).

Дополнение 25.10.2026 для 0.1.24, без изменения номеров схем:

- Дневная подготовка хранит необязательный `preparation_state.cancelled_day`. Пользовательская отмена блокирует автоматическую подготовку до нового дня или явного «Подготовить день»; Cancel→Exit сохраняет отмену до завершения service. Штатный shutdown использует отдельный токен и не означает пользовательскую отмену.
- Активные неподтверждённые сообщения ограничиваются 128; прежние сохраняются как типизированные `library_meta.pending_notice:<noticeID>` в одной транзакции с checkpoint, claims и журналом. Подтверждение прежнего сообщения удаляет его из архива и записывает confirmed, но не отмечает сегодняшний завтрак/отдых и не запускает озвучку. В UI архив доступен страницами по 10. Полный профиль сохраняет архив, профиль без библиотеки его исключает. Для новых профилей с такими ключами нужна 0.1.24+: прежний строгий reader откажет без потери данных; штатный откат использует резерв до обновления.
- Библиотека отдаётся service страницами по 50. Выбранный день, язык и часовой пояс принадлежат service; точечный поиск и очередь не требуют передачи полной истории в UI. История защиты от повторов не сокращается пагинацией.
- Для объединённых событий полный встроенный текст и системный голос имеют приоритет перед несовместимой одиночной записью. Специальная генерация объединённой записи выбранной TTS-моделью пока не реализована.

Проверки и ограничения — [кандидат 0.1.24](RUST-SLINT-CANDIDATE-0.1.24.md). Это дополнение не закрывает P4–P8 или аппаратную приёмку.

Дата: 23.10.2026. Заменяет промежуточную редакцию [DATA-CONTRACT](RUST-SLINT-DATA-CONTRACT.md). Фиксирует решения, текущие форматы и ожидаемое поведение ещё не подключённых функций. Не объявляет выполненными P2–P8. Проверки и условия закрытия: [приёмка P1](RUST-SLINT-P1-ACCEPTANCE.md).

## 1. Владельцы и границы

| Область | Владелец записи | Контракт / проверка |
|---|---|---|
| Настройки, очередь, claims, история, подготовка | Rust service → Sandbox/SQLite | UI посылает команды; worker получает снимок, не путь к БД. Prefs/state/claim фиксируются одной транзакцией до физической доставки |
| Расписание, активность, события | Чистое core с Clock/Input/Environment | Нет Slint/Win32/Python; будущая платформа заменяет адаптеры, а не правила |
| Результат модели | Один worker в Windows Job | Только jobs/job-*; проверка путей/размера/task ID; готовый файл, затем транзакция строки |
| Снимок старой установки | preview под LegacyGuard | Оригинал SQLite не открывается; DB/WAL/SHM захватываются в частную копию, Backup API работает на ней |
| Программа, current/transaction | maintenance под install.lock | Неизменяемые versions/<semver>; переключение после проверки пакета и probe |
| Внешние модели/старые среды | Пользовательский источник | Чтение по прежнему пути; новые компоненты в components рабочего поколения. Не удалять и не исправлять внешние файлы |

Матрица функций и проверок — разделы 4–8 [аудита](RUST-SLINT-PLAN-AUDIT.md): A01–O11, S01–S20, D01–D14, R1–R7. Открытые строки сохраняют обязательность. Контракт P1 не подменяет подключение функций тестами вспомогательных правил.

## 2. Версии и сочетания

Версия необходима, но недостаточна: форма, типы, пути, целостность и компоненты тоже проверяются. Единица обновления программы — согласованный пакет app + worker + backend + каталог. Веса и данные независимы от пакета; произвольный будущий app со старым worker не объявляется совместимым.

| Слой | Текущий диапазон | Несовместимость / восстановление |
|---|---|---|
| App | Локальные 0.1.0 → 0.1.1; Cargo 0.1.0, release задаётся Build-package | Probe сравнивает скомпилированную версию с manifest. Semver не заменяет проверку перехода |
| Старый settings.json | SchemaVersion отсутствует или целое 0/1/2; UTF-8, допустим BOM | Отрицательное/дробь/строка/null/bool/>2 — отказ без reset. Другие поля проверяет конвертер |
| Старая SQLite | user_version 0/1/2 + известная форма Store (§4) | Незнакомая таблица/view/trigger, версия или битая БД — отказ; repair только отдельно на копии |
| Снимок | Report/Journal/SettingsSnapshot format=1; library.snapshot.sqlite3 user_version=3 | Архив переноса, не рабочая БД старого Store; Ready и совпадающий источник обязательны |
| Рабочая SQLite | Пустая БД 0, прежние 1/2 → текущая 3; Preferences.format=1, State.format=1 | Непустой корень требует owner. Будущая схема/неизвестные типизированные поля/неверные значения — ошибка без сброса; версия 0 не разрешает чужую БД |
| Старый checkpoint | reminder-state.json Version=1 или отсутствует | Неверная версия/даты/границы серии — отказ, не новая серия. RFC3339/.NET round-trip и /Date(ms±offset)/; null/0001 — отсутствие даты |
| Пресет | version=1; character / voice / visual | Неподдержанный kind/version — отказ; неизвестная metadata не исполняется и не экспортируется. Невалидный выбранный пресет блокирует активацию; остальные дают предупреждение |
| IPC | JSON lines version=1, task ID, один worker | Иной task/version не принимается; EOF/error/timeout освобождает задание. Версия не доказывает работоспособность модели |
| Установка | Manifest/Current/Journal format=1, platform=windows-x64, semver | Неизвестное не перезаписывать. Без совместимого probe/резерва current прежний, журнал остаётся |
| Runtime/model | Закреплённый assets/components.json, таблица ниже | Отсутствующее — явный статус и выбранный repair/install; наличие папки не доказывает запуск |

Эволюция: добавочное поле требует документированного default и fixture прежней формы. Изменение смысла/обязательной формы/операции IPC/хранения — новая соответствующая версия и конвертер. Понижать схему на месте или запускать старый EXE на новой БД нельзя. Откат восстанавливает проверенную прежнюю БД, новая работа сохраняется отдельно; автоматического слияния контракт не требует.

IPC v1: операции generate/speech/install/probe/export/preset_import; ответы progress/text/phrase/result/error. Запрос до 4 MB, отдельный payload rows/known_texts до 64 MB, строка ответа до 1 MB; worker timeout 1800 секунд. Runtime-доставка имеет собственный короткий предел. Диагностика — stderr, stdout — JSON. UI.environment — предварительная индикация по файлам, не квалификация движка. Backend дополнительно проверяет marker revision; окончательное доказательство — запуск. Единая полная диагностика совместимости остаётся B12/P6.

<!-- P1-MODELS-BEGIN -->
| ID модели | Формат | Закреплённая ревизия | Байт весов/файлов |
|---|---|---|---|
| text-1.7b | transformers | 70d244cc86ccca08cf5af4e1e306ecf908b1ad5e | 4079423234 |
| text-gemma4-12b | gguf | 29d097773436b69ff9feafd636ab4cf873786537 | 6975879296 |
| text-gemma4-e4b | gguf | 4b4a2c1d584be7264f87aac328a1bc739ce81b6c | 5154941280 |
| text-ministral3-8b | gguf | ad82bf81321f4b22de70014ecd5135730115f6a8 | 5198387456 |
| text-qvikhr3-8b | gguf | 0014437899c19be34ec84d532912ea2dc5347489 | 5027783552 |
| text-qwen35-4b | gguf | 4168f45a16a1290d65a4ec0fa312ae917a4c15d6 | 3013027808 |
| text-qwen35-9b | gguf | 182be2fd6c7bc44887d88a91cb03ff009cc9f549 | 6169341984 |
| text-saiga-llama3-8b | gguf | d9e7e9e52b9268a6764bd3ba73756684c7e4fb8e | 4920734016 |
| tts-0.6b | transformers | 5d83992436eae1d760afd27aff78a71d676296fc | 2516100892 |
| tts-1.7b | transformers | fd4b254389122332181a7c3db7f27e918eec64e3 | 4544170364 |
| tts-cosyvoice3 | cosyvoice3 | 29e01c4e8d000f4bcd70751be16fa94bf3d85a18 | 5427041085 |
| system | Windows SAPI | Установленные системные RU/EN-голоса | 0 |
<!-- P1-MODELS-END -->

Точная фиксация каталога/сред: [p1-compatibility.json](../next/contracts/p1-compatibility.json). Llama CPU/CUDA — разные runtime; CUDA llama и Torch имеют разные зависимости. GPU 6–8 ГБ не означает, что любая модель помещается. Текст выгружается до речи. Явный CUDA не должен скрывать ошибку переходом на CPU; Auto выбирает устройство до задания. Базовые напоминания независимы от этого. Полная матрица запусков — P6, все модели одновременно скачивать не нужно.

| Маркер среды | Реально читаемые старые формы | Граница доверия |
|---|---|---|
| runtime/python-bootstrap.json | Новый закреплённый Python 3.12.10, путь/хеш exe после проверки ZIP | Проверка bootstrap, не свидетельство наличия Torch/моделей; старый runtime/python.exe может использоваться отдельно |
| runtime-ok.json | Legacy JSON-object без собственной format-версии; gpu:bool либо старый torch со строкой +cu для CUDA | Не распространять совместимость на любые будущие среды. Rust-индикация проверяет слабее backend status с import probe; реальный запуск обязателен. Повреждённое/неподходящее — environment unavailable, не reset библиотеки |
| llama/verified.json | revision равен выбранному CPU/CUDA runtime каталога, gpu:bool; старое отсутствие gpu означает CUDA | Дополнительно exe и требуемые DLL. Незнакомая revision/неверный тип gpu — не ready; не перезаписывать внешнюю среду |
| cosy/verified.json | revision равен cosy_runtime.revision каталога и требуемые файлы пакетов | Папка без подходящего маркера не ready; полноценный Torch/запуск проверяется отдельно |

Кандидат установки может пройти data/probe без загрузки моделей: это совместимость запуска базовой программы, не утверждение о совместимости всех весов. Repair выбранного компонента создаёт новую управляемую среду, затем проверяет её; старые внешние пути сохраняются. Совместное изменение каталога/worker требует повторной квалификации затронутых runtime/model-ячеек, не перекачивания всего каталога.

## 3. Поля: источник → снимок → активная модель → экспорт

[p1-fields.json](../next/contracts/p1-fields.json) содержит каждое разрешённое поле и синтетическое non-default значение. Unit-проверка требует точного покрытия allowlist и сравнивает активные поля с результатом настоящего конвертера. active — Preferences; state — State; derived — преобразование; shared — выбор из двух источников; policy/source — управление импортом; archive — сохранено без активной настройки.

Все разрешённые поля сохраняются в санированном SettingsSnapshot (shared — только desktop). Неизвестные настройки исключаются по значениям, сохраняются имена; settings в копии очищается с secure_delete + VACUUM. Это фильтрация настроек, не обещание удалить произвольный секрет из пользовательского текста/добавочного столбца. Личные снимки, записи и пресеты не входят в пакет/Git.

TextModel/text_model, RuntimeTextModel/runtime_text_model, VoiceModel/tts_model, Swear/swear, RuntimeGeneration/runtime: JSON при BackendSettingsMigrated=true или ручном backend-key в BackendSettingsOverrides; иначе SQLite. Отсутствующий выбранный источник → второй → известный default. AutomaticPreparation/auto_daily и AutoKeepDaily/daily_auto_keep имеют независимый BackendDailySettingsMigrated. SpeechLanguage/VoiceProfile/AddressForm/UserName: JSON, затем SQLite. System принудительно выбирает системную речь, сохраняя старый VoiceModel в снимке. Если нет TextGeneration, он выводится из VoiceProvider.

<!-- P1-FIELDS-BEGIN -->
| Источник | Поле | Режим | Назначение / правило |
|---|---|---|---|
| desktop | SchemaVersion | source | legacy schema |
| desktop | SetupComplete | active | /desktop/setup_complete |
| desktop | PersonalizationMigrated | archive | migration provenance |
| desktop | BackendPersonalizationMigrated | archive | migration provenance |
| desktop | BackendSettingsMigrated | policy | JSON wins shared settings |
| desktop | BackendDailySettingsMigrated | policy | JSON wins daily settings independently |
| desktop | BackendSettingsOverrides | policy | manual backend-key overrides |
| desktop | LastNotifiedUpdate | active | /desktop/last_notified_update |
| desktop | UiLanguage | active | /ui_language |
| desktop | SpeechLanguage | active | /speech_language |
| desktop | ThemeMode | active | /theme |
| desktop | AddressForm | active | /generation/address_form |
| desktop | UserName | active | /user_name |
| desktop | TimeZoneId | active | /desktop/time_zone |
| desktop | WorkDays | active | /schedule/work_days |
| desktop | WorkStart | active | /schedule/work_start |
| desktop | WorkEnd | active | /schedule/work_end |
| desktop | BreakEvery | active | /schedule/break_every |
| desktop | BreakfastTime | active | /schedule/breakfast |
| desktop | LunchTime | active | /schedule/lunch |
| desktop | DinnerTime | active | /schedule/dinner |
| desktop | MealsEnabled | active | /schedule/meals_enabled |
| desktop | ExerciseAfter | active | /schedule/exercise_after_minutes |
| desktop | IdleMinutes | active | /idle_minutes |
| desktop | AllowedClicks | active | /allowed_clicks |
| desktop | RepeatMinutes | active | /repeat_minutes |
| desktop | BreakRepeats | active | /reminders/break_repeats |
| desktop | ExerciseRepeats | active | /reminders/exercise_repeats |
| desktop | MealRepeats | active | /reminders/meal_repeats |
| desktop | TrustConfirmation | active | /trust_confirmation |
| desktop | PauseUntilUtc | state | /pause_until |
| desktop | FullscreenVisual | active | /reminders/fullscreen_visual |
| desktop | FullscreenAudio | active | /reminders/fullscreen_audio |
| desktop | DeferGenerationFullscreen | active | /desktop/defer_generation_fullscreen |
| desktop | PetX | state | /pet_position/0 |
| desktop | PetY | state | /pet_position/1 |
| desktop | PetSize | active | /desktop/pet_size |
| desktop | VisualFile | active | /desktop/visual_preset |
| desktop | VoiceProfile | active | /generation/voice_profile |
| desktop | CharacterFile | active | /desktop/character_preset |
| desktop | PetVisible | active | /reminders/pet_visible |
| desktop | AutomaticUpdates | active | /desktop/automatic_updates |
| desktop | RunAtLogin | active | /desktop/run_at_login |
| desktop | AutomaticPreparation | active | /generation/auto_daily |
| desktop | AutoKeepDaily | active | /generation/daily_auto_keep |
| desktop | TextGeneration | active | /generation/text_enabled |
| desktop | RuntimeGeneration | active | /generation/runtime |
| desktop | TextModel | active | /generation/text_model |
| desktop | RuntimeTextModel | active | /generation/runtime_text_model |
| desktop | VoiceModel | active | /generation/tts_model |
| desktop | HandledEvents | derived | State.handled IDs resolved in selected time zone |
| desktop | VoiceProvider | derived | System forces tts_model=system; absent TextGeneration derived from provider |
| desktop | Swear | active | /generation/swear |
| desktop | Voice | derived | with NotificationSound -> reminders.audio |
| desktop | NotificationSound | derived | with Voice -> reminders.audio |
| desktop | Paused | derived | State.pause_until/pause_forever; expired pause not resumed |
| desktop | ExerciseEnabled | active | /schedule/exercise_enabled |
| desktop | LastMeal | derived | handled notification only; never breakfast confirmation |
| desktop | LastBreak | derived | pre-v2 hour marker -> handled break |
| desktop | ExerciseUtc | derived | exercise due and inferred recent confirmed breakfast |
| desktop | History | derived | deleted history rows for repetition protection |
| backend | tts_model | shared | VoiceModel |
| backend | runtime | shared | RuntimeGeneration |
| backend | batch_count | active | /generation/batch_count |
| backend | categories | active | /generation/categories |
| backend | auto_daily | shared | AutomaticPreparation |
| backend | daily_auto_keep | shared | AutoKeepDaily |
| backend | swear | shared | Swear |
| backend | device | active | /generation/device |
| backend | model_paths | active | /generation/model_paths |
| backend | character_description | active | /generation/character_description |
| backend | text_model | shared | TextModel |
| backend | runtime_text_model | shared | RuntimeTextModel |
| backend | cosy_style | active | /generation/cosy_style |
| backend | cosy_instruction | active | /generation/cosy_instruction |
| backend | delivery_catalog | active | /generation/delivery_catalog |
| backend | speech_language | shared | SpeechLanguage |
| backend | voice_profile | shared | VoiceProfile |
| backend | reminder_followup | archive | per-task context replaces global legacy switch |
| backend | address_form | shared | AddressForm |
| backend | user_name | shared | UserName |
<!-- P1-FIELDS-END -->

Отсутствие редактора UI не удаляет поле. Сохранённый JSON не означает активную настройку. Помимо отдельных пресетов и phrases.json+audio ZIP реализован полный профиль (§5). Его реализации и доступным проверкам посвящена [приёмка P5](RUST-SLINT-P5-ACCEPTANCE.md); открытый ENOSPC и полная квалификация установочного канала не считаются пройденными.

Новые поля без старого эквивалента получают defaults Preferences/Generation/State: базовый режим без локальной генерации/Python, ac_only=true, runtime_wait_seconds=10, recovery_limit_bytes=2_000_000_000, animation=true. Начальный текстовый ID тяжёлой модели сохраняется, но text_enabled=false — веса самовольно не скачиваются. Импорт сохраняет старый fullscreen; при отсутствии поля старые Both/All, у чистой установки Nothing/Silent. R3–R5 ниже ещё не подключены целиком.

## 4. База, история, checkpoint

Известные формы phrases из backend/kavychka_local/store.py: базовая таблица и её добавочные seed/voice_*/followup/language/rejected. При отсутствии seed=42, voice_*=пусто, followup=false, language=ru, rejected=false. Недостающие обязательные поля не угадываются — импорт останавливается. Индексы допустимы; добавочные скалярные столбцы известных таблиц сохраняются в архиве, активный смысл не получают; BLOB в phrases отклоняется. day_plans — термин плана, фактическая старая таблица plans; незнакомая day_plans не принимается молча.

| Поле / таблица | Снимок | Активная модель | Экспорт |
|---|---|---|---|
| phrases.id/text/category/model/profile/day/audio_path | Точно | Одноимённые Phrase; локальное имя аудио, категория/язык проверяются | phrases.json+audio; профиль сохраняет ID |
| approved/deleted/rejected/used_at/created_at/followup/language | Точно | Исключённое/использованное не возвращается в очередь; preview не расходует | phrases.json; профиль сохраняет исключения |
| seed/voice_style/voice_instruction/voice_reference | Значения либо старые defaults | Одноимённые Phrase; переозвучка не меняет текст/ID/отбор | phrases.json и профиль |
| normalized | Точное старое | lowercase(text); worker дополнительно нормализует всю историю | Нет в Phrase/ZIP; точное значение остаётся в снимке профиля |
| cache_key | Точное старое | Старый кеш не доказывает совпадение новой озвучки; готовый звук сохраняется | Нет в Phrase/ZIP; остаётся в снимке профиля |
| Добавочные столбцы известных таблиц | Точно, в поддержанных SQLite-типах | Без активного отображения | Только архивная часть профиля |
| settings.key/value | Разрешённый Snapshot, таблица очищена | Preferences/Generation (§3) | Полный профиль P5: санированный Snapshot |
| plans.day/target/status | Все строки | День пересчитывается из расписания/пригодных строк; старый ready не доказывает готовность нового дня | Снимок профиля |
| preparation.slot/day/category/followup/phrase_id/state/attempts/retry_at/error | Все строки/связи | Для state != ready MAX(attempts)/MAX(retry_at) по дню → legacy_preparation_states, бюджет до 3; далее preparation_state(day/attempts/next). Старые слоты/ошибки не становятся новыми задачами | Только архивная часть профиля |
| voice_rotation.id/next_index | Точно | id=1 → library_meta.voice_rotation, отрицательное ограничено 0; обновляется при принятии результата в транзакции | Состояние профиля, не текущий ZIP |
| builtin-days + History | Копия + разрешённое поле | Служебные исключённые строки истории с устойчивыми ID/дедупликацией; не готовая очередь | Строки библиотеки; исходники в профиле |

Checkpoint: Version, Category, Text, MessageUtc, CheckpointUtc, Pending, Manual, Followup, SuspendedUtc, Cycle{Category,StartedUtc,NextRepeatUtc,RepeatsSent,HiddenByConfirmation,Finished}. Сохраняются bytes; конверсия проверяет категории/даты/границы/бюджет. Свежий Pending восстанавливается тихо; скрытая подтверждением серия не показывается заново. Checkpoint старше 5 минут не делает сообщение видимым. LastMeal — доставленное уведомление, не подтверждённый завтрак. ExerciseUtc восстанавливает недавнюю отметку завтрака, включая полночь; таймер независим от WorkDays. PetX/Y ограничивает доступным экраном platform.

## 5. Конфликты и архив

| Ситуация | Решение | Подтверждение / остаток |
|---|---|---|
| Первый пустой импорт | Snapshot → Preferences/State, rows/медиа; origin после файлов/транзакции | storage bootstrap/semantic tests |
| Уже есть Preferences/State | Текущие prefs, включая desktop, сохраняются; из старого state только недостающие handled. legacy_keep_current_preferences защищает от semantic upgrade | Регрессия P1; прежний preview без маркера однократно получает ранее недостающие desktop |
| Тот же / иной origin | Тот же без повторного импорта; другой — отказ. Изменённый источник preview не смешивается с прежним fingerprint | bootstrap/preview tests; не синхронизация приложений |
| Один Phrase ID | Текущая строка/исключения побеждают; добавить недостающее | P1 fixture; не заменять current ради старого approved |
| Одно имя медиа/папка пресета | Одинаковый SHA — reuse; разные bytes — отказ до origin/строк, текущий файл сохранён. Частичные новые файлы остаются для повтора в собственной папке | P1 collision fixture; не полный restore UI |
| Отдельный импорт пресета | Новый локальный ID; одинаковое имя допустимо; выбранный неверный kind — ошибка | presets tests/E04 |
| Выбранный диск исчез | Ошибка, без новой пустой C. Ранее импортированная копия может открыться без оригинала; отсутствующая внешняя модель не installed | location/bootstrap tests |
| Занято / чужой writer / reparse | Не закрывать старый процесс; отказ/повтор при Busy, изменении fingerprint, опасном пути | Local\\KavychkaBreaks-v1 и запрет записи во время чтения не доказывают защиту от всех межсессионных писателей; B14/P5 |
| Откат после новой работы | Прежняя проверенная БД; новое отдельно after-failed-*. Битая forensic-копия не считается целой | E07; автоматическое слияние не требуется |

Полный архив профиля v1 для B07/P5/P7: data-only ZIP, manifest format=1, версия app/schema, относительные пути/размеры/SHA-256. Состав: настройки/State, фразы/история/связи, пресеты/используемые медиа, санированный старый снимок. Исключены код/веса/среды/секреты/приватные логи. Начальный лимит — пользовательский бюджет хранения, не лимит пресета 200 MiB; нехватка места останавливает операцию до замены данных.

Имена профиля: manifest.json с {format:1, kind:"kavychka-profile", app_version, working_schema:3, created_utc, options:{library,audio}, entries:[{path,bytes,sha256}]}; library.sqlite3 — согласованный SQLite backup единственного активного владельца, пересозданный из разрешённых типизированных таблиц prefs/State/meta/phrases/claims. audio/, presets/, builtin-days/, необязательный legacy/library.snapshot.sqlite3 и legacy/reminder-state.json. При повторных слияниях допускаются legacy/imported/<archive-SHA>/ и legacy/profiles/<archive-SHA>/library.sqlite3 с архивными конфликтами и отображением ссылок. Они не становятся рабочей БД. Отдельной второй копии изменяемых prefs нет. Raw settings.json/capture/logs не включать; app_settings в старом снимке санирован. origin/recovery из архива становятся неактивными подсказками; model_paths сохраняются как пути совместимости/поиска существующих весов и сами не запускают загрузку. Точный набор таблиц/версий проверяется; DDL из архива не исполняется. Новая схема требует явного конвертера. Manifest ограничен 4 МиБ, состав — 20 000 файлов и пользовательским лимитом распаковки; не путать с лимитами отдельного пресета ниже.

Restore: отдельная staging-папка → проверка лимитов/путей/хешей/схемы/ссылок → сводка → проверенный резерв → транзакционное применение. В непустом профиле текущие настройки/состояние/исключения побеждают, добавляется недостающее. Один ID с разным содержимым: фраза — оставить current, конфликт сохранить в отчёте/архиве; пресет — новый ID, переписать ссылки только импортных строк. Медиа по хешу с обновлением импортных ссылок; точные совпадения reuse. Не одобрять/не расходовать автоматически. В чистом профиле восстановить всё совместимое; текущие указатели данных/рабочих сред и лимит recovery этой установки сохраняются. Выбранный в установщике лимит до первого пользовательского сохранения существует как pending metadata и не превращает пустой мастер в непустой профиль. Живое сообщение не озвучивать само. Реализованный путь и ограничения — P5; обычный ZIP фраз остаётся отдельной функцией.

Перенос установленного общего хранилища: current/locator проверены → состав/разбивка/пик/новая отдельная папка → приложение завершает writer и worker → копирование при удержании исходного владельца → SQLite backup/probe → журнал/current/locator → durable commit → fencing прежнего поколения. До commit восстанавливаются прежние указатели, новая копия остаётся неактивной; после commit завершается переход, исходные файлы не удаляются. Изменяются только известные пути внутри собственного common; внешние остаются внешними. Windows canonical prefix/регистр/разделители не должны ломать это сопоставление. Смена папки при первой установке не заменяет эту операцию. В preview без установленного владельца общего common предлагается профильный экспорт, а не перенос произвольных внешних моделей.

Служебные форматы P5: profile-restore.json v1 связывает только новые публикации и SQL-маркер commit; profile-job.owner v1 определяет отменяемые jobs. storage-move.json v1 / storage-transfer.json v1 / storage-paths.json обеспечивают recovery/редирект и перебазирование восстановленных старых копий. recovery-index.json v1 хранит явные версии/UTC/sequence/SHA проверенных before-backup; существующий backup.manifest.json v1 не меняет формат. Контрольные markers не являются импортируемыми настройками. Суммарный recovery/quarantine бюджет не включает веса; неизвестный/последний пригодный резерв не удаляется ради лимита.

Материалы: `.kavychka`/preset.json до 200 MiB, ZIP до 150 записей/200 MiB распаковки, manifest до 150 KB. Voice — 1–8 WAV/FLAC, каждый 1–30 секунд, 16–192 kHz; другой звук — отдельная конвертация worker, чистую среду проверяет P6. Visual — PNG/JPEG/GIF/WebP, image или Codex atlas 1536×1872 (8×9 ячеек 192×208); GIF до 300 кадров/40 млн пикселей суммарно. Traversal/ADS/device/reparse/symlink отвергаются. Личные материалы вне публичного пакета; права на загружаемое — ответственность пользователя.

## 6. Состояния и системные политики

Захват: capturing → copying → verifying → ready; отмена не публикует Ready. Повтор сверяет fingerprint; другой источник требует другого назначения. Raw capture удаляется после сверки. Старый pointer не меняется. Установленное поколение <common>/.kavychka-next имеет owner; только его legacy — допустимое вложенное назначение. Sandbox: shell-sandbox.owner/shell.lock; программа: install.owner/install.lock. Старый shared pointer и новый locator — разные владельцы.

Доставка: due → ожидание до deadline → claim+State commit → показ/звук → подтверждение/разрешённый повтор. Preview не claim. Совпавшие еда+перерыв объединяются с независимыми ID; остальные первичные: еда → разминка → перерыв → повторы. Непоказанное устаревает через 5 минут; показанное этим не закрывается. Runtime 1–120 секунд, default 10, от due включая очередь; позднее не заменяет показанное. После сбоя актуальный текст тихо: физический факт услышанного недоказуем. Подготовка: до 3 попыток/день, интервал не меньше 60 секунд, сохранённый бюджет; ручной запуск отдельный. Более гибкие retries требуют отдельной приёмки P6.

Установка: prepared → probing → activating → complete; ошибка: rolling_back → rolled_back. Незнакомое состояние оставлять для разбора. Current только после manifest/files и --prepare-install --expected-version. Резерв — SQLite Backup API + проверенные файлы, без weights/components/cache/jobs; до 7 дней, максимум две предыдущие версии, совместный с карантином начальный бюджет 2 ГБ. Последнюю пригодную копию молча не удалять. Третья версия/сроки/ENOSPC — P7. Пик: старые данные + capture(DB/WAL/SHM) + backup/VACUUM + copied assets + резерв поколения + candidate/extraction, без копирования весов. При неизвестном объёме/нехватке места остановиться до переключения и предложить место/бюджет.

| Политика | Интерфейс и ожидаемое поведение | Сейчас / обязательная проверка |
|---|---|---|
| R3 DND | Адаптер Allowed/Blocked/Unknown отдельно от fullscreen/lock; явное allow_during_dnd=false. Blocked стопит звук/скрывает инициативу без replay; исключение не отменяет lock/pause. Unknown — явное ограничение и ручная пауза | В P4 подключены prefs/Engine/service/UI; E15 включает реальную отмену PCM сигнал/речь при подставленном запрете и tray Pause. SHQuery сообщает уместность, не доказывает переключатель DND: [Microsoft](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shqueryusernotificationstate). Ограничение и ручная пауза показаны явно; физические системные переходы остаются P4 |
| R4 audio | Default либо стабильный endpoint ID в Preferences; недоступный пин сохраняется. Фраза фиксирует endpoint; потеря/смена стопит без переноса/replay, текст остаётся | P3/P4: pin/редактор/сохранение/реальное воспроизведение и missing pin проверены E14/E15. Физические смена/отключение при сигнале, старте/середине речи, preview и следующем событии остаются B05/P4 |
| R5 power | ac_only=true; разовое разрешение привязано к task, постоянную настройку не меняет. Прекратить новые стадии, принять готовый проверенный результат, сохранить бюджет, освободить модели; на AC продолжать только актуальное | P4: ручные команды ограничены, разовое разрешение одной партии реализовано/сквозно проверено E15. Физический AC↔battery открыт. Hard cancel Job не доказывает безопасную границу: B17/P6/IPC v2 stop_after_stage/ack на text accepted / audio file closed / result accepted; timeout+kill при зависании |

Решение P1 для R3 — публичный SHQueryUserNotificationState как сигнал уместности, без чтения недокументированных внутренних структур Windows; точное следование DND заявлять только после системной квалификации, иначе явно объяснять ограничение и давать ручную паузу, как разрешено R3 плана. Для R4 — WASAPI/IMMDevice endpoint ID, без неявного резервного устройства. Расширение кооперативной отмены R5 — IPC v2 согласованной пары app/worker; IPC v1 не объявлять поддерживающим ack. Эти изменения реализуются и проверяются в своих этапах; текущая матрица исполняемого IPC остаётся v1.

Lock разрешает тихую подготовку с учётом pause/AC/fullscreen, запрещает звук/показ. Потеря ввода не доказывает отдых; неизвестный сеанс не засчитывать в idle, не копить инициативу. Для сна использовать подтверждённые временные границы. Настоящие системные события — проверки P4/P8, injected clock их не заменяет.

## 7. Матрица квалификации платформы и доверие

Дополнение P4 к данным: `Preferences.format=1` получает serde-default `reminders.allow_during_dnd=false`. Старый JSON читается новой версией без потери прежних настроек; старый бинарник с deny_unknown_fields не обязан читать новый JSON — для отката применяется проверенный резерв, а не удаление нового поля на месте. SQLite user_version остаётся 3, State/worker IPC остаются 1. В расширяемой `library_meta` namespace `delivery_journal:<noticeID>` имеет собственный format=1: notice, event_ids, phrase_id, phases{committed,shown,audio_requested,audio_preparing,audio_started,audio_completed,audio_cancelled,audio_failed,confirmed} с UTC-датами. Неизвестный format/переопределение phrase_id отвергается, журнал не сбрасывается. Claim+pending+committed и confirmed+удаление pending атомарны; физические показ/PCM и их отдельные записи не могут быть одной транзакцией, поэтому неоднозначное восстановление тихое. Журнал не содержит текста/медиа-пути/кодов ввода. Подробности и доказательства — [P4](RUST-SLINT-P4-ACCEPTANCE.md).

Это целевые проверки, не список уже подтверждённой поддержки. Первая стабильная Next — Windows 11 x64; основные линии build 26200 (25H2) и 26300 (26H2), актуальное накопительное обновление на дату выпуска, точный UBR в результате. Номера проверены по [Microsoft release information](https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information). Старые линии, 26H1/28000, Windows 10/Server, ARM64, Linux/macOS пока вне квалификации. Core архитектурно независим. Текущий вызов OSVersion дал 10.0.26300.0, полного UBR он не сообщает.

| Профиль | Обязательные комбинации / проверки | Что подтверждено |
|---|---|---|
| W1 базовый | 26200/26300 x64, 8 ГБ без GPU/Python/моделей; RU/EN SAPI, темы, DPI 100/150/200%, два монитора, raw input, lock/sleep/DND/fullscreen, автозапуск | Ноутбука 8 ГБ нет; E02/E03/E10 — лишь части на мощном ПК |
| W2 CPU | Обе линии, pinned llama CPU + один GGUF; RAM/возможности CPU и отказ модели; базовый режим продолжает работать | Полный реальный CPU-набор не пройден |
| W3 CUDA | Обе линии, NVIDIA 6–8 ГБ; отдельно pinned llama CUDA/Torch, Auto/явный CUDA, освобождение до речи, OOM/DLL/драйвер | E05 — одна установленная Gemma/Cosy. Сейчас nvidia-smi: RTX 4080 SUPER, 16376 MiB, 617.42; это 16 ГБ, не целевой тест 6–8. Старым логам без metadata этот драйвер не приписывать |
| W4 voice | System, CosyVoice3, Qwen3-TTS 0.6B/1.7B, RU/EN, refs, CPU/CUDA после отдельного probe | E05 только CosyVoice; остальные реальные ячейки — P6 |
| W5 обслуживание | Чистый per-user install, Unicode/пробелы, внешняя библиотека, диск offline, мастер/ярлыки/перевход, обновление/откат, удаление/переустановка, нет сети | E07–E10 — копии и ограниченные системные действия; полный P7/P8 открыт |

Один «минимальный драйвер NVIDIA» для всех сред не назначается: каталог содержит llama CUDA 13.4 и Torch cu128. Работоспособность квалифицируется отдельным probe на точном драйвере. Непроверенную CPU/GPU-комбинацию нельзя выдавать за гарантированную локальную модель. Сводка до выбранной установки/запуска должна объяснять диск/RAM/VRAM и отказ — B12. Миграция не меняет сохранённый выбор ради нового default.

Методика P8: release без debugger; базовый бюджет без моделей, закрытые настройки, прогрев 60 секунд → 10 минут покоя и отдельно 10 минут анимации/напоминаний → 24 часа повторяемых событий. CPU = process time / wall time / logical processors; память = private committed bytes всего принадлежащего дерева, GPU/weights отдельно. Cold/warm launch и UI p95 фиксируются отдельно. Бюджеты исходного плана сохраняются. Необъяснённый рост private bytes >20 MiB между сопоставимыми первой и последней из четырёх последних часовых медиан без возврата после выгрузки/успокоения — незакрытый дефект, а не успешный прогон. Мощный ПК W1 не заменяет.

Доверие текущего канала: фиксированный GitHub myjname/kawichka, стабильный next-vX.Y.Z, HTTPS, asset size/SHA-256 от GitHub, внутренний manifest/file hashes, явный restart/probe. Публикация — отдельным поручением. Это не независимая подпись; локальный кандидат неподписан. Не доверять произвольным URL/сертификатам, не обещать отсутствие SmartScreen. P7 завершает лицензионную инвентаризацию/канал; решение о покупке/включении Authenticode — отдельно до стабильного выпуска. Digest не совпал/источник недоступен — остановить обновление, сохранить рабочее; не отключать проверку. Новая схема подписи/отзыва потребует версии контракта, фиктивного ключа сейчас нет.

## 8. Граница приёмки

P1 требует полного набора источников/полей, версий, детерминированных конфликтов, владельцев/состояний и назначенных проверок. Эталонные сценарии остаются S01–S20 плана; R1–R7 не изменены. Coverage allowlist, настоящие преобразования non-default значений, приоритет каждой shared-пары, старые формы/версии БД, коллизии, архив/активная модель и неизменность оригинала проверяются в [приёмке P1](RUST-SLINT-P1-ACCEPTANCE.md).

Не закрываются вместе с контрактом: DND/пин/кооперативная остановка; полный архив/restore/перенос диска; единый runtime probe и чистая ML-среда; crash/ENOSPC/права/межсессионный writer; полный installer/удаление/лицензии/подпись; ОС/GPU/суточные/аппаратные прогоны. Это B04/B05/B07/B08/B12/B14/B16/B17/B18/B20(P5)/B21. Никакой поддержки всех неизвестных исторических форматов не обещано.

### Совместимое чтение старых настроек после P3

P3 добавляет reminders.audio_device (пусто = системный endpoint) и reminders.animation_intensity (off/subtle/expressive; default subtle). Для старого JSON поля восстанавливаются через serde default; preferences format 1 / SQLite schema 3 и исходные 61+20 legacy-полей не меняются. Тесты P3 проверяют значения по умолчанию и merge. Добавление/удаление model_paths исправлено без сброса чужих/параллельных ключей. Сборка с прежним deny_unknown_fields может не читать новый JSON: откат обязан использовать соответствующий snapshot, его полная проверка остаётся P7.
