//! Cost attribution preserved independently of aggregate usage totals.
use std::collections::BTreeMap;
pub type ClientCosts = BTreeMap<String, f64>;

pub fn with_legacy_fallback(mut costs: ClientCosts, total: f64) -> ClientCosts {
    if costs.is_empty() {
        costs.insert("Unknown".into(), total);
    }
    costs
}

pub fn merge(target: &mut ClientCosts, source: ClientCosts) {
    for (client, cost) in source {
        *target.entry(client).or_default() += cost;
    }
}
