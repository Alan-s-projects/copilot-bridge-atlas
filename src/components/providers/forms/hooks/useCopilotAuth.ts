import { useState, useCallback, useRef, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { authApi, settingsApi } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import type {
  ManagedAuthStatus,
  ManagedAuthDeviceCodeResponse,
} from "@/lib/api";

type PollingState = "idle" | "polling" | "success" | "error";
type LoginRequest = {
  generation: number;
};
const authProvider = "github_copilot";
const queryKey = ["managed-auth-status", authProvider] as const;

export function useCopilotAuth(githubDomain?: string) {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  const [pollingState, setPollingState] = useState<PollingState>("idle");
  const [deviceCode, setDeviceCode] =
    useState<ManagedAuthDeviceCodeResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const pollingTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pollingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const flowGenerationRef = useRef(0);

  const {
    data: authStatus,
    isLoading: isLoadingStatus,
    isSuccess: isStatusSuccess,
    isError: isStatusError,
    refetch: refetchStatus,
  } = useQuery<ManagedAuthStatus>({
    queryKey,
    queryFn: () => authApi.authGetStatus(authProvider),
    staleTime: 30000,
    refetchInterval: false,
  });

  const stopPolling = useCallback(() => {
    if (pollingTimerRef.current) {
      clearTimeout(pollingTimerRef.current);
      pollingTimerRef.current = null;
    }
    if (pollingTimeoutRef.current) {
      clearTimeout(pollingTimeoutRef.current);
      pollingTimeoutRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => {
      flowGenerationRef.current += 1;
      stopPolling();
    };
  }, [githubDomain, stopPolling]);

  const startLoginMutation = useMutation({
    mutationFn: (_request: LoginRequest) =>
      authApi.authStartLogin(authProvider, githubDomain),
    onSuccess: async (response, request) => {
      if (request.generation !== flowGenerationRef.current) {
        return;
      }
      setDeviceCode(response);
      setPollingState("polling");
      setError(null);
      const expiresAt = Date.now() + response.expires_in * 1000;
      const expire = () => {
        if (request.generation !== flowGenerationRef.current) return;
        stopPolling();
        flowGenerationRef.current += 1;
        setPollingState("error");
        setError("Device code expired. Please try again.");
      };
      pollingTimeoutRef.current = setTimeout(
        expire,
        response.expires_in * 1000,
      );

      try {
        await copyText(response.user_code);
      } catch (e) {
        console.debug("[ManagedAuth] Failed to copy user code:", e);
      }
      if (request.generation !== flowGenerationRef.current) return;

      try {
        await settingsApi.openExternal(response.verification_uri);
      } catch (e) {
        console.debug("[ManagedAuth] Failed to open browser:", e);
      }
      if (request.generation !== flowGenerationRef.current) return;

      // Add a small buffer on top of GitHub's suggested interval to avoid
      // hitting slow_down responses too aggressively during device polling.
      const interval = Math.max((response.interval || 5) + 3, 8) * 1000;

      const pollOnce = async () => {
        if (request.generation !== flowGenerationRef.current) return;
        if (Date.now() >= expiresAt) {
          expire();
          return;
        }

        try {
          const newAccount = await authApi.authPollForAccount(
            authProvider,
            response.device_code,
            githubDomain,
          );
          if (request.generation !== flowGenerationRef.current) return;
          if (newAccount) {
            stopPolling();
            flowGenerationRef.current += 1;
            const completionGeneration = flowGenerationRef.current;
            setPollingState("success");
            await queryClient.invalidateQueries({ queryKey });
            if (completionGeneration !== flowGenerationRef.current) return;
            setPollingState("idle");
            setDeviceCode(null);
            return;
          }
        } catch (e) {
          if (request.generation !== flowGenerationRef.current) return;
          // The backend returns null while authorization is pending.
          stopPolling();
          flowGenerationRef.current += 1;
          setPollingState("error");
          setError(e instanceof Error ? e.message : String(e));
          return;
        }
        if (request.generation === flowGenerationRef.current) {
          pollingTimerRef.current = setTimeout(() => void pollOnce(), interval);
        }
      };

      void pollOnce();
    },
    onError: (e, request) => {
      if (request.generation !== flowGenerationRef.current) return;
      setPollingState("error");
      setError(e instanceof Error ? e.message : String(e));
    },
  });

  const logoutMutation = useMutation({
    mutationFn: () => authApi.authLogout(authProvider),
    onSuccess: async () => {
      setPollingState("idle");
      setDeviceCode(null);
      setError(null);
      queryClient.setQueryData(queryKey, {
        provider: authProvider,
        authenticated: false,
        default_account_id: null,
        accounts: [],
      });
      await queryClient.invalidateQueries({ queryKey });
    },
    onError: async (e) => {
      console.error("[ManagedAuth] Failed to logout:", e);
      setError(e instanceof Error ? e.message : String(e));
      await refetchStatus();
    },
  });

  const removeAccountMutation = useMutation({
    mutationFn: (accountId: string) =>
      authApi.authRemoveAccount(authProvider, accountId),
    onSuccess: async () => {
      setPollingState("idle");
      setDeviceCode(null);
      setError(null);
      toast.success(
        t("managedAuth.accountRemoved", {
          defaultValue: "Account removed",
        }),
      );
      await queryClient.invalidateQueries({ queryKey });
    },
    onError: (e) => {
      console.error("[ManagedAuth] Failed to remove account:", e);
      setError(e instanceof Error ? e.message : String(e));
    },
  });

  const setDefaultAccountMutation = useMutation({
    mutationFn: (accountId: string) =>
      authApi.authSetDefaultAccount(authProvider, accountId),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey });
    },
    onError: (e) => {
      console.error("[ManagedAuth] Failed to set default account:", e);
      setError(e instanceof Error ? e.message : String(e));
    },
  });

  const startAuth = useCallback(() => {
    const generation = ++flowGenerationRef.current;
    stopPolling();
    setPollingState("idle");
    setDeviceCode(null);
    setError(null);
    startLoginMutation.mutate({ generation });
  }, [startLoginMutation, stopPolling]);

  const cancelAuth = useCallback(() => {
    flowGenerationRef.current += 1;
    stopPolling();
    setPollingState("idle");
    setDeviceCode(null);
    setError(null);
  }, [stopPolling]);

  const logout = useCallback(() => {
    cancelAuth();
    logoutMutation.mutate();
  }, [cancelAuth, logoutMutation]);

  const removeAccount = useCallback(
    (accountId: string) => {
      cancelAuth();
      removeAccountMutation.mutate(accountId);
    },
    [cancelAuth, removeAccountMutation],
  );

  const setDefaultAccount = useCallback(
    (accountId: string) => {
      setDefaultAccountMutation.mutate(accountId);
    },
    [setDefaultAccountMutation],
  );

  const accounts = authStatus?.accounts ?? [];

  return {
    isLoadingStatus,
    // Distinguish "status loaded successfully" from "loading / failed" so
    // callers don't treat a failed query's empty `accounts` as authoritative.
    isStatusSuccess,
    isStatusError,
    accounts,
    hasAnyAccount: accounts.length > 0,
    defaultAccountId: authStatus?.default_account_id ?? null,
    pollingState,
    deviceCode,
    error,
    isPolling: pollingState === "polling",
    isAddingAccount: startLoginMutation.isPending || pollingState === "polling",
    isRemovingAccount: removeAccountMutation.isPending,
    isSettingDefaultAccount: setDefaultAccountMutation.isPending,
    addAccount: startAuth,
    cancelAuth,
    logout,
    removeAccount,
    setDefaultAccount,
    refetchStatus,
  };
}
