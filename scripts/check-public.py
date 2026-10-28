"""Check source files selected by Git, without reading ignored user storage."""
import json
import re
import subprocess
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
ALLOWED_ROOTS = {".github", "assets", "backend", "docs", "scripts", "src", "tests", "next", "prototypes"}
ALLOWED_TOP = {".gitignore", ".gitattributes", "README.md", "START.bat", "LICENSE"}
ALLOWED_EXTENSIONS = {".cs", ".py", ".json", ".ps1", ".bat", ".md", ".yml", ".yaml", ".txt", ".rs", ".toml", ".lock", ".slint", ".svg", ".csproj", ".config", ".manifest"}
PRESET_SOURCE_MODULES = {
    "next/crates/storage/src/presets/packs.rs",
    "next/crates/storage/src/presets/packs_tests.rs",
}
SECRET_PATTERNS = (
    re.compile(rb"\bgh[pousr]_[A-Za-z0-9]{30,}\b"),
    re.compile(rb"\bgithub_pat_[A-Za-z0-9_]{40,}\b"),
    re.compile(rb"\bsk-(?:proj-)?[A-Za-z0-9_-]{24,}\b"),
    re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    re.compile(rb'''(?i)(?:api[_-]?key|access[_-]?token|secret[_-]?key)["']?\s*[:=]\s*["'][A-Za-z0-9_\-]{24,}["']'''),
)


def check():
    output = subprocess.check_output(["git", "-C", str(ROOT), "ls-files", "--cached", "--others", "--exclude-standard", "-z"])
    names = {name.decode("utf-8") for name in output.split(b"\0") if name}
    if "src/Kavychka.cs" not in names or "assets/voice/default-en.wav" not in names:
        raise ValueError("The public source selection is incomplete")
    issues = []
    for name in sorted(names):
        path = PurePosixPath(name)
        if name not in ALLOWED_TOP and (len(path.parts) < 2 or path.parts[0] not in ALLOWED_ROOTS):
            issues.append(name + ": unexpected source path")
            continue
        if any(part in {"presets", "runtime", "models", "__pycache__", "user-legacy", "data"} for part in path.parts) and name not in PRESET_SOURCE_MODULES:
            issues.append(name + ": user data or runtime in source")
            continue
        if path.suffix.lower() == ".wav":
            if name != "assets/voice/default-en.wav":
                issues.append(name + ": non-default audio reference")
            continue
        if path.suffix.lower() == ".png":
            if not name.startswith("docs/design/") and name != "prototypes/stack-comparison/common/mascot.png":
                issues.append(name + ": unexpected image")
            continue
        if name not in ALLOWED_TOP and path.name != ".gitignore" and path.suffix.lower() not in ALLOWED_EXTENSIONS and name != "next/crates/storage/src/legacy_fixture.sql":
            issues.append(name + ": unexpected source file type")
            continue
        if name.startswith("assets/voice/") and name != "assets/voice/profile.json":
            issues.append(name + ": non-default voice metadata")
            continue
        data = (ROOT / name).read_bytes()
        if any(pattern.search(data) for pattern in SECRET_PATTERNS):
            issues.append(name + ": possible credential; value suppressed")
    for name in ("assets/character.json", "assets/voice/profile.json"):
        if not json.loads((ROOT / name).read_text(encoding="utf-8-sig")).get("public_default"):
            issues.append(name + ": personal default")
    if issues:
        raise ValueError("Public source check failed:\n" + "\n".join(issues))
    print("PASS public source selection: %d files; no user storage, private references or recognized credential patterns" % len(names))


if __name__ == "__main__":
    check()
