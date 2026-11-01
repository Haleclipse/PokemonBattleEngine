use std::collections::BTreeMap;

/// Deterministic coverage pool for differential-campaign generation.
///
/// A campaign should prefer the least exercised mechanics instead of sampling uniformly forever.
/// Ties are returned in lexical order; the campaign PRNG may choose among the tied candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveragePool {
    counts: BTreeMap<String, u64>,
}

impl CoveragePool {
    pub fn new(values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            counts: values.into_iter().map(|value| (value.into(), 0)).collect(),
        }
    }

    pub fn mark_used(&mut self, value: &str) {
        if let Some(count) = self.counts.get_mut(value) {
            *count = count.saturating_add(1);
        }
    }

    pub fn least_covered(&self) -> Vec<&str> {
        let Some(min) = self.counts.values().min().copied() else {
            return Vec::new();
        };
        self.counts
            .iter()
            .filter_map(|(value, count)| (*count == min).then_some(value.as_str()))
            .collect()
    }

    pub fn cycles_completed(&self) -> u64 {
        self.counts.values().min().copied().unwrap_or(0)
    }

    pub fn count(&self, value: &str) -> Option<u64> {
        self.counts.get(value).copied()
    }
}
