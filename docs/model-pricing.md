# GitHub Copilot Pricing

Atlas bundles 34 model/mode entries from GitHub's
[Models and pricing](https://docs.github.com/en/copilot/reference/copilot-billing/models-and-pricing)
table. The initial 33 entries were verified September 27, 2026; GPT-6.1 Sol
was added using the rates verified October 8, 2026.
The snapshot is compiled from `src-tauri/src/resources/model-pricing.json`;
Atlas does not fetch or scrape pricing at runtime.

Prices are decimal USD per one million tokens. They estimate token charges,
not the final subscription bill, included allowance, tax, or annual-plan
request-based billing. GitHub defines one AI credit as USD 0.01.

## Long Context

The `longContext` map records 12 published long-context tiers:

- 272,000 input tokens: GPT-5.4, GPT-5.5, GPT-5.6 Sol/Terra, GPT-6 Astra/Luna/Sol,
  and GPT-6.1 Sol.
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

When a new bundled price is inserted at startup or when loading Cost Pricing,
Atlas applies local overrides and deletions first, then fills missing costs in
retained request logs. Requests with a recorded price tier or positive cost
remain unchanged. Daily rollups cannot be repriced once their individual
requests have been pruned.

Models absent from the price list remain usable and are reported as unpriced.
Lookup does not borrow a price from a different model. Existing GPT date and
reasoning aliases still resolve to their explicitly priced base model.

## GPT-6.1 Sol

GPT-6.1 Sol has its own entry; its cached-input price is not inherited from
GPT-6 Sol. Rates are USD per one million tokens:

| Total input per request | Fresh input | Output | Cached input | Cache write |
| --- | ---: | ---: | ---: | ---: |
| Up to 272,000 tokens | $2.00 | $10.00 | $0.10 | $2.50 |
| Above 272,000 tokens | $4.00 | $15.00 | $0.20 | $5.00 |

## Snapshot Limits

GitHub's Gemini 3.6/3.7/3.8 Flash rates are promotional through December 31, 2026.
The snapshot needs a reviewed update when that promotion or any published rate
changes. Cache writes listed as not applicable, or absent from a vendor's table,
have no separate charge. No unpublished storage, audio, batch, or tool fees are
invented.
