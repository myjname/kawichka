# Кавычка — макеты P2, review-05

Статус: **review-05 согласован пользователем 23.10.2026, P2 закрыт.** [Приёмка](../../RUST-SLINT-P2-ACCEPTANCE.md).

62 состояния, 552 Slint-рендера. RU/EN и обе темы для каждого; десять контрольных форм дополнительно в узком размере и DPI 150/200%. Это синтетический рендер, не физический Windows/Narrator-тест.

[Локальная галерея с переключателями и переходами](../../../work/p2-review-20261023/gallery/index.html) · [Решения и границы P2](../../RUST-SLINT-P2-DESIGN.md)

## Основные экраны

- [welcome-ru-light-desktop-top](welcome-ru-light-desktop-top.png)
- [appearance-ru-dark-desktop-top](appearance-ru-dark-desktop-top.png)
- [audio-missing-ru-light-desktop-top](audio-missing-ru-light-desktop-top.png)
- [generation-ru-light-desktop-top](generation-ru-light-desktop-top.png)
- [storage-ru-dark-desktop-top](storage-ru-dark-desktop-top.png)
- [restore-review-ru-light-desktop-top](restore-review-ru-light-desktop-top.png)
- [diagnostics-ru-dark-desktop-top](diagnostics-ru-dark-desktop-top.png)
- [library-ru-dark-desktop-top](library-ru-dark-desktop-top.png)
- [manual-batch-ru-light-desktop-top](manual-batch-ru-light-desktop-top.png)
- [animation-ru-light-desktop-top](animation-ru-light-desktop-top.png)
- [paused-ru-dark-desktop-top](paused-ru-dark-desktop-top.png)
- [uninstall-ru-light-desktop-top](uninstall-ru-light-desktop-top.png)
- [erase-data-ru-dark-desktop-top](erase-data-ru-dark-desktop-top.png)
- [welcome-en-dark-dpi200-bottom](welcome-en-dark-dpi200-bottom.png)

## Все состояния

Полная локальная галерея и матрица игнорируются Git; их можно повторно получить из review.json рендерером и scripts/build-p2-gallery.py. Изображения выше включены в комплект проекта.

| ID | Экран | Требования | Состояние для просмотра |
|---|---|---|---|
| welcome | Давай познакомимся | B02, B03, S01 | [RU / light](../../../work/p2-review-20261023/gallery/frames/welcome-ru-light-desktop-top.png) |
| first-schedule | Подстроимся под твой день | B02, S03, S13 | [RU / light](../../../work/p2-review-20261023/gallery/frames/first-schedule-ru-light-desktop-top.png) |
| first-reminder | Вот как я напомню | B02, B05, S01 | [RU / light](../../../work/p2-review-20261023/gallery/frames/first-reminder-ru-light-desktop-top.png) |
| first-models | Модели — по желанию | B02, S01 | [RU / light](../../../work/p2-review-20261023/gallery/frames/first-models-ru-light-desktop-top.png) |
| migration-review | Нашлась прежняя Кавычка | B02, B20, S02 | [RU / light](../../../work/p2-review-20261023/gallery/frames/migration-review-ru-light-desktop-top.png) |
| today | Сегодня можно чуть спокойнее | S03, S13, B18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/today-ru-light-desktop-top.png) |
| schedule | Твой ритм | S03, S08, S13 | [RU / light](../../../work/p2-review-20261023/gallery/frames/schedule-ru-light-desktop-top.png) |
| appearance | Понятно на твоём языке | B03, S08, B18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/appearance-ru-light-desktop-top.png) |
| activity | Когда отдых считается отдыхом | S04, S05, B03 | [RU / light](../../../work/p2-review-20261023/gallery/frames/activity-ru-light-desktop-top.png) |
| fullscreen | Как напоминать поверх игры | B04, S16, S15 | [RU / light](../../../work/p2-review-20261023/gallery/frames/fullscreen-ru-light-desktop-top.png) |
| dnd-unavailable | Системный режим не определён | B04, S16 | [RU / light](../../../work/p2-review-20261023/gallery/frames/dnd-unavailable-ru-light-desktop-top.png) |
| audio | Где будет звучать голос | B05, S17 | [RU / light](../../../work/p2-review-20261023/gallery/frames/audio-ru-light-desktop-top.png) |
| audio-missing | Наушники недоступны | B05, S17 | [RU / light](../../../work/p2-review-20261023/gallery/frames/audio-missing-ru-light-desktop-top.png) |
| animation | Живая, но не отвлекающая | B06, B18, D14 | [RU / light](../../../work/p2-review-20261023/gallery/frames/animation-ru-light-desktop-top.png) |
| paused | Тихий час для Кавычки | S05, D14 | [RU / light](../../../work/p2-review-20261023/gallery/frames/paused-ru-light-desktop-top.png) |
| character | Характер отдельно от голоса | B03, S07, S08 | [RU / light](../../../work/p2-review-20261023/gallery/frames/character-ru-light-desktop-top.png) |
| voice-preset | Своя манера речи | S07, B12, B03 | [RU / light](../../../work/p2-review-20261023/gallery/frames/voice-preset-ru-light-desktop-top.png) |
| visual-preset | Своя внешность | S07, B22, B06 | [RU / light](../../../work/p2-review-20261023/gallery/frames/visual-preset-ru-light-desktop-top.png) |
| models | Выбери текст и голос | B03, B12, S01, S09 | [RU / light](../../../work/p2-review-20261023/gallery/frames/models-ru-light-desktop-top.png) |
| model-review | Перед загрузкой | B12, S09 | [RU / light](../../../work/p2-review-20261023/gallery/frames/model-review-ru-light-desktop-top.png) |
| model-download | Скачиваем выбранное | S09, B12 | [RU / light](../../../work/p2-review-20261023/gallery/frames/model-download-ru-light-desktop-top.png) |
| model-error | Компонент требует проверки | B12, S09 | [RU / light](../../../work/p2-review-20261023/gallery/frames/model-error-ru-light-desktop-top.png) |
| generation | Заранее или при событии | B03, S06, S08, B17 | [RU / light](../../../work/p2-review-20261023/gallery/frames/generation-ru-light-desktop-top.png) |
| battery | Работаем от батареи | B17, S18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/battery-ru-light-desktop-top.png) |
| library | Оставь то, что нравится | S03, S06, B18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/library-ru-light-desktop-top.png) |
| library-empty | Пока нет своих фраз | S01, S06, B18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/library-empty-ru-light-desktop-top.png) |
| library-partial | День готов не полностью | S06, B12, B13 | [RU / light](../../../work/p2-review-20261023/gallery/frames/library-partial-ru-light-desktop-top.png) |
| revoice | Обновить только озвучку | S06, S08, B13 | [RU / light](../../../work/p2-review-20261023/gallery/frames/revoice-ru-light-desktop-top.png) |
| clear-audio | Очистить записи? | S10, B18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/clear-audio-ru-light-desktop-top.png) |
| storage | Одна папка для всех версий | B08, S10, S11 | [RU / light](../../../work/p2-review-20261023/gallery/frames/storage-ru-light-desktop-top.png) |
| move-review | Перенос на другой диск | B08, S10 | [RU / light](../../../work/p2-review-20261023/gallery/frames/move-review-ru-light-desktop-top.png) |
| move-progress | Переносим и проверяем | B08, S10, B14 | [RU / light](../../../work/p2-review-20261023/gallery/frames/move-progress-ru-light-desktop-top.png) |
| move-failed | Перенос не завершён | B08, S10, B14 | [RU / light](../../../work/p2-review-20261023/gallery/frames/move-failed-ru-light-desktop-top.png) |
| storage-offline | Не вижу выбранную папку | S02, S10, B08 | [RU / light](../../../work/p2-review-20261023/gallery/frames/storage-offline-ru-light-desktop-top.png) |
| archive-export | Сохранить переносимый архив | B07, S12 | [RU / light](../../../work/p2-review-20261023/gallery/frames/archive-export-ru-light-desktop-top.png) |
| restore-review | Что добавить из архива | B07, B20, S12, D08 | [RU / light](../../../work/p2-review-20261023/gallery/frames/restore-review-ru-light-desktop-top.png) |
| restore-conflicts | Как разрешатся совпадения | B07, B20, S12 | [RU / light](../../../work/p2-review-20261023/gallery/frames/restore-conflicts-ru-light-desktop-top.png) |
| archive-invalid | Архив не прошёл проверку | B07, S12, B14 | [RU / light](../../../work/p2-review-20261023/gallery/frames/archive-invalid-ru-light-desktop-top.png) |
| diagnostics | Проверь отчёт перед сохранением | B09, S13, S20 | [RU / light](../../../work/p2-review-20261023/gallery/frames/diagnostics-ru-light-desktop-top.png) |
| issues | Причина и следующий шаг | B09, B12, B18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/issues-ru-light-desktop-top.png) |
| update | Есть новая версия | S11, S08, B16 | [RU / light](../../../work/p2-review-20261023/gallery/frames/update-ru-light-desktop-top.png) |
| update-ready | Обновление готово | S11, S08, B16 | [RU / light](../../../work/p2-review-20261023/gallery/frames/update-ready-ru-light-desktop-top.png) |
| update-failed | Обновление не включено | S11, B14, B16 | [RU / light](../../../work/p2-review-20261023/gallery/frames/update-failed-ru-light-desktop-top.png) |
| rollback | Вернуться к прежней версии? | S11, B15 | [RU / light](../../../work/p2-review-20261023/gallery/frames/rollback-ru-light-desktop-top.png) |
| unsaved | Есть несохранённые изменения | S08, B18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/unsaved-ru-light-desktop-top.png) |
| reminder | Один повод — одно сообщение | S04, S14, S15 | [RU / light](../../../work/p2-review-20261023/gallery/frames/reminder-ru-light-desktop-top.png) |
| followup | Напоминание, чуть настойчивее | S04, S15 | [RU / light](../../../work/p2-review-20261023/gallery/frames/followup-ru-light-desktop-top.png) |
| first-sound-missing | Системный голос не найден | B02, B05, S01 | [RU / light](../../../work/p2-review-20261023/gallery/frames/first-sound-missing-ru-light-desktop-top.png) |
| storage-space | Места пока не хватает | S10, S11, B15 | [RU / light](../../../work/p2-review-20261023/gallery/frames/storage-space-ru-light-desktop-top.png) |
| archive-progress | Восстанавливаем архив | S12, B07, B14 | [RU / light](../../../work/p2-review-20261023/gallery/frames/archive-progress-ru-light-desktop-top.png) |
| archive-done | Архив восстановлен | S12, B07 | [RU / light](../../../work/p2-review-20261023/gallery/frames/archive-done-ru-light-desktop-top.png) |
| update-download | Загружаем обновление | S11, B16 | [RU / light](../../../work/p2-review-20261023/gallery/frames/update-download-ru-light-desktop-top.png) |
| cache-cleanup | Что очистить | S10, B15 | [RU / light](../../../work/p2-review-20261023/gallery/frames/cache-cleanup-ru-light-desktop-top.png) |
| uninstall | Удалить программу, сохранить своё | S19, B10 | [RU / light](../../../work/p2-review-20261023/gallery/frames/uninstall-ru-light-desktop-top.png) |
| library-loading | Открываем библиотеку | B18, S13 | [RU / light](../../../work/p2-review-20261023/gallery/frames/library-loading-ru-light-desktop-top.png) |
| runtime-fallback | Напоминание пришло вовремя | S06, B13 | [RU / light](../../../work/p2-review-20261023/gallery/frames/runtime-fallback-ru-light-desktop-top.png) |
| download-paused | Загрузка остановлена | S09, B12 | [RU / light](../../../work/p2-review-20261023/gallery/frames/download-paused-ru-light-desktop-top.png) |
| manual-batch | Небольшая партия фраз | B03, S06 | [RU / light](../../../work/p2-review-20261023/gallery/frames/manual-batch-ru-light-desktop-top.png) |
| pause-duration | Пауза на твоих условиях | S05, B18 | [RU / light](../../../work/p2-review-20261023/gallery/frames/pause-duration-ru-light-desktop-top.png) |
| install-review | Где будет жить Кавычка | B02, B08, S01, S02, S11 | [RU / light](../../../work/p2-review-20261023/gallery/frames/install-review-ru-light-desktop-top.png) |
| erase-data | Удалить данные безвозвратно? | B10, S19 | [RU / light](../../../work/p2-review-20261023/gallery/frames/erase-data-ru-light-desktop-top.png) |
| archive-export-done | Архив сохранён | B07, S12 | [RU / light](../../../work/p2-review-20261023/gallery/frames/archive-export-done-ru-light-desktop-top.png) |
