use std::collections::BinaryHeap;

use bevy::prelude::*;

use crate::{
    nanobot::{
        BuildAssignment, BuildProgress, BuildSite, Cargo, Charge, Charger, ChargerAssignment,
        ChargerProgress, ChargerPulseProgress, Commitment, DefenderAttackCooldown,
        DefenderResponse, DirectMovementComponent, ExtractProgress, GatherAssignment,
        HaulerAssignment, HaulerLoading, Health, LogisticsReservation, MaintenanceAssignment,
        MaintenanceProgress, Nanobot, NanobotType, OwnerSwarm, PlannedKind, PlannedStructure,
        PlannedStructureClaim, PlannedStructureProgress, ProductionFacility, Structure, SwarmId,
        SwarmMember, VelocityComponent, WorkBlocked,
    },
    resources::{ResourceDeposit, ResourceKind, Stockpile, StockpileRole},
};

pub(super) const EXECUTION_COLLECTION_LIMIT: usize = 256;

pub(super) fn collect(world: &mut World) -> serde_json::Value {
    serde_json::json!({
        "nanobots": collect_nanobots(world),
        "structures": collect_structures(world),
        "deposits": collect_deposits(world),
    })
}

fn lowest_entity_ids(entities: impl Iterator<Item = Entity>) -> (usize, Vec<Entity>) {
    let mut total = 0usize;
    let mut lowest = BinaryHeap::<(u64, Entity)>::with_capacity(EXECUTION_COLLECTION_LIMIT);
    for entity in entities {
        total += 1;
        let candidate = (entity.to_bits(), entity);
        if lowest.len() < EXECUTION_COLLECTION_LIMIT {
            lowest.push(candidate);
        } else if lowest.peek().is_some_and(|highest| candidate.0 < highest.0) {
            lowest.pop();
            lowest.push(candidate);
        }
    }
    let mut selected = lowest
        .into_iter()
        .map(|(_, entity)| entity)
        .collect::<Vec<_>>();
    selected.sort_by_key(|entity| entity.to_bits());
    (total, selected)
}

fn collection(total: usize, items: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "total": total,
        "truncated": total > items.len(),
        "items": items,
    })
}

fn position(transform: Option<&Transform>) -> serde_json::Value {
    transform.map_or(serde_json::Value::Null, |transform| {
        serde_json::json!({
            "x": transform.translation.x,
            "y": transform.translation.y,
        })
    })
}

fn cell(cell: IVec2) -> serde_json::Value {
    serde_json::json!({ "x": cell.x, "y": cell.y })
}

fn entity_id(entity: Entity) -> u64 {
    entity.to_bits()
}

fn resource_kind(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Minerals => "minerals",
    }
}

fn nanobot_kind(kind: NanobotType) -> &'static str {
    match kind {
        NanobotType::Worker => "worker",
        NanobotType::Hauler => "hauler",
        NanobotType::Defender => "defender",
    }
}

fn commitment_name(commitment: Commitment) -> &'static str {
    match commitment {
        Commitment::Idle => "idle",
        Commitment::Carrying => "carrying",
        Commitment::Working => "working",
    }
}

fn planned_kind(kind: PlannedKind) -> &'static str {
    match kind {
        PlannedKind::SourceStockpile => "source_stockpile",
        PlannedKind::SinkStockpile => "sink_stockpile",
        PlannedKind::ProductionFacility => "production_facility",
        PlannedKind::Charger => "charger",
    }
}

fn stockpile_role(role: StockpileRole) -> &'static str {
    match role {
        StockpileRole::Source => "source",
        StockpileRole::Sink => "sink",
    }
}

fn collect_nanobots(world: &mut World) -> serde_json::Value {
    let (total, entities) = {
        let mut query = world.query_filtered::<Entity, (With<Nanobot>, With<SwarmMember>)>();
        lowest_entity_ids(query.iter(world))
    };
    let items = entities
        .into_iter()
        .map(|entity| {
            let entity_ref = world.entity(entity);
            let member = entity_ref
                .get::<SwarmMember>()
                .expect("selected Nanobot has SwarmMember");
            let kind = entity_ref.get::<NanobotType>();
            let health = entity_ref.get::<Health>();
            let charge = entity_ref.get::<Charge>();
            let movement = entity_ref.get::<DirectMovementComponent>();
            let velocity = entity_ref.get::<VelocityComponent>();
            let commitment = entity_ref.get::<Commitment>();
            let response = entity_ref.get::<DefenderResponse>();
            let charger_assignment = entity_ref.get::<ChargerAssignment>();
            let charger_progress = entity_ref.get::<ChargerProgress>();
            let charger_pulse_progress = entity_ref.get::<ChargerPulseProgress>();
            let defender_attack_cooldown = entity_ref.get::<DefenderAttackCooldown>();
            let cargo = entity_ref.get::<Cargo>();
            let gather_assignment = entity_ref.get::<GatherAssignment>();
            let extract_progress = entity_ref.get::<ExtractProgress>();
            let returning = entity_ref.get::<crate::nanobot::ReturningToStockpile>();
            let build_assignment = entity_ref.get::<BuildAssignment>();
            let build_progress = entity_ref.get::<BuildProgress>();
            let planned_claim = entity_ref.get::<PlannedStructureClaim>();
            let planned_progress = entity_ref.get::<PlannedStructureProgress>();
            let maintenance_assignment = entity_ref.get::<MaintenanceAssignment>();
            let maintenance_progress = entity_ref.get::<MaintenanceProgress>();
            let hauler_assignment = entity_ref.get::<HaulerAssignment>();
            let reservation = entity_ref.get::<LogisticsReservation>();

            serde_json::json!({
                "id": entity_id(entity),
                "owner": member.0.0,
                "type": kind.map(|kind| nanobot_kind(*kind)),
                "position": position(entity_ref.get::<Transform>()),
                "health": health.map(|health| serde_json::json!({
                    "current": health.current,
                    "max": health.max,
                })),
                "charge": charge.map(|charge| serde_json::json!({
                    "current": charge.current,
                    "max": charge.max,
                })),
                "movement": movement.map(|movement| serde_json::json!({
                    "target": {"x": movement.xy.x, "y": movement.xy.y},
                    "stop_radius": movement.stop_radius,
                    "speed": movement.speed,
                    "has_interaction_region": movement.interaction.is_some(),
                })),
                "velocity": velocity.map(|velocity| serde_json::json!({
                    "x": velocity.value.x,
                    "y": velocity.value.y,
                })),
                "work_blocked": entity_ref.contains::<WorkBlocked>(),
                "commitment": commitment.map(|commitment| commitment_name(*commitment)),
                "defender_response": response.map(|response| serde_json::json!({
                    "target_id": entity_id(response.target),
                })),
                "charger_assignment": charger_assignment.map(|assignment| serde_json::json!({
                    "charger_id": entity_id(assignment.charger),
                })),
                "charger_progress": charger_progress.map(|progress| serde_json::json!({
                    "charger_id": entity_id(progress.charger),
                })),
                "charger_pulse_progress": charger_pulse_progress.map(|progress| serde_json::json!({
                    "ticks_elapsed": progress.ticks_elapsed,
                })),
                "defender_attack_cooldown": defender_attack_cooldown.map(|cooldown| serde_json::json!({
                    "ticks_remaining": cooldown.ticks_remaining,
                })),
                "cargo": cargo.map(|cargo| serde_json::json!({
                    "kind": resource_kind(cargo.kind),
                    "amount": cargo.amount,
                })),
                "gather_assignment": gather_assignment.map(|assignment| serde_json::json!({
                    "cell": cell(assignment.cell),
                    "deposit_id": entity_id(assignment.deposit),
                })),
                "extract_progress": extract_progress.map(|progress| serde_json::json!({
                    "collected": progress.collected,
                })),
                "returning_to_stockpile": returning.map(|returning| serde_json::json!({
                    "stockpile_id": entity_id(returning.stockpile),
                })),
                "build_assignment": build_assignment.map(|assignment| serde_json::json!({
                    "cell": cell(assignment.cell),
                    "target_id": entity_id(assignment.target),
                })),
                "build_progress": build_progress.map(|progress| serde_json::json!({
                    "cell": cell(progress.cell),
                    "target_id": entity_id(progress.target),
                })),
                "planned_structure_claim": planned_claim.map(|claim| serde_json::json!({
                    "cell": cell(claim.cell),
                    "target_id": entity_id(claim.target),
                })),
                "planned_structure_progress": planned_progress.map(|progress| serde_json::json!({
                    "cell": cell(progress.cell),
                    "target_id": entity_id(progress.target),
                })),
                "maintenance_assignment": maintenance_assignment.map(|assignment| serde_json::json!({
                    "cell": cell(assignment.cell),
                    "target_id": entity_id(assignment.target),
                })),
                "maintenance_progress": maintenance_progress.map(|progress| serde_json::json!({
                    "cell": cell(progress.cell),
                    "target_id": entity_id(progress.target),
                    "ticks_worked": progress.ticks_worked,
                })),
                "hauler_assignment": hauler_assignment.map(|assignment| serde_json::json!({
                    "source_id": entity_id(assignment.source),
                    "sink_id": entity_id(assignment.sink),
                })),
                "hauler_loading": entity_ref.contains::<HaulerLoading>(),
                "logistics_reservation": reservation.map(|reservation| serde_json::json!({
                    "source_id": entity_id(reservation.source),
                    "destination_id": entity_id(reservation.destination),
                    "kind": resource_kind(reservation.kind),
                    "amount": reservation.amount,
                    "source_remaining": reservation.source_remaining,
                    "destination_remaining": reservation.destination_remaining,
                })),
            })
        })
        .collect::<Vec<_>>();
    collection(total, items)
}

fn collect_structures(world: &mut World) -> serde_json::Value {
    let (total, entities) = {
        let mut query = world.query_filtered::<Entity, Or<(
            With<Structure>,
            With<Charger>,
            With<Stockpile>,
            With<ProductionFacility>,
            With<BuildSite>,
            With<PlannedStructure>,
        )>>();
        lowest_entity_ids(query.iter(world))
    };
    let items = entities
        .into_iter()
        .map(|entity| {
            let entity_ref = world.entity(entity);
            let structure = entity_ref.get::<Structure>();
            let charger = entity_ref.get::<Charger>();
            let stockpile = entity_ref.get::<Stockpile>();
            let facility = entity_ref.get::<ProductionFacility>();
            let build_site = entity_ref.get::<BuildSite>();
            let planned = entity_ref.get::<PlannedStructure>();
            let owner = entity_ref.get::<OwnerSwarm>();
            let kind = if charger.is_some() {
                "charger"
            } else if stockpile.is_some() {
                "stockpile"
            } else if facility.is_some() {
                "production_facility"
            } else if build_site.is_some() {
                "build_site"
            } else if planned.is_some() {
                "planned_structure"
            } else {
                "basic"
            };

            serde_json::json!({
                "id": entity_id(entity),
                "owner": owner.and_then(|owner| world.get::<SwarmId>(owner.0)).map(|id| id.0),
                "owner_entity_id": owner.map(|owner| entity_id(owner.0)),
                "position": position(entity_ref.get::<Transform>()),
                "kind": kind,
                "health": structure.map(|structure| serde_json::json!({
                    "current": structure.health,
                    "max": crate::nanobot::STRUCTURE_MAX_HEALTH,
                })),
                "maintenance_age_ticks": structure.map(|structure| structure.ticks_since_maintained),
                "charger": charger.map(|charger| serde_json::json!({
                    "cell": cell(charger.cell),
                    "kind": resource_kind(charger.kind),
                    "amount": charger.amount,
                    "capacity": charger.capacity,
                    "radius": charger.radius,
                })),
                "stockpile": stockpile.map(|stockpile| serde_json::json!({
                    "kind": resource_kind(stockpile.kind),
                    "amount": stockpile.amount,
                    "capacity": stockpile.capacity,
                    "radius": stockpile.radius,
                    "role": entity_ref.get::<StockpileRole>().map(|role| stockpile_role(*role)),
                })),
                "production_facility": facility.map(|facility| serde_json::json!({
                    "input_kind": resource_kind(facility.input_kind),
                    "input_amount": facility.input_amount,
                    "input_capacity": facility.input_capacity,
                    "progress": facility.progress,
                    "finished_at": facility.finished_at,
                    "current_target": facility.current_target.map(nanobot_kind),
                })),
                "build_site": build_site.map(|site| serde_json::json!({
                    "cell": cell(site.cell),
                    "kind": "basic",
                    "consumed_materials": site.consumed_materials,
                    "required_materials": site.required_materials,
                })),
                "planned_structure": planned.map(|planned| {
                    let (completed, total) = planned.construction_progress();
                    serde_json::json!({
                        "cell": cell(planned.cell),
                        "kind": planned_kind(planned.kind),
                        "completed_work": completed,
                        "total_work": total,
                        "active_worker_id": planned.active_worker().map(entity_id),
                    })
                }),
            })
        })
        .collect::<Vec<_>>();
    collection(total, items)
}

fn collect_deposits(world: &mut World) -> serde_json::Value {
    let (total, entities) = {
        let mut query = world.query_filtered::<Entity, With<ResourceDeposit>>();
        lowest_entity_ids(query.iter(world))
    };
    let items = entities
        .into_iter()
        .map(|entity| {
            let entity_ref = world.entity(entity);
            let deposit = entity_ref
                .get::<ResourceDeposit>()
                .expect("selected entity has ResourceDeposit");
            serde_json::json!({
                "id": entity_id(entity),
                "position": position(entity_ref.get::<Transform>()),
                "kind": resource_kind(deposit.kind),
                "amount": deposit.amount,
                "capacity": deposit.capacity,
            })
        })
        .collect::<Vec<_>>();
    collection(total, items)
}
