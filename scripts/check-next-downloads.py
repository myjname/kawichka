"""Read-only opt-in HEAD qualification of pinned component URLs.
Does not download weights and does not replace install-time SHA verification.
"""
import argparse
import hashlib
import io
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import urllib.error
import urllib.parse
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]


class RedirectMetadata(urllib.request.HTTPRedirectHandler):
    def __init__(self):
        super().__init__()
        self.linked_size = self.linked_sha = None

    def redirect_request(self, request, response, code, message, headers, newurl):
        self.linked_size = headers.get("X-Linked-Size", self.linked_size)
        self.linked_sha = headers.get("X-Linked-ETag", self.linked_sha)
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise ValueError("Refuse redirect to an insecure URL")
        return super().redirect_request(request, response, code, message, headers, newurl)


def files(value, prefix=""):
    if isinstance(value, dict):
        if {"url", "size", "sha256"} <= value.keys():
            yield prefix, value
        else:
            for name, child in value.items():
                yield from files(child, prefix + "/" + name)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from files(child, prefix + "/" + str(index))


def check(item, small_sources=False):
    name, spec = item
    parsed = urllib.parse.urlsplit(spec["url"])
    record = {"component": name, "url": spec["url"], "expected_size": spec["size"], "expected_sha256": spec["sha256"]}
    try:
        if parsed.scheme != "https" or parsed.username or parsed.password:
            raise ValueError("Public HTTPS URL required")
        redirects = RedirectMetadata()
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), redirects)
        request = urllib.request.Request(spec["url"], method="HEAD", headers={"User-Agent": "Kavychka-Catalog-Verification/1.0", "Accept-Encoding": "identity"})
        with opener.open(request, timeout=25) as response:
            raw_size = response.headers.get("X-Linked-Size") or redirects.linked_size or response.headers.get("Content-Length")
            size = int(raw_size) if raw_size else None
            sha = (response.headers.get("X-Linked-ETag") or redirects.linked_sha or "").strip('"')
            sha = sha if re.fullmatch("[0-9a-fA-F]{64}", sha) else None
            record.update(http_status=response.status, remote_size=size, remote_sha256=sha, payload_bytes_read=0)
        if size is None and small_sources and name.startswith("/cosy_runtime/files/") and parsed.hostname == "codeload.github.com" and 0 < spec["size"] <= 16 * 1024 * 1024:
            # Explicit opt-in for three small SOURCE archives, never weights.
            request = urllib.request.Request(spec["url"], headers={"User-Agent": "Kavychka-Catalog-Verification/1.0", "Accept-Encoding": "identity"})
            with opener.open(request, timeout=30) as response:
                body = response.read(spec["size"] + 1)
            size, sha = len(body), hashlib.sha256(body).hexdigest()
            record.update(remote_size=size, remote_sha256=sha, payload_bytes_read=size, verification="source_archive_full_sha256")
            if size == spec["size"] and sha == spec["sha256"]:
                with zipfile.ZipFile(io.BytesIO(body)) as archive:
                    record["source_license_files"] = [{"path": entry.filename, "sha256": hashlib.sha256(archive.read(entry)).hexdigest()} for entry in archive.infolist() if not entry.is_dir() and entry.file_size <= 1024 * 1024 and Path(entry.filename).name.upper() in {"LICENSE", "LICENSE.TXT", "LICENSE.MD", "NOTICE", "NOTICE.TXT"}][:20]
        if size is not None and size != spec["size"]:
            record["status"] = "size_mismatch"
        elif sha is not None and sha.lower() != spec["sha256"].lower():
            record["status"] = "sha_metadata_mismatch"
        else:
            record["status"] = "ok" if size is not None else "unverified_size"
    except urllib.error.HTTPError as error:
        record.update(status="http_error", http_status=error.code)
    except (OSError, ValueError, urllib.error.URLError, zipfile.BadZipFile) as error:
        # Error class only: do not export signed redirect URLs or proxy details.
        record.update(status="request_failed", error_type=type(error).__name__)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", action="store_true", help="Explicitly allow read-only HTTPS HEAD checks")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--component-prefix", default="", help="Check only this manifest path prefix")
    parser.add_argument("--small-source-archives", action="store_true", help="If HEAD lacks size, GET only pinned cosy_runtime source ZIPs <=16 MiB; never models")
    args = parser.parse_args()
    if not args.network:
        parser.error("Use --network to authorize HEAD requests; no model payload is downloaded")
    if args.output.exists():
        parser.error("Choose a fresh report path")
    manifest = json.loads((ROOT / "assets/components.json").read_text(encoding="utf-8-sig"))
    with ThreadPoolExecutor(max_workers=4) as pool:
        selected = [item for item in files(manifest) if item[0].startswith(args.component_prefix)]
        if not selected:
            parser.error("No matching manifest files")
        records = list(pool.map(lambda item: check(item, args.small_source_archives), selected))
    counts = {status: sum(row["status"] == status for row in records) for status in sorted({row["status"] for row in records})}
    result = {"format": 1, "passed": all(row["status"] == "ok" for row in records), "files": len(records), "counts": counts, "model_payload_downloaded": False, "source_archives_downloaded":args.small_source_archives,"component_prefix":args.component_prefix,"scope": "Declared pinned manifest URLs: HEAD metadata; optional bounded full SHA check of small cosy_runtime SOURCE ZIPs only. Not a provider/runtime/quality qualification. Model SHA must still be checked during installation.", "records": records}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=True, indent=2), encoding="utf-8")
    print(json.dumps({name: value for name, value in result.items() if name != "records"}))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
