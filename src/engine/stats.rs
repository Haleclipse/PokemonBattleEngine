use crate::dex::{MoveCategory, PokemonType, ResolvedMove};
use crate::ids::PokemonId;
use crate::rules::AccuracyOrder;
use crate::rules::{
    ChanceScale, Fraction, IdentifiedEvasion, ParalysisSpeedStage, ResolvedRules, StageTable,
};
use crate::state::{BattleState, NonVolatileStatus, Stat, WeatherKind};

use super::damage::{chain_modifiers, fraction_4096, showdown_modify};
use super::{DefensiveStat, OffensiveStat};

/// Showdown's neutral 4096-based modifier.
const BASE_MODIFIER: u32 = 4096;

fn fraction_4096_of(fraction: Fraction) -> u32 {
    fraction_4096(
        u32::from(fraction.numerator),
        u32::from(fraction.denominator),
    )
}

/// A stat at `stage` (DESIGN.md §31.2): Gen 3+ `(2 + n) / 2` and `2 / (2 + n)`; Gen 1-2
/// Showdown's tables (×1 … ×4 up; 100, 66, 50, 40, 33, 28, 25 % down), floored.
pub(crate) fn stage_value(table: StageTable, value: u16, stage: i8) -> u32 {
    let value = u32::from(value);
    let stage = stage.clamp(-6, 6);
    match table {
        StageTable::Modern => {
            if stage >= 0 {
                value.saturating_mul(2 + stage as u32) / 2
            } else {
                value.saturating_mul(2) / (2 + (-stage) as u32)
            }
        }
        StageTable::Legacy => {
            const UP: [u32; 7] = [100, 150, 200, 250, 300, 350, 400];
            const DOWN: [u32; 7] = [100, 66, 50, 40, 33, 28, 25];
            if stage >= 0 {
                value.saturating_mul(UP[stage as usize]) / 100
            } else {
                value.saturating_mul(DOWN[(-stage) as usize]) / 100
            }
        }
    }
}

/// Whether the Pokémon's stages count double as they are read (Gen 4 Simple,
/// `AbilityBehavior::doubles_read_stages`).
fn doubles_read_stages(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &crate::state::PokemonState,
) -> bool {
    super::ability_handler(state, rules, pokemon, |b| b.doubles_read_stages)
}

/// The stage of `stat` as a stat reading sees it (Showdown `getStat`'s `ModifyBoost`, the
/// cartridge's `BattleSystem_CalcMoveDamage` / `CompareSpeed_ApplySimple`): doubled by Gen 4
/// Simple, within ±6.
pub(crate) fn read_stage(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &crate::state::PokemonState,
    stat: Stat,
) -> i8 {
    let stage = pokemon.stages.get(stat);
    if doubles_read_stages(state, rules, pokemon) {
        (stage * 2).clamp(-6, 6)
    } else {
        stage
    }
}

pub(crate) fn apply_stage(rules: &ResolvedRules, value: u16, stage: i8) -> u32 {
    stage_value(rules.stats.stage_table, value, stage)
}

/// Showdown gen2 `getStat(stat, false, true)`, the reading `StatRules::raise_stops_at_cap` and
/// Gen 2 Belly Drum compare with 999: the stored stat at `stage`, Speed quartered by paralysis,
/// clamped to the cap, then the holder's species item (`StatRules::species_item_stats`).
pub(crate) fn capped_stat_reading(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: &crate::state::PokemonState,
    stat: Stat,
    stage: i8,
) -> u32 {
    let mut value = apply_stage(rules, pokemon.battle_stats().get(stat).unwrap_or(0), stage);
    if stat == Stat::Speed && pokemon.has_status(NonVolatileStatus::Paralysis) {
        value /= 4;
    }
    if let Some(cap) = rules.stats.cap {
        value = value.clamp(1, u32::from(cap));
    }
    super::legacy_damage::species_item_stat(state, rules, pokemon, stat, value)
}

/// Showdown `getBestStat(false, true)` (Quark Drive, Protosynthesis): the highest of Attack,
/// Defense, Special Attack, Special Defense and Speed at its stage, no modifier; a tie goes to
/// the first in that order.
pub(crate) fn best_stat(state: &BattleState, rules: &ResolvedRules, pokemon: PokemonId) -> Stat {
    let Some(pokemon) = state.pokemon(pokemon) else {
        return Stat::Attack;
    };
    let mut best = (Stat::Attack, 0);
    for stat in [
        Stat::Attack,
        Stat::Defense,
        Stat::SpecialAttack,
        Stat::SpecialDefense,
        Stat::Speed,
    ] {
        // `getStat(…, unmodified)`: under Wonder Room the stat reads the other defense's stage.
        let stage_stat = if wonder_room(state) {
            wonder_room_swap(stat)
        } else {
            stat
        };
        let value = apply_stage(
            rules,
            pokemon.battle_stats().get(stat).unwrap_or(0),
            pokemon.stages.get(stage_stat),
        );
        if value > best.1 {
            best = (stat, value);
        }
    }
    best.0
}

pub fn effective_speed(state: &BattleState, rules: &ResolvedRules, pokemon: PokemonId) -> u32 {
    let id = pokemon;
    let Some(pokemon) = state.pokemon(pokemon) else {
        return 0;
    };

    // Gen 1: the modified Speed already carries the stage and the paralysis cut.
    if let Some(modified) = pokemon.modified_stats {
        return u32::from(modified.speed).max(1);
    }
    let mut speed = apply_stage(
        rules,
        pokemon.battle_stats().speed,
        read_stage(state, rules, pokemon, Stat::Speed),
    );
    // Showdown `par.onModifySpe` leaves a Quick Feet holder alone (`hasAbility('quickfeet')`).
    let paralysis = (pokemon.has_status(NonVolatileStatus::Paralysis)
        && !super::has_ability(state, rules, pokemon, crate::ids::AbilityId::QUICK_FEET))
    .then_some(rules.status.paralysis_speed_stage);
    if paralysis == Some(ParalysisSpeedStage::Stat) {
        speed = rules.status.paralysis_speed_modifier.apply_u32(speed);
    }
    // Gen 1-2 clamp the stat itself (after stages and paralysis) to 1..=999.
    if let Some(cap) = rules.stats.cap {
        speed = speed.clamp(1, u32::from(cap));
    }
    // Showdown's `ModifySpe` chain: Quark Drive's / Protosynthesis's volatile, the ability
    // (Surge Surfer, Chlorophyll …), the
    // item (Choice Scarf, the Power items, Iron Ball, Quick Powder on Ditto), Tailwind and
    // (Gen 3-4) paralysis combine into one 4096-based modifier applied once (`chainModify`,
    // then `modify`).
    let mut chain = BASE_MODIFIER;
    // The volatile's `onModifySpe` asks `ignoringAbility()`: Neutralizing Gas leaves the volatile
    // in place but its boost out.
    if pokemon.volatiles.paradox_boost().map(|(_, stat)| stat) == Some(Stat::Speed)
        && !super::ignoring_ability(state, rules, pokemon)
    {
        chain = chain_modifiers(chain, 6144);
    }
    // Unburden's volatile (`onModifySpe`): ×2 while the holder holds nothing and its ability
    // has effect (`!pokemon.ignoringAbility()`: Neutralizing Gas, Gastro Acid).
    if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::Unburden)
        && pokemon.held_item.is_none()
        && !super::ignoring_ability(state, rules, pokemon)
    {
        chain = chain_modifiers(chain, 8192);
    }
    if let Some(modifier) = super::ability_handler(state, rules, pokemon, |b| b.speed_boost)
        .and_then(|boost| boost.modifier_for(state, rules, id))
    {
        chain = chain_modifiers(chain, modifier);
    }
    if let Some((_, behavior)) = super::held_item(state, rules, id) {
        // Choice Scarf's `onModifySpe`: no boost while the holder is Dynamaxed (§44.5).
        if let Some(fraction) = behavior.modify_speed
            && !(behavior.boosts_suspended_by_dynamax && super::dynamaxed(state, id))
        {
            chain = chain_modifiers(chain, fraction_4096_of(fraction));
        }
        if let Some(modifier) = behavior
            .speed_boost
            .and_then(|boost| boost.modifier_for(rules, pokemon))
        {
            chain = chain_modifiers(chain, modifier);
        }
    } else if rules.stats.slowing_items_ignore_suppression
        && let Some(fraction) = pokemon
            .held_item
            .and_then(|item| super::item_behavior(rules, item).modify_speed)
            .filter(|fraction| fraction.numerator < fraction.denominator)
    {
        // Gen 4: a slowing item set aside (Klutz, Embargo) still slows its holder (I52).
        chain = chain_modifiers(chain, fraction_4096_of(fraction));
    }
    if state
        .side(pokemon.side)
        .conditions
        .tailwind_turns()
        .is_some()
    {
        chain = chain_modifiers(
            chain,
            fraction_4096_of(rules.conditions.tailwind_speed_modifier),
        );
    }
    // A combined Grass Pledge's side (`onModifySpe` 0.25, DESIGN.md §42.9): the side's Speed
    // quartered, in the same chain as Tailwind.
    if state
        .side(pokemon.side)
        .conditions
        .contains(crate::state::SideConditionKind::GrassPledge)
    {
        chain = chain_modifiers(chain, 1024);
    }
    if paralysis == Some(ParalysisSpeedStage::Chained) {
        chain = chain_modifiers(
            chain,
            fraction_4096_of(rules.status.paralysis_speed_modifier),
        );
    }
    if chain != BASE_MODIFIER {
        speed = showdown_modify(speed, chain);
    }
    // Gen 5+: paralysis last, on the modified Speed (`finalModify`, then floored).
    if paralysis == Some(ParalysisSpeedStage::AfterChain) {
        speed = rules.status.paralysis_speed_modifier.apply_u32(speed);
    }
    speed.max(1)
}

/// The Speed a Pokémon acts at (Showdown `getActionSpeed`, the `pokemon.speed` every speed
/// sort reads: the turn order, switch-ins, the residual): its effective Speed, or 65535 for a
/// Gen 2-3 Quick Claw holder in a turn whose roll succeeded (`quick_claw` overrides the
/// state's roll for a preview).
pub(crate) fn action_speed(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: PokemonId,
    quick_claw: Option<bool>,
) -> u32 {
    let rolled = quick_claw.unwrap_or(state.quick_claw_roll);
    if rolled
        && rules.action_order.quick_claw_turn_roll.is_some()
        && super::held_item_id(state, rules, pokemon) == Some(crate::ids::ItemId::QUICK_CLAW)
    {
        return 65535;
    }
    effective_speed(state, rules, pokemon)
}

/// The speed event handlers are ordered by mid-turn (redirection): Showdown's `pokemon.speed`,
/// as last refreshed by `update_handler_speeds` — not the speed of the moment: a Speed drop
/// earlier in the turn leaves it alone until Gen 8.
pub(crate) fn handler_speed(state: &BattleState, pokemon: PokemonId) -> u32 {
    state.handler_speed(pokemon)
}

/// Showdown `getActionSpeed` now: the action speed, inverted under Trick Room — what
/// `updateSpeed` stores, and what the residual orders by (it refreshes the speed as it starts).
pub(crate) fn current_handler_speed(
    state: &BattleState,
    rules: &ResolvedRules,
    pokemon: PokemonId,
) -> u32 {
    let speed = action_speed(state, rules, pokemon, None);
    if state
        .field
        .contains(crate::state::FieldConditionKind::TrickRoom)
    {
        10_000u32.saturating_sub(speed)
    } else {
        speed
    }
}

/// Showdown `updateSpeed`: every active Pokémon's handler speed becomes its `getActionSpeed`.
pub(crate) fn update_handler_speeds(state: &mut BattleState, rules: &ResolvedRules) {
    let active: Vec<PokemonId> = state
        .sides
        .iter()
        .flat_map(|side| side.active.iter().flatten().copied())
        .collect();
    for pokemon in active {
        let speed = current_handler_speed(state, rules, pokemon);
        state.set_handler_speed(pokemon, speed);
    }
}

/// A stat a hit reads, before anything multiplies it: which stat, its stored value, and the
/// stage that counts (the Gen 4 cartridge multiplies the stored value first, DESIGN.md R125).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatParts {
    pub stat: Stat,
    pub stored: u16,
    pub stage: i8,
}

/// The attacking stat's parts: the user's Attack or Special Attack by category — or the
/// move's own source (Foul Play: the target's Attack; Body Press: the user's Defense) — and its
/// stage, which a critical hit ignores when negative where the generation says so. `None` for a
/// status move.
pub(crate) fn offensive_parts(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
    critical: bool,
) -> Option<StatParts> {
    let sources = move_def.stat_sources;
    let source = if sources.offense_from_target {
        target
    } else {
        attacker
    };
    let pokemon = state.pokemon(source)?;
    let stat = match (sources.offense_stat, move_def.category) {
        // Wonder Room's `onModifyMove`: Body Press takes the other defense (its stage), whose
        // stored value `calculateStat` then swaps back.
        (Some(stat), _) if wonder_room(state) => wonder_room_swap(stat),
        (Some(stat), _) => stat,
        (None, MoveCategory::Physical) => Stat::Attack,
        (None, MoveCategory::Special) => Stat::SpecialAttack,
        (None, MoveCategory::Status) => return None,
    };
    // A move aimed at an Unaware Pokémon reads its user's attacking stat at stage 0 — Foul
    // Play's the target's own Attack too (Showdown runs `ModifyBoost` on the user).
    let stage = if super::unaware_ignores(state, rules, target, attacker) {
        0
    } else {
        read_stage(state, rules, pokemon, stat)
    };
    let stage = if critical && rules.damage.critical_ignores_negative_offense {
        stage.max(0)
    } else {
        stage
    };
    Some(StatParts {
        stat,
        stored: stored_stat(state, pokemon, stat),
        stage,
    })
}

/// Whether Wonder Room is up.
fn wonder_room(state: &BattleState) -> bool {
    state
        .field
        .contains(crate::state::FieldConditionKind::WonderRoom)
}

/// Defense for Special Defense and back; any other stat as it is.
fn wonder_room_swap(stat: Stat) -> Stat {
    match stat {
        Stat::Defense => Stat::SpecialDefense,
        Stat::SpecialDefense => Stat::Defense,
        other => other,
    }
}

/// A stat's stored value as `calculateStat` reads it: under Wonder Room the stored Defense and
/// Special Defense trade places (the stages and modifiers stay the named stat's).
pub(crate) fn stored_stat(
    state: &BattleState,
    pokemon: &crate::state::PokemonState,
    stat: Stat,
) -> u16 {
    let stat = if wonder_room(state) {
        wonder_room_swap(stat)
    } else {
        stat
    };
    pokemon.battle_stats().get(stat).unwrap_or(1)
}

/// The attacking stat (DESIGN.md §22.6): `offensive_parts` at its stage.
pub(crate) fn offensive_stat(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
    critical: bool,
) -> OffensiveStat {
    OffensiveStat(
        offensive_parts(state, rules, attacker, target, move_def, critical)
            .map_or(0, |parts| apply_stage(rules, parts.stored, parts.stage)),
    )
}

/// The defending stat's parts: the target's Defense or Special Defense by category — or the
/// move's own (Psyshock: Defense) — and its stage (a critical hit ignores a positive one where
/// the generation says so). `None` for a status move.
pub(crate) fn defensive_parts(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
    critical: bool,
) -> Option<StatParts> {
    let pokemon = state.pokemon(target)?;
    let stat = match (move_def.stat_sources.defense_stat, move_def.category) {
        (Some(stat), _) => stat,
        (None, MoveCategory::Physical) => Stat::Defense,
        (None, MoveCategory::Special) => Stat::SpecialDefense,
        (None, MoveCategory::Status) => return None,
    };
    let stage = if move_def.stat_sources.ignore_defensive_stage
        || super::unaware_ignores(state, rules, attacker, target)
    {
        0
    } else {
        read_stage(state, rules, pokemon, stat)
    };
    let stage = if critical && rules.damage.critical_ignores_positive_defense {
        stage.min(0)
    } else {
        stage
    };
    Some(StatParts {
        stat,
        stored: stored_stat(state, pokemon, stat),
        stage,
    })
}

/// The boost the weather gives `target`'s `stat` (Showdown `onModifyDef` / `onModifySpD` at
/// priority 10): snow for an Ice type's Defense, sandstorm for a Rock type's Special Defense.
pub(crate) fn weather_defense_boost(
    state: &BattleState,
    rules: &ResolvedRules,
    target: PokemonId,
    stat: Stat,
) -> Fraction {
    let Some(pokemon) = state.pokemon(target) else {
        return Fraction::ONE;
    };
    let weather = super::weather_for(state, rules, target);
    match stat {
        Stat::Defense
            if weather == Some(WeatherKind::Snow)
                && crate::engine::types_of(state, rules, pokemon).contains(PokemonType::Ice) =>
        {
            rules.weather.snow_ice_defense
        }
        Stat::SpecialDefense
            if weather == Some(WeatherKind::Sandstorm)
                && crate::engine::types_of(state, rules, pokemon).contains(PokemonType::Rock) =>
        {
            rules.weather.sandstorm_rock_special_defense
        }
        _ => Fraction::ONE,
    }
}

/// The defending stat: `defensive_parts` at its stage, then the weather's boost.
pub(crate) fn defensive_stat(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
    move_def: &ResolvedMove,
    critical: bool,
) -> DefensiveStat {
    let Some(parts) = defensive_parts(state, rules, attacker, target, move_def, critical) else {
        return DefensiveStat(1);
    };
    let staged = apply_stage(rules, parts.stored, parts.stage);
    DefensiveStat(weather_defense_boost(state, rules, target, parts.stat).apply_u32(staged))
}

/// The accuracy modifiers of one check, in the form each generation applies them
/// (`HitRules::accuracy_order`, `effects::abilities::AccuracyModifier`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccuracyModifiers {
    /// Gen 5+: Showdown's 4096-based `ModifyAccuracy` chain.
    pub chain: u32,
    /// Gen 5+: the 4096-based `Accuracy` event chain, once the stages are in (Micle Berry).
    pub after_stages: u32,
    /// Gen 3-4: the cartridge's factors (`numerator / denominator`, a percentage over 100 but
    /// Gravity's 10 / 6), in its order.
    pub percentages: Vec<(u32, u32)>,
    /// Gen 2: what comes off the accuracy byte (Bright Powder).
    pub byte_penalty: u32,
}

impl AccuracyModifiers {
    pub(crate) fn none() -> Self {
        Self {
            chain: BASE_MODIFIER,
            after_stages: BASE_MODIFIER,
            percentages: Vec::new(),
            byte_penalty: 0,
        }
    }
}

/// The Gen 3-4 cartridge's stage fractions for the net stage `6 + accuracy − evasion`
/// (pokeemerald `sAccuracyStageRatios`, pokeplatinum `HitRateByStage`).
const CARTRIDGE_STAGE_RATIOS: [(u32, u32); 13] = [
    (33, 100),
    (36, 100),
    (43, 100),
    (50, 100),
    (60, 100),
    (75, 100),
    (1, 1),
    (133, 100),
    (166, 100),
    (2, 1),
    (233, 100),
    (133, 50),
    (3, 1),
];

/// The accuracy check as a ratio (`None`: no roll, the move hits): Gen 5+ Showdown's —
/// `ModifyAccuracy` on the move's accuracy, then the net stage, truncated; Gen 3-4 the
/// cartridge's — the net stage's fraction, then the modifiers' percentages, floored at every
/// step (issues.md I5); both out of 100. Gen 1-2 the byte `⌊accuracy × 255 / 100⌋`, times the
/// accuracy stage then the evasion stage (Showdown's decimal tables, floored once), clamped to
/// 1..=255 out of 256 — 255 never rolls. A move that ignores evasion (`ignore_evasion`) reads
/// the target's evasion stage as 0, and `ignore_accuracy_stage` (the target's Unaware) the
/// user's accuracy stage; an identified target's stages count as `IdentifyRules::evasion` says.
#[allow(clippy::too_many_arguments)]
pub(crate) fn accuracy_ratio(
    state: &BattleState,
    rules: &ResolvedRules,
    attacker: PokemonId,
    target: PokemonId,
    base_accuracy: Option<u8>,
    self_target: bool,
    ignore_evasion: bool,
    ignore_accuracy_stage: bool,
    modifiers: &AccuracyModifiers,
) -> Option<(u16, u16)> {
    let base = base_accuracy?;
    let (Some(attacker), Some(target)) = (state.pokemon(attacker), state.pokemon(target)) else {
        return Some((0, 1));
    };
    // Gen 4 Simple doubles each stage (pokeplatinum `CheckMoveHitAccuracy`): the cartridge
    // then clamps only their sum (`AccuracyOrder::StagesThenPercentages`).
    let doubled = |pokemon: &crate::state::PokemonState, stage: i8| {
        if doubles_read_stages(state, rules, pokemon) {
            stage * 2
        } else {
            stage
        }
    };
    let evasion_stage = if ignore_evasion {
        0
    } else {
        doubled(target, target.stages.evasion)
    };
    let accuracy_stage = if ignore_accuracy_stage {
        0
    } else {
        doubled(attacker, attacker.stages.accuracy)
    };
    // An identified target (Foresight, Odor Sleuth, Miracle Eye; `IdentifyRules::evasion`).
    let identified = target
        .volatiles
        .contains(crate::state::VolatileConditionKind::Foresight)
        || target
            .volatiles
            .contains(crate::state::VolatileConditionKind::MiracleEye);
    let (accuracy_stage, evasion_stage) = match rules.conditions.identify.evasion {
        _ if !identified => (accuracy_stage, evasion_stage),
        IdentifiedEvasion::NeitherWhenEvasionAtLeastAccuracy if evasion_stage >= accuracy_stage => {
            (0, 0)
        }
        IdentifiedEvasion::NeitherWhenEvasionAtLeastAccuracy => (accuracy_stage, evasion_stage),
        IdentifiedEvasion::Ignored => (accuracy_stage, 0),
        IdentifiedEvasion::RaisedIgnored => (accuracy_stage, evasion_stage.min(0)),
    };
    match rules.hits.chance_scale {
        ChanceScale::Percent => match rules.hits.accuracy_order {
            AccuracyOrder::ModifiersThenStages => {
                let mut accuracy = u32::from(base);
                if modifiers.chain != BASE_MODIFIER {
                    accuracy = super::damage::showdown_modify(accuracy, modifiers.chain);
                }
                let net_stage = (accuracy_stage - evasion_stage).clamp(-6, 6);
                if net_stage > 0 {
                    accuracy = accuracy * (3 + net_stage as u32) / 3;
                } else if net_stage < 0 {
                    accuracy = accuracy * 3 / (3 + (-net_stage) as u32);
                }
                if modifiers.after_stages != BASE_MODIFIER {
                    accuracy = super::damage::showdown_modify(accuracy, modifiers.after_stages);
                }
                (accuracy < 100).then_some((accuracy as u16, 100))
            }
            AccuracyOrder::StagesThenPercentages => {
                let net = (6 + i32::from(accuracy_stage) - i32::from(evasion_stage)).clamp(0, 12)
                    as usize;
                let (numerator, denominator) = CARTRIDGE_STAGE_RATIOS[net];
                let mut accuracy = u32::from(base) * numerator / denominator;
                for (numerator, denominator) in &modifiers.percentages {
                    accuracy = accuracy * numerator / denominator;
                }
                // `Random() % 100 + 1 > calc` misses: from 100 on it never does.
                (accuracy < 100).then_some((accuracy as u16, 100))
            }
        },
        ChanceScale::Byte => byte_accuracy(
            accuracy_stage,
            evasion_stage,
            u32::from(base) * 255 / 100,
            modifiers.byte_penalty,
        ),
        ChanceScale::ByteAlwaysRolls => {
            // Showdown gen1 `tryMoveHit`: `[25, 28, 33, 40, 50, 66, 100, 150 … 400]`, floored
            // after the accuracy stage and again after the evasion stage. Thrash's lock carries
            // the byte its last use ended with instead of the move's (the cartridge's
            // `wPlayerMoveAccuracy` left as `CalcHitChance` wrote it, §15.12), and so does Rage's
            // (§15.19; Showdown reads Rage's after Thrash's).
            const TABLE: [u32; 13] = [25, 28, 33, 40, 50, 66, 100, 150, 200, 250, 300, 350, 400];
            let mut accuracy = attacker
                .volatiles
                .rage_lock()
                .or_else(|| attacker.volatiles.rampage().map(|rampage| rampage.accuracy))
                .map_or(u32::from(base) * 255 / 100, u32::from);
            let boost = accuracy_stage.clamp(-6, 6);
            accuracy = accuracy * TABLE[(boost + 6) as usize] / 100;
            let evasion = evasion_stage.clamp(-6, 6);
            accuracy = accuracy * TABLE[(6 - evasion) as usize] / 100;
            let accuracy = accuracy.clamp(1, 255) + u32::from(self_target);
            (accuracy < 256).then_some((accuracy as u16, 256))
        }
    }
}

/// Gen 2's accuracy from a byte (Showdown gen2 `tryMoveHit`): times the accuracy stage, then
/// the evasion stage (decimal tables), floored once, clamped to 1..=255, less `penalty`
/// (`ModifyAccuracy`: Bright Powder, not below 0), out of 256; 255 never rolls.
pub(crate) fn byte_accuracy(
    accuracy_stage: i8,
    evasion_stage: i8,
    byte: u32,
    penalty: u32,
) -> Option<(u16, u16)> {
    const UP: [f64; 7] = [1.0, 1.33, 1.66, 2.0, 2.33, 2.66, 3.0];
    const DOWN: [f64; 7] = [1.0, 0.75, 0.6, 0.5, 0.43, 0.36, 0.33];
    let mut accuracy = f64::from(byte.min(255));
    let boost = accuracy_stage.clamp(-6, 6);
    accuracy *= if boost > 0 {
        UP[boost as usize]
    } else {
        DOWN[(-boost) as usize]
    };
    let evasion = evasion_stage.clamp(-6, 6);
    if evasion > 0 {
        accuracy *= DOWN[evasion as usize];
    } else if evasion < 0 {
        accuracy *= UP[(-evasion) as usize];
    }
    let accuracy = ((accuracy.floor() as u32).clamp(1, 255)).saturating_sub(penalty) as u16;
    (accuracy < 255).then_some((accuracy, 256))
}
