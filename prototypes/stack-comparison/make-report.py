"""Collect evidence without altering the measured applications or their data."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT=Path(__file__).resolve().parent
WORK=ROOT.parents[1]/"work/stack-comparison-20261022"
OUT=ROOT/"artifacts/results"
OUT.mkdir(parents=True,exist_ok=True)

def read(path):return json.loads(path.read_text(encoding="utf-8-sig"))
def size(path):return sum(p.stat().st_size for p in path.rglob("*") if p.is_file())
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def interval(values,places=1):
    low,high=min(values),max(values)
    return f"{low:.{places}f}" if round(low,places)==round(high,places) else f"{low:.{places}f}–{high:.{places}f}"

data=read(WORK/"final-gpu/comparison.json")
machine=read(WORK/"final-gpu/machine.json")
gpu=subprocess.run(["nvidia-smi","--query-gpu=name,memory.total,driver_version","--format=csv,noheader"],capture_output=True,text=True)
machine["gpu"]=gpu.stdout.strip() if gpu.returncode==0 else "not detected"
evidence=dict(machine=machine,primary=data,final_native={},bundled_worker_native={},adaptive={},post_fix=read(WORK/"slint-after-lifecycle-fix/result.json"),packages={},binary_hashes={},primary_binary_hashes=read(WORK/"final-gpu/binary-hashes.json"))
final_startups=read(WORK/"final-packaged-startups/startups.json")
evidence["final_packaged_startups"]=final_startups
for key,exe in (("slint","Quote.Slint.exe"),("avalonia","Quote.Avalonia.exe")):
    evidence["final_native"][key]=read(WORK/("native-slint-fixed" if key=="slint" else "native-avalonia")/"native.json")
    evidence["bundled_worker_native"][key]=read(WORK/("bundled-worker-"+key)/"native.json")
    evidence["adaptive"][key]=dict(first=read(WORK/"adaptive"/(key+".json")),warm=read(WORK/"adaptive-warm"/(key+".json")))
    evidence["packages"][key]=size(ROOT/"artifacts"/key)
    evidence["binary_hashes"][key]=digest(ROOT/"artifacts"/key/exe)
    for suffix in ("pet","settings","bubble"):
        for theme in ("light","dark"):
            src=WORK/"final-gpu"/("run-2-"+key)/(theme+"-"+suffix+".png")
            if src.exists():shutil.copyfile(src,OUT/(key+"-"+theme+"-"+suffix+".png"))
evidence["packages"]["shared_demonstration_python"]=size(ROOT/"artifacts/python")
evidence["binary_hashes"]["avalonia_managed_assembly"]=digest(ROOT/"artifacts/avalonia/Quote.Avalonia.dll")
evidence["binary_hashes"]["slint_vcruntime140"]=digest(ROOT/"artifacts/slint/vcruntime140.dll")
evidence["source_hashes"]={str(p.relative_to(ROOT)):digest(p) for p in ROOT.rglob("*") if p.is_file() and p.suffix in (".cs",".rs",".slint",".csproj",".toml") and not any(x in ("artifacts","obj","bin","target") for x in p.relative_to(ROOT).parts)}
(OUT/"measurements.json").write_text(json.dumps(evidence,ensure_ascii=False,indent=2),encoding="utf-8")

def metric(key,field,sub=None):
    return [r[field][sub] if sub else r[field] for r in data["runs"][key]]
def row(label,fn):return f"| {label} | {fn('slint')} | {fn('avalonia')} |"
table=["| Показатель | Rust + Slint | C# + Avalonia |","|---|---:|---:|"]
table += [row("Повторный запуск финального пакета, медиана / p95, мс",lambda k:f"{final_startups['summary'][k]['median']:.0f} / {final_startups['summary'][k]['p95']:.0f}"),
    row("Private Bytes: один маскот сразу после запуска, МБ",lambda k:interval([r['idle_pet']['private_bytes']['median']/1e6 for r in data['runs'][k]])),
    row("Working set: тот же режим, МБ",lambda k:interval([r['idle_pet']['working_set']['median']/1e6 for r in data['runs'][k]])),
    row("Private Bytes: открытые настройки, МБ",lambda k:interval([r['settings_idle']['private_bytes']['median']/1e6 for r in data['runs'][k]])),
    row("Private Bytes: после 200 циклов, настройки открыты, МБ",lambda k:interval([r['cycles']['after']['private_bytes']/1e6 for r in data['runs'][k]])),
    row("CPU: непрерывная анимация, % от четырёх CPU",lambda k:interval([r['idle_pet']['cpu_percent_allocated_cores'] for r in data['runs'][k]],2)),
    row("CPU: неподвижный маскот, %",lambda k:interval([r['static_pet']['cpu_percent_allocated_cores'] for r in data['runs'][k]],2)),
    row("CPU: 1 с анимации / 9 с покоя после прогрева, %",lambda k:f"{evidence['adaptive'][k]['warm']['metrics']['cpu_percent_allocated_cores']:.2f}"),
    row("Очередь UI при фоновой нагрузке, p95, мс",lambda k:interval([r['ui_queue_load_ms']['p95'] for r in data['runs'][k]],2)),
    row("Переоткрытие настроек до сигнала кадра*, p95, мс",lambda k:interval([r['cycles']['open_frame_ms']['p95'] for r in data['runs'][k]])),
    row("Отмена до завершения фонового процесса, мс",lambda k:interval([r['cancel_until_exit_ms'] for r in data['runs'][k]])),
    row("Автономная папка GUI без общего Python, МБ",lambda k:f"{evidence['packages'][k]/1e6:.1f}")]

report="""# Результаты сравнения оболочек Кавычки

Дата: 22 октября 2026 года. Это проверка двух прототипов, не аттестация полного приложения.

## Рекомендация

**Для следующей версии Кавычки рекомендую Rust + Slint.**

Это изменение предварительной рекомендации Avalonia. Проверка сняла основную
неопределённость Slint: прозрачный маскот, область клика, работа без активации и
фоновая жизнь реализуются небольшим Windows-адаптером без второго движка отрисовки.
Для небольшого постоянно работающего помощника существенны меньший автономный
пакет, меньшая начальная память и более быстрый повторный запуск.

**Rust не оказался быстрее во всём.** У Avalonia меньше CPU при непрерывной
анимации и меньше задержки очереди UI. Обе очереди остаются далеко ниже цели
100 мс. По этим данным нельзя объяснять лаги прежней Кавычки самим языком C#.
Если приоритетом станет постоянная интенсивная анимация и кратчайший перенос
существующего C#-кода, аргументы в пользу Avalonia усилятся.

## Условия

"""
report+=f"Windows 11, {machine['cpu']}, {machine['logical_cpus']} логических CPU, примерно {machine['ram_bytes']/1024**3:.0f} ГиБ RAM. GPU: {machine['gpu']}.\n\n"
report+="""Каждый прототип ограничен четырьмя логическими CPU; одинаковая фоновая задача
использует два потока SHA-256 и буфер 32 МиБ. Это **не эмуляция ноутбука 8 ГБ**.
NVIDIA в этом тесте используется для интерфейса, вычисления моделей не запускались.

- Slint 1.18.1 / Rust 1.99.0 MSVC, Release, thin LTO, Winit + FemtoVG.
- Avalonia 12.1.4 / .NET 10.0.13, Release, self-contained, ReadyToRun,
  штатный Windows backend. Отдельная системная установка .NET для пакета не нужна.
- Одинаковое изображение маскота, его размеры и интервал анимации 33 мс.
  Настройки содержат одинаковые функции, но не являются пиксельно идентичными.
- По 10 чередующихся запусков и два полных прогона с обратным порядком стеков.
  Серия из 10 запусков каждого стека повторена для окончательно собранных пакетов;
  таблица запуска использует именно её.
  В каждом прогоне 200 циклов создания/закрытия настроек и показа/подтверждения
  сообщения; по 400 циклов на стек в основной серии.
- Фазы памяти/CPU в основной серии — 30 секунд для маскота, настроек и фоновой
  нагрузки; дополнительные короткие фазы статического маскота и библиотеки.
- Процесс контроллера не входит в CPU GUI. Фоновый worker учтён отдельно в JSON,
  включая суммарную память дерева процессов.
- Системный файловый/драйверный кэш не очищался. Строка запуска относится к
  повторным запускам, а не гарантированно холодному старту после перезагрузки.

## Измерения

МБ — 1 000 000 байт. Диапазон отражает два основных прогона; память — медианы
сэмплов в фазе. CPU 100% означает суммарную мощность выделенных четырёх CPU.

"""
report+='\n'.join(table)+"\n\n"
report+="""*Точки сигнала кадра различаются: Slint уведомляет после рендеринга,
Avalonia — через RequestAnimationFrame и фоновую очередь Dispatcher.
Это не измерение «до фотона на экране». Эти значения нельзя использовать как
доказательство, что один интерфейс физически отображает действие в N раз быстрее.
Задержка очереди UI измерена отдельно одинаковым протоколом.

Самостоятельная GUI-папка сравнивается вместе с необходимым рантаймом. У Slint
проверены нативные импорты и рядом с EXE добавлена VCRUNTIME140.dll. Общий
embedded Python для демонстрационной нагрузки добавляет примерно 22,5 МБ к любому
варианту. Это не установщик полного приложения и не размер модели.

## Проверки поведения

- В основной серии оба прототипа завершились с кодом 0. Нет оставшихся workers.
- Отдельно пройдено по 30 циклов hide/show маскота: no-activate, toolwindow,
  topmost и область окна сохраняются после обработки событий.
- WindowFromPoint подтверждает: прозрачный угол разрешается в другое окно,
  непрозрачная часть — в маскота. Запрос WM_MOUSEACTIVATE собственному HWND
  возвращает MA_NOACTIVATE. Реальные клики/клавиши не внедрялись.
- Регион рассчитан по общему PNG с вертикальным припуском 3 px для покачивания.
  Это проверка прозрачных углов и контура, не доказательство идеального hit test
  каждого полупрозрачного пикселя во всех фазах.
- Дополнительно проверено завершение главного процесса при работающем worker:
  дочерний процесс завершается. Проверка повторена с отдельным embedded Python,
  которым пользуются готовые START-файлы.
- Библиотека содержит 10 000 строк; отображаются 50. После прогрева не наблюдается
  роста GDI/USER-ресурсов на каждый цикл. Колебания Private Bytes/handles сами
  по себе не доказывают ни утечку, ни её отсутствие за сутки.
- Просмотрены снимки обоих интерфейсов в светлой и тёмной теме и PNG маскота.

## Что показал режим спокойной работы

Непрерывные ~30 FPS не укладываются в предложенный CPU-бюджет 0,5% на четырёх CPU
ни у одного стека. Нельзя выдавать этот режим за дешёвый фон только потому, что
маскот небольшой. У неподвижного маскота расход значительно ниже.

Внешний контроллер дополнительно проверил политику «1 секунда анимации / 9 секунд
покоя»: два 60-секундных измерения с разным порядком, второе после 15 с прогрева.
В прогретом тесте оба варианта ниже 0,5%. Первый короткий тест Avalonia был выше
бюджета; поэтому одно раннее измерение не использовано как окончательный вывод.
Это эксперимент с политикой, а не уже реализованный адаптивный режим приложения.
Установленные прототипы по умолчанию показывают непрерывную анимацию для стенда.

## Особенности, обнаруженные прототипированием

1. Winit после создания окна может перезаписать extended styles. В обоих
   Windows-адаптерах закреплена политика no-activate/toolwindow через subclass
   WM_STYLECHANGING и WM_MOUSEACTIVATE. Это локальный компонент, не новый renderer.
2. Slint должен работать через run_event_loop_until_quit: скрытие последнего окна
   не должно завершать фоновое приложение. Эта правка сделана после основной
   серии и отдельно проверена 30 hide/show-циклами и повторным полным сценарием
   с 30 циклами настроек. Основная серия не скрывала последнего окна; новый
   короткий прогон подтвердил прежний диапазон памяти/CPU/задержек. Хэши основной
   и окончательной сборки сохранены раздельно.
3. Для настройки создания окна Slint используется закреплённый feature
   unstable-winit-030. Это риск совместимости обновлений API, который нужно
   закрывать pinning и повторением нативных проверок при обновлении Slint.

## Границы доказательств

Не проверены физический ноутбук 8 ГБ/Intel iGPU, реальные 125–200% DPI и перенос
между мониторами с разным DPI, эксклюзивные fullscreen-игры, физический сон/вход
Windows, доступность с экранным диктором, реальное изменение системной темы,
суточный прогон, 10-минутный CPU-бюджет и нагрузка настоящих LLM/TTS/CUDA.
Все нативные проверки выполнены на текущем мониторе при 96 DPI.

Расписание, хранение/миграции, обновления, импорт пресетов и реальное аудио в
прототипах отсутствуют. Их критерии относятся к будущему приложению и этим
стендом не объявляются пройденными. Mac/Linux не собирались.

## Следствие для архитектуры

При принятии рекомендации: ядро и координация задач на Rust; интерфейс Slint;
отдельный Windows-адаптер; единый владелец SQLite/настроек; генерация через
изолированный Python-worker. Не переносить текущую связность MainForm в Rust.
Сохранить общий формат данных и план миграции старых настроек/голосов/истории.
До полной разработки проверить оставшиеся критические пункты на целевом железе
и спроектировать режимы анимации/освобождения ресурсов. Полное приложение сейчас
не переписывалось, публикации и Git push не выполнялись.

## Как посмотреть

- [START-SLINT.bat](START-SLINT.bat)
- [START-AVALONIA.bat](START-AVALONIA.bat)
- [Все измерения и хэши](artifacts/results/measurements.json)
- [Методика и повторение](README.md)

| Rust + Slint | C# + Avalonia |
|---|---|
| ![Slint](artifacts/results/slint-dark-settings.png) | ![Avalonia](artifacts/results/avalonia-dark-settings.png) |

Это минимальные интерфейсы для измерений, не предложение финального дизайна.

Документация: [Slint](https://docs.slint.dev/latest/docs/slint/),
[Winit integration](https://docs.rs/i-slint-backend-winit/latest/i_slint_backend_winit/trait.WinitWindowAccessor.html),
[Avalonia Windows](https://docs.avaloniaui.net/docs/platform-specific-guides/windows/).
"""
(ROOT/"RESULTS.md").write_text(report,encoding="utf-8")
print("PASS collected report and measurement evidence")
