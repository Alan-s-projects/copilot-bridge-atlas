# GitHub Copilot Pricing

Atlas bundles the 33 model/mode entries listed in GitHub's
[Models and pricing](https://docs.github.com/en/copilot/reference/copilot-billing/models-and-pricing)
table, verified September 27, 2026. No unrelated vendor models are seeded.
The snapshot is compiled from `src-tauri/src/resources/model-pricing.json`;
Atlas does not fetch or scrape pricing at runtime.

Prices are decimal USD per one million tokens. They estimate token charges,
not the final subscription bill, included allowance, tax, or annual-plan
request-based billing. GitHub defines one AI credit as USD 0.01.

## Long Context

The `longContext` map records all 11 published long-context tiers:

- 272,000 input tokens: GPT-5.4, GPT-5.5, GPT-5.6 Sol/Terra, GPT-6 Astra/Luna/Sol.
- 200,000 input tokens: GPT-5.6 Luna and Grok 4.5/4.6/4.7.

The higher tier applies **strictly above** the threshold, to the entire request,
not just excess tokens. The threshold uses total input, including cached reads
and cache writes; output tokens do not select the tier. Fresh input, cached
input, cache writes, and output then use their respective rates in that tier.
This is shared by request logging and missing-cost backfill. Fresh and total
input semantics are normalized before choosing a tier.
Both request tables show `Pricing Tier` as `Default` or `Long context`, recorded
with the calculated cost. Errors and unpriced requests have no tier. Later price
edits do not relabel recorded tiers, including zero-cost requests.

The Cost Pricing table displays each long-context tier underneath its default
rate. The price editor saves both tiers, so editing a default does not silently
discard its long-context rates. Add and Edit both offer an optional long-context
switch, editable positive-integer input threshold, and all four rates, including
for models without a bundled tier. A custom flat price with no `longContext`
field remains flat.

## Overrides

Atlas 6 seeds prices from the bundled GitHub snapshot. Explicit local overrides
and deletion tombstones apply to the current installation. Reset to defaults
clears overrides and tombstones and restores the bundled entries and tiers.
Recorded request costs are not repriced.

Models absent from the price list remain usable and are reported as unpriced.
Lookup does not borrow a price from a different model. Existing GPT date and
reasoning aliases still resolve to their explicitly priced base model.

## Snapshot Limits

GitHub's Gemini 3.6/3.7/3.8 Flash rates are promotional through December 31, 2026.
The snapshot needs a reviewed update when that promotion or any published rate
changes. Cache writes listed as not applicable, or absent from a vendor's table,
have no separate charge. No unpublished storage, audio, batch, or tool fees are
invented.
