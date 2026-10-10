"""Check Atlas's Intel macOS bundle, metadata, signature, and disk image."""
import argparse
import hashlib
import json
import plistlib
import struct
import subprocess
import sys
import tempfile
from pathlib import Path


def run(*args):
    try:
        return subprocess.run(
            [str(arg) for arg in args], check=True, capture_output=True, text=True
        )
    except subprocess.CalledProcessError as error:
        details = "\n".join(
            text.strip() for text in (error.stdout, error.stderr) if text and text.strip()
        )
        raise RuntimeError(
            f"{args[0]} failed with status {error.returncode}:\n{details}"
        ) from error


def macho_metadata(binary):
    with Path(binary).open("rb") as stream:
        header = stream.read(32)
        if len(header) != 32 or header[:4] != b"\xcf\xfa\xed\xfe":
            raise ValueError("The app must contain a single 64-bit Intel Mach-O executable.")
        _, cpu, _, _, count, command_bytes, _, _ = struct.unpack("<8I", header)
        if cpu != 0x01000007:
            raise ValueError("The app executable is not x86_64.")
        if command_bytes > 16 * 1024 * 1024:
            raise ValueError("The Mach-O load command header is invalid.")
        commands = stream.read(command_bytes)
    if len(commands) != command_bytes:
        raise ValueError("The Mach-O load commands are truncated.")
    minimum = None
    offset = 0
    for _ in range(count):
        if offset + 8 > len(commands):
            raise ValueError("The Mach-O load command is truncated.")
        command, size = struct.unpack_from("<2I", commands, offset)
        if size < 8 or offset + size > len(commands):
            raise ValueError("The Mach-O load command size is invalid.")
        if command == 0x24 and size >= 16:  # LC_VERSION_MIN_MACOSX
            minimum = struct.unpack_from("<I", commands, offset + 8)[0]
        elif command == 0x32 and size >= 24:  # LC_BUILD_VERSION
            platform, version = struct.unpack_from("<2I", commands, offset + 8)
            if platform != 1:
                raise ValueError("The executable targets an Apple platform other than macOS.")
            minimum = version
        offset += size
    if minimum is None:
        raise ValueError("The executable does not declare a macOS deployment target.")
    return {
        "architecture": "x86_64",
        "minimum_os_version": f"{minimum >> 16}.{(minimum >> 8) & 255}.{minimum & 255}",
    }


def version_tuple(version):
    numbers = tuple(int(part) for part in version.split("."))
    if not 1 <= len(numbers) <= 3:
        raise ValueError(f"Invalid version: {version}")
    return numbers + (0,) * (3 - len(numbers))


def check_metadata(app, root):
    app, root = Path(app), Path(root)
    config = json.loads((root / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
    mac_config = json.loads(
        (root / "src-tauri/tauri.macos.conf.json").read_text(encoding="utf-8")
    )
    package_version = json.loads((root / "package.json").read_text(encoding="utf-8"))["version"]
    if config["version"] != package_version:
        raise ValueError("Package and Tauri versions disagree.")
    with (app / "Contents/Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    for key, expected in {
        "CFBundleIdentifier": config["identifier"],
        "CFBundleShortVersionString": package_version,
        "CFBundleVersion": package_version,
        "CFBundleExecutable": "copilot-bridge-atlas",
    }.items():
        if info.get(key) != expected:
            raise ValueError(f"{key} must be {expected!r}, got {info.get(key)!r}.")
    minimum = mac_config["bundle"]["macOS"]["minimumSystemVersion"]
    if version_tuple(info.get("LSMinimumSystemVersion", "")) != version_tuple(minimum):
        raise ValueError("Info.plist has the wrong minimum macOS version.")
    icon_name = info.get("CFBundleIconFile", "")
    if not icon_name or Path(icon_name).name != icon_name:
        raise ValueError("The app must declare its bundled icon.")
    if not icon_name.endswith(".icns"):
        icon_name += ".icns"
    icon = app / "Contents/Resources" / icon_name
    icon_header = icon.read_bytes()[:8]
    if len(icon_header) != 8 or icon_header[:4] != b"icns":
        raise ValueError("The app does not contain a valid ICNS icon.")
    if struct.unpack(">I", icon_header[4:])[0] != icon.stat().st_size:
        raise ValueError("The bundled ICNS icon is truncated.")
    notices = app / "Contents/Resources/BUNDLED_LICENSES.txt"
    if notices.read_bytes() != (root / "BUNDLED_LICENSES.txt").read_bytes():
        raise ValueError("The app's bundled license notices do not match the source.")
    binary = app / "Contents/MacOS" / info["CFBundleExecutable"]
    macho = macho_metadata(binary)
    if version_tuple(macho["minimum_os_version"]) != version_tuple(minimum):
        raise ValueError("The executable deployment target disagrees with Info.plist.")
    return {
        "version": package_version,
        "bundle_id": info["CFBundleIdentifier"],
        "icon": icon_name,
        "license_notices_bundled": True,
        **macho,
        "executable_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    }


def check_signature(app):
    run("codesign", "--verify", "--deep", "--strict", app)
    signature = run("codesign", "--display", "--verbose=4", app).stderr
    return "ad-hoc" if "Signature=adhoc" in signature else "signed"


def check_disk_image(dmg, root, expected):
    run("hdiutil", "verify", dmg)
    with tempfile.TemporaryDirectory(prefix="atlas-dmg-check-") as directory:
        mount = Path(directory) / "volume"
        mount.mkdir()
        result = run(
            "hdiutil", "attach", "-readonly", "-nobrowse", "-plist",
            "-mountpoint", mount, dmg,
        )
        try:
            entities = plistlib.loads(result.stdout.encode())["system-entities"]
            volumes = [entry for entry in entities if "mount-point" in entry]
            if len(volumes) != 1 or Path(volumes[0]["mount-point"]).resolve() != mount.resolve():
                raise ValueError("The DMG did not mount a single application volume.")
            mounted_app = mount / "Copilot Bridge Atlas.app"
            actual = check_metadata(mounted_app, root)
            if actual != expected:
                raise ValueError("The DMG app does not match the built app.")
            check_signature(mounted_app)
            applications = mount / "Applications"
            if not applications.is_symlink() or applications.readlink() != Path("/Applications"):
                raise ValueError("The DMG is missing its Applications installation link.")
        finally:
            run("hdiutil", "detach", mount)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--dmg", type=Path)
    parser.add_argument("--static", action="store_true", help="Metadata-only checks on a non-Mac host.")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    report = check_metadata(args.app, root)
    if not args.static:
        if sys.platform != "darwin":
            parser.error("Signature and DMG checks require macOS; use --static for metadata only.")
        report["signature"] = check_signature(args.app)
        if args.dmg:
            check_disk_image(args.dmg, root, {k: v for k, v in report.items() if k != "signature"})
            report["dmg_verified"] = True
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
