use std::time::Duration;

use bevy::prelude::*;
use top_down_2d_rts_prototype_nano_swarm::{
    intent::{IntentGrid, IntentKind},
    nanobot::{
        ActionableOpportunity, ActionableProjection, AllocationCandidate, AllocationClock,
        AllocationRegion, CandidateBounds, CategoryEligibility, CategoryValues, LeaseDecision,
        OpportunityCategory, OpportunityTarget, REGIONAL_FAIRNESS_PROMOTION_TICKS,
        RUNTIME_MAX_CANDIDATES, ReassignmentPolicy, RegionalLease, RegionalLeaseConfig,
        RegionalLeaseState, RegionalPressure, allocate_category_budget,
        allocate_regional_candidates, choose_bounded_candidate,
        choose_bounded_candidate_with_claims, evaluate_lease, maintain_regional_leases_system,
        outward_pull_budget, project_actionable_opportunities_system, region_fairness_sort_key,
    },
};

fn region(x: i32, y: i32) -> AllocationRegion {
    AllocationRegion { x, y }
}

fn defend_opportunity(
    region: AllocationRegion,
    cell: IVec2,
    available_work: u32,
) -> ActionableOpportunity {
    ActionableOpportunity {
        region,
        category: OpportunityCategory::Defend,
        target: OpportunityTarget::Defend { cell },
        cell,
        owner: None,
        available_work,
    }
}

#[test]
fn allocation_clock_advances_at_exact_deterministic_ten_hertz_boundaries() {
    let mut clock = AllocationClock::default();

    assert_eq!(clock.advance_by(Duration::from_millis(99)), 0);
    assert_eq!(clock.tick(), 0);
    assert_eq!(clock.advance_by(Duration::from_millis(1)), 1);
    assert_eq!(clock.advance_by(Duration::from_millis(250)), 2);
    assert_eq!(clock.tick(), 3);
    assert_eq!(clock.advance_by(Duration::from_millis(50)), 1);
    assert_eq!(clock.tick(), 4);
}

#[test]
fn minimum_activation_covers_all_five_categories_before_weighted_pressure() {
    let pressure = CategoryValues::new([100, 1, 1, 1, 1]);

    let budget = allocate_category_budget(9, pressure);

    assert_eq!(budget.total(), 9);
    assert_eq!(budget.get(OpportunityCategory::Gather), 5);
    for category in [
        OpportunityCategory::PlannedBuild,
        OpportunityCategory::Maintenance,
        OpportunityCategory::Defend,
        OpportunityCategory::Haul,
    ] {
        assert_eq!(budget.get(category), 1);
    }
}

#[test]
fn one_worker_activates_planned_build_before_gather_pressure() {
    let pressure = CategoryValues::new([100, 1, 0, 0, 0]);

    let budget = allocate_category_budget(1, pressure);

    assert_eq!(budget.total(), 1);
    assert_eq!(budget.get(OpportunityCategory::PlannedBuild), 1);
    assert_eq!(budget.get(OpportunityCategory::Gather), 0);
}

#[test]
fn distant_valid_work_pulls_capacity_outward_within_the_bound() {
    let pressure = RegionalPressure {
        region: region(2, 0),
        categories: CategoryValues::new([0, 0, 0, 0, 1]),
    };

    let in_range = outward_pull_budget(region(0, 0), 3, &[pressure], 2);
    let out_of_range = outward_pull_budget(region(0, 0), 3, &[pressure], 1);

    assert_eq!(in_range.categories.get(OpportunityCategory::Haul), 3);
    assert_eq!(out_of_range.categories.total(), 0);
}

#[test]
fn local_choice_obeys_region_candidate_owner_and_category_bounds() {
    let local = [defend_opportunity(region(0, 0), IVec2::ZERO, 1)];
    let far = [defend_opportunity(region(2, 0), IVec2::new(16, 0), 9)];
    let pull = outward_pull_budget(
        region(0, 0),
        1,
        &[RegionalPressure {
            region: region(0, 0),
            categories: CategoryValues::new([0, 0, 0, 1, 0]),
        }],
        0,
    );
    let bot = AllocationCandidate {
        entity_bits: 7,
        region: region(0, 0),
        owner: None,
        eligibility: CategoryEligibility::only(OpportunityCategory::Defend),
    };

    let decision = choose_bounded_candidate(
        bot,
        pull,
        [
            (region(2, 0), far.as_slice()),
            (region(0, 0), local.as_slice()),
        ],
        CandidateBounds {
            max_regions: 1,
            max_candidates: 1,
        },
    )
    .expect("local defend work is eligible");

    assert_eq!(decision.opportunity.cell, IVec2::ZERO);
    assert_eq!(decision.regions_examined, 1);
    assert_eq!(decision.candidates_examined, 1);
}

#[test]
fn defend_threat_pressure_beats_distance_after_claims() {
    let calm = [defend_opportunity(region(0, 0), IVec2::ZERO, 1)];
    let hot = [defend_opportunity(region(1, 0), IVec2::new(8, 0), 3)];
    let pull = outward_pull_budget(
        region(0, 0),
        1,
        &[RegionalPressure {
            region: region(0, 0),
            categories: CategoryValues::new([0, 0, 0, 1, 0]),
        }],
        1,
    );
    let bot = AllocationCandidate {
        entity_bits: 7,
        region: region(0, 0),
        owner: None,
        eligibility: CategoryEligibility::only(OpportunityCategory::Defend),
    };

    let decision = choose_bounded_candidate_with_claims(
        bot,
        pull,
        [
            (region(0, 0), calm.as_slice()),
            (region(1, 0), hot.as_slice()),
        ],
        CandidateBounds {
            max_regions: 2,
            max_candidates: 2,
        },
        |_| Some(0),
    )
    .expect("Defend work available");

    assert_eq!(decision.opportunity.cell, IVec2::new(8, 0));
}

#[test]
fn bounded_choice_falls_back_from_full_exact_claims_within_runtime_limit() {
    let work = (0..256)
        .map(|x| defend_opportunity(region(0, 0), IVec2::new(x, 0), 1))
        .collect::<Vec<_>>();
    let pull = outward_pull_budget(
        region(0, 0),
        1,
        &[RegionalPressure {
            region: region(0, 0),
            categories: CategoryValues::new([0, 0, 0, 1, 0]),
        }],
        0,
    );
    let bot = AllocationCandidate {
        entity_bits: 7,
        region: region(0, 0),
        owner: None,
        eligibility: CategoryEligibility::only(OpportunityCategory::Defend),
    };
    let mut examined_by_adapter = 0;

    let decision = choose_bounded_candidate_with_claims(
        bot,
        pull,
        [(region(0, 0), work.as_slice())],
        CandidateBounds {
            max_regions: 1,
            max_candidates: RUNTIME_MAX_CANDIDATES,
        },
        |opportunity| {
            examined_by_adapter += 1;
            (opportunity.cell.x == RUNTIME_MAX_CANDIDATES as i32 - 1).then_some(0)
        },
    )
    .expect("last bounded opportunity remains claimable");

    assert_eq!(
        decision.opportunity.cell.x,
        RUNTIME_MAX_CANDIDATES as i32 - 1
    );
    assert_eq!(decision.candidates_examined, RUNTIME_MAX_CANDIDATES);
    assert_eq!(examined_by_adapter, RUNTIME_MAX_CANDIDATES);
}

#[test]
fn reassignment_burst_uses_percentage_with_a_small_floor() {
    let policy = ReassignmentPolicy {
        percentage_basis_points: 1_000,
        floor: 2,
    };

    assert_eq!(policy.limit(5), 2);
    assert_eq!(policy.limit(100), 10);
    assert_eq!(policy.limit(1), 1);
}

#[test]
fn regional_pass_uses_stable_bot_order_and_reassignment_limit() {
    let work = [defend_opportunity(region(0, 0), IVec2::ZERO, 1)];
    let candidates = [
        AllocationCandidate {
            entity_bits: 9,
            region: region(0, 0),
            owner: None,
            eligibility: CategoryEligibility::all(),
        },
        AllocationCandidate {
            entity_bits: 2,
            region: region(0, 0),
            owner: None,
            eligibility: CategoryEligibility::all(),
        },
    ];
    let pull = outward_pull_budget(
        region(0, 0),
        2,
        &[RegionalPressure {
            region: region(0, 0),
            categories: CategoryValues::new([0, 0, 0, 2, 0]),
        }],
        0,
    );

    let decisions = allocate_regional_candidates(
        20,
        &candidates,
        pull,
        &[(region(0, 0), work.as_slice())],
        CandidateBounds {
            max_regions: 1,
            max_candidates: 1,
        },
        ReassignmentPolicy {
            percentage_basis_points: 0,
            floor: 1,
        },
    );

    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].candidate.entity_bits, 2);
}

#[test]
fn progress_renews_a_lease_and_no_progress_expires_it() {
    let opportunity = defend_opportunity(region(0, 0), IVec2::ZERO, 1);
    let mut lease = RegionalLease::new(
        opportunity.region,
        opportunity.category,
        opportunity.target,
        None,
        0,
        0,
        3,
    );

    assert_eq!(
        evaluate_lease(&mut lease, 2, 1, true, 3),
        LeaseDecision::Keep
    );
    assert_eq!(lease.expires_at_tick(), 5);
    assert_eq!(
        evaluate_lease(&mut lease, 4, 1, true, 3),
        LeaseDecision::Keep
    );
    assert_eq!(
        evaluate_lease(&mut lease, 5, 1, true, 3),
        LeaseDecision::RevokeNoProgress
    );
}

#[test]
fn charge_suspension_requires_capacity_confirmation_to_resume() {
    let opportunity = defend_opportunity(region(0, 0), IVec2::ZERO, 1);
    let mut lease = RegionalLease::new(
        opportunity.region,
        opportunity.category,
        opportunity.target,
        None,
        0,
        0,
        3,
    );
    assert!(lease.counts_toward_capacity());

    lease.suspend_for_charge();
    assert_eq!(lease.state, RegionalLeaseState::SuspendedForCharge);
    assert!(!lease.counts_toward_capacity());
    lease.request_resume();
    assert_eq!(lease.state, RegionalLeaseState::ResumePending);
    assert!(!lease.counts_toward_capacity());
    assert!(!lease.activate_if_capacity(false));
    assert_eq!(lease.state, RegionalLeaseState::ResumePending);
    assert!(lease.activate_if_capacity(true));
    assert_eq!(lease.state, RegionalLeaseState::Active);
    assert!(lease.counts_toward_capacity());
}

#[test]
fn unsupported_opportunity_revokes_lease_in_the_same_app_update() {
    let mut app = App::new();
    app.insert_resource(IntentGrid::new(16, 16))
        .init_resource::<ActionableProjection>()
        .init_resource::<AllocationClock>()
        .init_resource::<RegionalLeaseConfig>()
        .add_systems(
            Update,
            (
                project_actionable_opportunities_system,
                maintain_regional_leases_system,
            )
                .chain(),
        );
    app.world_mut()
        .resource_mut::<IntentGrid>()
        .add(IVec2::ZERO, IntentKind::Defend);
    app.update();

    let opportunity = app
        .world()
        .resource::<ActionableProjection>()
        .opportunities(region(0, 0))[0];
    let bot = app
        .world_mut()
        .spawn(RegionalLease::new(
            opportunity.region,
            opportunity.category,
            opportunity.target,
            None,
            0,
            0,
            3,
        ))
        .id();
    app.update();
    assert!(app.world().entity(bot).contains::<RegionalLease>());

    app.world_mut()
        .resource_mut::<IntentGrid>()
        .remove(IVec2::ZERO, IntentKind::Defend);
    app.update();

    assert!(!app.world().entity(bot).contains::<RegionalLease>());
}

#[test]
fn starved_region_moves_ahead_of_near_regions_after_bounded_wait() {
    let source = region(0, 0);
    let far = region(30, 0);
    let mut regions = (0..17).map(|x| region(x, 0)).collect::<Vec<_>>();
    regions.push(far);
    regions.sort_by_key(|candidate| {
        let age = if *candidate == far {
            REGIONAL_FAIRNESS_PROMOTION_TICKS
        } else {
            0
        };
        region_fairness_sort_key(source, *candidate, age)
    });

    assert_eq!(regions[0], far);
}
