use crate::dex::{MoveCategory, PokemonType, ResolvedMove};
use crate::ids::{AbilityId, ItemId, MoveId, PokemonId, SideId};
use crate::rules::{EffectRules, Fraction, ResolvedRules, ResolvedTarget};
use crate::state::{
    BattleState, NonVolatileStatus, RandomRequestKind, SideConditionKind, Stat, TerrainKind,
    VolatileConditionKind, WeatherKind,
};

use super::{
    BattleTrace, CureReason, DamageAmount, DamageApplication, DamageDecision, DamageKind,
    DamageRequest, HealBlockReason, HealRequest, HealResult, StatChangeBlockReason,
    StatChangeCause, StatChangeRequest, StatChangeResult, TraceEvent, TryStatChangeResult,
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
mod illusion;
mod imprison;
mod items;
mod lock_on;
mod pp;
mod queue;
mod rage;
mod rampage;
pub use conditions::StallEffect;
mod screens;
mod self_faint;
mod side_start;
mod sky_drop;
mod sleep;
mod staleness;
mod steal_boosts;
mod stockpile;
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

    /// The dex, for a custom primary that has to look at moves other than its own (the callers
    /// of §42.1 draw among them). Only a custom primary's context has one.
    pub fn dex(&self) -> Option<&'a crate::dex::ResolvedDex> {
        self.dex
    }

    /// Pushes a trace event (an effect that records something of its own, §42.8).
    pub fn trace(&mut self, event: TraceEvent) {
        self.trace.push(event);
    }

    /// The battle state, for an effect that has to look beyond its own targets (`engine::calls`
    /// draws among the party and the whole dex).
    pub(crate) fn battle_state(&self) -> &crate::state::BattleState {
        self.state
    }

    /// The resolved rules, with the generation's tables (not the generation-less `EffectRules`
    /// a modifier phase sees).
    pub(crate) fn battle_rules(&self) -> &'a ResolvedRules {
        self.rules
    }

    /// Gen 3 Sleep Talk (`cant|p|nopp|<move>`): the draw landed on a move with no PP left, so
    /// the move is not used.
    pub fn moves_without_pp(&mut self, actor: PokemonId, move_id: MoveId) {
        self.trace
            .push(TraceEvent::MovedWithoutPp { actor, move_id });
    }

    pub(crate) fn with_dex(mut self, dex: &'a crate::dex::ResolvedDex) -> Self {
        self.dex = Some(dex);
        self
    }

    /// The TryHit of `target`'s ability and item against `move_id` from `user`
    /// (`engine::holders_try_hit`), traced as a blocked hit when one refuses it. Needs the dex.
    pub fn holders_refuse(&mut self, user: PokemonId, target: PokemonId, move_id: MoveId) -> bool {
        let Some(move_def) = self.dex.and_then(|dex| dex.move_by_id(move_id)) else {
            return false;
        };
        // The move as its user uses it (`modified_move`): Liquid Voice makes Perish Song a Water
        // move, which Water Absorb takes (DESIGN.md R-M9-28).
        let move_def =
            crate::engine::modified_move_cow(self.state, self.rules, user, move_def, false).0;
        let Some(reason) =
            crate::engine::holders_try_hit(self.state, self.rules, user, target, &move_def)
        else {
            return false;
        };
        // An absorbing ability's refusal has its side effect here too (Water Absorb's heal), as
        // in the hit gate.
        if let crate::engine::HitBlockReason::Ability(ability) = reason
            && let Some(absorb) =
                crate::engine::ability_behavior(self.rules, ability).absorb_reaction
        {
            absorb(self, target);
        }
        self.trace.push(TraceEvent::HitBlocked {
            actor: user,
            target,
            move_id,
            reason,
        });
        true
    }

    /// A field or side move's own `runEvent('TryHit')` on a Max Guard holder (Flower Shield, Gear
    /// Up, Magnetic Flux: `!pokemon.volatiles['maxguard'] || runEvent('TryHit', …)`): Max Guard
    /// refuses it (`-activate|target|move: Max Guard`), traced as a blocked hit.
    pub fn max_guard_refuses(
        &mut self,
        user: PokemonId,
        target: PokemonId,
        move_id: MoveId,
    ) -> bool {
        let guarded = self
            .state
            .pokemon(target)
            .and_then(|pokemon| pokemon.volatiles.protect_kind())
            == Some(crate::state::ProtectKind::MaxGuard);
        if guarded {
            self.trace.push(TraceEvent::HitBlocked {
                actor: user,
                target,
                move_id,
                reason: crate::engine::HitBlockReason::Protect,
            });
        }
        guarded
    }

    /// Showdown `runEvent('Invulnerability')` for a field-wide move's `onHitField` (Perish
    /// Song, Teatime): `pokemon` is out of reach in a semi-invulnerable charge (Fly, Dig …)
    /// unless No Guard — `user`'s or its own — reaches it.
    pub fn out_of_reach(&self, user: PokemonId, pokemon: PokemonId) -> bool {
        self.semi_invulnerable(pokemon)
            && !crate::engine::no_guard(self.state, self.rules, user, pokemon)
    }

    /// Showdown `isSemiInvulnerable`: `pokemon` is in a semi-invulnerable charge (Fly, Dig …)
    /// or carried up by Sky Drop.
    pub fn semi_invulnerable(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|p| crate::engine::two_turn::hiding(p).is_some())
    }

    pub fn rules(&self) -> EffectRules<'_> {
        self.rules.for_effects()
    }

    pub fn turn(&self) -> u32 {
        self.state.turn
    }

    pub fn side_of(&self, pokemon: PokemonId) -> Option<SideId> {
        self.state.pokemon(pokemon).map(|pokemon| pokemon.side)
    }

    pub fn max_hp(&self, pokemon: PokemonId) -> Option<u16> {
        self.state.pokemon(pokemon).map(|pokemon| pokemon.max_hp)
    }

    /// Showdown `baseMaxhp`: the max HP a Dynamax does not double.
    pub fn base_max_hp(&self, pokemon: PokemonId) -> Option<u16> {
        self.state
            .pokemon(pokemon)
            .map(|pokemon| pokemon.base_max_hp())
    }

    /// Every foe of `pokemon` on the field that is still up (Showdown `foes()`: adjacent or
    /// not).
    /// Anticipation's question (`engine::anticipates`).
    pub fn anticipates(&self, holder: PokemonId) -> bool {
        crate::engine::anticipates(self.state, self.rules, holder)
    }

    /// Forewarn's candidates (`engine::forewarn_candidates`).
    pub fn forewarn_candidates(&self, holder: PokemonId) -> Vec<(MoveId, PokemonId)> {
        crate::engine::forewarn_candidates(self.state, self.rules, holder)
    }

    /// Showdown `alliesAndSelf()`: `pokemon` and its active allies, in position order.
    pub fn allies_and_self(&self, pokemon: PokemonId) -> Vec<PokemonId> {
        let Some(side) = self.side_of(pokemon) else {
            return Vec::new();
        };
        self.state
            .side(side)
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|ally| !self.is_fainted(*ally))
            .collect()
    }

    pub fn active_foes(&self, pokemon: PokemonId) -> Vec<PokemonId> {
        let Some(side) = self.side_of(pokemon) else {
            return Vec::new();
        };
        self.state
            .side(side.other())
            .active
            .iter()
            .flatten()
            .copied()
            .filter(|foe| !self.is_fainted(*foe))
            .collect()
    }

    /// `engine::suppressing_ability` for `pokemon`.
    pub fn suppressing_ability(&self, pokemon: PokemonId) -> bool {
        self.state.pokemon(pokemon).is_some_and(|holder| {
            crate::engine::suppressing_ability(self.state, self.rules, holder)
        })
    }

    pub fn has_volatile(&self, pokemon: PokemonId, kind: VolatileConditionKind) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|p| p.volatiles.contains(kind))
    }

    /// Showdown `getStat(stat, false, true)`: the stored stat at its stage, no modifier.
    pub fn staged_stat(&self, pokemon: PokemonId, stat: Stat) -> u32 {
        self.stat_at_stage_of(pokemon, stat, stat)
    }

    /// `stat` as stored, at `stage_of`'s stage (Download under Wonder Room reads the other
    /// defensive stat's stage).
    pub fn stat_at_stage_of(&self, pokemon: PokemonId, stat: Stat, stage_of: Stat) -> u32 {
        self.state.pokemon(pokemon).map_or(0, |p| {
            crate::engine::stats::apply_stage(
                self.rules,
                p.battle_stats().get(stat).unwrap_or(0),
                p.stages.get(stage_of),
            )
        })
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

    /// `pokemon`'s own species (Showdown `baseSpecies`): Poison Puppeteer's Pecharunt.
    pub fn own_species(&self, pokemon: PokemonId) -> Option<crate::ids::SpeciesId> {
        self.state
            .pokemon(pokemon)
            .and_then(|pokemon| pokemon.species)
    }

    /// Showdown `clearBoosts()`: every stat stage of `target` back to 0, directly (no TryBoost);
    /// each change is a `StatChanged`. Gen 1's (the gen1 `clearBoosts`) also puts the modified
    /// stats back to the stored ones — a paralysis or burn cut included.
    pub fn clear_stages(&mut self, target: PokemonId, source: EffectSource) {
        for stat in STAGED_STATS {
            self.set_stage(target, stat, 0, source);
        }
        if let Some(pokemon) = self.state.pokemon_mut(target)
            && pokemon.modified_stats.is_some()
        {
            pokemon.modified_stats = Some(*pokemon.battle_stats());
        }
    }

    /// `target`'s stage in `stat` (0 for a Pokémon not in the battle).
    pub fn stage(&self, target: PokemonId, stat: Stat) -> i8 {
        self.state
            .pokemon(target)
            .map_or(0, |pokemon| pokemon.stages.get(stat))
    }

    /// Showdown `setBoost` / a direct `boosts[stat] = …`: `target`'s stage in `stat` becomes
    /// `to` with no TryBoost (Heart Swap, Topsy-Turvy); a change is a `StatChanged`.
    pub fn set_stage(&mut self, target: PokemonId, stat: Stat, to: i8, source: EffectSource) {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return;
        };
        let from = pokemon.stages.get(stat);
        if from != to {
            pokemon.stages.set(stat, to);
            self.trace.push(TraceEvent::StatChanged {
                target,
                stat,
                from,
                to,
                source,
            });
        }
    }

    /// `pokemon`'s stored stat (Showdown `storedStats`; 0 for accuracy / evasion).
    pub fn stored_stat(&self, pokemon: PokemonId, stat: Stat) -> u16 {
        self.state
            .pokemon(pokemon)
            .and_then(|p| p.battle_stats().get(stat))
            .unwrap_or(0)
    }

    /// A move rewrites `pokemon`'s stored stat until it leaves the field
    /// (`PokemonState::set_stored_stat`).
    pub fn set_stored_stat(&mut self, pokemon: PokemonId, stat: Stat, value: u16) {
        if let Some(p) = self.state.pokemon_mut(pokemon) {
            p.set_stored_stat(stat, value);
        }
    }

    /// Showdown gen2 `getStat(stat, false, true)` of `pokemon` at `stage`
    /// (`stats::capped_stat_reading`: Gen 2 Belly Drum's climb).
    pub fn capped_stat_reading(&self, pokemon: PokemonId, stat: Stat, stage: i8) -> u32 {
        self.state.pokemon(pokemon).map_or(0, |p| {
            crate::engine::stats::capped_stat_reading(self.state, self.rules, p, stat, stage)
        })
    }

    /// Showdown `sethp`: `target`'s HP becomes `hp` (at least 1, at most its max); nothing for a
    /// fainted Pokémon. Traced as `HpSet`.
    pub fn set_hp(&mut self, target: PokemonId, hp: u16, source: EffectSource) {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return;
        };
        if pokemon.is_fainted() {
            return;
        }
        let before = pokemon.hp;
        pokemon.hp = hp.clamp(1, pokemon.max_hp);
        let after = pokemon.hp;
        self.trace.push(TraceEvent::HpSet {
            target,
            hp_before: before,
            hp_after: after,
            source,
        });
    }

    pub fn hp(&self, pokemon: PokemonId) -> Option<u16> {
        self.state.pokemon(pokemon).map(|pokemon| pokemon.hp)
    }

    pub fn is_grounded(&self, pokemon: PokemonId) -> bool {
        self.state.pokemon(pokemon).is_some_and(|pokemon| {
            crate::engine::environment::grounded(self.state, self.rules, pokemon)
        })
    }

    /// Whether a terrain's guard covers `pokemon` (Misty and Electric Terrain's `onSetStatus`
    /// / `onTryAddVolatile`: `isGrounded() && !isSemiInvulnerable()`).
    pub fn terrain_covers(&self, pokemon: PokemonId) -> bool {
        self.is_grounded(pokemon) && !self.semi_invulnerable(pokemon)
    }

    pub fn has_type(&self, pokemon: PokemonId, type_: PokemonType) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|pokemon| crate::engine::has_type(self.state, self.rules, pokemon, type_))
    }

    /// Showdown `useItem` (Focus Sash, a gem, Red Card, Eject Button): the held item is used
    /// up and traced; it becomes the holder's last item and the holder has used an item this
    /// turn (§39.1). Returns the item. A fainted holder uses nothing but a gem (`useItem`: `!this.hp
    /// && !isGem`): Seed Sower's terrain as its holder falls leaves the seed held, while the gem
    /// of an Explosion user, down before its move strikes, still goes. (Showdown's holder out of
    /// play never reaches it here: only active Pokémon use items.)
    pub fn consume_item(&mut self, holder: PokemonId) -> Option<ItemId> {
        self.use_up_item(holder, true)
    }

    /// `consume_item` with no message (Natural Gift's berry: `setItem('')`).
    pub fn consume_item_unseen(&mut self, holder: PokemonId) -> Option<ItemId> {
        self.use_up_item(holder, false)
    }

    fn use_up_item(&mut self, holder: PokemonId, seen: bool) -> Option<ItemId> {
        let pokemon = self.state.pokemon(holder)?;
        if pokemon.hp == 0
            && !pokemon
                .held_item
                .is_some_and(|item| crate::engine::item_behavior(self.rules, item).is_gem)
        {
            return None;
        }
        let pokemon = self.state.pokemon_mut(holder)?;
        let item = pokemon.held_item.take()?;
        pokemon.item_state = crate::state::ItemState::default();
        pokemon.history.last_item = Some(item);
        pokemon.history.used_item_this_turn = true;
        self.trace
            .push(TraceEvent::ItemConsumed { holder, item, seen });
        self.after_use_item(holder);
        Some(item)
    }

    /// The ability `pokemon` has in effect (Showdown `hasAbility`, `engine::ability_active`).
    pub fn ability_of(&self, pokemon: PokemonId) -> Option<AbilityId> {
        self.state
            .pokemon(pokemon)
            .and_then(|pokemon| crate::engine::ability_active(self.state, self.rules, pokemon))
    }

    /// The ability whose handlers run for `pokemon` (`engine::ability_running`).
    pub fn running_ability(&self, pokemon: PokemonId) -> Option<AbilityId> {
        self.state
            .pokemon(pokemon)
            .and_then(|pokemon| crate::engine::ability_running(self.state, self.rules, pokemon))
    }

    /// The terrain on the field, if any.
    pub fn terrain(&self) -> Option<crate::state::TerrainKind> {
        self.state.field.terrain.map(|terrain| terrain.kind)
    }

    /// Whether `pokemon` battles as this species (Showdown `baseSpecies`, the one its set names;
    /// `engine::species_of` is what it battles as after a forme change).
    pub fn is_species(&self, pokemon: PokemonId, species: crate::ids::SpeciesId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|pokemon| pokemon.species == Some(species))
    }

    /// Whether the field has this condition (Trick Room, Gravity …).
    pub fn field_has(&self, kind: crate::state::FieldConditionKind) -> bool {
        self.state.field.contains(kind)
    }

    /// `pokemon`'s held item and its behavior, when its effects apply (`engine::held_item`).
    pub fn held_item(
        &self,
        pokemon: PokemonId,
    ) -> Option<(ItemId, &'static crate::effects::items::ItemBehavior)> {
        crate::engine::held_item(self.state, self.rules, pokemon)
    }

    /// Whether `pokemon` has `ability` in effect (Showdown `hasAbility`).
    pub fn has_ability(&self, pokemon: PokemonId, ability: AbilityId) -> bool {
        self.state.pokemon(pokemon).is_some_and(|pokemon| {
            crate::engine::has_ability(self.state, self.rules, pokemon, ability)
        })
    }

    /// Whether `pokemon`'s moves infiltrate (Showdown `move.infiltrates`: Infiltrator).
    pub fn infiltrates(&self, pokemon: PokemonId) -> bool {
        crate::engine::infiltrates(self.state, self.rules, pokemon)
    }

    pub fn is_active(&self, pokemon: PokemonId) -> bool {
        self.state
            .active_position(pokemon, &self.rules.topology)
            .is_some()
    }

    /// Whether `side` has a healthy Pokémon on the bench (Showdown `canSwitch`).
    pub fn can_switch(&self, side: SideId) -> bool {
        !self.state.healthy_bench(side).is_empty()
    }

    pub fn is_drag_pending(&self, pokemon: PokemonId) -> bool {
        self.state.drag_queue.contains(&pokemon)
    }

    /// Whether `pokemon` is leaving the field by its own switch, Pursuit striking it as it goes
    /// (Showdown `beingCalledBack`, `BattleState::called_back`).
    pub fn is_called_back(&self, pokemon: PokemonId) -> bool {
        self.state.called_back == Some(pokemon)
    }

    /// Orders `target` out for a random bench Pokémon at the end of the action (Roar, Dragon
    /// Tail, Red Card). Refused when the target is not standing, its side has no bench, or it
    /// is already being dragged.
    pub fn request_drag_out(&mut self, target: PokemonId, source: EffectSource) -> bool {
        let Some(side) = self.side_of(target) else {
            return false;
        };
        if self.is_fainted(target)
            || !self.is_active(target)
            || !self.can_switch(side)
            || self.is_drag_pending(target)
        {
            return false;
        }
        // Showdown `DragOut` event: Suction Cups and Ingrain refuse (the phazing status move
        // then fails; a damaging phazer or Red Card just does not drag).
        if let Some(by) = self.drag_out_blocker(target) {
            self.trace.push(TraceEvent::DragOutBlocked { target, by });
            return false;
        }
        self.state.drag_queue.push(target);
        self.trace
            .push(TraceEvent::DragOutRequested { target, source });
        true
    }

    fn drag_out_blocker(&self, target: PokemonId) -> Option<EffectSource> {
        let pokemon = self.state.pokemon(target)?;
        // Commander's two volatiles (`onDragOut` at priority 2, returning false; DESIGN.md
        // §43.4): neither the Tatsugiri in its ally's mouth nor its Dondozo is dragged out.
        for kind in [
            crate::state::VolatileConditionKind::Commanding,
            crate::state::VolatileConditionKind::Commanded,
        ] {
            if pokemon.volatiles.contains(kind) {
                return Some(EffectSource::Volatile(kind));
            }
        }
        if crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.blocks_drag_out) {
            return Some(EffectSource::Ability {
                pokemon: target,
                ability: crate::engine::raw_ability(pokemon),
            });
        }
        if pokemon.volatiles.contains(VolatileConditionKind::Ingrain) {
            return Some(EffectSource::Volatile(VolatileConditionKind::Ingrain));
        }
        // A Dynamaxed Pokémon is not dragged out (`dynamax.onDragOut`, DESIGN.md §44.5).
        if pokemon.volatiles.contains(VolatileConditionKind::Dynamax) {
            return Some(EffectSource::Volatile(VolatileConditionKind::Dynamax));
        }
        None
    }

    /// Whether any active Pokémon is already ordered out for a chosen replacement (U-turn,
    /// Eject Button).
    pub fn any_switch_out_pending(&self) -> bool {
        [SideId::P1, SideId::P2]
            .into_iter()
            .any(|side| self.state.side(side).switch_out.is_some())
    }

    /// Orders `pokemon` out for a replacement its trainer chooses (the mid-turn ForcedSwitch
    /// request); Eject Button uses it the way U-turn does.
    pub fn request_switch_out(&mut self, pokemon: PokemonId) -> bool {
        let Some(side) = self.side_of(pokemon) else {
            return false;
        };
        if self.is_fainted(pokemon) || !self.is_active(pokemon) || !self.can_switch(side) {
            return false;
        }
        let side = self.state.side_mut(side);
        side.switch_out = Some(pokemon);
        side.passing = None;
        side.switch_out_by_move = false;
        true
    }

    /// A move's order to its own user to leave (Parting Shot): `request_switch_out`, the order
    /// marked as the move's (`SideState::switch_out_by_move`).
    pub fn request_self_switch(&mut self, user: PokemonId) -> bool {
        let ordered = self.request_switch_out(user);
        if ordered && let Some(side) = self.side_of(user) {
            self.state.side_mut(side).switch_out_by_move = true;
        }
        ordered
    }

    /// Whether a Pokémon on the field is ordered out by an item or an ability (Showdown
    /// `switchFlag === true`), not by its own move.
    pub fn chosen_switch_pending(&self) -> bool {
        [SideId::P1, SideId::P2].into_iter().any(|side| {
            let side = self.state.side(side);
            side.switch_out.is_some() && !side.switch_out_by_move
        })
    }

    /// The move of the action under way (Showdown `activeMove.id`).
    pub fn active_move(&self) -> Option<MoveId> {
        self.state.active_move.map(|active| active.move_id)
    }

    /// `pokemon`'s held item's state (Showdown `itemState`).
    pub fn item_state(&self, pokemon: PokemonId) -> Option<&crate::state::ItemState> {
        self.state
            .pokemon(pokemon)
            .map(|pokemon| &pokemon.item_state)
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

    /// Showdown's `EmergencyExit` event (Emergency Exit, Wimp Out's `onEmergencyExit`): once
    /// `pokemon`'s HP has gone from above half its max (`hp_before`) to half or less while it
    /// stays up, it is ordered out for a replacement its trainer chooses — and every other
    /// chosen switch on the field (a U-turn's, an Eject Button's) is called off. Nothing happens
    /// when its side has no one to send in, it is being dragged out, or it already leaves.
    pub fn emergency_exit(&mut self, pokemon: PokemonId, hp_before: u16) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        let (hp, max_hp) = (holder.hp, holder.max_hp);
        let crossed = crossed_half(hp, max_hp, hp_before);
        self.emergency_exit_crossed(pokemon, crossed);
    }

    /// The residual's own check (`battle.mjs`'s `residual` action): both the HP and the half-max
    /// threshold read through `getUndynamaxedHP` (DESIGN.md §44.5) — unlike the after-move
    /// checks above, which compare the doubled numbers as they stand. `hp_before` is the
    /// undynamaxed HP the residual started with. Whether the ability ordered the holder out.
    pub fn emergency_exit_residual(&mut self, pokemon: PokemonId, hp_before: u16) -> bool {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return false;
        };
        let max_hp = holder.undynamaxed_hp(Some(holder.max_hp));
        let crossed = crossed_half(holder.undynamaxed_hp(None), max_hp, hp_before);
        self.emergency_exit_crossed(pokemon, crossed)
    }

    fn emergency_exit_crossed(&mut self, pokemon: PokemonId, crossed: bool) -> bool {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return false;
        };
        let (side, ability) = (holder.side, crate::engine::raw_ability(holder));
        let exits =
            crate::engine::ability_handler(self.state, self.rules, holder, |b| b.emergency_exit);
        if !crossed
            || !exits
            || !self.is_active(pokemon)
            || !self.can_switch(side)
            || self.is_drag_pending(pokemon)
            || self.state.side(side).switch_out == Some(pokemon)
        {
            return false;
        }
        for each in [SideId::P1, SideId::P2] {
            self.state.side_mut(each).switch_out = None;
        }
        self.state.side_mut(side).switch_out = Some(pokemon);
        self.state.side_mut(side).switch_out_by_move = false;
        self.ability_activated(pokemon, ability);
        true
    }

    pub fn cancel_switch_out(&mut self, pokemon: PokemonId) {
        if let Some(side) = self.side_of(pokemon)
            && self.state.side(side).switch_out == Some(pokemon)
        {
            self.state.side_mut(side).switch_out = None;
        }
    }

    /// Showdown `activeTurns`: 0 on the turn the Pokémon came in, then counted at every turn
    /// start.
    /// The holder's level (`PokemonState::level`; Schooling's 20).
    pub fn level(&self, pokemon: PokemonId) -> u8 {
        self.state
            .pokemon(pokemon)
            .map_or(0, |pokemon| pokemon.level)
    }

    /// The species the holder battles as (`engine::species_of`).
    pub fn species_of(&self, pokemon: PokemonId) -> Option<crate::ids::SpeciesId> {
        self.state
            .pokemon(pokemon)
            .and_then(crate::engine::species_of)
    }

    /// The self-changing ability's data the holder's own (`ability_handlers`: suppressed
    /// abilities run nothing, DESIGN.md §43.4).
    pub fn self_forme(&self, pokemon: PokemonId) -> Option<crate::effects::abilities::SelfForme> {
        self.state.pokemon(pokemon).and_then(|pokemon| {
            crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.self_forme)
        })
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

    /// The battle state (`BattleState`), for the free functions a handler shares with the
    /// damage pipeline.
    pub fn state(&self) -> &BattleState {
        self.state
    }

    /// The resolved rules (`ResolvedRules`), as `Self::rules` gives the narrower `EffectRules`.
    pub fn resolved_rules(&self) -> &ResolvedRules {
        self.rules
    }

    /// The absorbing forme the holder's ability answers, if any (Disguise, Ice Face; the
    /// ability in effect, DESIGN.md §43.4).
    pub fn absorbing(
        &self,
        holder: PokemonId,
    ) -> Option<crate::effects::abilities::AbsorbingForme> {
        self.state.pokemon(holder).and_then(|pokemon| {
            crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.absorbing)
        })
    }

    /// Whether the holder has transformed (Showdown `pokemon.transformed`, DESIGN.md §41.7):
    /// the forme-changing abilities of §43.4 ask it before they act.
    pub fn has_transformed(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|pokemon| pokemon.transform.is_some())
    }

    /// The category of the move being used (`state.active_move`; Ice Face asks it).
    pub fn active_move_category(&self) -> Option<crate::dex::MoveCategory> {
        self.state.active_move.map(|active| active.category)
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

    /// Whether the holder is Terastallized (`pokemon.terastallized`, DESIGN.md §44.6).
    pub fn terastallized(&self, holder: PokemonId) -> bool {
        self.state
            .pokemon(holder)
            .is_some_and(|pokemon| pokemon.terastallized.is_some())
    }

    /// The item the holder has (`pokemon.item`), whether or not it is suppressed: Showdown's
    /// species conditions read the field itself (`zacian`'s `pokemon.item !== 'rustedsword'`),
    /// and they run before the holder is on the field (DESIGN.md §43.4).
    pub fn raw_held_item(&self, holder: PokemonId) -> Option<ItemId> {
        self.state
            .pokemon(holder)
            .and_then(|pokemon| pokemon.held_item)
    }

    /// The ability the holder has (`raw_ability`), none's when it is gone.
    pub fn raw_ability(&self, pokemon: PokemonId) -> crate::ids::AbilityId {
        self.state
            .pokemon(pokemon)
            .map_or(crate::ids::AbilityId::NONE, crate::engine::raw_ability)
    }

    pub fn active_turns(&self, pokemon: PokemonId) -> u16 {
        self.state.pokemon(pokemon).map_or(0, |p| p.active_turns)
    }

    /// Showdown `queue.willMove(pokemon)`: whether `pokemon` still has a move action queued
    /// this turn.
    pub fn will_move(&self, pokemon: PokemonId) -> bool {
        self.state.will_move_with(pokemon).is_some()
    }

    /// Supreme Overlord's `onStart`: the holder remembers its side's fainted Pokémon (Showdown
    /// `side.totalFainted`), at most 5, and says how many.
    pub fn count_fallen_allies(&mut self, holder: PokemonId) -> u8 {
        let Some(side) = self.state.pokemon(holder).map(|p| p.side) else {
            return 0;
        };
        let fallen = self.state.side(side).total_fainted.min(5);
        if let Some(pokemon) = self.state.pokemon_mut(holder) {
            pokemon.ability_state.fallen = fallen;
        }
        fallen
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

    /// Intimidate and similar: a Substitute shields the target (Showdown `-immune`); in Gen 4
    /// so does a Substitute U-turn just broke.
    pub fn shielded_from_intimidation(&self, target: PokemonId) -> bool {
        self.state.pokemon(target).is_some_and(|p| {
            p.volatiles.contains(VolatileConditionKind::Substitute)
                || (self
                    .rules
                    .conditions
                    .intimidate_blocked_by_u_turn_broken_substitute
                    && p.volatiles.substitute_broken_by() == Some(MoveId::U_TURN))
        })
    }

    pub fn has_status(&self, pokemon: PokemonId) -> bool {
        self.status_of(pokemon).is_some()
    }

    /// Asleep, or Comatose (`engine::counts_as_asleep`).
    pub fn counts_as_asleep(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|p| crate::engine::counts_as_asleep(self.state, self.rules, p))
    }

    pub fn status_of(&self, pokemon: PokemonId) -> Option<NonVolatileStatus> {
        self.state
            .pokemon(pokemon)
            .and_then(|p| p.status.map(|status| status.kind))
    }

    /// The whole team of a side, bench included, in team order.
    pub fn team(&self, side: SideId) -> Vec<PokemonId> {
        self.state.side(side).team.clone()
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

    /// Removes the Pokémon's major status (Shed Skin, Natural Cure, Heal Bell …); false when
    /// it had none or is down. The toxic counter goes with the status.
    pub fn cure_status(&mut self, target: PokemonId, reason: CureReason) -> bool {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return false;
        };
        if pokemon.is_fainted() {
            return false;
        }
        let Some(status) = pokemon.status.take() else {
            return false;
        };
        // Showdown `cureStatus`: a Nightmare ends with the sleep.
        let nightmare_ended = status.kind == NonVolatileStatus::Sleep
            && pokemon.volatiles.remove(VolatileConditionKind::Nightmare);
        self.trace.push(TraceEvent::StatusCured {
            target,
            status: status.kind,
            reason,
        });
        if nightmare_ended {
            self.trace.push(TraceEvent::VolatileEnded {
                target,
                kind: VolatileConditionKind::Nightmare,
                seen: true,
            });
        }
        true
    }

    pub fn is_fainted(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_none_or(|pokemon| pokemon.is_fainted())
    }

    /// The ability `pokemon` has, in effect or not (`engine::raw_ability`).
    pub fn raw_ability_of(&self, pokemon: PokemonId) -> Option<AbilityId> {
        self.state.pokemon(pokemon).map(crate::engine::raw_ability)
    }

    pub fn has_item(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|pokemon| pokemon.held_item.is_some())
    }

    /// Every active Pokémon, side by side and slot by slot (Showdown `getAllActive`).
    pub fn active_pokemon(&self) -> Vec<PokemonId> {
        [SideId::P1, SideId::P2]
            .into_iter()
            .flat_map(|side| self.state.side(side).active.iter().copied().flatten())
            .collect()
    }

    pub fn adjacent_foes(&self, source: PokemonId) -> Vec<PokemonId> {
        let Some(position) = self.state.active_position(source, &self.rules.topology) else {
            return Vec::new();
        };

        self.rules
            .topology
            .adjacent_foes(position)
            .into_iter()
            .filter_map(|position| {
                self.state
                    .pokemon_at_position(position, &self.rules.topology)
            })
            .filter(|pokemon| {
                self.state
                    .pokemon(*pokemon)
                    .is_some_and(|p| !p.is_fainted())
            })
            .collect()
    }

    pub fn adjacent_allies(&self, source: PokemonId) -> Vec<PokemonId> {
        let Some(position) = self.state.active_position(source, &self.rules.topology) else {
            return Vec::new();
        };

        self.rules
            .topology
            .adjacent_allies(position)
            .into_iter()
            .filter_map(|position| {
                self.state
                    .pokemon_at_position(position, &self.rules.topology)
            })
            .filter(|pokemon| {
                self.state
                    .pokemon(*pokemon)
                    .is_some_and(|p| !p.is_fainted())
            })
            .collect()
    }

    /// `item` names itself on `holder` (Showdown `-item|holder|item`: an Air Balloon's `onStart`).
    pub fn announce_item(&mut self, holder: PokemonId, item: ItemId) {
        self.trace.push(TraceEvent::ItemAnnounced { holder, item });
    }

    /// Frisk names `holder`'s item, whatever sets it aside (Showdown reads `target.item`).
    /// Returns whether it held one.
    /// Returns whether it held one; `identified` whether the line lets everyone tell whose it is
    /// (`TraceEvent::ItemRevealed`).
    pub fn reveal_item(&mut self, holder: PokemonId, by: PokemonId, identified: bool) -> bool {
        let Some(item) = self.state.pokemon(holder).and_then(|p| p.held_item) else {
            return false;
        };
        self.trace.push(TraceEvent::ItemRevealed {
            holder,
            item,
            by,
            identified,
        });
        true
    }

    pub fn item_activated(&mut self, holder: PokemonId, item: ItemId) {
        self.trace.push(TraceEvent::ItemActivated { holder, item });
    }

    /// The ability announces itself: everyone sees it.
    /// Showdown `checkMoveMakesContact(move, source, target, true)` (Rough Skin, Iron Barbs,
    /// Aftermath, Gooey, Tangling Hair; Mummy and Lingering Aroma against a foe, R-M10-51): a
    /// contact move the attacker's Protective Pads kept from making contact names the holder's
    /// ability and the pads (`-activate|holder|ability: …`, `-activate|attacker|item: Protective
    /// Pads`, R-M10-44).
    pub fn padded_contact(&mut self, hit: DamagingHit, ability: AbilityId) {
        if hit.padded {
            self.ability_activated(hit.holder, ability);
            self.item_activated(hit.attacker, crate::ids::ItemId::PROTECTIVE_PADS);
        }
    }

    pub fn ability_activated(&mut self, source: PokemonId, ability: AbilityId) {
        self.trace
            .push(TraceEvent::AbilityActivated { source, ability });
    }

    /// A status `holder`'s ability inflicts on its own (Static, Flame Body, Poison Point, Effect
    /// Spore, Poison Touch): the ability shows itself only on the status's line — a sleep from Gen
    /// 5 (`StatusLifecycleRules::ability_sleep_named`); a status that does not land shows nothing.
    pub fn inflict_status_by_ability(
        &mut self,
        holder: PokemonId,
        target: PokemonId,
        status: NonVolatileStatus,
        ability: AbilityId,
    ) -> super::StatusApplyResult {
        let result = self.try_set_status(
            target,
            status,
            EffectSource::Ability {
                pokemon: holder,
                ability,
            },
        );
        if matches!(result, super::StatusApplyResult::Applied { .. })
            && (status != NonVolatileStatus::Sleep
                || self.rules.status.lifecycle.ability_sleep_named)
        {
            self.ability_activated(holder, ability);
        } else {
            self.ability_at_work(holder, ability);
        }
        result
    }

    /// The ability is at work, shown to no one but its holder's side (`AbilityAtWork`).
    pub fn ability_at_work(&mut self, holder: PokemonId, ability: AbilityId) {
        self.trace
            .push(TraceEvent::AbilityAtWork { holder, ability });
    }

    /// Forewarn named `foe`'s `move_id` (`MoveForewarned`).
    pub fn forewarned(
        &mut self,
        holder: PokemonId,
        foe: PokemonId,
        move_id: MoveId,
        identified: bool,
    ) {
        self.trace.push(TraceEvent::MoveForewarned {
            holder,
            foe,
            move_id,
            identified,
        });
    }

    fn commit_hp_loss(
        &mut self,
        target: PokemonId,
        amount: u16,
        source: EffectSource,
        kind: DamageKind,
        future_move: bool,
    ) -> u16 {
        let Some(pokemon) = self.state.pokemon_mut(target) else {
            return 0;
        };

        let before = pokemon.hp;
        let applied = amount.min(before);
        pokemon.hp -= applied;
        let after = pokemon.hp;
        // Showdown `hurtThisTurn` (Assurance), set by every `spreadDamage` that took HP — not by
        // `directDamage` (Struggle's recoil, a Substitute's HP).
        if applied > 0 && !kind.is_direct() {
            pokemon.history.hurt_this_turn = true;
        }
        if after == 0 && before > 0 && !self.state.faint_queue.contains(&target) {
            self.state.faint_queue.push(target);
            self.state.faint_causes.push(crate::state::FaintCause {
                target,
                source: source.pokemon(),
                by_move: kind == DamageKind::Move,
                future_move,
                listeners_run: false,
                bonds_run: false,
                after_faint_run: false,
                counted: false,
            });
        }

        // A move's price is traced apart: the replay leaves it out of the move's HP outcome.
        self.trace.push(if kind == DamageKind::Cost {
            TraceEvent::HpCostPaid {
                target,
                amount: applied,
                hp_before: before,
                hp_after: after,
                source,
            }
        } else {
            TraceEvent::DamageApplied {
                target,
                amount: applied,
                hp_before: before,
                hp_after: after,
                source,
            }
        });

        applied
    }

    /// Request → Resolve → Commit for HP loss (DESIGN.md §20). Move damage is routed through
    /// the Substitute first (a Substitute hit resolves nothing: Showdown's Substitute condition
    /// takes it before `onDamage`); everything that reaches HP passes the target's damage
    /// resolution behavior (Magic Guard, Sturdy, Focus Sash) before the commit.
    pub fn apply_damage(&mut self, request: DamageRequest) -> DamageApplication {
        // Showdown `spreadDamage`: a Pokémon off the field takes nothing (`!target.isActive`) —
        // a benched attacker a Jaboca or Rowap Berry answers, its foreseen attack having struck
        // after it left (DESIGN.md R277). (`directDamage`, a self-hit's or a cost's, is
        // always on the field.)
        if self
            .state
            .active_position(request.target, &self.rules.topology)
            .is_none()
        {
            return DamageApplication::Prevented {
                target: request.target,
            };
        }
        // Gen 1 `battle.lastDamage` (§15.8): a move's damage, a confusion self-hit or a crash —
        // not recoil, drain or a residual. A move's damage on the body is capped at the target's
        // HP first (the gen1 `moveHit`'s `damage > target.hp`; pokered caps `wDamage` as the HP
        // reaches 0); a Substitute's takes it uncapped, and so do the self-hit and the crash
        // (`directDamage` of `confusion` and Jump Kick — not a Substitute's price).
        if self.rules.conditions.retaliation.gen1_last_damage
            && matches!(
                request.kind,
                DamageKind::Move | DamageKind::SelfHit | DamageKind::Direct
            )
        {
            let behind_substitute = !request.flags.bypass_substitute
                && self.state.pokemon(request.target).is_some_and(|p| {
                    p.volatiles
                        .contains(crate::state::VolatileConditionKind::Substitute)
                });
            let hp = self.state.pokemon(request.target).map_or(0, |p| p.hp);
            self.state.last_damage = if request.kind == DamageKind::Move && !behind_substitute {
                request.amount.0.min(hp)
            } else {
                request.amount.0
            };
        }
        if request.kind == DamageKind::Move
            && !request.flags.bypass_substitute
            && let Some(application) = self.absorb_with_substitute(request.target, request.amount)
        {
            if let DamageApplication::Substitute { broken: true, .. } = application
                && self.rules.conditions.substitute_broken_marker
                && let EffectSource::Move { move_id, .. } = request.source
                && let Some(pokemon) = self.state.pokemon_mut(request.target)
            {
                pokemon.volatiles.mark_substitute_broken(move_id);
            }
            return application;
        }
        match self.resolve_damage(request) {
            Some(amount) => {
                // Bide's `onDamage` (§15.10), the last handler before the HP cap.
                if request.kind == DamageKind::Move
                    && let Some(source) = request.source.pokemon()
                {
                    self.store_bide_damage(request.target, amount.0, source);
                }
                let applied = self.commit_hp_loss(
                    request.target,
                    amount.0,
                    request.source,
                    request.kind,
                    request.flags.future_move,
                );
                // `damage(…, instafaint)` knocking its target out: `faintMessages(true)` at once —
                // the target first (`lastFirst`), then the faints already waiting, each announced
                // (no longer a Pokémon left for `foePokemonLeft`), counted and heard as the hit
                // loop's end announces them, before anything after the damage (Mind Blown's
                // recoil before the `Update` that eats a foe's Apicot Berry: with no foe left, it
                // raises nothing, R-M10-80).
                if request.flags.instafaint
                    && self.is_fainted(request.target)
                    && let Some(index) = self
                        .state
                        .faint_queue
                        .iter()
                        .position(|pokemon| *pokemon == request.target)
                {
                    let target = self.state.faint_queue.remove(index);
                    self.state.faint_queue.insert(0, target);
                    self.announce_knocked_out(None);
                }
                DamageApplication::Hp {
                    target: request.target,
                    amount: DamageAmount(applied),
                }
            }
            None => DamageApplication::Prevented {
                target: request.target,
            },
        }
    }

    fn absorb_with_substitute(
        &mut self,
        target: PokemonId,
        amount: DamageAmount,
    ) -> Option<DamageApplication> {
        let (applied, before, after, broken) = self
            .state
            .pokemon_mut(target)
            .and_then(|pokemon| pokemon.volatiles.damage_substitute(amount.0))?;
        self.trace.push(TraceEvent::SubstituteDamaged {
            target,
            amount: applied,
            hp_before: before,
            hp_after: after,
            broken,
        });
        if broken {
            self.trace.push(TraceEvent::VolatileEnded {
                target,
                kind: VolatileConditionKind::Substitute,
                seen: true,
            });
        }
        Some(DamageApplication::Substitute {
            target,
            amount: DamageAmount(applied),
            broken,
        })
    }

    /// The Resolve stage of a DamageRequest: the target's ability and item behavior in Showdown's
    /// `onDamage` order (Magic Guard, then Sturdy, then Focus Sash). Each handler sees the amount
    /// the previous one left, so Sturdy's "leave 1 HP" keeps Focus Sash from being spent.
    fn resolve_damage(&mut self, request: DamageRequest) -> Option<DamageAmount> {
        // Berserk / Anger Shell's `onDamage` (priority 0: before Endure, Sturdy and Focus Sash
        // decide anything) only notes the damage.
        if let Some(ability) = self.running_ability(request.target) {
            let behavior = crate::engine::ability_behavior(self.rules, ability);
            crate::effects::abilities::damaged(self, behavior, &request);
        }
        let Some(pokemon) = self.state.pokemon(request.target) else {
            return Some(request.amount);
        };
        let mut behavior: Vec<(i16, DamageResolutionHandler, EffectSource)> = Vec::new();
        if let Some(ordered) =
            crate::engine::ability_handler(self.state, self.rules, pokemon, |b| b.damage_resolution)
        {
            behavior.push((
                ordered.priority,
                ordered.handler,
                EffectSource::Ability {
                    pokemon: request.target,
                    ability: crate::engine::raw_ability(pokemon),
                },
            ));
        }
        if let Some((item, item_behavior)) =
            crate::engine::held_item(self.state, self.rules, request.target)
            && let Some(ordered) = item_behavior.damage_resolution
        {
            behavior.push((
                ordered.priority,
                ordered.handler,
                EffectSource::Item {
                    pokemon: request.target,
                    item,
                },
            ));
        }
        // Endure (`onDamage` priority −10): after Magic Guard, before Sturdy and Focus Sash.
        if pokemon.volatiles.contains(VolatileConditionKind::Endure) {
            behavior.push((
                10,
                crate::effects::moves::endure_damage as DamageResolutionHandler,
                EffectSource::Move {
                    pokemon: request.target,
                    move_id: MoveId::ENDURE,
                },
            ));
        }
        // The move's own `onDamage` (False Swipe, Hold Back, priority -20).
        if request.flags.spare_one_hp {
            behavior.push((
                20,
                crate::effects::moves::leave_one_hp as DamageResolutionHandler,
                request.source,
            ));
        }
        behavior.sort_by_key(|(priority, _, _)| *priority);

        let mut current = request;
        for (_, handler, by) in behavior {
            match handler(self, &current) {
                DamageDecision::Allow => {}
                DamageDecision::Reduce(amount) => {
                    self.trace.push(TraceEvent::DamageReduced {
                        target: current.target,
                        requested: current.amount.0,
                        applied: amount.0,
                        by,
                    });
                    current.amount = amount;
                }
                DamageDecision::Prevent => {
                    self.trace.push(TraceEvent::DamagePrevented {
                        target: current.target,
                        amount: current.amount.0,
                        source: current.source,
                        by,
                    });
                    return None;
                }
            }
        }
        Some(current.amount)
    }

    /// Showdown `runEvent('TryHeal')` refusing a heal: Heal Block (`onTryHeal`, before the
    /// full-HP check). From Gen 5 every heal fails; the gen4 mod stops only a drain's, Leech
    /// Seed's and Wish's. Under Heal Block a Pokémon's own move heals it only by draining (the
    /// moves that heal are held at BeforeMove); a Wish is stopped by its id, whoever it heals
    /// (the wisher's replacement too).
    pub fn heal_refused(&self, target: PokemonId, source: EffectSource) -> bool {
        self.state.pokemon(target).is_some_and(|pokemon| {
            pokemon.volatiles.contains(VolatileConditionKind::HealBlock)
                && (self.rules.conditions.heal_block_stops_every_heal
                    || match source {
                        EffectSource::Volatile(VolatileConditionKind::LeechSeed) => true,
                        EffectSource::Move { pokemon, move_id } => {
                            pokemon == target || move_id == crate::ids::MoveId::WISH
                        }
                        _ => false,
                    })
        })
    }

    pub fn apply_heal(&mut self, request: HealRequest) -> HealResult {
        let Some(pokemon) = self.state.pokemon(request.target) else {
            return HealResult::Blocked(HealBlockReason::InvalidTarget);
        };
        if pokemon.is_fainted() {
            return HealResult::Blocked(HealBlockReason::Fainted);
        }
        // The `TryHeal` event, which a direct heal never runs.
        if !request.direct {
            if self.heal_refused(request.target, request.source) {
                return HealResult::Blocked(HealBlockReason::HealBlock);
            }
            // At full HP too: `TryHeal` comes before `heal` finds nothing to do.
            self.ripen_try_heal(request.target, request.source);
        }
        let Some(pokemon) = self.state.pokemon_mut(request.target) else {
            return HealResult::Blocked(HealBlockReason::InvalidTarget);
        };
        if pokemon.hp >= pokemon.max_hp {
            return HealResult::Blocked(HealBlockReason::FullHp);
        }
        let from = pokemon.hp;
        let to = pokemon
            .hp
            .saturating_add(request.amount)
            .min(pokemon.max_hp);
        pokemon.hp = to;
        self.trace.push(TraceEvent::HealApplied {
            target: request.target,
            amount: to - from,
            hp_before: from,
            hp_after: to,
            source: request.source,
        });
        HealResult::Applied { from, to }
    }

    /// Ripen's `onTryHeal` announces itself for Leftovers and Berry Juice (a berry's heal it
    /// doubles: `berry_heal`).
    pub(crate) fn ripen_try_heal(&mut self, target: PokemonId, source: EffectSource) {
        if let EffectSource::Item {
            item: ItemId::LEFTOVERS | ItemId::BERRY_JUICE,
            ..
        } = source
            && let Some(ability) = self.running_ability(target)
            && crate::engine::ability_behavior(self.rules, ability).ripen
        {
            self.ability_activated(target, ability);
        }
    }

    pub fn try_set_status(
        &mut self,
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
    ) -> super::StatusApplyResult {
        super::status::apply_status(
            self.state, self.rules, self.trace, target, status, source, false,
        )
    }

    /// `try_set_status` for a status the effect carries itself (`TrySetStatusContext::carried`:
    /// a status move's own, one Synchronize passes on).
    pub fn try_set_carried_status(
        &mut self,
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
    ) -> super::StatusApplyResult {
        super::status::apply_status(
            self.state, self.rules, self.trace, target, status, source, true,
        )
    }

    /// `try_set_status` for a status whose doer the source does not name (Yawn's sleep: the
    /// yawner's side).
    pub fn try_set_status_inflicted(
        &mut self,
        target: PokemonId,
        status: NonVolatileStatus,
        source: EffectSource,
        inflicted_by: Option<crate::ids::SideId>,
    ) -> super::StatusApplyResult {
        super::status::set_status(
            self.state,
            self.rules,
            self.trace,
            target,
            status,
            source,
            super::status::SetStatusHow {
                replace: false,
                inflicted_by,
                carried: false,
            },
        )
    }

    /// Request → Resolve → Commit for a stat stage change, in Showdown's `boost()` order:
    /// ChangeBoost (Contrary / Simple rewrite the stages) → cap to ±6 → TryBoost (Clear Body
    /// refuses, Mirror Armor sends the drop back) → commit.
    pub fn try_change_stat_stage(&mut self, request: StatChangeRequest) -> StatChangeResult {
        let result = self.change_stat_stage_resolved(request);
        // Showdown `runEvent('AfterBoost')` of the holder's item, after its ability's (which
        // `change_stat_stage_resolved` runs only where the stage moved): whether or not it moved,
        // but not for a Pokémon down or off the field (`boost()` returns before).
        if matches!(
            result,
            StatChangeResult::Blocked(
                StatChangeBlockReason::InvalidTarget
                    | StatChangeBlockReason::Fainted
                    | StatChangeBlockReason::NoFoeLeft
            )
        ) {
            return result;
        }
        if let Some((_, item)) = crate::engine::held_item(self.state, self.rules, request.target)
            && let Some(handler) = item.after_boost
        {
            handler(self, request.target, request, result);
        }
        // Then the `onFoeAfterBoost` of the abilities and items of its foes on the field
        // (Opportunist, Mirror Herb), which only keep what they saw: their order does not
        // matter.
        for foe in self.active_foes(request.target) {
            if let Some(handler) = self.state.pokemon(foe).and_then(|p| {
                crate::engine::ability_handler(self.state, self.rules, p, |b| b.foe_after_boost)
            }) {
                handler(self, foe, request, result);
            }
            if let Some((_, item)) = crate::engine::held_item(self.state, self.rules, foe)
                && let Some(handler) = item.foe_after_boost
            {
                handler(self, foe, request, result);
            }
        }
        result
    }

    fn change_stat_stage_resolved(&mut self, request: StatChangeRequest) -> StatChangeResult {
        let Some(pokemon) = self.state.pokemon(request.target) else {
            return StatChangeResult::Blocked(StatChangeBlockReason::InvalidTarget);
        };
        if pokemon.is_fainted() {
            return StatChangeResult::Blocked(StatChangeBlockReason::Fainted);
        }
        // Gen 6+ `boost`: nothing once the side facing the Pokémon has none left.
        if self.rules.stats.boosts_need_foe_pokemon_left && !self.foe_pokemon_left(request.target) {
            return StatChangeResult::Blocked(StatChangeBlockReason::NoFoeLeft);
        }
        let ability = crate::engine::raw_ability(pokemon);
        let handlers = crate::engine::ability_handlers(self.state, self.rules, pokemon);
        let item = crate::engine::held_item_id(self.state, self.rules, request.target);
        let from = pokemon.stages.get(request.stat);

        // ChangeBoost.
        let mut request = request;
        if let Some(handler) = handlers.modify_stat_change {
            let resolved = handler(
                &StatChangeContext::new(self.state, self.rules, request),
                request.stages,
            );
            if resolved != request.stages {
                self.trace.push(TraceEvent::StatChangeModified {
                    target: request.target,
                    stat: request.stat,
                    requested: request.stages,
                    resolved,
                    by: EffectSource::Ability {
                        pokemon: request.target,
                        ability,
                    },
                });
                request.stages = resolved;
            }
        }

        // Cap: what can still move within ±6.
        let to = (from + request.stages).clamp(-6, 6);
        if from == to {
            return StatChangeResult::Blocked(StatChangeBlockReason::NoChange);
        }
        request.stages = to - from;

        // Guard Dog's `onTryBoost` (`onTryBoostPriority` 2, ahead of everything below) takes
        // the change over.
        if let Some(handler) = handlers.try_boost_first
            && handler(self, request)
        {
            // Its own rise (`-ability|holder|Guard Dog|boost`) shows it, where there is one.
            let reason = StatChangeBlockReason::Ability(ability);
            self.trace.push(TraceEvent::StatChangeBlocked {
                target: request.target,
                stat: request.stat,
                stages: request.stages,
                source: request.source,
                reason,
                shown_by: None,
            });
            return StatChangeResult::Blocked(reason);
        }

        // TryBoost, in Showdown's handler order: the item first (Clear Amulet's
        // `onTryBoostPriority` 1), then the ability — Mirror Armor's return of a drop included —
        // and Mist, a side's handler, last: Showdown sorts a Pokémon's handlers before a side's at
        // the same priority, a side having no speed (issues.md I9).
        // Each refusal with the Pokémon whose ability or item refused it (a Flower Veil an ally's)
        // and whether its line shows.
        let refusal = {
            let ctx = StatChangeContext::new(self.state, self.rules, request);
            let refused = |result, holder| match result {
                TryStatChangeResult::Allow => None,
                TryStatChangeResult::Blocked { reason, shown } => Some((reason, shown, holder)),
            };
            let by_item = item
                .and_then(|item| crate::engine::item_behavior(self.rules, item).try_stat_change)
                .and_then(|handler| refused(handler(&ctx), request.target));
            by_item
                .or_else(|| {
                    handlers
                        .try_stat_change
                        .and_then(|handler| refused(handler(&ctx), request.target))
                })
                // Showdown `onAllyTryBoost` (Flower Veil), the target's own included: the faster
                // holder's first (`speedSort`, R-M10-53).
                .or_else(|| {
                    let mut holders: Vec<_> = crate::engine::allied_ability_holders(
                        self.state,
                        self.rules,
                        request.target,
                    )
                    .into_iter()
                    .filter_map(|(holder, behavior)| {
                        behavior
                            .ally_try_stat_change
                            .map(|handler| (holder, handler))
                    })
                    .collect();
                    holders.sort_by_key(|(holder, _)| {
                        std::cmp::Reverse(crate::engine::stats::handler_speed(self.state, *holder))
                    });
                    holders
                        .into_iter()
                        .find_map(|(holder, handler)| refused(handler(&ctx), holder))
                })
        };
        if let Some((reason, shown, holder)) = refusal {
            let shown_by = shown
                .then_some(match reason {
                    StatChangeBlockReason::Ability(ability) => Some(EffectSource::Ability {
                        pokemon: holder,
                        ability,
                    }),
                    StatChangeBlockReason::Item(item) => Some(EffectSource::Item {
                        pokemon: holder,
                        item,
                    }),
                    _ => None,
                })
                .flatten();
            self.trace.push(TraceEvent::StatChangeBlocked {
                target: request.target,
                stat: request.stat,
                stages: request.stages,
                source: request.source,
                reason,
                shown_by,
            });
            return StatChangeResult::Blocked(reason);
        }
        if self.mirror_armor_refuses(request, handlers) {
            let reason = StatChangeBlockReason::Ability(ability);
            // `-ability|holder|Mirror Armor` while the drop's culprit has HP (`source.hp`),
            // whether the drop sent back lands or not.
            let shown_by = request
                .source
                .pokemon()
                .is_some_and(|culprit| !self.is_fainted(culprit))
                .then_some(EffectSource::Ability {
                    pokemon: request.target,
                    ability,
                });
            self.trace.push(TraceEvent::StatChangeBlocked {
                target: request.target,
                stat: request.stat,
                stages: request.stages,
                source: request.source,
                reason,
                shown_by,
            });
            // Sent back with Showdown `boost()`, which does nothing to a Pokémon down or off
            // the field.
            if let Some(culprit) = request.source.pokemon()
                && !self.is_fainted(culprit)
                && self.is_active(culprit)
            {
                self.try_change_stat_stage(StatChangeRequest {
                    target: culprit,
                    stat: request.stat,
                    stages: request.stages,
                    source: EffectSource::Ability {
                        pokemon: request.target,
                        ability,
                    },
                    cause: StatChangeCause::Ability,
                });
            }
            return StatChangeResult::Blocked(reason);
        }
        if crate::engine::protections::mist_refuses(self.state, self.rules, &request) {
            let reason = StatChangeBlockReason::Mist;
            self.trace.push(TraceEvent::StatChangeBlocked {
                target: request.target,
                stat: request.stat,
                stages: request.stages,
                source: request.source,
                reason,
                shown_by: None,
            });
            return StatChangeResult::Blocked(reason);
        }

        // Gen 2 (the gen2 `boostBy`): a raise of a stat that reads exactly 999 changes nothing
        // (a species item doubling it past the clamp reads more). Gen 2 has no Gen 1
        // `modified_stats`, so this comes before the commit below as it did after it.
        if self.rules.stats.raise_stops_at_cap
            && request.stages > 0
            && self.capped_stat_reading(request.target, request.stat, from) == 999
        {
            return StatChangeResult::Blocked(StatChangeBlockReason::NoChange);
        }

        // Commit.
        let stage_table = self.rules.stats.stage_table;
        let Some(pokemon) = self.state.pokemon_mut(request.target) else {
            return StatChangeResult::Blocked(StatChangeBlockReason::InvalidTarget);
        };
        // Gen 1 (Showdown gen1 `boostBy`): the modified stat is recomputed from the stored one
        // (a paralysis / burn cut on it is gone), capped at 999 when raised; raising a stat at
        // 999 (lowering one at 1) moves the stage one short and does not count as a change.
        if let Some(modified) = pokemon.modified_stats.and_then(|m| m.get(request.stat)) {
            let stuck =
                (request.stages > 0 && modified == 999) || (request.stages < 0 && modified == 1);
            if stuck {
                let back = if request.stages > 0 { to - 1 } else { to + 1 };
                pokemon.stages.set(request.stat, back);
                self.trace.push(TraceEvent::StatChanged {
                    target: request.target,
                    stat: request.stat,
                    from,
                    to: back,
                    source: request.source,
                });
                return StatChangeResult::Blocked(StatChangeBlockReason::NoChange);
            }
            let stored = pokemon.battle_stats().get(request.stat).unwrap_or(1);
            let mut next = crate::engine::stats::stage_value(stage_table, stored, to).max(1);
            if request.stages > 0 {
                next = next.min(999);
            }
            if let Some(stats) = &mut pokemon.modified_stats {
                stats.set(request.stat, next.min(u32::from(u16::MAX)) as u16);
            }
        }
        pokemon.stages.set(request.stat, to);
        // Showdown `statsLoweredThisTurn` (Lash Out) / `statsRaisedThisTurn` (Burning Jealousy).
        if to < from {
            pokemon.history.stats_lowered_this_turn = true;
        }
        if to > from {
            pokemon.history.stats_raised_this_turn = true;
        }

        self.trace.push(TraceEvent::StatChanged {
            target: request.target,
            stat: request.stat,
            from,
            to,
            source: request.source,
        });
        // Showdown `onAfterEachBoost` (Defiant, Competitive) and `onAfterBoost` (Rattled) of
        // the Pokémon whose stage moved.
        if let Some(handler) = handlers.after_each_boost {
            handler(self, request.target, request);
        }
        if let Some(handler) = handlers.after_boost {
            handler(self, request.target, request);
        }
        StatChangeResult::Applied { from, to }
    }

    /// Mirror Armor's `onTryBoost`: the holder refuses a drop (the capped stages: Showdown
    /// reflects after `getCappedBoost`) unless it caused the drop itself or Mirror Armor sent it
    /// (`effect.name === 'Mirror Armor'`); the drop goes back to whoever caused it.
    fn mirror_armor_refuses(
        &self,
        request: StatChangeRequest,
        handlers: &crate::effects::abilities::AbilityBehavior,
    ) -> bool {
        if request.stages >= 0
            || !handlers.reflects_stat_drops
            || request.source.pokemon() == Some(request.target)
        {
            return false;
        }
        !matches!(request.source, EffectSource::Ability { ability: sent, .. }
            if crate::engine::ability_behavior(self.rules, sent).reflects_stat_drops)
    }

    pub fn change_stat_stage(
        &mut self,
        target: PokemonId,
        stat: Stat,
        delta: i8,
        source: EffectSource,
    ) -> StatChangeResult {
        self.try_change_stat_stage(StatChangeRequest {
            target,
            stat,
            stages: delta,
            source,
            cause: StatChangeCause::Rule,
        })
    }

    pub fn target_side(targets: &[ResolvedTarget]) -> Option<SideId> {
        targets.iter().find_map(|target| match target {
            ResolvedTarget::Side(side) => Some(*side),
            _ => None,
        })
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
