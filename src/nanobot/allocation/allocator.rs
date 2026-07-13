//! Pure regional allocation decisions.
//!
//! These APIs consume projection snapshots and produce deterministic decisions.
//! They do not mutate category-specific work components; the runtime cutover can
//! adapt accepted decisions to those components later.

use std::{cmp::Reverse, collections::BTreeMap, time::Duration};

use bevy::prelude::Resource;

use super::{ActionableOpportunity, AllocationRegion, OpportunityCategory};
use crate::nanobot::SwarmId;

/// Allocation period mandated by ADR-0009 (10 Hz).
pub const ALLOCATION_TICK_PERIOD: Duration = Duration::from_millis(100);

/// Manually advanced deterministic allocation clock.
#[derive(Debug, Default, Resource)]
pub struct AllocationClock {
    tick: u64,
    remainder: Duration,
}

impl AllocationClock {
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Advance simulation time and return the number of elapsed 10 Hz ticks.
    pub fn advance_by(&mut self, delta: Duration) -> u32 {
        self.remainder += delta;
        let elapsed = self.remainder.as_nanos() / ALLOCATION_TICK_PERIOD.as_nanos();
        let elapsed = elapsed.min(u128::from(u32::MAX)) as u32;
        if elapsed > 0 {
            self.remainder -= ALLOCATION_TICK_PERIOD * elapsed;
            self.tick = self.tick.saturating_add(u64::from(elapsed));
        }
        elapsed
    }
}

/// Stable category-indexed values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CategoryValues([u32; OpportunityCategory::COUNT]);

impl CategoryValues {
    pub const fn new(values: [u32; OpportunityCategory::COUNT]) -> Self {
        Self(values)
    }

    pub fn get(self, category: OpportunityCategory) -> u32 {
        self.0[category.index()]
    }

    pub fn set(&mut self, category: OpportunityCategory, value: u32) {
        self.0[category.index()] = value;
    }

    pub fn total(self) -> u32 {
        self.0.into_iter().fold(0, u32::saturating_add)
    }
}

/// Relative category pressure multipliers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryWeights(CategoryValues);

impl CategoryWeights {
    pub const fn new(values: [u32; OpportunityCategory::COUNT]) -> Self {
        Self(CategoryValues::new(values))
    }

    pub fn get(self, category: OpportunityCategory) -> u32 {
        self.0.get(category)
    }
}

impl Default for CategoryWeights {
    fn default() -> Self {
        Self::new([1; OpportunityCategory::COUNT])
    }
}

/// Work pressure in one allocation region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionalPressure {
    pub region: AllocationRegion,
    pub categories: CategoryValues,
}

/// Capacity requested from bots currently in `source_region`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionalPullBudget {
    pub source_region: AllocationRegion,
    pub categories: CategoryValues,
}

/// Convert projected work into weighted, saturating pressure.
pub fn regional_pressure(
    region: AllocationRegion,
    opportunities: &[ActionableOpportunity],
    weights: CategoryWeights,
) -> RegionalPressure {
    let mut categories = CategoryValues::default();
    for opportunity in opportunities {
        let contribution = opportunity
            .available_work
            .saturating_mul(weights.get(opportunity.category));
        categories.set(
            opportunity.category,
            categories
                .get(opportunity.category)
                .saturating_add(contribution),
        );
    }
    RegionalPressure { region, categories }
}

/// Allocate capacity across every active category before distributing the
/// remainder proportionally by weighted pressure.
pub fn allocate_category_budget(capacity: u32, pressure: CategoryValues) -> CategoryValues {
    let active = OpportunityCategory::ALL
        .into_iter()
        .filter(|category| pressure.get(*category) > 0)
        .collect::<Vec<_>>();
    let mut budget = CategoryValues::default();
    let activated = capacity.min(active.len() as u32);
    for category in active.iter().take(activated as usize) {
        budget.set(*category, 1);
    }

    let remaining = capacity - activated;
    let total_pressure = active
        .iter()
        .map(|category| u64::from(pressure.get(*category)))
        .sum::<u64>();
    if remaining == 0 || total_pressure == 0 {
        return budget;
    }

    let mut assigned = 0;
    let mut remainders = Vec::with_capacity(active.len());
    for category in active {
        let numerator = u64::from(remaining) * u64::from(pressure.get(category));
        let share = (numerator / total_pressure) as u32;
        budget.set(category, budget.get(category) + share);
        assigned += share;
        remainders.push((numerator % total_pressure, Reverse(category), category));
    }
    remainders.sort_by(|left, right| right.cmp(left));
    for (_, _, category) in remainders.into_iter().take((remaining - assigned) as usize) {
        budget.set(category, budget.get(category) + 1);
    }
    budget
}

/// Build a local pull budget from work in concentric Manhattan rings.
/// Distance attenuation keeps local work preferred while valid distant work
/// still requests capacity.
pub fn outward_pull_budget(
    source_region: AllocationRegion,
    capacity: u32,
    pressures: &[RegionalPressure],
    max_radius: u32,
) -> RegionalPullBudget {
    let mut combined = CategoryValues::default();
    let mut ordered = pressures.to_vec();
    ordered.sort_by_key(|pressure| {
        (
            region_distance(source_region, pressure.region),
            pressure.region.y,
            pressure.region.x,
        )
    });
    for pressure in ordered {
        let distance = region_distance(source_region, pressure.region);
        if distance > max_radius {
            continue;
        }
        for category in OpportunityCategory::ALL {
            let raw = pressure.categories.get(category);
            let attenuated = raw.saturating_add(distance) / (distance + 1);
            combined.set(category, combined.get(category).saturating_add(attenuated));
        }
    }
    RegionalPullBudget {
        source_region,
        categories: allocate_category_budget(capacity, combined),
    }
}

/// Build deterministic pull budgets for every supplied capacity region.
pub fn outward_pull_budgets(
    capacities: &[(AllocationRegion, u32)],
    pressures: &[RegionalPressure],
    max_radius: u32,
) -> BTreeMap<AllocationRegion, RegionalPullBudget> {
    capacities
        .iter()
        .map(|(region, capacity)| {
            (
                *region,
                outward_pull_budget(*region, *capacity, pressures, max_radius),
            )
        })
        .collect()
}

/// Hard bounds for one bot's local choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateBounds {
    pub max_regions: usize,
    pub max_candidates: usize,
}

/// Category eligibility adapter for later nanobot-type integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryEligibility([bool; OpportunityCategory::COUNT]);

impl CategoryEligibility {
    pub const fn all() -> Self {
        Self([true; OpportunityCategory::COUNT])
    }

    pub const fn only(category: OpportunityCategory) -> Self {
        let mut values = [false; OpportunityCategory::COUNT];
        values[category.index()] = true;
        Self(values)
    }

    pub const fn worker() -> Self {
        let mut values = [false; OpportunityCategory::COUNT];
        values[OpportunityCategory::Gather.index()] = true;
        values[OpportunityCategory::PlannedBuild.index()] = true;
        values[OpportunityCategory::Maintenance.index()] = true;
        Self(values)
    }

    pub fn allows(self, category: OpportunityCategory) -> bool {
        self.0[category.index()]
    }
}

/// Category-neutral bot snapshot supplied by a future runtime adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocationCandidate {
    pub entity_bits: u64,
    pub region: AllocationRegion,
    pub owner: Option<SwarmId>,
    pub eligibility: CategoryEligibility,
}

/// Observable result of a bounded local search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateDecision {
    pub opportunity: ActionableOpportunity,
    pub regions_examined: usize,
    pub candidates_examined: usize,
}

/// Choose deterministically from bounded nearby projection buckets. The pull
/// budget gates categories, while exact work remains unclaimed until the
/// caller accepts this decision.
pub fn choose_bounded_candidate<'a>(
    bot: AllocationCandidate,
    pull: RegionalPullBudget,
    regions: impl IntoIterator<Item = (AllocationRegion, &'a [ActionableOpportunity])>,
    bounds: CandidateBounds,
) -> Option<CandidateDecision> {
    choose_bounded_candidate_with_claims(bot, pull, regions, bounds, |_| Some(0))
}

/// Choose bounded work while accounting for exact claims supplied by caller.
/// `claim_count` returns `None` for work that cannot currently be accepted.
pub fn choose_bounded_candidate_with_claims<'a, F>(
    bot: AllocationCandidate,
    pull: RegionalPullBudget,
    regions: impl IntoIterator<Item = (AllocationRegion, &'a [ActionableOpportunity])>,
    bounds: CandidateBounds,
    claim_count: F,
) -> Option<CandidateDecision>
where
    F: FnMut(ActionableOpportunity) -> Option<usize>,
{
    let mut by_distance = regions.into_iter().collect::<Vec<_>>();
    by_distance
        .sort_by_key(|(region, _)| (region_distance(bot.region, *region), region.y, region.x));
    choose_bounded_candidate_from_ordered_regions_with_claims(
        bot,
        pull,
        by_distance,
        bounds,
        claim_count,
    )
}

/// Choose bounded work from region buckets already ordered nearest-first.
/// Runtime adapters use this form to avoid sorting all active regions per bot.
#[allow(clippy::type_complexity)]
pub fn choose_bounded_candidate_from_ordered_regions_with_claims<'a, F>(
    bot: AllocationCandidate,
    pull: RegionalPullBudget,
    ordered_regions: impl IntoIterator<Item = (AllocationRegion, &'a [ActionableOpportunity])>,
    bounds: CandidateBounds,
    mut claim_count: F,
) -> Option<CandidateDecision>
where
    F: FnMut(ActionableOpportunity) -> Option<usize>,
{
    if bounds.max_regions == 0 || bounds.max_candidates == 0 {
        return None;
    }

    let mut examined = 0;
    let mut best: Option<((usize, usize, u64, u32, usize), ActionableOpportunity)> = None;
    let mut regions_examined = 0;
    for (region, opportunities) in ordered_regions.into_iter().take(bounds.max_regions) {
        regions_examined += 1;
        let distance = region_distance(bot.region, region);
        for opportunity in opportunities {
            if examined == bounds.max_candidates {
                break;
            }
            examined += 1;
            if pull.categories.get(opportunity.category) == 0
                || !bot.eligibility.allows(opportunity.category)
                || !owners_compatible(bot.owner, opportunity.owner)
            {
                continue;
            }
            let Some(claims) = claim_count(*opportunity) else {
                continue;
            };
            let pressure = opportunity.available_work;
            let score = (
                category_priority(opportunity.category),
                claims,
                u64::from(distance),
                u32::MAX - pressure,
                examined,
            );
            if best.as_ref().is_none_or(|(current, _)| score < *current) {
                best = Some((score, *opportunity));
            }
        }
        if examined == bounds.max_candidates {
            break;
        }
    }
    best.map(|(_, opportunity)| CandidateDecision {
        opportunity,
        regions_examined,
        candidates_examined: examined,
    })
}

fn category_priority(category: OpportunityCategory) -> usize {
    match category {
        OpportunityCategory::PlannedBuild => 0,
        OpportunityCategory::Maintenance => 1,
        OpportunityCategory::Gather => 2,
        OpportunityCategory::Defend => 3,
        OpportunityCategory::Haul => 4,
    }
}

/// Per-region reassignment burst policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReassignmentPolicy {
    pub percentage_basis_points: u32,
    pub floor: u32,
}

impl ReassignmentPolicy {
    pub fn limit(self, region_population: u32) -> u32 {
        let percentage = region_population
            .saturating_mul(self.percentage_basis_points)
            .saturating_add(9_999)
            / 10_000;
        percentage.max(self.floor).min(region_population)
    }
}

/// Candidate and chosen work produced by one regional allocation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionalAllocationDecision {
    pub candidate: AllocationCandidate,
    pub choice: CandidateDecision,
}

/// Apply stable bot tie-breaking, the regional reassignment cap, pull-budget
/// consumption, and bounded local choice in one pure pass.
pub fn allocate_regional_candidates(
    region_population: u32,
    candidates: &[AllocationCandidate],
    mut pull: RegionalPullBudget,
    regions: &[(AllocationRegion, &[ActionableOpportunity])],
    bounds: CandidateBounds,
    reassignment: ReassignmentPolicy,
) -> Vec<RegionalAllocationDecision> {
    let mut ordered = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.region == pull.source_region)
        .collect::<Vec<_>>();
    ordered.sort_by_key(|candidate| candidate.entity_bits);
    let limit = reassignment.limit(region_population) as usize;
    let mut decisions = Vec::with_capacity(limit.min(ordered.len()));
    for candidate in ordered.into_iter().take(limit) {
        let Some(choice) =
            choose_bounded_candidate(candidate, pull, regions.iter().copied(), bounds)
        else {
            continue;
        };
        let category = choice.opportunity.category;
        pull.categories
            .set(category, pull.categories.get(category).saturating_sub(1));
        decisions.push(RegionalAllocationDecision { candidate, choice });
    }
    decisions
}

fn owners_compatible(left: Option<SwarmId>, right: Option<SwarmId>) -> bool {
    left.is_none() || right.is_none() || left == right
}

fn region_distance(left: AllocationRegion, right: AllocationRegion) -> u32 {
    left.x.abs_diff(right.x) + left.y.abs_diff(right.y)
}

/// Build pressure snapshots without exposing projection internals.
pub fn pressure_map<'a>(
    regions: impl IntoIterator<Item = (AllocationRegion, &'a [ActionableOpportunity])>,
    weights: CategoryWeights,
) -> BTreeMap<AllocationRegion, RegionalPressure> {
    regions
        .into_iter()
        .map(|(region, opportunities)| (region, regional_pressure(region, opportunities, weights)))
        .collect()
}
