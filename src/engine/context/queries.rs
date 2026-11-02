//! `BattleContext`'s readings of the battle: the rules and the dex, a Pokémon's HP, stats, stages,
//! types, ability, item, status and position, its allies and foes, the field — nothing changed.

use crate::dex::PokemonType;
use crate::engine::TraceEvent;
use crate::ids::{AbilityId, ItemId, MoveId, PokemonId, SideId};
use crate::rules::{EffectRules, ResolvedRules, ResolvedTarget};
use crate::state::{BattleState, NonVolatileStatus, Stat, VolatileConditionKind};

use super::*;

impl<'a> BattleContext<'a> {
    /// The dex, for a custom primary that has to look at moves other than its own (the callers
    /// of §42.1 draw among them). Only a custom primary's context has one.
    pub fn dex(&self) -> Option<&'a crate::dex::ResolvedDex> {
        self.dex
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

    /// `pokemon`'s own species (Showdown `baseSpecies`): Poison Puppeteer's Pecharunt.
    pub fn own_species(&self, pokemon: PokemonId) -> Option<crate::ids::SpeciesId> {
        self.state
            .pokemon(pokemon)
            .and_then(|pokemon| pokemon.species)
    }

    /// `target`'s stage in `stat` (0 for a Pokémon not in the battle).
    pub fn stage(&self, target: PokemonId, stat: Stat) -> i8 {
        self.state
            .pokemon(target)
            .map_or(0, |pokemon| pokemon.stages.get(stat))
    }

    /// `pokemon`'s stored stat (Showdown `storedStats`; 0 for accuracy / evasion).
    pub fn stored_stat(&self, pokemon: PokemonId, stat: Stat) -> u16 {
        self.state
            .pokemon(pokemon)
            .and_then(|p| p.battle_stats().get(stat))
            .unwrap_or(0)
    }

    /// Showdown gen2 `getStat(stat, false, true)` of `pokemon` at `stage`
    /// (`stats::capped_stat_reading`: Gen 2 Belly Drum's climb).
    pub fn capped_stat_reading(&self, pokemon: PokemonId, stat: Stat, stage: i8) -> u32 {
        self.state.pokemon(pokemon).map_or(0, |p| {
            crate::engine::stats::capped_stat_reading(self.state, self.rules, p, stat, stage)
        })
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

    pub fn target_side(targets: &[ResolvedTarget]) -> Option<SideId> {
        targets.iter().find_map(|target| match target {
            ResolvedTarget::Side(side) => Some(*side),
            _ => None,
        })
    }
}
