import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Loader2,
  Github,
  LogOut,
  Copy,
  Check,
  ExternalLink,
  Plus,
  X,
  User,
  AlertTriangle,
  RefreshCw,
} from "lucide-react";
import { useCopilotAuth } from "./hooks/useCopilotAuth";
import { copyText } from "@/lib/clipboard";
import type { GitHubAccount } from "@/lib/api";

interface CopilotAuthSectionProps {
  className?: string;
  /** select mode only selects an account; manage mode provides account management. */
  mode?: "manage" | "select";
  /** 当前选中的 GitHub 账号 ID */
  selectedAccountId?: string | null;
  /** 账号选择回调 */
  onAccountSelect?: (accountId: string | null) => void;
}

/**
 * Copilot OAuth 认证区块
 *
 * 显示 GitHub Copilot 的认证状态，支持多账号管理和选择。
 */
export const CopilotAuthSection: React.FC<CopilotAuthSectionProps> = ({
  className,
  mode = "manage",
  selectedAccountId,
  onAccountSelect,
}) => {
  const { t } = useTranslation();
  const [copied, setCopied] = React.useState(false);
  const [deploymentType, setDeploymentType] = React.useState<
    "github.com" | "enterprise"
  >("github.com");
  const [enterpriseDomain, setEnterpriseDomain] = React.useState("");

  // 根据部署类型计算实际的 GitHub 域名
  const effectiveGithubDomain =
    deploymentType === "enterprise" && enterpriseDomain.trim()
      ? enterpriseDomain
          .trim()
          .replace(/^https?:\/\//, "")
          .replace(/\/$/, "")
      : undefined;

  const {
    accounts,
    defaultAccountId,
    migrationError,
    isStatusSuccess,
    isStatusError,
    hasAnyAccount,
    pollingState,
    deviceCode,
    error,
    isPolling,
    isAddingAccount,
    isRemovingAccount,
    isSettingDefaultAccount,
    addAccount,
    removeAccount,
    setDefaultAccount,
    cancelAuth,
    logout,
    refetchStatus,
  } = useCopilotAuth(effectiveGithubDomain);

  // 复制用户码
  const copyUserCode = async () => {
    if (deviceCode?.user_code) {
      await copyText(deviceCode.user_code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  // 处理账号选择
  const handleAccountSelect = (value: string) => {
    onAccountSelect?.(value === "none" ? null : value);
  };

  React.useEffect(() => {
    // Only clear a bound account once the status query has *successfully*
    // loaded and the account is genuinely gone. A failed/pending query yields
    // an empty `accounts` array, which must not silently unbind the provider.
    if (
      mode !== "select" ||
      !selectedAccountId ||
      !onAccountSelect ||
      !isStatusSuccess
    ) {
      return;
    }

    if (!accounts.some((account) => account.id === selectedAccountId)) {
      onAccountSelect(null);
    }
  }, [accounts, isStatusSuccess, mode, onAccountSelect, selectedAccountId]);

  // 处理移除账号
  const handleRemoveAccount = (accountId: string, e: React.MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
    removeAccount(accountId);
    // 如果移除的是当前选中的账号，清除选择
    if (selectedAccountId === accountId) {
      onAccountSelect?.(null);
    }
  };

  // 渲染账号头像
  const renderAvatar = (account: GitHubAccount) => {
    return <CopilotAccountAvatar account={account} />;
  };

  const accountSelect = isStatusSuccess &&
    onAccountSelect &&
    (mode === "select" || hasAnyAccount) && (
      <div className="space-y-2">
        <Label className="text-sm text-muted-foreground">
          {mode === "select"
            ? t("copilot.githubAccount", "GitHub account")
            : t("copilot.selectAccount", "Select Account")}
        </Label>
        <Select
          value={selectedAccountId || "none"}
          onValueChange={handleAccountSelect}
        >
          <SelectTrigger>
            <SelectValue
              placeholder={t(
                "copilot.selectAccountPlaceholder",
                "Select a GitHub account",
              )}
            />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="none">
              <span className="text-muted-foreground">
                {t("copilot.useDefaultAccount", "Use default account")}
              </span>
            </SelectItem>
            {accounts.map((account) => (
              <SelectItem key={account.id} value={account.id}>
                <div className="flex items-center gap-2">
                  {renderAvatar(account)}
                  <span>{account.login}</span>
                </div>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    );

  return (
    <div className={`space-y-4 ${className || ""}`}>
      {/* 认证状态标题 */}
      {mode === "manage" && (
        <div className="flex items-center justify-between">
          <Label>{t("copilot.authStatus", "Authentication Status")}</Label>
          <Badge
            variant={
              isStatusError
                ? "destructive"
                : hasAnyAccount
                  ? "default"
                  : "secondary"
            }
            className={
              isStatusSuccess && hasAnyAccount
                ? "bg-green-500 hover:bg-green-600"
                : ""
            }
          >
            {isStatusError
              ? t("copilot.statusUnavailable", "Status unavailable")
              : !isStatusSuccess
                ? t("copilot.statusLoading", "Loading account status...")
                : hasAnyAccount
                  ? t("copilot.accountCount", {
                      count: accounts.length,
                      defaultValue: "{{count}} account(s)",
                    })
                  : t("copilot.notAuthenticated", "Not authenticated")}
          </Badge>
        </div>
      )}

      {isStatusError && (
        <div
          role="alert"
          className="flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive"
        >
          <AlertTriangle className="h-4 w-4 shrink-0" />
          <span className="min-w-0 flex-1">
            {t(
              "copilot.statusLoadFailed",
              "Failed to load GitHub Copilot account status. Please retry.",
            )}
          </span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-7 shrink-0"
            onClick={() => void refetchStatus()}
          >
            <RefreshCw className="mr-1 h-3.5 w-3.5" />
            {t("copilot.retry", "Retry")}
          </Button>
        </div>
      )}

      {!isStatusSuccess && !isStatusError && (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("copilot.statusLoading", "Loading account status...")}
        </div>
      )}

      {/* GitHub 部署类型选择 */}
      {mode === "manage" && (
        <div className="space-y-2">
          <Label className="text-sm text-muted-foreground">
            {t("copilot.deploymentType", "GitHub Deployment Type")}
          </Label>
          <Select
            value={deploymentType}
            onValueChange={(v) =>
              setDeploymentType(v as "github.com" | "enterprise")
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="github.com">
                {t("copilot.deploymentGitHubCom", "GitHub.com")}
              </SelectItem>
              <SelectItem value="enterprise">
                {t("copilot.deploymentEnterprise", "GitHub Enterprise Server")}
              </SelectItem>
            </SelectContent>
          </Select>
          {deploymentType === "enterprise" && (
            <Input
              placeholder={t(
                "copilot.enterpriseDomainPlaceholder",
                "e.g. company.ghe.com",
              )}
              value={enterpriseDomain}
              onChange={(e) => setEnterpriseDomain(e.target.value)}
            />
          )}
        </div>
      )}

      {mode === "manage" && migrationError && (
        <p className="text-sm text-amber-600 dark:text-amber-400">
          {t("copilot.migrationFailed", {
            error: migrationError,
            defaultValue: "Legacy auth migration failed: {{error}}",
          })}
        </p>
      )}

      {/* 账号选择器（有账号时显示） */}
      {accountSelect}

      {/* 已登录账号列表 */}
      {mode === "manage" && isStatusSuccess && hasAnyAccount && (
        <div className="space-y-2">
          <Label className="text-sm text-muted-foreground">
            {t("copilot.loggedInAccounts", "Logged in accounts")}
          </Label>
          <div className="space-y-1">
            {accounts.map((account) => (
              <div
                key={account.id}
                className="flex items-center justify-between p-2 rounded-md border bg-muted/30"
              >
                <div className="flex items-center gap-2">
                  {renderAvatar(account)}
                  <span className="text-sm font-medium">{account.login}</span>
                  {defaultAccountId === account.id && (
                    <Badge variant="secondary" className="text-xs">
                      {t("copilot.defaultAccount", "Default")}
                    </Badge>
                  )}
                  {account.github_domain &&
                    account.github_domain !== "github.com" && (
                      <Badge variant="outline" className="text-xs">
                        {account.github_domain}
                      </Badge>
                    )}
                  {selectedAccountId === account.id && (
                    <Badge variant="outline" className="text-xs">
                      {t("copilot.selected", "Selected")}
                    </Badge>
                  )}
                </div>
                <div className="flex items-center gap-1">
                  {defaultAccountId !== account.id && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-xs text-muted-foreground"
                      onClick={() => setDefaultAccount(account.id)}
                      disabled={isSettingDefaultAccount}
                    >
                      {t("copilot.setAsDefault", "Set as default")}
                    </Button>
                  )}
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 text-muted-foreground hover:text-red-500"
                    onClick={(e) => handleRemoveAccount(account.id, e)}
                    disabled={isRemovingAccount}
                    title={t("copilot.removeAccount", "Remove account")}
                  >
                    <X className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 未认证状态 - 登录按钮 */}
      {mode === "manage" &&
        isStatusSuccess &&
        !hasAnyAccount &&
        pollingState === "idle" && (
          <Button
            type="button"
            onClick={addAccount}
            className="w-full"
            variant="outline"
            disabled={
              deploymentType === "enterprise" && !enterpriseDomain.trim()
            }
          >
            <Github className="mr-2 h-4 w-4" />
            {t("copilot.loginWithGitHub", "Login with GitHub")}
          </Button>
        )}

      {/* 已有账号 - 添加更多账号按钮 */}
      {mode === "manage" &&
        isStatusSuccess &&
        hasAnyAccount &&
        pollingState === "idle" && (
          <Button
            type="button"
            onClick={addAccount}
            className="w-full"
            variant="outline"
            disabled={
              isAddingAccount ||
              (deploymentType === "enterprise" && !enterpriseDomain.trim())
            }
          >
            <Plus className="mr-2 h-4 w-4" />
            {t("copilot.addAnotherAccount", "Add another account")}
          </Button>
        )}

      {/* 轮询中状态 */}
      {mode === "manage" && isPolling && deviceCode && (
        <div className="space-y-3 p-4 rounded-lg border border-border bg-muted/50">
          <div className="flex items-center justify-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("copilot.waitingForAuth", "Waiting for authorization...")}
          </div>

          {/* 用户码 */}
          <div className="text-center">
            <p className="text-xs text-muted-foreground mb-1">
              {t("copilot.enterCode", "Please enter the code in your browser:")}
            </p>
            <div className="flex items-center justify-center gap-2">
              <code className="text-2xl font-mono font-bold tracking-wider bg-background px-4 py-2 rounded border">
                {deviceCode.user_code}
              </code>
              <Button
                type="button"
                size="icon"
                variant="ghost"
                onClick={copyUserCode}
                title={t("copilot.copyCode", "Copy code")}
              >
                {copied ? (
                  <Check className="h-4 w-4 text-green-500" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </Button>
            </div>
          </div>

          {/* 验证链接 */}
          <div className="text-center">
            <a
              href={deviceCode.verification_uri}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-sm text-blue-500 hover:underline"
            >
              {deviceCode.verification_uri}
              <ExternalLink className="h-3 w-3" />
            </a>
          </div>

          {/* 取消按钮 */}
          <div className="text-center">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={cancelAuth}
            >
              {t("common.cancel", "Cancel")}
            </Button>
          </div>
        </div>
      )}

      {/* 错误状态 */}
      {mode === "manage" && pollingState === "error" && error && (
        <div className="space-y-2">
          <p className="text-sm text-red-500">{error}</p>
          <div className="flex gap-2">
            <Button
              type="button"
              onClick={addAccount}
              variant="outline"
              size="sm"
            >
              {t("copilot.retry", "Retry")}
            </Button>
            <Button
              type="button"
              onClick={cancelAuth}
              variant="ghost"
              size="sm"
            >
              {t("common.cancel", "Cancel")}
            </Button>
          </div>
        </div>
      )}

      {/* 注销所有账号按钮 */}
      {mode === "manage" &&
        isStatusSuccess &&
        hasAnyAccount &&
        accounts.length > 1 && (
          <Button
            type="button"
            variant="outline"
            onClick={logout}
            className="w-full text-red-500 hover:text-red-600 hover:bg-red-50 dark:hover:bg-red-950"
          >
            <LogOut className="mr-2 h-4 w-4" />
            {t("copilot.logoutAll", "Logout all accounts")}
          </Button>
        )}
    </div>
  );
};

const CopilotAccountAvatar: React.FC<{ account: GitHubAccount }> = ({
  account,
}) => {
  const [failed, setFailed] = React.useState(false);

  if (!account.avatar_url || failed) {
    return <User className="h-5 w-5 text-muted-foreground" />;
  }

  return (
    <img
      src={account.avatar_url}
      alt={account.login}
      className="h-5 w-5 rounded-full"
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setFailed(true)}
    />
  );
};
