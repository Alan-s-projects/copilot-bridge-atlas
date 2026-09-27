import { Fragment, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useDeleteModelPricing,
  useModelPricing,
  useResetModelPricingToDefaults,
} from "@/lib/query/usage";
import { PricingEditModal } from "./PricingEditModal";
import type { ModelPricing } from "@/types/usage";
import { settingsApi } from "@/lib/api";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  ExternalLink,
  Loader2,
  Pencil,
  Plus,
  RotateCcw,
  Search,
  Trash2,
  X,
} from "lucide-react";

const BUILT_IN_PRICING_SOURCE_URL =
  "https://github.com/Alan-s-projects/copilot-bridge-atlas/blob/atlas/src-tauri/src/resources/model-pricing.json";

export function PricingConfigPanel() {
  const { t } = useTranslation();
  const { data: pricing, isLoading, error } = useModelPricing();
  const deleteMutation = useDeleteModelPricing();
  const resetMutation = useResetModelPricingToDefaults();
  const [editingModel, setEditingModel] = useState<ModelPricing | null>(null);
  const [isAddingNew, setIsAddingNew] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null);
  const [resetConfirm, setResetConfirm] = useState(false);
  const [search, setSearch] = useState("");
  const filteredPricing = useMemo(() => {
    const terms = search.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return (pricing ?? []).filter((model) => {
      const text = `${model.modelId} ${model.displayName}`.toLowerCase();
      return terms.every((term) => text.includes(term));
    });
  }, [pricing, search]);

  const openBuiltInPricingSource = () => {
    void settingsApi
      .openExternal(BUILT_IN_PRICING_SOURCE_URL)
      .catch((error) => {
        toast.error(String(error));
      });
  };

  const handleDelete = (modelId: string) => {
    deleteMutation.mutate(modelId, {
      onSuccess: () => setDeleteConfirm(null),
    });
  };

  const handleReset = () => {
    resetMutation.mutate(undefined, {
      onSuccess: () => {
        setResetConfirm(false);
        toast.success(
          t("usage.pricingReset", "Pricing reset to bundled defaults"),
        );
      },
      onError: (error) => toast.error(String(error)),
    });
  };

  const handleAddNew = () => {
    setIsAddingNew(true);
    setEditingModel({
      modelId: "",
      displayName: "",
      inputCostPerMillion: "0",
      outputCostPerMillion: "0",
      cacheReadCostPerMillion: "0",
      cacheCreationCostPerMillion: "0",
    });
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center p-4">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertDescription>
          {t("usage.loadPricingError")}: {String(error)}
        </AlertDescription>
      </Alert>
    );
  }

  return (
    <div className="space-y-6">
      {/* 模型定价配置 */}
      <div className="space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="relative w-full min-w-0 sm:w-72">
            <Search
              aria-hidden
              className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground"
            />
            <Input
              aria-label={t("usage.searchPricing", "Search model prices")}
              placeholder={t("usage.searchPricing", "Search model prices")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") setSearch("");
              }}
              className="h-9 pl-9 pr-9"
            />
            {search && (
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="absolute right-1 top-1 h-7 w-7"
                title={t("usage.clearPricingSearch", "Clear pricing search")}
                aria-label={t(
                  "usage.clearPricingSearch",
                  "Clear pricing search",
                )}
                onClick={() => setSearch("")}
              >
                <X aria-hidden className="h-3.5 w-3.5" />
              </Button>
            )}
          </div>
          <div className="flex flex-wrap items-center justify-end gap-2">
            <Button asChild variant="link" size="sm">
              <a
                href={BUILT_IN_PRICING_SOURCE_URL}
                target="_blank"
                rel="noreferrer"
                onClick={(event) => {
                  event.preventDefault();
                  openBuiltInPricingSource();
                }}
              >
                <ExternalLink className="h-4 w-4" />
                {t("usage.viewBuiltInPricing", "View built-in prices")}
              </a>
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={resetMutation.isPending}
              onClick={() => setResetConfirm(true)}
            >
              {resetMutation.isPending ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <RotateCcw className="h-4 w-4" />
              )}
              {t("usage.resetPricing", "Reset to code defaults")}
            </Button>
            <Button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                handleAddNew();
              }}
              size="sm"
            >
              <Plus className="h-4 w-4" />
              {t("common.add")}
            </Button>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">{t("usage.perMillion")}</p>

        <div className="space-y-4">
          {!pricing || pricing.length === 0 ? (
            <Alert>
              <AlertDescription>{t("usage.noPricingData")}</AlertDescription>
            </Alert>
          ) : (
            <div className="rounded-md bg-card/60 shadow-sm">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("usage.model")}</TableHead>
                    <TableHead>{t("usage.displayName")}</TableHead>
                    <TableHead className="text-right">
                      {t("usage.inputCost")}
                    </TableHead>
                    <TableHead className="text-right">
                      {t("usage.outputCost")}
                    </TableHead>
                    <TableHead className="text-right">
                      {t("usage.cacheReadCost")}
                    </TableHead>
                    <TableHead className="text-right">
                      {t("usage.cacheWriteCost")}
                    </TableHead>
                    <TableHead className="text-right">
                      {t("common.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filteredPricing.map((model) => (
                    <Fragment key={model.modelId}>
                      <TableRow>
                        <TableCell className="font-mono text-sm">
                          {model.modelId}
                        </TableCell>
                        <TableCell>{model.displayName}</TableCell>
                        <TableCell className="text-right font-mono text-sm">
                          ${model.inputCostPerMillion}
                        </TableCell>
                        <TableCell className="text-right font-mono text-sm">
                          ${model.outputCostPerMillion}
                        </TableCell>
                        <TableCell className="text-right font-mono text-sm">
                          ${model.cacheReadCostPerMillion}
                        </TableCell>
                        <TableCell className="text-right font-mono text-sm">
                          ${model.cacheCreationCostPerMillion}
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex justify-end gap-1">
                            <Button
                              variant="ghost"
                              size="icon"
                              onClick={() => {
                                setIsAddingNew(false);
                                setEditingModel(model);
                              }}
                              title={t("common.edit")}
                            >
                              <Pencil className="h-4 w-4" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              onClick={() => setDeleteConfirm(model.modelId)}
                              title={t("common.delete")}
                              className="text-destructive hover:text-destructive"
                            >
                              <Trash2 className="h-4 w-4" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                      {model.longContext && (
                        <TableRow className="text-muted-foreground">
                          <TableCell colSpan={2} className="pl-6 text-xs">
                            Long context: &gt;{" "}
                            {model.longContext.thresholdInputTokens.toLocaleString()}{" "}
                            input tokens
                          </TableCell>
                          {(
                            [
                              "inputCostPerMillion",
                              "outputCostPerMillion",
                              "cacheReadCostPerMillion",
                              "cacheCreationCostPerMillion",
                            ] as const
                          ).map((key) => (
                            <TableCell
                              key={key}
                              className="text-right font-mono text-sm"
                            >
                              ${model.longContext![key]}
                            </TableCell>
                          ))}
                          <TableCell />
                        </TableRow>
                      )}
                    </Fragment>
                  ))}
                  {filteredPricing.length === 0 && (
                    <TableRow>
                      <TableCell
                        colSpan={7}
                        className="py-8 text-center text-sm text-muted-foreground"
                      >
                        <span role="status">
                          {t(
                            "usage.noPricingMatches",
                            "No matching model prices.",
                          )}
                        </span>
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      </div>

      {editingModel && (
        <PricingEditModal
          open={!!editingModel}
          model={editingModel}
          isNew={isAddingNew}
          onClose={() => {
            setEditingModel(null);
            setIsAddingNew(false);
          }}
        />
      )}

      <Dialog open={resetConfirm} onOpenChange={setResetConfirm}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("usage.resetPricingTitle", "Reset prices to code defaults?")}
            </DialogTitle>
            <DialogDescription>
              {t(
                "usage.resetPricingDesc",
                "This removes all custom price overrides and deletion tombstones, then restores prices bundled with Atlas. Custom models without a bundled default will become unpriced. Retired metadata and previously recorded request costs are unchanged.",
              )}
              <a
                href={BUILT_IN_PRICING_SOURCE_URL}
                target="_blank"
                rel="noreferrer"
                className="mt-3 inline-flex items-center gap-1 text-blue-600 hover:underline dark:text-blue-400"
                onClick={(event) => {
                  event.preventDefault();
                  openBuiltInPricingSource();
                }}
              >
                {t(
                  "usage.viewBuiltInPricingFile",
                  "View bundled defaults in model-pricing.json",
                )}
                <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
              </a>
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={resetMutation.isPending}
              onClick={() => setResetConfirm(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={resetMutation.isPending}
              onClick={handleReset}
            >
              {resetMutation.isPending
                ? t("common.loading", "Loading...")
                : t("usage.resetPricingConfirm", "Reset prices")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={!!deleteConfirm}
        onOpenChange={() => setDeleteConfirm(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("usage.deleteConfirmTitle")}</DialogTitle>
            <DialogDescription>
              {t("usage.deleteConfirmDesc")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteConfirm(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={() => deleteConfirm && handleDelete(deleteConfirm)}
              disabled={deleteMutation.isPending}
            >
              {deleteMutation.isPending
                ? t("common.deleting")
                : t("common.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
