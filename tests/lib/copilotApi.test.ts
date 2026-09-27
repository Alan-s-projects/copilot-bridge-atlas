import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { copilotOpenModelCatalog } from "@/lib/api/copilot";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

it("opens only the server-selected generated model catalog", async () => {
  await copilotOpenModelCatalog();
  expect(invoke).toHaveBeenCalledWith("open_generated_model_catalog");
});
