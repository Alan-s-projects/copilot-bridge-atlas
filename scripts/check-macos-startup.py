"""Launch the packaged Mac app with disposable data and check its local proxy."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import build_opener, ProxyHandler


def wait_for(process, predicate, label, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Atlas exited before {label}, status {process.returncode}.")
        try:
            result = predicate()
            if result:
                return result
        except (FileNotFoundError, sqlite3.OperationalError, HTTPError, URLError):
            pass
        time.sleep(0.25)
    raise RuntimeError(f"Atlas did not reach {label} within {timeout} seconds.")


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


@contextmanager
def preserve_diagnostics(directory, report_path):
    try:
        yield
    except Exception as error:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(
            json.dumps({"startup_passed": False, "error": str(error)}, indent=2) + "\n",
            encoding="utf-8",
        )
        raise
    finally:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        logs = {
            "macos-process.log": directory / "process.log",
            "macos-app.log": directory / ".copilot-bridge-atlas/logs/copilot-bridge-atlas.log",
        }
        for name, source in logs.items():
            if source.is_file():
                shutil.copyfile(source, report_path.parent / name)


def check_startup(app, report_path):
    executable = app.resolve() / "Contents/MacOS/copilot-bridge-atlas"
    opener = build_opener(ProxyHandler({}))
    with tempfile.TemporaryDirectory(prefix="atlas-startup-check-") as directory, preserve_diagnostics(
        Path(directory), report_path
    ):
        home = Path(directory)
        client = home / ".codex"
        client.mkdir()
        sentinels = {
            "config.toml": "# preserved validation file\nmodel_provider = 'openai'\n",
            "auth.json": "{}\n",
            "AGENTS.md": "Preserve this validation instruction file.\n",
        }
        for name, text in sentinels.items():
            (client / name).write_text(text, encoding="utf-8")
        before = {name: digest(client / name) for name in sentinels}
        env = {**os.environ, "COPILOT_BRIDGE_ATLAS_TEST_HOME": str(home)}
        data = home / ".copilot-bridge-atlas"
        log = data / "logs/copilot-bridge-atlas.log"

        with (home / "process.log").open("w", encoding="utf-8") as output:
            first = subprocess.Popen([str(executable)], env=env, stdout=output, stderr=output)
            try:
                wait_for(
                    first,
                    lambda: log.exists() and "Main page loaded;" in log.read_text(encoding="utf-8"),
                    "initial WebView page load",
                )
            finally:
                stop(first)
            with socket.socket() as probe:
                probe.bind(("127.0.0.1", 0))
                port = probe.getsockname()[1]
            with sqlite3.connect(data / "copilot-bridge-atlas.db") as db:
                provider_id, raw = db.execute(
                    "SELECT id, settings_config FROM providers WHERE app_type='codex'"
                ).fetchone()
                settings = json.loads(raw)
                settings["modelCatalog"] = {
                    "models": [{"model": "atlas-validation-model", "contextWindow": 16000}]
                }
                db.execute(
                    "UPDATE providers SET settings_config=? WHERE id=? AND app_type='codex'",
                    (json.dumps(settings), provider_id),
                )
                db.execute(
                    "UPDATE proxy_config SET proxy_enabled=1, listen_address='127.0.0.1', listen_port=? WHERE app_type='codex'",
                    (port,),
                )
            second = subprocess.Popen([str(executable)], env=env, stdout=output, stderr=output)
            try:
                def get(path):
                    with opener.open(f"http://127.0.0.1:{port}{path}", timeout=2) as response:
                        if response.status != 200:
                            raise RuntimeError(f"{path} did not return HTTP 200.")
                        return json.load(response)

                health = wait_for(
                    second,
                    lambda: get("/health"),
                    "local proxy health check",
                )
                if health.get("status") != "healthy":
                    raise RuntimeError("The proxy health response was not healthy.")
                models = get("/v1/models")
                if [model["slug"] for model in models["models"]] != ["atlas-validation-model"]:
                    raise RuntimeError("The proxy did not serve the saved validation model catalog.")
                catalog = json.loads((data / "copilot-model-catalog.json").read_text(encoding="utf-8"))
                if catalog["models"][0]["slug"] != "atlas-validation-model":
                    raise RuntimeError("Startup did not write Atlas's generated catalog.")
                if {name: digest(client / name) for name in sentinels} != before:
                    raise RuntimeError("Startup changed a Codex validation file.")
                report = {
                    "webview_page_loaded": True,
                    "proxy_health_http_status": 200,
                    "model_discovery_http_status": 200,
                    "generated_catalog_written": True,
                    "codex_files_preserved": True,
                    "live_authentication_used": False,
                }
                report_path.parent.mkdir(parents=True, exist_ok=True)
                report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
                print(json.dumps(report, indent=2))
            finally:
                stop(second)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("Run the packaged startup check on macOS.")
    check_startup(args.app, args.report)


if __name__ == "__main__":
    main()
