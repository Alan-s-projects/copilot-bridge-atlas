import importlib.util
import json
import plistlib
import struct
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "macos_bundle_check", Path(__file__).parents[1] / "check-macos-bundle.py"
)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def macho(cpu=0x01000007, minimum=13 << 16):
    command = struct.pack("<6I", 0x32, 24, 1, minimum, minimum, 0)
    return struct.pack("<8I", 0xFEEDFACF, cpu, 3, 2, 1, len(command), 0, 0) + command


class MacBundleValidation(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "src-tauri").mkdir()
        (self.root / "package.json").write_text('{"version":"6.0.6"}', encoding="utf-8")
        (self.root / "src-tauri/tauri.conf.json").write_text(
            json.dumps({"version": "6.0.6", "identifier": "com.alansprojects.copilotbridgeatlas"}),
            encoding="utf-8",
        )
        (self.root / "src-tauri/tauri.macos.conf.json").write_text(
            '{"bundle":{"macOS":{"minimumSystemVersion":"13.0"}}}', encoding="utf-8"
        )
        self.app = self.root / "Copilot Bridge Atlas.app"
        (self.app / "Contents/MacOS").mkdir(parents=True)
        (self.app / "Contents/Resources").mkdir()
        self.info = {
            "CFBundleIdentifier": "com.alansprojects.copilotbridgeatlas",
            "CFBundleShortVersionString": "6.0.6",
            "CFBundleVersion": "6.0.6",
            "CFBundleExecutable": "copilot-bridge-atlas",
            "CFBundleIconFile": "icon.icns",
            "LSMinimumSystemVersion": "13.0",
        }
        self.write_info()
        self.binary = self.app / "Contents/MacOS/copilot-bridge-atlas"
        self.binary.write_bytes(macho())
        (self.app / "Contents/Resources/icon.icns").write_bytes(b"icns" + struct.pack(">I", 8))
        notices = b"Atlas application and dependency license notices.\n"
        (self.root / "BUNDLED_LICENSES.txt").write_bytes(notices)
        (self.app / "Contents/Resources/BUNDLED_LICENSES.txt").write_bytes(notices)

    def write_info(self):
        (self.app / "Contents/Info.plist").write_bytes(plistlib.dumps(self.info))

    def test_accepts_matching_intel_bundle(self):
        report = checker.check_metadata(self.app, self.root)
        self.assertEqual(report["architecture"], "x86_64")
        self.assertEqual(report["minimum_os_version"], "13.0.0")

    def test_rejects_arm64_or_non_macho_executables(self):
        for data in (macho(cpu=0x0100000C), b"not a macOS binary"):
            with self.subTest(data=data[:16]):
                self.binary.write_bytes(data)
                with self.assertRaises(ValueError):
                    checker.check_metadata(self.app, self.root)

    def test_rejects_wrong_bundle_identity_or_version(self):
        for key in ("CFBundleIdentifier", "CFBundleVersion", "CFBundleShortVersionString"):
            with self.subTest(key=key):
                old = self.info[key]
                self.info[key] = "wrong"
                self.write_info()
                with self.assertRaises(ValueError):
                    checker.check_metadata(self.app, self.root)
                self.info[key] = old

    def test_rejects_mismatched_deployment_target(self):
        self.binary.write_bytes(macho(minimum=15 << 16))
        with self.assertRaisesRegex(ValueError, "deployment target"):
            checker.check_metadata(self.app, self.root)

    def test_rejects_truncated_load_commands(self):
        self.binary.write_bytes(macho()[:-1])
        with self.assertRaisesRegex(ValueError, "truncated"):
            checker.check_metadata(self.app, self.root)

    def test_rejects_a_missing_or_truncated_icon(self):
        icon = self.app / "Contents/Resources/icon.icns"
        icon.write_bytes(b"icns" + struct.pack(">I", 24))
        with self.assertRaisesRegex(ValueError, "truncated"):
            checker.check_metadata(self.app, self.root)
        icon.unlink()
        with self.assertRaises(FileNotFoundError):
            checker.check_metadata(self.app, self.root)

    def test_detaches_disk_image_when_mount_metadata_is_invalid(self):
        attached = SimpleNamespace(
            stdout=plistlib.dumps({"system-entities": []}).decode("utf-8")
        )
        with patch.object(checker, "run", side_effect=[None, attached, None]) as run:
            with self.assertRaisesRegex(ValueError, "single application volume"):
                checker.check_disk_image(self.root / "test.dmg", self.root, {})
        self.assertEqual(run.call_args_list[0].args[:2], ("hdiutil", "verify"))
        self.assertEqual(run.call_args_list[1].args[:2], ("hdiutil", "attach"))
        self.assertEqual(run.call_args_list[2].args[:2], ("hdiutil", "detach"))

    def test_rejects_missing_or_changed_license_notices(self):
        notices = self.app / "Contents/Resources/BUNDLED_LICENSES.txt"
        notices.write_text("Incomplete notices", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "license notices"):
            checker.check_metadata(self.app, self.root)
        notices.unlink()
        with self.assertRaises(FileNotFoundError):
            checker.check_metadata(self.app, self.root)

    def test_native_tool_failure_keeps_diagnostic_stderr(self):
        failure = subprocess.CalledProcessError(
            1, ["hdiutil", "attach"], stderr="hdiutil: attach failed - no mountable file systems"
        )
        with patch.object(checker.subprocess, "run", side_effect=failure):
            with self.assertRaisesRegex(RuntimeError, "no mountable file systems"):
                checker.run("hdiutil", "attach", self.root / "test.dmg")


if __name__ == "__main__":
    unittest.main()
