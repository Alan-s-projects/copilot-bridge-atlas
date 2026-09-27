import type { Provider, Settings } from "@/types";
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
let providers: Record<string, Provider> = {};
let settings: Settings = {};
export const resetProviderState = () => {
  providers = {
    copilot: {
      id: "copilot",
      name: "GitHub Copilot",
      settingsConfig: {},
      meta: { providerType: "github_copilot" },
    },
  };
  settings = {};
};
resetProviderState();
export const getProviders = () => clone(providers);
export const getCurrentProviderId = () => "copilot";
export const updateProvider = (provider: Provider) => {
  providers[provider.id] = clone(provider);
};
export const getSettings = () => clone(settings);
export const setSettings = (value: Partial<Settings>) => {
  settings = { ...settings, ...value };
};
