use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use super::TuningField;

macro_rules! tuning_schema {
    ($( $section:ident: $section_type:ident { $( $field:ident: $ty:ty = $default:expr => ($label:literal, $unit:literal, $scale:expr, $step:expr, $min:expr, $max:expr); )* } )*) => {
        #[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
        #[serde(deny_unknown_fields)]
        pub struct Tuning { $(pub $section: $section_type,)* }
        $(
            #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
            #[serde(deny_unknown_fields)]
            pub struct $section_type { $(pub $field: $ty,)* }
            impl Default for $section_type {
                fn default() -> Self { Self { $($field: $default,)* } }
            }
        )*
        impl Tuning {
            pub fn fields() -> &'static [TuningField] {
                &[$($(TuningField {
                    key: concat!(stringify!($section), ".", stringify!($field)),
                    group: stringify!($section), label: $label, unit: $unit,
                    scale: $scale, step: $step,
                    min: ($min) * ($scale), max: ($max) * ($scale),
                },)*)*]
            }
            pub fn get(&self, key: &str) -> Option<f64> {
                match key {
                    $($(concat!(stringify!($section), ".", stringify!($field)) => Some(self.$section.$field as f64 * ($scale)),)*)*
                    _ => None,
                }
            }
            pub fn with_value(&self, key: &str, display_value: f64) -> Result<Self, String> {
                if !display_value.is_finite() { return Err("Value must be finite".into()); }
                let mut next = self.clone();
                match key {
                    $($(concat!(stringify!($section), ".", stringify!($field)) => {
                        let native = display_value / ($scale);
                        if native < ($min) || native > ($max) {
                            return Err(format!("{} must be between {} and {} {}", $label, ($min) * ($scale), ($max) * ($scale), $unit));
                        }
                        let converted = native as $ty;
                        if stringify!($ty).starts_with('u') && (native - native.round()).abs() > 0.000001 {
                            return Err(format!("{} must use whole native units", $label));
                        }
                        next.$section.$field = if stringify!($ty).starts_with('u') { native.round() as $ty } else { converted };
                    },)*)*
                    _ => return Err(format!("Unknown tuning key: {key}")),
                }
                next.validate()?;
                Ok(next)
            }
            pub fn validate(&self) -> Result<(), String> {
                $($(let value = self.$section.$field as f64;
                if !value.is_finite() || value < ($min) || value > ($max) {
                    return Err(format!("Invalid {}.{}", stringify!($section), stringify!($field)));
                })*)*
                self.validate_relationships()
            }
        }
    }
}

tuning_schema! {
    production: ProductionTuning {
        cost_per_bot: u32 = 40 => ("Cost per bot", "", 1.0, 1.0, 1.0, 1_000_000.0);
        ticks_per_bot: u32 = 600 => ("Production time", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
        input_capacity: u32 = 80 => ("Input capacity", "", 1.0, 1.0, 1.0, 1_000_000.0);
        pressure_ticks: u32 = 60 => ("Expansion delay", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
    }
    construction: ConstructionTuning {
        work_ticks: u32 = 1500 => ("Work duration", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
        max_health: u32 = 100 => ("Max health", "", 1.0, 1.0, 1.0, 1_000_000.0);
    }
    logistics: LogisticsTuning {
        worker_carry_capacity: u32 = 4 => ("Worker carry capacity", "", 1.0, 1.0, 1.0, 1_000_000.0);
        hauler_carry_capacity: u32 = 20 => ("Hauler carry capacity", "", 1.0, 1.0, 1.0, 1_000_000.0);
        worker_extract_per_pulse: u32 = 1 => ("Worker extract per pulse", "", 1.0, 1.0, 1.0, 1_000_000.0);
        hauler_transfer_per_pulse: u32 = 1 => ("Hauler transfer per pulse", "", 1.0, 1.0, 1.0, 1_000_000.0);
        material_work_ticks: u32 = 75 => ("Material transfer interval", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
        stockpile_capacity: u32 = 150 => ("Stockpile capacity", "", 1.0, 1.0, 1.0, 1_000_000.0);
        source_proximity_radius: f32 = 384.0 => ("Source proximity radius", "world", 1.0, 16.0, 1.0, 1_000_000.0);
        bot_speed: f32 = 5.0 => ("Bot speed", "world/tick", 1.0, 0.5, 0.01, 1_000_000.0);
        corridor_cost_multiplier: f32 = 0.35 => ("Corridor cost multiplier", "", 1.0, 0.05, 0.01, 1.0);
    }
    combat: CombatTuning {
        attack_range: f32 = 96.0 => ("Attack range", "world", 1.0, 8.0, 1.0, 1_000_000.0);
        attack_interval_ticks: u16 = 30 => ("Attack interval", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 65535.0);
        structure_damage_factor: f32 = 0.5 => ("Structure damage factor", "", 1.0, 0.05, 0.0, 100.0);
        nanobot_max_health: u32 = 100 => ("Nanobot max health", "", 1.0, 1.0, 1.0, 1_000_000.0);
        base_attack: f32 = 10.0 => ("Base attack", "", 1.0, 1.0, 0.0, 1_000_000.0);
        base_defense: f32 = 10.0 => ("Base defense", "", 1.0, 1.0, 0.0, 1_000_000.0);
    }
    charge: ChargeTuning {
        max_charge: f32 = 1.0 => ("Max charge", "", 1.0, 0.05, 0.000001, 100.0);
        drain_per_tick: f32 = 0.000125 => ("Charge drain", "/s", 60.0, 0.001, 0.0, 1.0);
        pulse_interval_ticks: u16 = 150 => ("Charging interval", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 65535.0);
        per_pulse: f32 = 0.03 => ("Per pulse", "", 1.0, 0.01, 0.000001, 100.0);
        material_per_pulse: u32 = 2 => ("Material per pulse", "", 1.0, 1.0, 1.0, 1_000_000.0);
        weakened_threshold: f32 = 0.3 => ("Weakened threshold", "", 1.0, 0.05, 0.0, 100.0);
        low_threshold: f32 = 0.5 => ("Low threshold", "", 1.0, 0.05, 0.0, 100.0);
        empty_damage_interval_ticks: u8 = 6 => ("Empty charge damage interval", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 255.0);
        empty_health_damage: u32 = 1 => ("Empty health damage", "", 1.0, 1.0, 1.0, 1_000_000.0);
        charger_capacity: u32 = 120 => ("Charger capacity", "", 1.0, 1.0, 1.0, 1_000_000.0);
        max_defenders_per_charger: u32 = 3 => ("Max defenders per charger", "", 1.0, 1.0, 1.0, 1_000_000.0);
    }
    maintenance: MaintenanceTuning {
        buffer_ticks: u32 = 3600 => ("Maintenance buffer", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
        work_ticks: u32 = 30 => ("Work duration", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
        health_per_tick: u32 = 2 => ("Health per tick", "", 1.0, 1.0, 1.0, 1_000_000.0);
        degradation_per_step: u32 = 1 => ("Degradation per step", "", 1.0, 1.0, 1.0, 1_000_000.0);
        degradation_interval_ticks: u32 = 6 => ("Degradation interval", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
        needs_threshold: u32 = 1800 => ("Maintenance threshold", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
    }
    autonomy: AutonomyTuning {
        gather_weight: u32 = 1 => ("Gather weight", "", 1.0, 1.0, 1.0, 1_000_000.0);
        planned_build_weight: u32 = 1 => ("Planned build weight", "", 1.0, 1.0, 1.0, 1_000_000.0);
        maintenance_weight: u32 = 1 => ("Maintenance weight", "", 1.0, 1.0, 1.0, 1_000_000.0);
        haul_weight: u32 = 1 => ("Haul weight", "", 1.0, 1.0, 1.0, 1_000_000.0);
        terminal_fairness_promotion_ticks: u32 = 8 => ("Terminal fairness promotion passes", "passes", 1.0, 1.0, 1.0, 1_000_000_000.0);
        regional_fairness_promotion_ticks: u32 = 8 => ("Regional fairness promotion passes", "passes", 1.0, 1.0, 1.0, 1_000_000_000.0);
        lease_no_progress_ttl_ticks: u64 = 30 => ("Lease stall timeout", "s", 0.1, 0.1, 1.0, 1_000_000_000.0);
        controller_review_period_ticks: u64 = 30 => ("Controller review interval", "s", 1.0 / 60.0, 1.0 / 60.0, 1.0, 1_000_000_000.0);
        controller_switch_margin: f32 = 1.15 => ("Controller switch margin", "", 1.0, 0.05, 0.0, 1_000_000.0);
        controller_switch_bias: f32 = 10.0 => ("Controller switch bias", "", 1.0, 1.0, 0.0, 1_000_000.0);
        min_second_front_defenders: u32 = 4 => ("Min second front defenders", "", 1.0, 1.0, 1.0, 1_000_000.0);
        min_front_health_fraction: f32 = 0.5 => ("Min front health fraction", "", 1.0, 0.05, 0.0, 1.0);
        min_front_charge: f32 = 0.45 => ("Min front charge", "", 1.0, 0.05, 0.0, 1.0);
        strategy_pressure_anchor_exposure_cost: f32 = 12.0 => ("pressure anchor exposure cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_ledger_supply_weight: f32 = 0.1 => ("ledger supply weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_buffer_supply_weight: f32 = 0.05 => ("buffer supply weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_readiness_supply_target: f32 = 80.0 => ("readiness supply target", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_readiness_penalty_weight: f32 = 2.0 => ("readiness penalty weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hold_base: f32 = 25.0 => ("hold base", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hold_supply_cap: f32 = 100.0 => ("hold supply cap", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hold_supply_weight: f32 = 0.1 => ("hold supply weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hold_force_weight: f32 = 0.02 => ("hold force weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_scarcity_supply_target: f32 = 60.0 => ("scarcity supply target", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_scarcity_weight: f32 = 0.4 => ("scarcity weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_secure_base: f32 = 75.0 => ("secure base", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_secure_force_weight: f32 = 0.025 => ("secure force weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_secure_enemy_weight: f32 = 0.01 => ("secure enemy weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_pressure_base: f32 = 160.0 => ("pressure base", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_pressure_supply_cap: f32 = 120.0 => ("pressure supply cap", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_pressure_supply_weight: f32 = 0.7 => ("pressure supply weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_pressure_force_weight: f32 = 0.04 => ("pressure force weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_pressure_enemy_weight: f32 = 0.035 => ("pressure enemy weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_pressure_low_charge_penalty: f32 = 18.0 => ("pressure low charge penalty", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_pressure_distance_cost: f32 = 2.0 => ("pressure distance cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_pressure_exposure_cost: f32 = 12.0 => ("pressure exposure cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_charger_value: f32 = 95.0 => ("raid charger value", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_stockpile_value: f32 = 80.0 => ("raid stockpile value", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_facility_value: f32 = 60.0 => ("raid facility value", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_vulnerability_weight: f32 = 0.8 => ("raid vulnerability weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_base: f32 = 220.0 => ("raid base", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_supply_cap: f32 = 120.0 => ("raid supply cap", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_supply_weight: f32 = 0.7 => ("raid supply weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_cargo_weight: f32 = 0.15 => ("raid cargo weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_force_weight: f32 = 0.045 => ("raid force weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_enemy_weight: f32 = 0.04 => ("raid enemy weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_distance_cost: f32 = 3.0 => ("raid distance cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_raid_exposure_cost: f32 = 14.0 => ("raid exposure cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_defender_value: f32 = 60.0 => ("hunt defender value", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_hauler_value: f32 = 25.0 => ("hunt hauler value", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_worker_value: f32 = 15.0 => ("hunt worker value", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_vulnerability_weight: f32 = 0.5 => ("hunt vulnerability weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_supported_base: f32 = 140.0 => ("hunt supported base", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_remnant_base: f32 = 500.0 => ("hunt remnant base", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_force_weight: f32 = 0.04 => ("hunt force weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_distance_cost: f32 = 2.0 => ("hunt distance cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_hunt_exposure_cost: f32 = 14.0 => ("hunt exposure cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_force_base_weight: f32 = 0.35 => ("force base weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_force_charge_weight: f32 = 0.65 => ("force charge weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_deposit_amount_weight: f32 = 10.0 => ("deposit amount weight", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_deposit_distance_cost: f32 = 6.0 => ("deposit distance cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
        strategy_deposit_exposure_cost: f32 = 10.0 => ("deposit exposure cost", "", 1.0, 0.05, 0.0, 1_000_000.0);
    }
    presentation: PresentationTuning {
        camera_accel: f32 = 1800.0 => ("Camera accel", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        camera_max_speed: f32 = 600.0 => ("Camera max speed", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        camera_friction: f32 = 1050.0 => ("Camera friction", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        camera_zoom_speed: f32 = 0.15 => ("Camera zoom speed", "", 1.0, 0.01, 0.000001, 1_000_000.0);
        overlay_zoom_threshold: f32 = 8.0 => ("Overlay zoom threshold", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        tactical_merge_radius_world: f32 = 2048.0 => ("Tactical merge radius world", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        tactical_far_merge_radius_world: f32 = 6000.0 => ("Tactical far merge radius world", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        tactical_far_merge_zoom: f32 = 16.0 => ("Tactical far merge zoom", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        tactical_marker_screen_size: f32 = 32.0 => ("Tactical marker screen size", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        structure_bar_width: f32 = 48.0 => ("Structure bar width", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        structure_bar_height: f32 = 6.0 => ("Structure bar height", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        cargo_bar_width: f32 = 32.0 => ("Cargo bar width", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        cargo_bar_height: f32 = 4.0 => ("Cargo bar height", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        condition_bar_width: f32 = 48.0 => ("Condition bar width", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        condition_bar_height: f32 = 3.0 => ("Condition bar height", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        condition_bar_gap: f32 = 2.0 => ("Condition bar gap", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        structure_label_gap: f32 = 12.0 => ("Structure label gap", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        cargo_label_gap: f32 = 8.0 => ("Cargo label gap", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_pulse_duration_secs: f32 = 0.06 => ("Combat pulse duration", "s", 1.0, 0.01, 0.000001, 1_000_000.0);
        combat_recovery_duration_secs: f32 = 0.18 => ("Combat recovery duration", "s", 1.0, 0.01, 0.000001, 1_000_000.0);
        combat_death_duration_secs: f32 = 0.32 => ("Combat death duration", "s", 1.0, 0.01, 0.000001, 1_000_000.0);
        combat_jab_distance: f32 = 12.0 => ("Combat jab distance", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_recoil_distance: f32 = 10.0 => ("Combat recoil distance", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_minimum_impact_screen_distance: f32 = 2.0 => ("Combat minimum impact screen distance", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_reaction_flash_per_hit: f32 = 0.65 => ("Combat reaction flash per hit", "", 1.0, 0.01, 0.000001, 1_000_000.0);
        combat_reaction_flash_cap: f32 = 0.9 => ("Combat reaction flash cap", "", 1.0, 0.01, 0.000001, 1_000_000.0);
        combat_pulse_thickness: f32 = 3.0 => ("Combat pulse thickness", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_decoration_length: f32 = 10.0 => ("Combat decoration length", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_decoration_thickness: f32 = 2.0 => ("Combat decoration thickness", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_structure_ring_start_radius: f32 = 18.0 => ("Combat structure ring start radius", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_structure_ring_end_radius: f32 = 42.0 => ("Combat structure ring end radius", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_structure_ring_thickness: f32 = 3.0 => ("Combat structure ring thickness", "", 1.0, 1.0, 0.000001, 1_000_000.0);
        combat_zoom_cutoff: f32 = 8.0 => ("Combat zoom cutoff", "", 1.0, 1.0, 0.000001, 1_000_000.0);
    }
}
