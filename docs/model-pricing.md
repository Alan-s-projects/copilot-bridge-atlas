# GitHub Copilot Pricing

Atlas bundles 35 model/mode entries from GitHub's
[Models and pricing](https://docs.github.com/en/copilot/reference/copilot-billing/models-and-pricing)
table. The initial 33 entries were verified September 27, 2026; GPT-6.1 Sol
was added using the rates verified October 8, 2026, and GPT-6.1 Terra using
the rates verified October 29, 2026.
The snapshot is compiled from `src-tauri/src/resources/model-pricing.json`;
Atlas does not fetch or scrape pricing at runtime. A separate Microsoft Decision 1
Foundry reference estimate is bundled as described below (36 entries total).

Prices are decimal USD per one million tokens. They estimate token charges,
not the final subscription bill, included allowance, tax, or annual-plan
request-based billing. GitHub defines one AI credit as USD 0.01.

## Long Context

The `longContext` map records 13 published long-context tiers:

- 272,000 input tokens: GPT-5.4, GPT-5.5, GPT-5.6 Sol/Terra, GPT-6 Astra/Luna/Sol,
  and GPT-6.1 Sol/Terra.
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

## GPT-6.1 Terra

GPT-6.1 Terra is priced separately from GPT-6.1 Sol and GPT-5.6 Terra.
Rates verified against GitHub's Copilot pricing table on October 29, 2026,
in USD per one million tokens:

| Total input per request | Fresh input | Output | Cached input | Cache write |
| --- | ---: | ---: | ---: | ---: |
| Up to 272,000 tokens | $1.00 | $5.00 | $0.05 | $1.25 |
| Above 272,000 tokens | $2.00 | $7.50 | $0.10 | $2.50 |

## Snapshot Limits

GitHub's Gemini 3.6/3.7/3.8 Flash rates are promotional through December 31, 2026.
The snapshot needs a reviewed update when that promotion or any published rate
changes. Cache writes listed as not applicable, or absent from a vendor's table,
have no separate charge. No unpublished storage, audio, batch, or tool fees are
invented.

## Microsoft Decision 1 reference estimate

`microsoft-decision-1` uses **Microsoft Foundry reference pricing**, not a verified
GitHub Copilot rate: **$0.042 per million input tokens and $0 per million output
tokens**, verified **2026-11-01** against [Microsoft's official Foundry announcement](https://commandline.microsoft.com/microsoft-decision-1-model-foundry/).
**Actual Copilot price unpublished.** The old catalog multiplier of 1 is obsolete
for this usage-based account and is not a USD/token rate.

The optional bundled `metadata` map preserves the existing six-string price rows
and records source, verification date, and `all_input_at_base_rate` policy.
This policy bills reported total input once, including cached input, at the base
input rate. It neither subtracts cached tokens nor adds separate cache charges.
Legacy fresh-input rows reconstruct total input from their disjoint fresh/cache
buckets. At multiplier 1, 28 input tokens and 1 output token cost **$0.000001176**.
Other model calculations and rates are unchanged.

Cost Pricing (not Routing endpoint details) shows this source and warning.
Numeric edits become **User override — not publisher verified**, retain the
explicit policy, and clear the source verification claim. The output override is
honored; cache rates are ignored under this policy. Older local overrides without
metadata inherit this model's policy without changing their numeric rates.
Reset restores reference metadata; deletion tombstones remain effective across
reseeding. Metadata is current pricing configuration only: historical request
costs are not labeled with today's source. Only missing retained costs are
backfilled, at full decimal precision; existing costs and pruned totals stay intact.
