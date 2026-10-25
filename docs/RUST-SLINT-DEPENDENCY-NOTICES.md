# Notices выбранной Windows-среды — 24 октября 2026

Дополнение к [проверке 0.1.22](RUST-SLINT-CANDIDATE-0.1.22.md). Это реестр происхождения и целостности текстов для обследованной конфигурации; не заключение о праве распространения всего будущего продукта. Установленные файлы не менялись, исходные архивы не выполнялись.

[Машиночитаемый снимок](dependency-notices/selected-environment-20261024.json): 94 базовых Python-дистрибутива и 37 записей Cosy-слоя, 251 локальный notice с проверенным SHA-256 wheel RECORD. Одинаковое имя в разных слоях не означает один и тот же установленный вариант.

Шесть дистрибутивов имели только декларацию лицензии без локального текста. Дополнительные тексты сохранены отдельно от wheels:

| Проект / версия | Сохранённый текст | Первичный источник |
|---|---|---|
| flatbuffers 25.12.19 | [LICENSE](dependency-notices/flatbuffers-25.12.19.txt) | [Официальный tag commit 7e163021](https://raw.githubusercontent.com/google/flatbuffers/7e163021e59cca4f8e1e35a7c828b5c6b7915953/LICENSE), [точная версия PyPI](https://pypi.org/project/flatbuffers/25.12.19/) |
| gradio_client 2.5.0 | [LICENSE](dependency-notices/gradio_client-2.5.0.txt) | [Официальный tag commit 37a5f7fb](https://raw.githubusercontent.com/gradio-app/gradio/37a5f7fb7922c776e64fd2c6c7a7454a4ab5ff50/LICENSE), [точная версия PyPI](https://pypi.org/project/gradio-client/2.5.0/) |
| tokenizers 0.22.2 | [LICENSE](dependency-notices/tokenizers-0.22.2.txt) | LICENSE в sdist [точной версии PyPI](https://pypi.org/project/tokenizers/0.22.2/); полный SHA архива и путь файла — в снимке |
| tokenizers 0.21.4 | [LICENSE](dependency-notices/tokenizers-0.21.4.txt) | LICENSE в sdist [точной версии PyPI](https://pypi.org/project/tokenizers/0.21.4/); полный SHA архива и путь файла — в снимке |
| antlr4-python3-runtime 4.9.3 | [LICENSE](dependency-notices/antlr4-python3-runtime-4.9.3.txt) | [Официальный tag commit e4c1a74c](https://raw.githubusercontent.com/antlr/antlr4/e4c1a74c66bd5290364ea2b36c97cd724b247357/LICENSE.txt), [точная версия PyPI](https://pypi.org/project/antlr4-python3-runtime/4.9.3/) |
| loguru 0.7.3 | [LICENSE](dependency-notices/loguru-0.7.3.txt) | [Официальный tag commit ae3bfd1b](https://raw.githubusercontent.com/Delgan/loguru/ae3bfd1b85b6b4a3db535f69b975687c79498be4/LICENSE), [точная версия PyPI](https://pypi.org/project/loguru/0.7.3/) |

Дополнительно: [основной LICENSE llama.cpp b11719](dependency-notices/llama.cpp-b11719.txt), источник и точный commit — в том же снимке. В обследованной папке llama был отдельный `LICENSE-LLVM-OpenMP`; он не заменяет основной текст проекта.

SHA каждого сохранённого текста и его URL/commit либо SHA целого PyPI-архива зафиксированы в JSON. Отсутствие файла в wheel не означает отсутствие лицензии проекта; получение текста из исходников не доказывает тождественность всех исходников и бинарника. Пакеты не переустанавливались и их RECORD не переписывался.

Rust-installer содержит свои 495 notice-файлов для 263 выбранных пакетов. Python, llama, CUDA и веса в этот installer не входят. Этот документ не закрывает модельные условия, все CUDA/native redistributables, условия Slint, подпись/SmartScreen или проверку иных версий и платформ. Эти выпускные условия P7 сохраняются; положительные результаты проверки файлов не подменяют их.
