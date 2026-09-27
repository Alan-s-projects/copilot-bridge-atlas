import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "catalog_check", Path(__file__).resolve().parents[1] / "check-codex-catalog.py"
)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def entry(slug, text):
    return {
        "slug": slug,
        "base_instructions": text,
        "model_messages": {"instructions_template": text, "instructions_variables": None},
    }


class InstructionVerification(unittest.TestCase):
    def test_literal_examples_and_trailing_newlines_are_not_rewritten(self):
        text = "An example: {{connector_id}}\n"
        self.assertEqual(check.expected_instructions(entry("astra", text)), text)
        check.verify_instructions(
            [{"model": "astra", "instructions": text}], {"astra": entry("astra", text)}
        )

    def test_every_model_and_request_is_compared_not_only_the_first(self):
        entries = {"astra": entry("astra", "A\n"), "luna": entry("luna", "L\n")}
        with self.assertRaises(AssertionError):
            check.verify_instructions([
                {"model": "astra", "instructions": "A\n"},
                {"model": "luna", "instructions": "A\n"},
            ], entries)
        with self.assertRaises(AssertionError):
            check.verify_instructions([{"model": "astra", "instructions": "A"}], entries)

    def test_placeholder_syntax_cannot_skip_verification(self):
        entries = {"astra": entry("astra", "{{connector_id}}\n")}
        with self.assertRaises(AssertionError):
            check.verify_instructions([{"model": "astra", "instructions": "wrong"}], entries)

    def test_switch_requires_the_current_literal_prompt_and_a_known_base(self):
        entries = {"astra": entry("astra", "A\n"), "luna": entry("luna", "L\n")}
        request = {"model": "luna", "instructions": "A\n",
                   "model_switch_messages": ["<model_switch>\nL\n</model_switch>"]}
        check.verify_instructions([request], entries)
        self.assertEqual(request["instruction_delivery"], "model_switch")
        with self.assertRaises(AssertionError):
            check.verify_instructions([{**request, "instructions": "our custom prompt"}], entries)
        with self.assertRaises(AssertionError):
            check.verify_instructions([{**request, "model_switch_messages": [
                "<model_switch>\nL\n</model_switch>", "<model_switch>\nA\n</model_switch>",
            ]}], entries)

    def test_missing_or_conflicting_instruction_fields_fail(self):
        with self.assertRaises(ValueError):
            check.expected_instructions({"slug": "missing"})
        broken = entry("astra", "A")
        broken["base_instructions"] = "B"
        with self.assertRaises(ValueError):
            check.expected_instructions(broken)

    def test_legacy_only_and_empty_templates_remain_literal(self):
        self.assertEqual(check.expected_instructions({"slug": "old", "base_instructions": "old"}), "old")
        self.assertEqual(check.expected_instructions(entry("empty", "")), "")


if __name__ == "__main__":
    unittest.main()
