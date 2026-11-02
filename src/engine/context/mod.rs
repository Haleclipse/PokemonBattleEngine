use crate::dex::{MoveCategory, PokemonType, ResolvedMove};
use crate::ids::{AbilityId, ItemId, MoveId, PokemonId};
use crate::rules::{EffectRules, Fraction, ResolvedRules};
use crate::state::{
    BattleState, NonVolatileStatus, RandomRequestKind, SideConditionKind, Stat, TerrainKind,
    VolatileConditionKind, WeatherKind,
};

use super::{
    BattleTrace, CureReason, DamageDecision, DamageRequest, HealRequest, StatChangeCause,
    StatChangeRequest, TraceEvent, TryStatChangeResult,
};

mod ability_change;
mod ability_end;
mod trace_ability;
mod types;
pub use ability_end::AbilityEndCause;
pub(crate) use ability_end::restart_ties;
mod bide;
mod conditions;
mod crit;
mod curse;
mod drain;
mod eating;
pub use drain::{DrainedHeal, HealDrain};
pub(crate) use eating::unnerved_by_a_foe;
mod faint;
mod field;
mod focus;
mod forme;
pub use forme::FormeChangeKind;
mod fury_cutter;
mod grounding;
mod hazards;
mod hp;
mod illusion;
mod imprison;
mod items;
mod lock_on;
mod pp;
mod queries;
mod queue;
mod rage;
mod rampage;
pub use conditions::StallEffect;
mod reveal;
mod screens;
mod self_faint;
mod side_start;
mod sky_drop;
mod sleep;
mod stages;
mod staleness;
mod status;
mod steal_boosts;
mod stockpile;
mod switching;
mod terrain;
mod transform;
mod trapping;
mod truant;
mod update;
mod uproar;
pub(crate) use imprison::{imprisoned_by, tormented};
pub(crate) use lock_on::{LOCK_ON_TURNS, lock_on_reaches, locked_on};
pub(crate) use pp::{pressure_holders, takes_over_under_pressure};
pub(crate) use self_faint::damp_holder;
pub use sky_drop::sky_dropped;
pub(crate) use staleness::RESTORATIVE_BERRIES;
pub(crate) use truant::truant_loafs_next;
pub(crate) use update::PendingFaints;
pub use uproar::uproar_wakes_before_move;
pub(crate) use uproar::{keeping_awake, kept_awake};
mod weather;
mod weight;

/// One hit that reached a Pokémon's HP, handed to the target's ability / item reactions
/// (ROADMAP §6.5: Rough Skin, Iron Barbs, Rocky Helmet …). Runs once per hit of a multi-hit
/// move, after the hit's secondaries, ability before item (Showdown `onDamagingHitOrder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamagingHit {
    pub holder: PokemonId,
    pub attacker: PokemonId,
    pub move_id: MoveId,
    /// The move's type and category as it was used (after the user's ability retyped it;
    /// Shell Side Arm may have turned physical).
    pub move_type: PokemonType,
    pub category: MoveCategory,
    pub contact: bool,
    /// A contact move made no contact through the attacker's Protective Pads
    /// (`engine::move_makes_contact`): `padded_contact` announces both (R-M10-44).
    pub padded: bool,
    /// Showdown's `wind` flag (Wind Power).
    pub wind: bool,
    pub damage: u16,
    /// The HP the move's earlier hits took from the holder (a multi-hit move; Showdown
    /// `move.totalDamage` before this hit).
    pub earlier_damage: u16,
    /// The hit was super effective (Showdown `getMoveHitData(move).typeMod > 0`).
    pub super_effective: bool,
    /// The move deals a fixed damage (Showdown `move.damage` / `damageCallback`).
    pub fixed_damage: bool,
}

/// An ability or item reacting to a hit that reached its holder's HP (Rough Skin, Rocky
/// Helmet).
/// Showdown: `onDamagingHit`.
pub type DamagingHitReactionHandler = for<'a> fn(&mut BattleContext<'a>, DamagingHit);

/// A damaging move that hit the holder's HP, seen once after the whole move (Showdown
/// `onAfterMoveSecondary`): Red Card drags the attacker out, Eject Button orders the holder
/// out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AfterHit {
    pub holder: PokemonId,
    pub attacker: PokemonId,
    pub move_id: MoveId,
    /// The move's category as it was used.
    pub category: MoveCategory,
}

/// An item reacting once to the damaging move that hit its holder (Red Card, Eject Button).
/// Showdown: `onAfterMoveSecondary`.
pub type AfterHitHandler = for<'a> fn(&mut BattleContext<'a>, AfterHit);

/// The Resolve stage of a DamageRequest: sees the request before commit and may lower or
/// refuse it. Gets the mutable context because deciding can cost something (Focus Sash is
/// consumed, Sturdy announces itself).
/// Showdown: `onDamage`.
pub type DamageResolutionHandler =
    for<'a> fn(&mut BattleContext<'a>, &DamageRequest) -> DamageDecision;

/// ChangeBoost stage: rewrite the requested stages (Contrary negates, Simple doubles).
pub type ModifyStatChangeHandler = for<'a> fn(&StatChangeContext<'a>, i8) -> i8;
/// TryBoost stage: refuse the (already rewritten and capped) request (Clear Body).
pub type TryStatChangeHandler = for<'a> fn(&StatChangeContext<'a>) -> TryStatChangeResult;

/// Read-only view of a StatChangeRequest for the resolution behavior.
pub struct StatChangeContext<'a> {
    state: &'a BattleState,
    rules: &'a ResolvedRules,
    request: StatChangeRequest,
}

impl<'a> StatChangeContext<'a> {
    pub(crate) fn new(
        state: &'a BattleState,
        rules: &'a ResolvedRules,
        request: StatChangeRequest,
    ) -> Self {
        Self {
            state,
            rules,
            request,
        }
    }

    pub fn rules(&self) -> EffectRules<'_> {
        self.rules.for_effects()
    }

    pub fn request(&self) -> StatChangeRequest {
        self.request
    }

    pub fn target(&self) -> PokemonId {
        self.request.target
    }

    pub fn stat(&self) -> Stat {
        self.request.stat
    }

    pub fn stages(&self) -> i8 {
        self.request.stages
    }

    pub fn source(&self) -> EffectSource {
        self.request.source
    }

    /// The Pokémon behind the source, when there is one (a move's user, an ability's or
    /// item's holder).
    pub fn source_pokemon(&self) -> Option<PokemonId> {
        self.request.source.pokemon()
    }

    /// The change comes from a berry (Showdown `effect.isBerry`: an item eaten for an effect,
    /// `ItemBehavior::eat_effect`).
    pub fn source_is_berry(&self) -> bool {
        matches!(self.request.source, EffectSource::Item { item, .. }
            if crate::engine::item_behavior(self.rules, item).eat_effect.is_some())
    }

    /// Showdown `!effect.secondaries`: the change is no damaging move's secondary effect, which
    /// most refusals do not announce.
    pub fn is_secondary(&self) -> bool {
        self.request.cause == StatChangeCause::Secondary
    }

    /// Octolock's drops at the turn's end (`effect.id === 'octolock'`), which Clear Body and its
    /// kind refuse unannounced.
    pub fn is_octolock(&self) -> bool {
        matches!(
            self.request.source,
            EffectSource::Move {
                move_id: MoveId::OCTOLOCK,
                ..
            }
        )
    }

    /// Showdown `target === source`: the target caused the change itself (Close Combat's
    /// drops, Swords Dance), so Clear Body-style prevention does not apply.
    pub fn is_self_inflicted(&self) -> bool {
        self.source_pokemon() == Some(self.request.target)
    }

    pub fn target_stage(&self) -> i8 {
        self.state
            .pokemon(self.request.target)
            .map_or(0, |pokemon| pokemon.stages.get(self.request.stat))
    }

    pub fn target_has_type(&self, type_: PokemonType) -> bool {
        self.state
            .pokemon(self.request.target)
            .is_some_and(|pokemon| {
                crate::engine::types_of(self.state, self.rules, pokemon).contains(type_)
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectSource {
    Ability {
        pokemon: PokemonId,
        ability: AbilityId,
    },
    Item {
        pokemon: PokemonId,
        item: ItemId,
    },
    Move {
        pokemon: PokemonId,
        move_id: MoveId,
    },
    Status(NonVolatileStatus),
    Volatile(VolatileConditionKind),
    SideCondition(SideConditionKind),
    Weather(WeatherKind),
    Terrain(TerrainKind),
    Rule,
}

impl EffectSource {
    /// The Pokémon the effect belongs to, when there is one.
    pub fn pokemon(self) -> Option<PokemonId> {
        match self {
            Self::Ability { pokemon, .. }
            | Self::Item { pokemon, .. }
            | Self::Move { pokemon, .. } => Some(pokemon),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionApplyError {
    AlreadyActive,
    NotEnoughHp,
    InvalidTarget,
    /// The target is immune to the condition (Leech Seed on a Grass type) or protected from it
    /// (a foe's confusion under Safeguard).
    Immune,
    /// The move had nothing to do (Gen 6+ Defog, Court Change with no side condition).
    NothingToDo,
}

/// Every stat with a stage, in Showdown's `boosts` order.
pub const STAGED_STATS: [Stat; 7] = [
    Stat::Attack,
    Stat::Defense,
    Stat::SpecialAttack,
    Stat::SpecialDefense,
    Stat::Speed,
    Stat::Accuracy,
    Stat::Evasion,
];

/// The only mutation-oriented context exposed to state-changing effects.
///
/// Read-only modifier phases use narrower contexts below so an ability that merely modifies base
/// power cannot directly change HP or arbitrary battle state.
pub struct BattleContext<'a> {
    state: &'a mut BattleState,
    rules: &'a ResolvedRules,
    trace: &'a mut BattleTrace,
    /// The dex, where the caller has it (a custom primary): what a move does to the Pokémon
    /// using it (a semi-invulnerable charge).
    dex: Option<&'a crate::dex::ResolvedDex>,
}

impl<'a> BattleContext<'a> {
    /// Gen 1 Metronome and Mirror Move (`CallRules::caller_selects_the_called_move`): the
    /// move called becomes the side's last selected move (R-M8-72).
    pub fn select_the_called_move(&mut self, user: PokemonId, move_id: crate::ids::MoveId) {
        if !self.rules.conditions.calls.caller_selects_the_called_move {
            return;
        }
        if let Some(side) = self.state.pokemon(user).map(|pokemon| pokemon.side) {
            self.state.side_mut(side).last_selected_move =
                Some(crate::state::SelectedMove::Move(move_id));
        }
    }

    pub(crate) fn new(
        state: &'a mut BattleState,
        rules: &'a ResolvedRules,
        trace: &'a mut BattleTrace,
    ) -> Self {
        Self {
            state,
            rules,
            trace,
            dex: None,
        }
    }

    /// Pushes a trace event (an effect that records something of its own, §42.8).
    pub fn trace(&mut self, event: TraceEvent) {
        self.trace.push(event);
    }

    pub(crate) fn with_dex(mut self, dex: &'a crate::dex::ResolvedDex) -> Self {
        self.dex = Some(dex);
        self
    }

    /// Intrepid Sword / Dauntless Shield (`swordBoost` / `shieldBoost`): whether the entry
    /// boost to `stat` is due — always, or from Gen 9 only the first time this battle
    /// (`ConditionRules::entry_boost_once_per_battle`), which it then records.
    pub fn first_entry_boost(&mut self, pokemon: PokemonId, stat: Stat) -> bool {
        let once = self.rules.conditions.entry_boost_once_per_battle;
        let Some(holder) = self.state.pokemon_mut(pokemon) else {
            return false;
        };
        let given = match stat {
            Stat::Attack => &mut holder.history.sword_boost,
            _ => &mut holder.history.shield_boost,
        };
        if once && *given {
            return false;
        }
        *given = true;
        true
    }

    /// Supersweet Syrup (`syrupTriggered`): whether it has yet to act this battle, which it
    /// then records.
    pub fn first_syrup(&mut self, pokemon: PokemonId) -> bool {
        let Some(holder) = self.state.pokemon_mut(pokemon) else {
            return false;
        };
        !std::mem::replace(&mut holder.history.syrup_triggered, true)
    }

    pub fn item_state_mut(&mut self, pokemon: PokemonId) -> Option<&mut crate::state::ItemState> {
        self.state
            .pokemon_mut(pokemon)
            .map(|pokemon| &mut pokemon.item_state)
    }

    /// `pokemon`'s ability's state (Showdown `abilityState`).
    pub fn ability_state_mut(
        &mut self,
        pokemon: PokemonId,
    ) -> Option<&mut crate::state::AbilityState> {
        self.state
            .pokemon_mut(pokemon)
            .map(|pokemon| &mut pokemon.ability_state)
    }

    /// Order Up (DESIGN.md §43.4): the user of a commanded Dondozo raises one stat by the
    /// forme of the Tatsugiri it holds — Attack for a Curly one, Defense for a Droopy,
    /// Speed for a Stretchy.
    pub fn order_up(&mut self, user: PokemonId) {
        if !self.has_volatile(user, crate::state::VolatileConditionKind::Commanded) {
            return;
        }
        let Some(source) = self.adjacent_allies(user).into_iter().next() else {
            return;
        };
        let Some(forme) = self
            .state
            .pokemon(source)
            .and_then(|tatsugiri| tatsugiri.species.zip(crate::engine::species_of(tatsugiri)))
        else {
            return;
        };
        let stat = match forme.1 {
            crate::ids::SpeciesId::TATSUGIRI_DROOPY => crate::state::Stat::Defense,
            crate::ids::SpeciesId::TATSUGIRI_STRETCHY => crate::state::Stat::Speed,
            _ => crate::state::Stat::Attack,
        };
        self.try_change_stat_stage(StatChangeRequest {
            target: user,
            stat,
            stages: 1,
            source: EffectSource::Move {
                pokemon: user,
                move_id: MoveId::ORDER_UP,
            },
            cause: StatChangeCause::Move,
        });
    }

    /// Commander: the Tatsugiri takes the `commanding` volatile (DESIGN.md §43.4).
    pub fn start_commanding(&mut self, holder: PokemonId) -> bool {
        let started = self
            .state
            .pokemon_mut(holder)
            .is_some_and(|pokemon| pokemon.volatiles.set_commanding());
        if started {
            self.trace.push(TraceEvent::VolatileStarted {
                target: holder,
                kind: crate::state::VolatileConditionKind::Commanding,
            });
        }
        started
    }

    /// Commander: the Dondozo takes `commanded` — its `onStart` raises its five stats by two
    /// (`this.boost({atk: 2, spa: 2, spe: 2, def: 2, spd: 2})`).
    pub fn start_commanded(&mut self, holder: PokemonId) -> bool {
        let started = self
            .state
            .pokemon_mut(holder)
            .is_some_and(|pokemon| pokemon.volatiles.set_commanded());
        if !started {
            return false;
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target: holder,
            kind: crate::state::VolatileConditionKind::Commanded,
        });
        for stat in [
            crate::state::Stat::Attack,
            crate::state::Stat::SpecialAttack,
            crate::state::Stat::Speed,
            crate::state::Stat::Defense,
            crate::state::Stat::SpecialDefense,
        ] {
            self.try_change_stat_stage(StatChangeRequest {
                target: holder,
                stat,
                stages: 2,
                source: EffectSource::Volatile(crate::state::VolatileConditionKind::Commanded),
                cause: StatChangeCause::Ability,
            });
        }
        true
    }

    /// Zen Mode's volatile goes on (`addVolatile('zenmode')`).
    pub fn set_zen_mode(&mut self, pokemon: PokemonId) -> bool {
        let started = self
            .state
            .pokemon_mut(pokemon)
            .is_some_and(|pokemon| pokemon.volatiles.set_zen_mode());
        if started {
            self.trace.push(TraceEvent::VolatileStarted {
                target: pokemon,
                kind: crate::state::VolatileConditionKind::ZenMode,
            });
        }
        started
    }

    /// Disguise / Ice Face: the holder's forme has been broken (`AbilityState::busted`).
    pub fn busted(&self, holder: PokemonId) -> bool {
        self.state
            .pokemon(holder)
            .is_some_and(|pokemon| pokemon.ability_state.busted)
    }

    /// Gen 7 Disguise takes a hit of `user`'s Rollout / Ice Ball chain (issues.md I212).
    pub fn delay_rolling(&mut self, user: PokemonId, move_id: crate::ids::MoveId) {
        if let Some(pokemon) = self.state.pokemon_mut(user) {
            pokemon.volatiles.delay_rolling(move_id);
        }
    }

    pub fn set_busted(&mut self, holder: PokemonId, busted: bool) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.busted = busted;
        }
    }

    /// Battle Bond: it has raised its holder already this battle.
    pub fn bond_triggered(&self, holder: PokemonId) -> bool {
        self.state
            .pokemon(holder)
            .is_some_and(|pokemon| pokemon.bond_triggered)
    }

    pub fn mark_bond_triggered(&mut self, holder: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.bond_triggered = true;
        }
    }

    /// Zero to Hero: its Hero forme has announced itself since it changed.
    pub fn hero_announced(&self, holder: PokemonId) -> bool {
        self.state
            .pokemon(holder)
            .is_some_and(|pokemon| pokemon.hero_message_displayed)
    }

    pub fn mark_hero_announced(&mut self, holder: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.hero_message_displayed = true;
        }
    }

    /// Embody Aspect: the Ogerpon `-Tera` forme has raised its stat this stint on the field
    /// (`abilityState.embodied`; the entry resets the ability state, DESIGN.md §44.6).
    pub fn embodied(&self, holder: PokemonId) -> bool {
        self.state
            .pokemon(holder)
            .is_some_and(|pokemon| pokemon.ability_state.embodied)
    }

    pub fn mark_embodied(&mut self, holder: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.embodied = true;
        }
    }

    /// Slow Start's `onStart`: its countdown starts at 5.
    pub fn start_slow_start(&mut self, holder: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.slow_start = 5;
        }
    }

    /// The Metronome item's `onStart`: the holder's `metronome` condition, counting from none
    /// (a holder that has one keeps its count).
    pub fn start_metronome(&mut self, holder: PokemonId) {
        if self
            .state
            .pokemon_mut(holder)
            .is_some_and(|pokemon| pokemon.volatiles.start_metronome())
        {
            self.trace.push(TraceEvent::VolatileStarted {
                target: holder,
                kind: crate::state::VolatileConditionKind::Metronome,
            });
        }
    }

    /// Slow Start's `onResidual`: one turn end off the countdown, if it still runs.
    pub fn tick_slow_start(&mut self, holder: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.slow_start = pokemon.ability_state.slow_start.saturating_sub(1);
        }
    }

    /// Showdown `sample(items)` over `len` items: the index drawn (`Sample`), nothing drawn when
    /// there is one.
    pub fn sample_index(&mut self, len: usize) -> usize {
        usize::from(self.state.rng.range_inclusive_for(
            RandomRequestKind::Sample,
            0,
            len.saturating_sub(1).min(usize::from(u16::MAX)) as u16,
        ))
    }

    /// A typed yes / no decision for an effect (Static's 30 %, Shed Skin's 33 %).
    pub fn chance(&mut self, kind: RandomRequestKind, fraction: Fraction) -> bool {
        self.state
            .rng
            .roll_ratio_for(kind, fraction.numerator, fraction.denominator)
    }

    pub fn shed_skin_checked(&mut self, holder: PokemonId, triggered: bool) {
        self.trace
            .push(TraceEvent::ShedSkinChecked { holder, triggered });
    }
}

pub struct TryHitContext<'a> {
    state: &'a BattleState,
    rules: &'a ResolvedRules,
    user: PokemonId,
    target: PokemonId,
    move_def: &'a ResolvedMove,
}

impl<'a> TryHitContext<'a> {
    pub(crate) fn new(
        state: &'a BattleState,
        rules: &'a ResolvedRules,
        user: PokemonId,
        target: PokemonId,
        move_def: &'a ResolvedMove,
    ) -> Self {
        Self {
            state,
            rules,
            user,
            target,
            move_def,
        }
    }

    pub fn user(&self) -> PokemonId {
        self.user
    }
    pub fn target(&self) -> PokemonId {
        self.target
    }
    pub fn move_id(&self) -> MoveId {
        self.move_def.id
    }
    pub fn move_type(&self) -> PokemonType {
        self.move_def.move_type
    }
    pub fn move_category(&self) -> MoveCategory {
        self.move_def.category
    }
    /// Showdown's `wind` flag (Wind Rider).
    pub fn move_is_wind(&self) -> bool {
        self.move_def.flags.wind
    }
    /// Whether the move's user carries `kind` (Sky Drop's `skydrop`).
    pub fn user_has_volatile(&self, kind: crate::state::VolatileConditionKind) -> bool {
        self.state
            .pokemon(self.user)
            .is_some_and(|pokemon| pokemon.volatiles.contains(kind))
    }
    /// The move's flags (`bullet`, `sound` …).
    pub fn move_flags(&self) -> &crate::dex::MoveFlags {
        &self.move_def.flags
    }
    /// Showdown `runEffectiveness(move) > 0 && runImmunity(move)` on the target: the move is
    /// super effective on the chart and nothing makes the target immune (Wonder Guard).
    pub fn super_effective_on_target(&self) -> bool {
        let effectiveness = crate::engine::move_pipeline::chart_effectiveness(
            self.state,
            self.rules,
            self.target,
            self.move_def,
        );
        effectiveness.numerator > effectiveness.denominator
            && crate::engine::type_immunity(self.state, self.rules, self.target, self.move_def)
                .1
                .is_none()
    }
    pub fn rules(&self) -> EffectRules<'_> {
        self.rules.for_effects()
    }
    pub fn target_has_type(&self, type_: PokemonType) -> bool {
        self.state.pokemon(self.target).is_some_and(|pokemon| {
            crate::engine::types_of(self.state, self.rules, pokemon).contains(type_)
        })
    }
    pub fn target_status(&self) -> Option<crate::state::NonVolatileStatus> {
        self.state
            .pokemon(self.target)
            .and_then(|pokemon| pokemon.status.map(|status| status.kind))
    }
    pub fn target_behind_substitute(&self) -> bool {
        self.state.pokemon(self.target).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .contains(crate::state::VolatileConditionKind::Substitute)
        })
    }
    /// The user is on the target's side (an ally's spread move, a move aimed at a partner).
    pub fn user_is_ally(&self) -> bool {
        self.user != self.target
            && self.state.pokemon(self.user).map(|p| p.side)
                == self.state.pokemon(self.target).map(|p| p.side)
    }
}

pub struct OffensiveStatContext<'a> {
    state: &'a BattleState,
    rules: &'a ResolvedRules,
    user: PokemonId,
    target: PokemonId,
    move_def: &'a ResolvedMove,
}

impl<'a> OffensiveStatContext<'a> {
    pub(crate) fn new(
        state: &'a BattleState,
        rules: &'a ResolvedRules,
        user: PokemonId,
        target: PokemonId,
        move_def: &'a ResolvedMove,
    ) -> Self {
        Self {
            state,
            rules,
            user,
            target,
            move_def,
        }
    }

    pub fn user(&self) -> PokemonId {
        self.user
    }
    pub fn target(&self) -> PokemonId {
        self.target
    }
    pub fn move_id(&self) -> MoveId {
        self.move_def.id
    }
    pub fn move_category(&self) -> MoveCategory {
        self.move_def.category
    }
    pub fn rules(&self) -> EffectRules<'_> {
        self.rules.for_effects()
    }
    pub fn user_is_burned(&self) -> bool {
        self.state
            .pokemon(self.user)
            .is_some_and(|pokemon| pokemon.has_status(NonVolatileStatus::Burn))
    }
}

pub struct BasePowerContext<'a> {
    state: &'a BattleState,
    rules: &'a ResolvedRules,
    user: PokemonId,
    target: Option<PokemonId>,
    move_def: &'a ResolvedMove,
}

impl<'a> BasePowerContext<'a> {
    pub(crate) fn new(
        state: &'a BattleState,
        rules: &'a ResolvedRules,
        user: PokemonId,
        target: Option<PokemonId>,
        move_def: &'a ResolvedMove,
    ) -> Self {
        Self {
            state,
            rules,
            user,
            target,
            move_def,
        }
    }

    pub fn user(&self) -> PokemonId {
        self.user
    }
    pub fn target(&self) -> Option<PokemonId> {
        self.target
    }
    pub fn move_id(&self) -> MoveId {
        self.move_def.id
    }
    pub fn move_type(&self) -> PokemonType {
        self.move_def.move_type
    }
    pub fn move_category(&self) -> MoveCategory {
        self.move_def.category
    }
    pub fn rules(&self) -> EffectRules<'_> {
        self.rules.for_effects()
    }

    pub fn user_is_grounded(&self) -> bool {
        self.state.pokemon(self.user).is_some_and(|pokemon| {
            crate::engine::environment::grounded(self.state, self.rules, pokemon)
        })
    }

    pub fn active_terrain(&self) -> Option<TerrainKind> {
        self.state.field.terrain.map(|terrain| terrain.kind)
    }
}

/// Whether `hp_before` was above half of `max_hp` and `hp` is not, with `hp` still standing:
/// the crossing Emergency Exit and Wimp Out react to (Showdown's `onEmergencyExit` callers
/// compare these numbers themselves, so which pair they hand in — doubled or undynamaxed —
/// is theirs, DESIGN.md §44.5).
fn crossed_half(hp: u16, max_hp: u16, hp_before: u16) -> bool {
    hp > 0 && u32::from(hp) * 2 <= u32::from(max_hp) && u32::from(hp_before) * 2 > u32::from(max_hp)
}
