import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const builds = {
  win32: [
    "powershell.exe",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      join(root, "scripts", "build-msi.ps1"),
    ],
  ],
  darwin: ["bash", [join(root, "scripts", "build-macos-intel.sh")]],
};
const build = builds[process.platform];
if (!build) {
  console.error("Atlas release builds require Windows x64 or macOS.");
  process.exit(1);
}
const result = spawnSync(build[0], build[1], { cwd: root, stdio: "inherit" });
if (result.error) console.error(result.error.message);
process.exit(result.status ?? 1);
