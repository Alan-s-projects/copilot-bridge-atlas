import copy
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "vendor_instructions", Path(__file__).resolve().parents[1] / "vendor-openai-instructions.py"
)
vendor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vendor)


class InstructionDrift(unittest.TestCase):
    def fixtures(self):
        messages = {
            "instructions_template": "Literal {{connector_id}}\n",
            "permissions": {"policies": [None, {"value": "unchanged"}]},
        }
        snapshot = {
            "models": {
                slug: {"modelMessages": copy.deepcopy(messages)}
                for slug in vendor.PROFILES
            }
        }
        upstream = {
            "models": [
                {"slug": slug, "model_messages": copy.deepcopy(messages)}
                for slug in vendor.PROFILES
            ]
        }
        return snapshot, upstream

    def test_unchanged_objects_need_no_review(self):
        self.assertFalse(vendor.compare_profiles(*self.fixtures())["reviewNeeded"])

    def test_missing_key_and_explicit_null_are_different(self):
        self.assertEqual(
            vendor.changed_paths({"instructions_variables": None}, {}),
            ["/instructions_variables"],
        )
        self.assertEqual(vendor.changed_paths({}, {"nested": None}), ["/nested"])

    def test_nested_policy_types_and_trailing_newlines_are_checked(self):
        snapshot, upstream = self.fixtures()
        messages = upstream["models"][0]["model_messages"]
        messages["permissions"]["policies"][1]["value"] = False
        messages["instructions_template"] = messages["instructions_template"].rstrip()
        result = vendor.compare_profiles(snapshot, upstream)
        self.assertEqual(
            result["profileChanges"][vendor.PROFILES[0]]["paths"],
            ["/instructions_template", "/permissions/policies/1/value"],
        )
        self.assertEqual(vendor.changed_paths({"flag": True}, {"flag": 1}), ["/flag"])

    def test_missing_dedicated_profile_cannot_look_unchanged(self):
        snapshot, upstream = self.fixtures()
        del snapshot["models"]["gpt-6.1-sol"]
        result = vendor.compare_profiles(snapshot, upstream)
        self.assertTrue(result["reviewNeeded"])
        self.assertEqual(
            result["profileChanges"]["gpt-6.1-sol"]["status"], "missing_snapshot"
        )

    def test_removed_upstream_and_new_gpt6_profiles_require_review(self):
        snapshot, upstream = self.fixtures()
        upstream["models"].pop()
        upstream["models"].append({"slug": "gpt-6.2-sol", "model_messages": {}})
        result = vendor.compare_profiles(snapshot, upstream)
        self.assertTrue(result["reviewNeeded"])
        self.assertEqual(
            result["profileChanges"]["gpt-6.1-sol"]["status"], "missing_upstream"
        )
        self.assertEqual(result["untrackedGpt6Profiles"], ["gpt-6.2-sol"])

    def test_json_pointer_paths_escape_literal_keys(self):
        self.assertEqual(
            vendor.changed_paths({"a/b~": [None]}, {"a/b~": [False]}),
            ["/a~1b~0/0"],
        )

    def test_compare_mode_returns_review_status_without_writing(self):
        snapshot, upstream = self.fixtures()
        snapshot["source"] = {"commit": "reviewed-pin"}
        upstream["models"][0]["model_messages"]["instructions_template"] = "New\n"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "snapshot.json"
            original = json.dumps(snapshot).encode()
            path.write_bytes(original)
            with (
                patch.object(vendor, "SNAPSHOT", path),
                patch.object(vendor, "download", return_value=json.dumps(upstream).encode()),
                patch("sys.argv", ["vendor", "--compare-ref", "candidate-ref"]),
                contextlib.redirect_stdout(io.StringIO()) as output,
            ):
                self.assertEqual(vendor.main(), 1)
            self.assertEqual(path.read_bytes(), original)
            self.assertTrue(json.loads(output.getvalue())["reviewNeeded"])


if __name__ == "__main__":
    unittest.main()
