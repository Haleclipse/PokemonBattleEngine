use crate::dex::PokemonType;
use crate::effects;
use crate::ids::{AbilityId, ItemId, PokemonId};
use crate::rules::{EffectRules, ResolvedRules};
use crate::state::{BattleState, NonVolatileStatus, RandomRequestKind, StatusState, TerrainKind};

use super::{BattleContext, BattleTrace, EffectSource, TraceEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBlockReason {
    InvalidTarget,
    AlreadyStatus,
    Fainted,
    TypeImmunity,
    /// Safeguard on the target's side.
    Safeguard,
    Terrain(TerrainKind),
    Ability(AbilityId),
    Item(ItemId),
    /// An uproar on the field keeps the target awake (`uproar`'s `onAnySetStatus`).
    Uproar,
    /// Sleep Clause Mod or Freeze Clause Mod (the format's battle clauses, DESIGN.md §45.7).
    Clause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrySetStatusResult {
    Allow,
    /// Refused; `shown`: the refusal's line names what refused it (`-immune|p|[from] ability:
    /// Limber`, `-block|p|ability: Sweet Veil|[of] holder`).
    Blocked {
        reason: StatusBlockReason,
        shown: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusApplyResult {
    Applied {
        target: PokemonId,
        status: NonVolatileStatus,
    },
    Blocked {
        target: PokemonId,
        status: NonVolatileStatus,
        reason: StatusBlockReason,
    },
}

pub struct TrySetStatusContext<'a> {
    state: &'a BattleState,
    rules: &'a ResolvedRules,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
    carried: bool,
}

impl<'a> TrySetStatusContext<'a> {
    pub(crate) fn new(
        state: &'a BattleState,
        rules: &'a ResolvedRules,
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
    ) -> Self {
        Self {
            state,
            rules,
            target,
            status,
            source,
            carried: false,
        }
    }

    /// Showdown `effect.status`: the effect carries the status itself — a status move's own
    /// (Thunder Wave, Toxic Thread), one Synchronize passes on — not a damaging move's secondary
    /// effect, an ability's or a hazard's. The abilities keeping a status off say so only then
    /// (`-immune|p|[from] ability: Limber`).
    pub fn carried(&self) -> bool {
        self.carried
    }

    pub fn target(&self) -> PokemonId {
        self.target
    }

    pub fn status(&self) -> NonVolatileStatus {
        self.status
    }

    pub fn source(&self) -> EffectSource {
        self.source
    }

    pub fn rules(&self) -> EffectRules<'_> {
        self.rules.for_effects()
    }

    pub fn target_has_type(&self, type_: PokemonType) -> bool {
        self.state.pokemon(self.target).is_some_and(|pokemon| {
            crate::engine::types_of(self.state, self.rules, pokemon).contains(type_)
        })
    }

    /// Whether the terrain's guard covers the target (Misty and Electric Terrain's
    /// `onSetStatus`: `isGrounded() && !isSemiInvulnerable()`).
    pub fn target_in_terrain(&self) -> bool {
        self.state.pokemon(self.target).is_some_and(|pokemon| {
            super::environment::grounded(self.state, self.rules, pokemon)
                && super::two_turn::hiding(pokemon).is_none()
        })
    }

    pub fn active_terrain(&self) -> Option<TerrainKind> {
        self.state.field.terrain.map(|terrain| terrain.kind)
    }

    /// The weather as the target has it (Showdown `effectiveWeather()`: none while suppressed,
    /// no sun or rain under a Utility Umbrella).
    pub fn target_weather(&self) -> Option<crate::state::WeatherKind> {
        super::weather_for(self.state, self.rules, self.target)
    }

    /// Someone other than the target causes the status (Showdown `source && target !==
    /// source`): a move, ability or item of another Pokémon, a side's hazard.
    pub fn by_another(&self) -> bool {
        !matches!(
            super::protections::Culprit::of(self.state, self.rules, self.source, self.target),
            super::protections::Culprit::None
        )
    }
}

/// `carried`: `TrySetStatusContext::carried`.
pub(crate) fn apply_status(
    state: &mut BattleState,
    rules: &ResolvedRules,
    trace: &mut BattleTrace,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
    carried: bool,
) -> StatusApplyResult {
    let inflicted_by = inflicter_side(state, source, target);
    set_status(
        state,
        rules,
        trace,
        target,
        status,
        source,
        SetStatusHow {
            replace: false,
            inflicted_by,
            carried,
        },
    )
}

/// The side whose doing a status from `source` is (Showdown `setStatus`'s `source`): the
/// Pokémon's whose move, ability or item it is; a foe's for a hazard (Toxic Spikes poisons
/// with `side.foe.active[0]` as its source); none for the weather, a terrain or a rule.
fn inflicter_side(
    state: &BattleState,
    source: EffectSource,
    target: PokemonId,
) -> Option<crate::ids::SideId> {
    match source {
        EffectSource::SideCondition(_) => state.pokemon(target).map(|pokemon| pokemon.side.other()),
        _ => source
            .pokemon()
            .and_then(|pokemon| state.pokemon(pokemon))
            .map(|pokemon| pokemon.side),
    }
}

/// Showdown `setStatus`, which Rest calls: like `apply_status`, but another status is replaced
/// rather than refusing the new one (no cure of its own: the old status's counters go with
/// it, a volatile it started stays). (Showdown's refusal of the same status needs a sleeping
/// Rest user, which only Sleep Talk makes.)
pub(crate) fn replace_status(
    state: &mut BattleState,
    rules: &ResolvedRules,
    trace: &mut BattleTrace,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
) -> StatusApplyResult {
    let inflicted_by = inflicter_side(state, source, target);
    set_status(
        state,
        rules,
        trace,
        target,
        status,
        source,
        SetStatusHow {
            replace: true,
            inflicted_by,
            carried: false,
        },
    )
}

/// How `set_status` sets a status.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SetStatusHow {
    /// Showdown `setStatus` (Rest): another status is replaced rather than refusing the new one.
    pub replace: bool,
    /// The side whose doing the status is (`inflicter_side`).
    pub inflicted_by: Option<crate::ids::SideId>,
    /// `TrySetStatusContext::carried`.
    pub carried: bool,
}

pub(crate) fn set_status(
    state: &mut BattleState,
    rules: &ResolvedRules,
    trace: &mut BattleTrace,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
    how: SetStatusHow,
) -> StatusApplyResult {
    let SetStatusHow {
        replace,
        inflicted_by,
        carried,
    } = how;
    let Some(target_state) = state.pokemon(target) else {
        return blocked(
            trace,
            target,
            status,
            source,
            StatusBlockReason::InvalidTarget,
        );
    };

    if target_state.is_fainted() {
        return blocked(trace, target, status, source, StatusBlockReason::Fainted);
    }
    if target_state.status.is_some() && !replace {
        return blocked(
            trace,
            target,
            status,
            source,
            StatusBlockReason::AlreadyStatus,
        );
    }

    {
        let ctx = TrySetStatusContext {
            carried,
            ..TrySetStatusContext::new(state, rules, target, status, source)
        };
        if status_type_immunity(&ctx) {
            return blocked(
                trace,
                target,
                status,
                source,
                StatusBlockReason::TypeImmunity,
            );
        }
        // The target's ability's `onImmunity` (`runStatusImmunity`, after the type's): Magma
        // Armor's freeze.
        let ability = super::ability_running(state, rules, target_state);
        let handlers = super::ability_handlers(state, rules, target_state);
        if let Some(ability) = ability
            && handlers.status_immunity == Some(effects::abilities::StatusImmunity::Status(status))
        {
            return blocked(
                trace,
                target,
                status,
                source,
                StatusBlockReason::Ability(ability),
            );
        }
        // The `SetStatus` handlers (Showdown `runEvent`, none with a priority) by their holder's
        // speed: those a Pokémon holds — an uproar's `onAnySetStatus` (Gen 3-4: not for a
        // Soundproof target), the target's ability's `onSetStatus`, its own and its allies'
        // `onAllySetStatus` (Flower Veil, Sweet Veil), its item's; on one holder a condition, then
        // an ability, then an item — before Safeguard (a side's) and the terrain (the field's).
        // The first that refuses is the refusal the line names: an Alcremie's Sweet Veil keeps
        // its Rest off before Electric Terrain does (R-M10-42).
        let named = |holder, reason, shown| match reason {
            StatusBlockReason::Ability(ability) if shown => Some(EffectSource::Ability {
                pokemon: holder,
                ability,
            }),
            _ => None,
        };
        let speed = |holder| super::stats::handler_speed(state, holder);
        let mut held: Vec<(u32, u8, StatusBlockReason, Option<EffectSource>)> = Vec::new();
        if status == NonVolatileStatus::Sleep {
            for uproarer in super::context::keeping_awake(state, rules, target) {
                held.push((speed(uproarer), 2, StatusBlockReason::Uproar, None));
            }
        }
        // The ability's `onSetStatus`: the statuses it keeps off (`blocks_statuses`; each says
        // so for a status its effect carries), then a handler of its own.
        if let Some(ability) = ability
            && (handlers.blocks_statuses.contains(&status)
                || handlers.in_forme_blocks.is_some_and(|guard| {
                    guard.statuses.contains(&status)
                        && state
                            .pokemon(target)
                            .is_some_and(|pokemon| guard.holds(pokemon))
                }))
        {
            let reason = StatusBlockReason::Ability(ability);
            held.push((speed(target), 7, reason, named(target, reason, carried)));
        } else if let TrySetStatusResult::Blocked { reason, shown } =
            effects::abilities::try_set_status(handlers, &ctx)
        {
            held.push((speed(target), 7, reason, named(target, reason, shown)));
        }
        // `onAllySetStatus`, the target's own included: the line names the holder
        // (`-block|p|ability: Sweet Veil|[of] holder`).
        for (holder, handler) in super::allied_ability_holders(state, rules, target)
            .into_iter()
            .filter_map(|(holder, behavior)| behavior.ally_try_set_status.map(|h| (holder, h)))
        {
            if let TrySetStatusResult::Blocked { reason, shown } = handler(&ctx) {
                held.push((speed(holder), 7, reason, named(holder, reason, shown)));
            }
        }
        if let Some((_, item)) = super::held_item(state, rules, target)
            && let TrySetStatusResult::Blocked { reason, .. } =
                effects::items::try_set_status(item, &ctx)
        {
            held.push((speed(target), 8, reason, None));
        }
        if let Some((_, _, reason, shown_by)) = held
            .into_iter()
            .min_by_key(|(speed, order, ..)| (std::cmp::Reverse(*speed), *order))
        {
            return blocked_shown(trace, target, status, source, reason, shown_by);
        }
        // Safeguard (`onSetStatus`): what someone else inflicts.
        if super::protections::safeguard_refuses(
            state,
            target,
            super::protections::Culprit::of(state, rules, source, target),
        ) {
            return blocked(trace, target, status, source, StatusBlockReason::Safeguard);
        }
        if let Some(terrain) = terrain_prevention(&ctx) {
            return blocked(
                trace,
                target,
                status,
                source,
                StatusBlockReason::Terrain(terrain),
            );
        }
        if let Some(reason) = clause_refuses(state, rules, target, status, inflicted_by) {
            return blocked(trace, target, status, source, reason);
        }
    }

    let mut installed = StatusState::new(status);
    installed.inflicted_by = inflicted_by;
    if status == NonVolatileStatus::Sleep {
        let lifecycle = rules.status.lifecycle;
        let rolled = state.rng.range_inclusive_for(
            RandomRequestKind::Duration,
            u16::from(lifecycle.sleep_time_min),
            u16::from(lifecycle.sleep_time_max),
        ) as u8;
        installed.sleep_turns_remaining = Some(rolled);
        installed.sleep_turns_rolled = Some(rolled);
    }
    if let Some(target_state) = state.pokemon_mut(target) {
        target_state.status = Some(installed);
        // Gen 1 (`StatRules::modified_stats`): paralysis quarters the modified Speed, burn
        // halves the modified Attack, the moment they are inflicted.
        target_state.apply_status_stat_cut();
    }
    trace.push(TraceEvent::StatusApplied {
        target,
        status,
        source,
    });
    // Gen 2 Toxic (`tox` `onStart`): the `residualdmg` counter starts (over) at 0.
    if status == NonVolatileStatus::Toxic
        && rules.status.toxic_model == crate::rules::ToxicModel::ResidualVolatile
        && state
            .pokemon_mut(target)
            .is_some_and(|p| p.volatiles.start_residual_damage())
    {
        trace.push(TraceEvent::VolatileStarted {
            target,
            kind: crate::state::VolatileConditionKind::ResidualDamage,
        });
    }

    // `AfterSetStatus` is a separate mutation boundary: prevention behavior have already completed and
    // the status is now visible in BattleState.
    let ability = state
        .pokemon(target)
        .and_then(|pokemon| crate::engine::ability_running(state, rules, pokemon));
    let item = super::held_item_id(state, rules, target);
    let mut ctx = BattleContext::new(state, rules, trace);
    // `frz`'s `onStart`: a frozen Shaymin-Sky is a Shaymin again, for good (DESIGN.md §43.2).
    if status == NonVolatileStatus::Freeze {
        ctx.freeze_sky_forme(target);
    }
    // Gen 4 Bide's `onAfterSetStatus` (§15.10): asleep or frozen, the holder lets it go.
    if rules.conditions.retaliation.bide.ends_on_sleep_or_freeze
        && matches!(status, NonVolatileStatus::Sleep | NonVolatileStatus::Freeze)
    {
        ctx.end_volatile(target, crate::state::VolatileConditionKind::Bide);
    }
    // Gen 3 (pokeemerald `SetMoveEffect`'s `CancelMultiTurnMoves`): an uproar's holder put to
    // sleep (a Soundproof one) or frozen loses it at once.
    if rules.conditions.uproar.frozen_or_asleep_cancels
        && matches!(status, NonVolatileStatus::Sleep | NonVolatileStatus::Freeze)
    {
        ctx.cancel_uproar(target);
    }
    // Gen 3-4 (the cartridges, §15.18): put to sleep or frozen, the Pokémon loses Fury Cutter's
    // count.
    if rules.conditions.fury_cutter.sleep_or_freeze_ends_it
        && matches!(status, NonVolatileStatus::Sleep | NonVolatileStatus::Freeze)
    {
        ctx.end_fury_cutter(target);
    }
    if let Some(ability) = ability {
        effects::abilities::after_set_status(
            super::ability_behavior(rules, ability),
            &mut ctx,
            target,
            status,
            source,
        );
    }
    // Showdown `onAnyAfterSetStatus` of every active Pokémon (Poison Puppeteer).
    let listeners: Vec<(PokemonId, effects::abilities::AnyAfterSetStatusHandler)> = ctx
        .active_pokemon()
        .into_iter()
        .filter_map(|holder| {
            let ability = ctx.running_ability(holder)?;
            super::ability_behavior(rules, ability)
                .any_after_set_status
                .map(|handler| (holder, handler))
        })
        .collect();
    for (holder, handler) in listeners {
        handler(&mut ctx, holder, target, status, source);
    }
    // The item last: Lum Berry's `onAfterSetStatusPriority` is −1, every other listener's 0.
    if let Some(item) = item {
        effects::items::after_set_status(
            super::item_behavior(rules, item),
            &mut ctx,
            target,
            status,
            source,
        );
    }
    // Gen 2: another Pokémon's move feeds the target its status berry at once (I33).
    let by = match source {
        EffectSource::Move { pokemon, .. } => Some(pokemon),
        _ => None,
    };
    ctx.status_inflicted(target, by);

    StatusApplyResult::Applied { target, status }
}

fn blocked(
    trace: &mut BattleTrace,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
    reason: StatusBlockReason,
) -> StatusApplyResult {
    blocked_shown(trace, target, status, source, reason, None)
}

/// `blocked`, its line naming `shown_by` (`TraceEvent::StatusBlocked`).
fn blocked_shown(
    trace: &mut BattleTrace,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
    reason: StatusBlockReason,
    shown_by: Option<EffectSource>,
) -> StatusApplyResult {
    trace.push(TraceEvent::StatusBlocked {
        target,
        status,
        source,
        reason,
        shown_by,
    });
    StatusApplyResult::Blocked {
        target,
        status,
        reason,
    }
}

fn status_type_immunity(ctx: &TrySetStatusContext<'_>) -> bool {
    // Corrosion (`setStatus`: `source.hasAbility('corrosion')`; no callback of its own): its
    // holder poisons Steel and Poison types too.
    let corrosive = matches!(
        ctx.status,
        NonVolatileStatus::Poison | NonVolatileStatus::Toxic
    ) && ctx
        .source
        .pokemon()
        .and_then(|source| ctx.state.pokemon(source))
        .is_some_and(|source| {
            super::has_ability(ctx.state, ctx.rules, source, AbilityId::CORROSION)
        });
    !corrosive && status_type_immune(ctx.state, ctx.rules, ctx.target, ctx.status)
}

/// Whether the target's ability, or one on its side, refuses `status` as it would be set
/// (`onImmunity`, `onSetStatus`, `onAllySetStatus`: Limber, Pastel Veil, Leaf Guard …). A status
/// move meets the refusal once it landed, after the accuracy check; shared with the replay
/// driver (DESIGN.md R277).
pub fn ability_refuses_status(
    state: &BattleState,
    rules: &ResolvedRules,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
) -> bool {
    let Some(pokemon) = state.pokemon(target) else {
        return false;
    };
    let handlers = super::ability_handlers(state, rules, pokemon);
    let ctx = TrySetStatusContext::new(state, rules, target, status, source);
    handlers.status_immunity == Some(effects::abilities::StatusImmunity::Status(status))
        || handlers.blocks_statuses.contains(&status)
        || handlers
            .in_forme_blocks
            .is_some_and(|guard| guard.statuses.contains(&status) && guard.holds(pokemon))
        || matches!(
            effects::abilities::try_set_status(handlers, &ctx),
            TrySetStatusResult::Blocked { .. }
        )
        || super::allied_abilities(state, rules, target)
            .into_iter()
            .filter_map(|behavior| behavior.ally_try_set_status)
            .any(|handler| matches!(handler(&ctx), TrySetStatusResult::Blocked { .. }))
}

/// Whether the target's type shields it from `status` (Showdown `runStatusImmunity`). A status
/// move meets it when it lands, after the accuracy check; shared with the replay driver.
pub fn status_type_immune(
    state: &BattleState,
    rules: &ResolvedRules,
    target: PokemonId,
    status: NonVolatileStatus,
) -> bool {
    let has_type = |type_: PokemonType| {
        state
            .pokemon(target)
            .is_some_and(|pokemon| crate::engine::types_of(state, rules, pokemon).contains(type_))
    };
    let immunities = rules.status;
    match status {
        NonVolatileStatus::Burn => {
            immunities.fire_type_burn_immunity && has_type(PokemonType::Fire)
        }
        NonVolatileStatus::Paralysis => {
            immunities.electric_type_paralysis_immunity && has_type(PokemonType::Electric)
        }
        NonVolatileStatus::Poison | NonVolatileStatus::Toxic => {
            (immunities.poison_type_poison_immunity && has_type(PokemonType::Poison))
                || (immunities.steel_type_poison_immunity && has_type(PokemonType::Steel))
        }
        NonVolatileStatus::Freeze => {
            immunities.ice_type_freeze_immunity && has_type(PokemonType::Ice)
        }
        NonVolatileStatus::Sleep => false,
    }
}

fn terrain_prevention(ctx: &TrySetStatusContext<'_>) -> Option<TerrainKind> {
    if !ctx.target_in_terrain() {
        return None;
    }
    match ctx.active_terrain() {
        Some(TerrainKind::Electric)
            if ctx.rules().status().electric_terrain_prevents_sleep
                && ctx.status() == NonVolatileStatus::Sleep =>
        {
            Some(TerrainKind::Electric)
        }
        Some(TerrainKind::Misty) if ctx.rules().status().misty_terrain_prevents_status => {
            Some(TerrainKind::Misty)
        }
        _ => None,
    }
}

/// The format's status clauses (`onSetStatus` of Sleep Clause Mod and Freeze Clause Mod,
/// DESIGN.md §45.7), on what a foe inflicts (`source?.isAlly(target)` returns): a sleep while a
/// Pokémon of the target's side still standing sleeps from a foe's doing (a Rest does not
/// count), a freeze while one of its Pokémon is frozen.
fn clause_refuses(
    state: &BattleState,
    rules: &ResolvedRules,
    target: PokemonId,
    status: NonVolatileStatus,
    inflicted_by: Option<crate::ids::SideId>,
) -> Option<StatusBlockReason> {
    let clauses = rules.clauses;
    let side = state.pokemon(target)?.side;
    if inflicted_by == Some(side) {
        return None;
    }
    let team = state
        .side(side)
        .team
        .iter()
        .filter_map(|id| state.pokemon(*id));
    let refused = match status {
        NonVolatileStatus::Sleep if clauses.sleep_clause_mod => team
            .filter(|pokemon| pokemon.hp > 0)
            .filter_map(|pokemon| pokemon.status)
            .any(|held| held.kind == NonVolatileStatus::Sleep && held.inflicted_by != Some(side)),
        NonVolatileStatus::Freeze if clauses.freeze_clause_mod => team
            .filter_map(|pokemon| pokemon.status)
            .any(|held| held.kind == NonVolatileStatus::Freeze),
        _ => false,
    };
    refused.then_some(StatusBlockReason::Clause)
}
