"""Stop the real loopback service, including Windows venv launchers."""
import os
import subprocess
import urllib.error
import urllib.request


def stop_server(process, address, token):
    try:
        if address and process.poll() is None:
            request = urllib.request.Request(address + "/shutdown", data=b"{}",
                                             headers={"Authorization": "Bearer " + token})
            try:
                with urllib.request.urlopen(request, timeout=5) as response:
                    response.read()
            except (OSError, urllib.error.URLError):
                pass
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            # Killing only python.exe can leave a Windows venv redirector's
            # child holding its database. Restrict cleanup to this exact PID.
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                               capture_output=True, check=False, timeout=10)
            else:
                process.terminate()
            process.wait(timeout=10)
    finally:
        process.stdout.close()
        process.stderr.close()
