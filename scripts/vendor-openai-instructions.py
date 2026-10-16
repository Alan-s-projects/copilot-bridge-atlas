"""Extract unchanged GPT-6 instruction objects from the user-selected Codex commit."""

import argparse
import hashlib
import json
from pathlib import Path
from urllib.request import urlopen

COMMIT = "83086188ae0614bd882818ff1aa23f3194962012"  # rust-v0.163.0
SOURCE_PATH = "codex-rs/models-manager/models.json"
BASE_URL = f"https://raw.githubusercontent.com/openai/codex/{COMMIT}"
PROFILES = ("gpt-6-astra", "gpt-6-luna", "gpt-6-sol", "gpt-6.1-sol")
ROOT = Path(__file__).resolve().parents[1]
SNAPSHOT = ROOT / "src-tauri/src/resources/openai-codex-instructions.json"
BUNDLED_LICENSES = ROOT / "BUNDLED_LICENSES.txt"


def sha256(content):
    return hashlib.sha256(content).hexdigest()


def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def download(path, ref=COMMIT):
    with urlopen(f"https://raw.githubusercontent.com/openai/codex/{ref}/{path}", timeout=30) as response:
        return response.read()


def changed_paths(before, after, path=""):
    """Compare every nested value; a missing key is distinct from JSON null."""
    if type(before) is not type(after):
        return [path or "/"]
    if isinstance(before, dict):
        changes = []
        for key in sorted(before.keys() | after.keys()):
            pointer = key.replace("~", "~0").replace("/", "~1")
            child = f"{path}/{pointer}"
            if key not in before or key not in after:
                changes.append(child)
            else:
                changes.extend(changed_paths(before[key], after[key], child))
        return changes
    if isinstance(before, list) and len(before) == len(after):
        return [
            change
            for index, (old, new) in enumerate(zip(before, after))
            for change in changed_paths(old, new, f"{path}/{index}")
        ]
    return [] if before == after else [path or "/"]


def compare_profiles(snapshot, upstream):
    models = {model["slug"]: model for model in upstream["models"]}
    profiles = snapshot["models"]
    changes = {}
    for slug in sorted(set(PROFILES) | profiles.keys()):
        old = profiles.get(slug)
        new = models.get(slug, {}).get("model_messages")
        if old is None or new is None:
            changes[slug] = {"status": "missing_snapshot" if old is None else "missing_upstream"}
            continue
        paths = changed_paths(old["modelMessages"], new)
        if paths:
            changes[slug] = {"status": "changed", "paths": paths}
    untracked = sorted(
        slug for slug, model in models.items()
        if slug.startswith("gpt-6") and slug not in profiles
        and isinstance(model.get("model_messages"), dict)
    )
    return {
        "profileChanges": changes,
        "untrackedGpt6Profiles": untracked,
        "reviewNeeded": bool(changes or untracked),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="Verify vendored content without writing")
    mode.add_argument("--compare-ref", metavar="CODEX_REF",
                      help="Review upstream profile drift without writing; exits 1 when review is needed")
    args = parser.parse_args()
    if args.compare_ref:
        snapshot = json.loads(SNAPSHOT.read_bytes())
        report = compare_profiles(
            snapshot, json.loads(download(SOURCE_PATH, args.compare_ref))
        )
        print(json.dumps({
            "pinnedCommit": snapshot["source"]["commit"],
            "comparedRef": args.compare_ref,
            **report,
        }, indent=2))
        return int(report["reviewNeeded"])
    raw = download(SOURCE_PATH)
    models = {model["slug"]: model for model in json.loads(raw)["models"]}
    profiles = {}
    for slug in PROFILES:
        messages = models[slug]["model_messages"]
        text = messages["instructions_template"]
        if not isinstance(text, str) or not text.startswith("You are Codex, an agent based on GPT-6."):
            raise ValueError(f"Unexpected upstream instructions for {slug}")
        # Copy the complete object, including nulls, examples and trailing newlines.
        profiles[slug] = {
            "instructionsSha256": sha256(text.encode("utf-8")),
            "modelMessagesSha256": sha256(canonical_json(messages)),
            "modelMessages": messages,
        }
    snapshot = {
        "source": {
            "repository": "https://github.com/openai/codex",
            "commit": COMMIT,
            "path": SOURCE_PATH,
            "catalogSha256": sha256(raw),
            "license": "Apache-2.0",
        },
        "fallbackModel": "gpt-6-astra",
        "models": profiles,
    }
    licenses = (
        "Copilot Bridge Atlas - bundled licenses and notices\n"
        "===================================================\n\n"
        "Atlas application source:\n\n"
        + (ROOT / "LICENSE").read_text(encoding="utf-8").rstrip()
        + "\n\nOpenAI Codex instruction data\n"
        "============================\n"
        f"Source: {BASE_URL}/{SOURCE_PATH}\n"
        "Modification: extracted GPT-6 model_messages objects into an Atlas JSON snapshot.\n"
        "Instruction wording and nested values are unchanged. Catalog metadata is separate.\n\n"
        + download("LICENSE").decode("utf-8")
        + "\n\nUpstream NOTICE (retained verbatim):\n\n"
        + download("NOTICE").decode("utf-8")
    ).encode("utf-8")
    if args.check:
        if json.loads(SNAPSHOT.read_bytes()) != snapshot:
            raise ValueError("Vendored instructions differ from the pinned upstream source")
        if BUNDLED_LICENSES.read_bytes() != licenses:
            raise ValueError("Bundled license/notice content differs")
    else:
        SNAPSHOT.write_bytes((json.dumps(snapshot, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
        BUNDLED_LICENSES.write_bytes(licenses)
    print(json.dumps({
        "verified" if args.check else "vendored": COMMIT,
        "profiles": {slug: profile["instructionsSha256"] for slug, profile in profiles.items()},
    }, indent=2))


if __name__ == "__main__":
    raise SystemExit(main())
