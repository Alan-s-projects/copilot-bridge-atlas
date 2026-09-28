import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Save, Plus } from "lucide-react";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useUpdateModelPricing } from "@/lib/query/usage";
import { isNonNegativeDecimalString, type ModelPricing } from "@/types/usage";
import { isValidModelId } from "@/utils/codexModelCatalog";

interface PricingEditModalProps {
  open: boolean;
  model: ModelPricing;
  isNew?: boolean;
  onClose: () => void;
}

const PRICE_INPUT_STEP = "0.0001";

export function PricingEditModal({
  open,
  model,
  isNew = false,
  onClose,
}: PricingEditModalProps) {
  const { t } = useTranslation();
  const updatePricing = useUpdateModelPricing();
  const [longContext, setLongContext] = useState(model.longContext);

  const [formData, setFormData] = useState({
    modelId: model.modelId,
    displayName: model.displayName,
    inputCost: model.inputCostPerMillion,
    outputCost: model.outputCostPerMillion,
    cacheReadCost: model.cacheReadCostPerMillion,
    cacheCreationCost: model.cacheCreationCostPerMillion,
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    // Validate model ID
    if (isNew && !formData.modelId.trim()) {
      toast.error(t("usage.modelIdRequired", "Model ID is required"));
      return;
    }
    if (!isValidModelId(formData.modelId)) {
      toast.error("Model IDs must not contain whitespace.");
      return;
    }
    if (
      longContext &&
      (!Number.isSafeInteger(longContext.thresholdInputTokens) ||
        longContext.thresholdInputTokens <= 0)
    ) {
      toast.error(
        "Long-context input threshold must be a positive whole number.",
      );
      return;
    }

    // Verify non-negative number
    const values = [
      formData.inputCost,
      formData.outputCost,
      formData.cacheReadCost,
      formData.cacheCreationCost,
      ...(longContext
        ? [
            longContext.inputCostPerMillion,
            longContext.outputCostPerMillion,
            longContext.cacheReadCostPerMillion,
            longContext.cacheCreationCostPerMillion,
          ]
        : []),
    ];

    for (const value of values) {
      if (!isNonNegativeDecimalString(value)) {
        toast.error(t("usage.invalidPrice", "Price must be non-negative"));
        return;
      }
    }

    try {
      await updatePricing.mutateAsync({
        modelId: formData.modelId.trim(),
        displayName: formData.displayName,
        inputCost: formData.inputCost,
        outputCost: formData.outputCost,
        cacheReadCost: formData.cacheReadCost,
        cacheCreationCost: formData.cacheCreationCost,
        longContext,
      });

      toast.success(
        isNew
          ? t("usage.pricingAdded", "Pricing added")
          : t("usage.pricingUpdated", "Pricing updated"),
        { closeButton: true },
      );

      onClose();
    } catch (error) {
      toast.error(String(error));
    }
  };

  return (
    <FullScreenPanel
      isOpen={open}
      title={
        isNew
          ? t("usage.addPricing", "Add Pricing")
          : `${t("usage.editPricing", "Edit Pricing")} - ${model.modelId}`
      }
      onClose={onClose}
      footer={
        <Button
          type="submit"
          form="pricing-form"
          disabled={updatePricing.isPending}
        >
          {isNew ? (
            <Plus className="h-4 w-4 mr-2" />
          ) : (
            <Save className="h-4 w-4 mr-2" />
          )}
          {updatePricing.isPending
            ? t("common.saving", "Saving...")
            : isNew
              ? t("common.add", "Add")
              : t("common.save", "Save")}
        </Button>
      }
    >
      <form id="pricing-form" onSubmit={handleSubmit} className="space-y-6">
        {isNew && (
          <div className="space-y-2">
            <Label htmlFor="modelId">{t("usage.modelId", "Model ID")}</Label>
            <Input
              id="modelId"
              value={formData.modelId}
              onChange={(e) =>
                setFormData({ ...formData, modelId: e.target.value })
              }
              placeholder={t("usage.modelIdPlaceholder", {
                defaultValue: "For example: gpt-6-astra",
              })}
              required
            />
          </div>
        )}

        <div className="space-y-2">
          <Label htmlFor="displayName">
            {t("usage.displayName", "Display Name")}
          </Label>
          <Input
            id="displayName"
            value={formData.displayName}
            onChange={(e) =>
              setFormData({ ...formData, displayName: e.target.value })
            }
            placeholder={t("usage.displayNamePlaceholder", {
              defaultValue: "For example: GPT-6 Astra",
            })}
            required
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="inputCost">
            {t(
              "usage.inputCostPerMillion",
              "Input Cost (per million tokens, USD)",
            )}
          </Label>
          <Input
            id="inputCost"
            type="number"
            step={PRICE_INPUT_STEP}
            min="0"
            value={formData.inputCost}
            onChange={(e) =>
              setFormData({ ...formData, inputCost: e.target.value })
            }
            required
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="outputCost">
            {t(
              "usage.outputCostPerMillion",
              "Output Cost (per million tokens, USD)",
            )}
          </Label>
          <Input
            id="outputCost"
            type="number"
            step={PRICE_INPUT_STEP}
            min="0"
            value={formData.outputCost}
            onChange={(e) =>
              setFormData({ ...formData, outputCost: e.target.value })
            }
            required
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="cacheReadCost">
            {t(
              "usage.cacheReadCostPerMillion",
              "Cache Read Cost (per million tokens, USD)",
            )}
          </Label>
          <Input
            id="cacheReadCost"
            type="number"
            step={PRICE_INPUT_STEP}
            min="0"
            value={formData.cacheReadCost}
            onChange={(e) =>
              setFormData({ ...formData, cacheReadCost: e.target.value })
            }
            required
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="cacheCreationCost">
            {t(
              "usage.cacheCreationCostPerMillion",
              "Cache Write Cost (per million tokens, USD)",
            )}
          </Label>
          <Input
            id="cacheCreationCost"
            type="number"
            step={PRICE_INPUT_STEP}
            min="0"
            value={formData.cacheCreationCost}
            onChange={(e) =>
              setFormData({ ...formData, cacheCreationCost: e.target.value })
            }
            required
          />
        </div>
        <section className="space-y-4 border-t pt-4">
          <div className="flex items-center justify-between gap-4">
            <Label htmlFor="long-context-pricing">Long-context pricing</Label>
            <Switch
              id="long-context-pricing"
              checked={!!longContext}
              onCheckedChange={(enabled) =>
                setLongContext(
                  enabled
                    ? {
                        thresholdInputTokens: 272000,
                        inputCostPerMillion: formData.inputCost,
                        outputCostPerMillion: formData.outputCost,
                        cacheReadCostPerMillion: formData.cacheReadCost,
                        cacheCreationCostPerMillion: formData.cacheCreationCost,
                      }
                    : undefined,
                )
              }
            />
          </div>
          {longContext && (
            <fieldset className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="long-context-threshold">
                  Input tokens (strictly above)
                </Label>
                <Input
                  id="long-context-threshold"
                  type="number"
                  min="1"
                  step="1"
                  required
                  value={longContext.thresholdInputTokens || ""}
                  onChange={(event) =>
                    setLongContext({
                      ...longContext,
                      thresholdInputTokens: Number(event.target.value),
                    })
                  }
                />
              </div>
              {(
                [
                  ["inputCostPerMillion", "Input"],
                  ["outputCostPerMillion", "Output"],
                  ["cacheReadCostPerMillion", "Cached input"],
                  ["cacheCreationCostPerMillion", "Cache write"],
                ] as const
              ).map(([key, label]) => (
                <div key={key} className="space-y-2">
                  <Label htmlFor={`long-${key}`}>{label} (USD / 1M)</Label>
                  <Input
                    id={`long-${key}`}
                    type="number"
                    min="0"
                    step={PRICE_INPUT_STEP}
                    value={longContext[key]}
                    required
                    onChange={(event) =>
                      setLongContext({
                        ...longContext,
                        [key]: event.target.value,
                      })
                    }
                  />
                </div>
              ))}
            </fieldset>
          )}
        </section>
      </form>
    </FullScreenPanel>
  );
}
