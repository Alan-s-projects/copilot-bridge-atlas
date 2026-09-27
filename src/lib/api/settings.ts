import { invoke } from "@tauri-apps/api/core";
import type { Settings } from "@/types";
import type { TrendGrouping, UsageRangeSelection } from "@/types/usage";

interface ConfigTransferResult {
  success: boolean;
  message: string;
  backupId?: string;
  warning?: string;
}

export const settingsApi = {
  async getUsageDateRange(): Promise<UsageRangeSelection> {
    return await invoke("get_usage_date_range");
  },

  async setUsageDateRange(
    range: UsageRangeSelection,
  ): Promise<UsageRangeSelection> {
    return await invoke("set_usage_date_range", { range });
  },

  async getUsageTrendGrouping(): Promise<TrendGrouping> {
    return await invoke("get_usage_trend_grouping");
  },

  async setUsageTrendGrouping(grouping: TrendGrouping): Promise<TrendGrouping> {
    return await invoke("set_usage_trend_grouping", { grouping });
  },

  async getUsageTableColumns(): Promise<UsageTableColumns> {
    return await invoke("get_usage_table_columns");
  },

  async setUsageTableColumns(
    table: UsageTableName,
    columns: string[],
  ): Promise<UsageTableColumns> {
    return await invoke("set_usage_table_columns", { table, columns });
  },

  async get(): Promise<Settings> {
    return await invoke("get_settings");
  },

  async save(settings: Settings): Promise<boolean> {
    return await invoke("save_settings", { settings });
  },

  async importConfigFromFile(filePath: string): Promise<ConfigTransferResult> {
    return await invoke("import_config_from_file", { filePath });
  },

  async checkUpdates(): Promise<void> {
    await invoke("check_for_updates");
  },

  async getAvailableReleaseVersion(): Promise<string | null> {
    return invoke("get_available_release_version");
  },

  async openExternal(url: string): Promise<void> {
    try {
      const u = new URL(url);
      const scheme = u.protocol.replace(":", "").toLowerCase();
      if (scheme !== "http" && scheme !== "https") {
        throw new Error("Unsupported URL scheme");
      }
    } catch {
      throw new Error("Invalid URL");
    }
    await invoke("open_external", { url });
  },

  async setAutoLaunch(enabled: boolean): Promise<boolean> {
    return await invoke("set_auto_launch", { enabled });
  },
};

export interface UsageTableColumns {
  requestLogs?: string[];
  modelStats?: string[];
}

export type UsageTableName = keyof UsageTableColumns;

interface BackupEntry {
  filename: string;
  sizeBytes: number;
  createdAt: string;
}

export const backupsApi = {
  async createDbBackup(): Promise<string> {
    return await invoke("create_db_backup");
  },

  async listDbBackups(): Promise<BackupEntry[]> {
    return await invoke("list_db_backups");
  },

  async restoreDbBackup(filename: string): Promise<string> {
    return await invoke("restore_db_backup", { filename });
  },

  async renameDbBackup(oldFilename: string, newName: string): Promise<string> {
    return await invoke("rename_db_backup", { oldFilename, newName });
  },

  async deleteDbBackup(filename: string): Promise<void> {
    await invoke("delete_db_backup", { filename });
  },
};
