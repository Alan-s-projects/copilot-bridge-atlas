"""Opt-in installed-Codex catalog check, using an isolated CODEX_HOME."""

import argparse
from contextlib import contextmanager, suppress
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import shutil
import stat
import subprocess
import tempfile
import threading
import time
import uuid
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from urllib.parse import urlparse


def expected_instructions(entry):
    messages = entry.get("model_messages") or {}
    text = messages.get("instructions_template")
    if text is None:
        text = entry.get("base_instructions")
    if not isinstance(text, str):
        raise ValueError(f"Missing instructions for {entry['slug']}")
    if "base_instructions" in entry and entry["base_instructions"] != text:
        raise ValueError(f"Conflicting instruction fields for {entry['slug']}")
    return text


def verify_instructions(requests, entries):
    known_prompts = {expected_instructions(entry) for entry in entries.values()}
    for request in requests:
        expected = expected_instructions(entries[request["model"]])
        if request["instructions"] not in known_prompts:
            raise AssertionError("Base instructions are not an unchanged catalog prompt")
        switches = request.get("model_switch_messages", [])
        # Codex retains the initial thread base and appends the selected model's
        # literal instructions in a developer message on subsequent model switches.
        if switches:
            if expected not in switches[-1]:
                raise AssertionError(f"Latest model-switch instructions differ for {request['model']}")
        elif request["instructions"] != expected:
            raise AssertionError(f"Effective base instructions differ for {request['model']}")
        request["instruction_delivery"] = "model_switch" if switches else "base"
        request["effective_instruction_sha256"] = hashlib.sha256(expected.encode("utf-8")).hexdigest()


@contextmanager
def temporary_workspace():
    # Windows mkdtemp uses a private ACL that sandbox accounts cannot access.
    # Ordinary mkdir inherits the parent ACL while the sandbox still bounds writes.
    parent = Path(tempfile.gettempdir()).resolve()
    root = parent / f"atlas-catalog-check-{uuid.uuid4().hex}"
    root.mkdir()
    try:
        yield str(root)
    finally:
        resolved = root.resolve()
        if resolved.parent != parent or not resolved.name.startswith("atlas-catalog-check-"):
            raise RuntimeError("Refusing cleanup outside the validation workspace")
        def remove_readonly(function, path, error):
            candidate = Path(path).resolve()
            if not candidate.is_relative_to(resolved) or not candidate.is_file():
                raise error
            os.chmod(candidate, stat.S_IWRITE | stat.S_IREAD)
            function(path)
        shutil.rmtree(resolved, onexc=remove_readonly)


class CaptureServer(ThreadingHTTPServer):
    def __init__(self, upstream=None):
        super().__init__(("127.0.0.1", 0), CaptureHandler)
        self.upstream = upstream
        self.requests = []
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=5)


class CaptureHandler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        data = json.loads(body)
        instructions = data.get("instructions", "")
        self.server.requests.append({
            "model": data["model"],
            "stream": data.get("stream"),
            "instruction_chars": len(instructions),
            "instruction_words": len(instructions.split()),
            "instruction_sha256": hashlib.sha256(instructions.encode("utf-8")).hexdigest(),
            "input_chars": len(json.dumps(data.get("input", []))),
            "tool_chars": len(json.dumps(data.get("tools", []))),
            "instructions": instructions,
            "model_switch_messages": [
                content.get("text", "") for item in data.get("input", [])
                if isinstance(item, dict) and item.get("role") == "developer"
                for content in item.get("content", [])
                if isinstance(content, dict) and "<model_switch>" in content.get("text", "")
            ],
            "permission_context": [
                content.get("text", "") for item in data.get("input", [])
                if isinstance(item, dict) and item.get("role") == "developer"
                for content in item.get("content", [])
                if isinstance(content, dict) and "<permissions" in content.get("text", "")
            ],
        })
        if self.server.upstream:
            request = Request(
                self.server.upstream.rstrip("/") + "/responses", body,
                {"Content-Type": "application/json", "Accept": "text/event-stream"},
            )
            try:
                with urlopen(request, timeout=120) as response:
                    self.send_response(response.status)
                    self.send_header("Content-Type", response.headers.get("Content-Type"))
                    self.end_headers()
                    while chunk := response.read(1024):
                        self.wfile.write(chunk)
                        self.wfile.flush()
            except HTTPError as error:
                self.send_response(error.code)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(error.read())
            return
        message = {
            "type": "message", "id": "msg_atlas_check", "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "ATLAS_CATALOG_OK", "annotations": []}],
        }
        response = {
            "id": "resp_atlas_check", "object": "response", "status": "completed",
            "model": data["model"], "output": [message],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2},
        }
        events = [
            {"type": "response.created", "response": {**response, "status": "in_progress", "output": []}},
            {"type": "response.output_item.added", "output_index": 0, "item": {**message, "content": []}},
            {"type": "response.output_text.delta", "output_index": 0, "content_index": 0,
             "item_id": message["id"], "delta": "ATLAS_CATALOG_OK"},
            {"type": "response.output_item.done", "output_index": 0, "item": message},
            {"type": "response.completed", "response": response},
        ]
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for event in events:
            self.wfile.write(("data: " + json.dumps(event) + "\n\n").encode())
            self.wfile.flush()


class AppServer:
    def __init__(self, binary, catalog, home, cwd, base_url=None):
        config = [
            "-c", f"model_catalog_json={json.dumps(str(catalog.resolve()))}",
            "-c", 'model_provider="atlas_catalog_check"',
            "-c", 'model_providers.atlas_catalog_check.name="Atlas catalog check"',
            "-c", 'model_providers.atlas_catalog_check.wire_api="responses"',
            "-c", 'model_providers.atlas_catalog_check.requires_openai_auth=false',
            "-c", f"model_providers.atlas_catalog_check.base_url={json.dumps(base_url or 'http://127.0.0.1:1/v1')}",
            "-c", "analytics.enabled=false",
            "-c", "feedback.enabled=false",
            "-c", 'windows.sandbox="unelevated"',
        ]
        self.process = subprocess.Popen(
            [str(binary), *config, "app-server", "--listen", "stdio://"],
            cwd=cwd,
            env={**os.environ, "CODEX_HOME": str(home)},
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8",
        )
        self.events = queue.Queue()
        self.errors = []
        self.next_id = 0
        self.pending = []
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self._read_errors, daemon=True).start()
        try:
            self.request("initialize", {
                "clientInfo": {"name": "atlas_catalog_check", "version": "1.0"},
                "capabilities": {"experimentalApi": True},
            })
            self.send({"method": "initialized"})
        except Exception:
            self.close()
            raise

    def _read(self):
        for line in self.process.stdout:
            try:
                self.events.put(json.loads(line))
            except json.JSONDecodeError:
                self.errors.append(line.rstrip())
        self.events.put(None)

    def _read_errors(self):
        for line in self.process.stderr:
            self.errors.append(line.rstrip())

    def send(self, value):
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()

    def receive(self, timeout=30):
        item = self.events.get(timeout=timeout)
        if item is None:
            raise RuntimeError("Codex exited: " + "\n".join(self.errors[-8:]))
        return item

    def request(self, method, params):
        self.next_id += 1
        request_id = self.next_id
        self.send({"id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            message = self.receive(timeout=max(0.1, deadline - time.monotonic()))
            if message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError(json.dumps(message["error"]))
                return message["result"]
            self.pending.append(message)
        raise TimeoutError(method)

    def turn(self, thread_id, model, prompt):
        self.request("turn/start", {
            "threadId": thread_id, "model": model, "effort": "low",
            "input": [{"type": "text", "text": prompt}],
        })
        events, self.pending = self.pending, []
        deadline = time.monotonic() + 180
        while not any(event.get("method") == "turn/completed" for event in events):
            event = self.receive(timeout=max(0.1, deadline - time.monotonic()))
            events.append(event)
            if "id" in event and "method" in event:
                raise RuntimeError("Unexpected approval/tool request: " + event["method"])
            if time.monotonic() >= deadline:
                raise TimeoutError("turn/completed")
        completed = next(event["params"]["turn"] for event in events
                         if event.get("method") == "turn/completed")
        assert completed["status"] == "completed", completed.get("error")
        return events

    def close(self):
        with suppress(OSError, ValueError):
            self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=10)
        self.process.stdout.close()
        self.process.stderr.close()


def prepare_settings(source, output):
    source_models = json.loads(source.read_text(encoding="utf-8"))["models"]
    settings = {"modelCatalog": {"models": [{
        "model": row["slug"],
        "displayName": row["display_name"],
        "contextWindow": row.get("context_window"),
        "maxContextWindow": row.get("max_context_window"),
        "supportsParallelToolCalls": row.get("supports_parallel_tool_calls"),
        "inputModalities": row.get("input_modalities"),
        "supportedReasoningLevels": [
            level["effort"] for level in row.get("supported_reasoning_levels", [])
            if level["effort"] != "ultra"
        ],
    } for row in source_models]}}
    output.write_text(json.dumps(settings, indent=2), encoding="utf-8")
    print(json.dumps({"fixture_models": len(source_models), "settings": str(output)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", type=Path)
    parser.add_argument("--catalog", type=Path)
    parser.add_argument("--source-catalog", type=Path)
    parser.add_argument("--prepare-settings", type=Path)
    parser.add_argument("--capture", action="store_true",
                        help="Capture synthetic Responses payloads with a local mock server")
    parser.add_argument("--live", help="Opt-in local Atlas base URL; consumes upstream tokens")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--models", nargs="+", default=["gpt-6-astra", "gpt-6-luna", "gpt-6-sol"],
                        help="Models to validate, in same-session switching order")
    parser.add_argument("--fresh-threads", action="store_true",
                        help="Also usable to check each model's initial base prompt separately")
    args = parser.parse_args()
    if args.prepare_settings:
        prepare_settings(args.source_catalog, args.prepare_settings)
        return
    if not args.codex or not args.catalog:
        parser.error("--codex and --catalog are required for catalog validation")
    if args.live:
        upstream = urlparse(args.live)
        if upstream.scheme != "http" or upstream.hostname not in ("127.0.0.1", "localhost", "::1") or upstream.username or upstream.password:
            parser.error("--live must be a credential-free loopback Atlas HTTP URL")
    entries = {model["slug"]: model for model in json.loads(args.catalog.read_text(encoding="utf-8"))["models"]}
    for entry in entries.values():
        expected_instructions(entry)
    with temporary_workspace() as directory:
        root = Path(directory)
        home = root / "home"
        home.mkdir()
        server = None
        capture = CaptureServer(args.live) if args.capture or args.live else None
        try:
            base_url = f"http://127.0.0.1:{capture.server_port}/v1" if capture else None
            server = AppServer(args.codex, args.catalog, home, root, base_url)
            result = server.request("model/list", {"includeHidden": True})
            ids = {model["id"] for model in result["data"]}
            expected = set(entries)
            assert expected <= ids, f"Missing catalog models: {sorted(expected - ids)}"
            report = {"catalog_loaded": True, "models": sorted(expected)}
            if capture:
                models = args.models
                assert set(models) <= expected
                started = server.request("thread/start", {
                    "model": models[0], "cwd": str(root), "ephemeral": True,
                    "approvalPolicy": "never", "sandbox": "workspace-write",
                })
                thread = started["thread"]["id"]
                report["permissions"] = {key: value for key, value in started.items()
                                         if key in ("sandbox", "approvalPolicy", "permissions")}
                fixture = root / "counter.txt"
                if args.live:
                    fixture.write_text("value=0\n", encoding="utf-8")
                report["turns"] = []
                for index, model in enumerate(models, start=1):
                    if args.fresh_threads and index > 1:
                        thread = server.request("thread/start", {
                            "model": model, "cwd": str(root), "ephemeral": True,
                            "approvalPolicy": "never", "sandbox": "workspace-write",
                        })["thread"]["id"]
                    start = len(capture.requests)
                    prompt = (
                        f"This is a synthetic catalog validation in an isolated temporary directory. "
                        f"Read counter.txt with a shell tool, use apply_patch to change value={index-1} "
                        f"to value={index}, then verify the exact file contents with a shell tool. "
                        f"Only edit counter.txt. Do not use external services. Reply ATLAS_STEP_{index}."
                    ) if args.live else "Reply exactly ATLAS_CATALOG_OK. Do not call tools."
                    events = server.turn(thread, model, prompt)
                    calls = capture.requests[start:]
                    assert calls and all(call["model"] == model for call in calls), model
                    assert all(call["stream"] for call in calls), "Expected streaming Responses"
                    completed_items = [
                        event["params"]["item"] for event in events
                        if event.get("method") == "item/completed"
                    ]
                    text = "\n".join(item.get("text", "") for item in completed_items)
                    if args.live:
                        assert fixture.read_text().strip() == f"value={index}", {
                            "text": text, "permissions": report["permissions"],
                            "context": calls[0]["permission_context"],
                        }
                        assert any(item["type"] == "fileChange" for item in completed_items), completed_items
                        assert any(item["type"] == "commandExecution" for item in completed_items), completed_items
                        for item in completed_items:
                            if item["type"] == "commandExecution":
                                assert item.get("exitCode") == 0, item
                            if item["type"] in ("commandExecution", "fileChange"):
                                assert item.get("status") == "completed", item
                        assert f"ATLAS_STEP_{index}" in text, text
                    else:
                        assert "ATLAS_CATALOG_OK" in text, text
                    report["turns"].append({
                        "model": model, "completed": True, "requests": len(calls),
                        "item_types": [item["type"] for item in completed_items],
                        "streamed_deltas": sum(event.get("method") == "item/agentMessage/delta" for event in events),
                        "context_windows": [
                            event["params"]["tokenUsage"].get("modelContextWindow")
                            for event in events if event.get("method") == "thread/tokenUsage/updated"
                        ],
                    })
                verify_instructions(capture.requests, entries)
                report["thread_mode"] = "fresh" if args.fresh_threads else "switching"
                report["payloads"] = [{key: value for key, value in request.items()
                                      if key not in ("instructions", "permission_context", "model_switch_messages")}
                                      for request in capture.requests]
            if args.report:
                args.report.write_text(json.dumps(report, indent=2), encoding="utf-8")
            print(json.dumps(report))
        finally:
            if server:
                server.close()
            if capture:
                capture.close()


if __name__ == "__main__":
    main()
