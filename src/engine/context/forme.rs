//! A forme change (DESIGN.md §43.2): Showdown's `formeChange`, which is `setSpecies` plus, for a
//! permanent change, the max HP and the forme's ability. The values of each forme come from the
//! Pokémon's forme book (`PokemonState::formes`), resolved as its team was built.

use super::{AbilityEndCause, BattleContext, EffectSource, TraceEvent};
use crate::ids::{PokemonId, SpeciesId};

/// How long a forme change lasts (Showdown `formeChange`'s `isPermanent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormeChangeKind {
    /// Until the Pokémon leaves the field (Stance Change, Zen Mode, Forecast …): leaving sets
    /// its own species again (`setSpecies(baseSpecies)`).
    InBattle,
    /// The Pokémon's own species from now on (`baseSpecies`): Power Construct, Shaymin's
    /// freeze, Primal Reversion. Its max HP follows the forme (`updateMaxHp`) and its ability
    /// becomes the forme's first one, starting anew.
    Permanent,
    /// Permanent, but the ability stays and keeps its state: Disguise, Ice Face (Showdown's
    /// `!['disguise', 'iceface'].includes(source.id)`).
    PermanentKeepingAbility,
    /// The species the Pokémon is from the battle's start, as `Permanent` but no forme change at
    /// all for Showdown: the species conditions' `onBattleStart` (`setSpecies`, then
    /// `baseSpecies` and the forme's first ability) — the Rusted Sword's and Shield's crowned
    /// formes. Nothing sets `formeRegression`, so a faint leaves the forme (R-M10-5).
    Base,
}

impl BattleContext<'_> {
    /// Showdown `formeChange(species, source, isPermanent)`: `pokemon` becomes `species`, one of
    /// the formes its forme book has. Types and stored stats become the forme's (Showdown's
    /// `setSpecies`: an added type goes, a stat a move rewrote is recomputed); boosts, volatiles
    /// and status stay. False when there is nothing to change (already that forme), or the book
    /// lacks it (a Pokémon built by hand). A transformed Pokémon changes no forme here: every
    /// caller asks for one that has not transformed (or its ability's `notransform`).
    pub fn forme_change(
        &mut self,
        pokemon: PokemonId,
        species: SpeciesId,
        kind: FormeChangeKind,
        source: EffectSource,
    ) -> bool {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return false;
        };
        if holder.transform.is_some() || crate::engine::species_of(holder) == Some(species) {
            return false;
        }
        let Some(forme) = holder.forme_of(species).copied() else {
            return false;
        };
        let own = holder.species.and_then(|own| holder.forme_of(own).copied());
        let old_ability = crate::engine::raw_ability(holder);
        let Some(holder) = self.state.pokemon_mut(pokemon) else {
            return false;
        };
        if kind == FormeChangeKind::InBattle {
            holder.change_into_forme(&forme, own.as_ref());
        } else {
            holder.take_forme(&forme);
        }
        // `setSpecies` sets `weighthg` to the forme's: Gen 6+ Autotomize's lightening goes, as
        // the forme change shows (@pkmn/client drops it at `-formechange`).
        let lightening_gone = self.rules.conditions.forme.resets_autotomize
            && holder
                .volatiles
                .contains(crate::state::VolatileConditionKind::Autotomize);
        if lightening_gone {
            holder.volatiles.set_autotomize_count(0);
        }
        self.trace.push(TraceEvent::FormeChanged {
            pokemon,
            species,
            permanent: kind != FormeChangeKind::InBattle,
            keeps_ability: kind == FormeChangeKind::PermanentKeepingAbility,
            source,
        });
        if lightening_gone {
            self.trace.push(TraceEvent::VolatileEnded {
                target: pokemon,
                kind: crate::state::VolatileConditionKind::Autotomize,
                seen: true,
            });
        }
        // Showdown `formeChange`'s `formeRegression`: a permanent change sourced at an item
        // (Mega Evolution, Primal Reversion, Ultra Burst) or at nothing (the `-Tera` formes —
        // the engine's `EffectSource::Rule` stands for its `null`) reverts as the Pokémon
        // faints (DESIGN.md §44.3); so does one of the abilities that set it themselves
        // (`FormeRules::regressing_abilities`: Zygarde-Complete, R-M8-111).
        let regresses = match source {
            EffectSource::Item { .. } | EffectSource::Rule => kind == FormeChangeKind::Permanent,
            EffectSource::Ability { ability, .. } => {
                kind != FormeChangeKind::InBattle
                    && self
                        .rules
                        .conditions
                        .forme
                        .regressing_abilities
                        .contains(&ability)
            }
            _ => false,
        };
        if regresses && let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.forme_regression = true;
        }
        if matches!(kind, FormeChangeKind::Permanent | FormeChangeKind::Base)
            && self.state.pokemon(pokemon).is_some_and(|p| p.hp > 0)
        {
            // `setAbility(ability, null, null, true)`: no `cantsuppress` check and no
            // `SetAbility` event; the old ability ends and the new one starts even when it is
            // the same. It stays as the Pokémon leaves (`baseAbility`).
            self.end_ability(pokemon, AbilityEndCause::Replaced);
            if let Some(holder) = self.state.pokemon_mut(pokemon) {
                holder.take_ability(forme.first_ability);
            }
            self.state.restart_ability_state(pokemon);
            self.trace.push(TraceEvent::AbilityChanged {
                pokemon,
                from: old_ability,
                to: forme.first_ability,
                line: crate::engine::AbilityChangeLine::Forme,
            });
            self.start_ability(pokemon);
        }
        true
    }

    /// A forme an **item** fixes (Showdown's species conditions, DESIGN.md §43.4): the Rusted
    /// Sword's and Shield's crowned formes at `BattleStart`, the Primal Orbs' reversion as the
    /// holder comes in, permanent; Gen 4's Arceus until it leaves the field. A permanent
    /// forme's ability starts (`setAbility(…, source)` prints nothing for a forme change).
    pub fn fix_own_forme(
        &mut self,
        pokemon: PokemonId,
        forme: SpeciesId,
        kind: FormeChangeKind,
        source: EffectSource,
    ) -> bool {
        self.forme_change(pokemon, forme, kind, source)
    }

    /// The **species conditions'** `onBeforeSwitchIn` (DESIGN.md §43.4): only Gen 4's Arceus
    /// has one — it enters with the Normal type (`setType('Normal')`, which the Multitype lock
    /// refuses) before anything reads its types (entry hazards, the frame), so a forme built as
    /// another type comes in Normal and can be retyped from there.
    pub fn species_before_switch_in(&mut self, pokemon: PokemonId) {
        if !self.rules.conditions.plate.before_switch_in_normal {
            return;
        }
        let Some(own) = self.own_species(pokemon) else {
            return;
        };
        if !own.same_species(SpeciesId::ARCEUS) {
            return;
        }
        self.set_type(
            pokemon,
            crate::state::PokemonTypes::mono(crate::dex::PokemonType::Normal),
            EffectSource::Rule,
        );
    }

    /// The **species conditions'** `onSwitchIn` (DESIGN.md §43.4): the Pokémon's own species'
    /// condition, run in the SwitchIn event like its ability's. Only Gen 4's Arceus has one
    /// modelled: it takes the forme of the Plate it holds as it comes in (`arceus`,
    /// `onSwitchIn`; from Gen 5 the type alone follows the Plate, `engine::types_of`).
    pub fn species_switch_in(&mut self, pokemon: PokemonId) {
        if !self.rules.conditions.plate.switch_in_forme {
            return;
        }
        let Some(own) = self.own_species(pokemon) else {
            return;
        };
        if !own.same_species(SpeciesId::ARCEUS)
            || self.has_transformed(pokemon)
            || self.raw_ability(pokemon) != crate::ids::AbilityId::MULTITYPE
        {
            return;
        }
        let Some(item) = self.raw_held_item(pokemon) else {
            return;
        };
        let forme = crate::engine::item_behavior(self.rules, item)
            .plate
            .map_or(SpeciesId::ARCEUS, crate::effects::items::arceus_forme);
        // Until it leaves the field (`formeChange(targetForme)`, not permanent): it goes back to
        // its own species on the bench (DESIGN.md R-M8-81).
        self.fix_own_forme(
            pokemon,
            forme,
            FormeChangeKind::InBattle,
            EffectSource::Item { pokemon, item },
        );
    }

    /// The Rusted Sword's and Shield's Iron Head (`baseMoveSlots[ironHeadIndex]`): the slot's
    /// move becomes the crowned one, its PP the new move's with the slot's PP Ups (Showdown
    /// `calculatePP(move, pokemon.ppUps[i])`, three from the set).
    pub fn swap_move(
        &mut self,
        pokemon: PokemonId,
        from: crate::ids::MoveId,
        to: crate::ids::MoveId,
        max_pp: u8,
    ) -> bool {
        let Some(holder) = self.state.pokemon_mut(pokemon) else {
            return false;
        };
        let Some(slot) = holder
            .move_slots_mut()
            .iter_mut()
            .find(|slot| slot.move_id == from)
        else {
            return false;
        };
        slot.move_id = to;
        slot.pp = max_pp;
        slot.max_pp = max_pp;
        true
    }

    /// `frz`'s `onStart`: a frozen Shaymin-Sky that is a Shaymin of its own (not one that
    /// transformed) changes back to its Land Forme for good, with Natural Cure.
    pub fn freeze_sky_forme(&mut self, pokemon: PokemonId) {
        let sky = self.state.pokemon(pokemon).is_some_and(|holder| {
            crate::engine::species_of(holder) == Some(SpeciesId::SHAYMIN_SKY)
                && holder
                    .species
                    .is_some_and(|own| own.same_species(SpeciesId::SHAYMIN))
        });
        if sky {
            self.forme_change(
                pokemon,
                SpeciesId::SHAYMIN,
                FormeChangeKind::Permanent,
                EffectSource::Status(crate::state::NonVolatileStatus::Freeze),
            );
        }
    }

    /// Stance Change (DESIGN.md §43.4): `pokemon`, an Aegislash of its own that has not
    /// transformed, takes the forme for `move_id` — the attacking one for a damaging move, the
    /// shielding one for its shield move.
    pub fn stance_change(
        &mut self,
        pokemon: PokemonId,
        stance: crate::effects::abilities::StanceChange,
        move_id: crate::ids::MoveId,
        category: crate::dex::MoveCategory,
    ) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        // A transformed Pokémon changes no forme: the primitive refuses it (Showdown asks
        // `attacker.transformed` here).
        if !holder
            .species
            .is_some_and(|own| own.same_species(stance.shield))
            || (category == crate::dex::MoveCategory::Status && move_id != stance.shield_move)
        {
            return;
        }
        let forme = if move_id == stance.shield_move {
            stance.shield
        } else {
            stance.attack
        };
        let ability = crate::engine::raw_ability(holder);
        self.forme_change(
            pokemon,
            forme,
            FormeChangeKind::InBattle,
            EffectSource::Ability { pokemon, ability },
        );
    }

    /// Relic Song's switch (DESIGN.md §43.4): `pokemon`, of `formes`' species (its own, so a
    /// Ditto transformed into one stays out of it), takes the other of the two.
    pub fn toggle_forme(
        &mut self,
        pokemon: PokemonId,
        formes: (SpeciesId, SpeciesId),
        source: EffectSource,
    ) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        // A transformed Pokémon changes no forme: the primitive refuses it.
        if !holder.species.is_some_and(|own| own.same_species(formes.0)) {
            return;
        }
        let forme = if crate::engine::species_of(holder) == Some(formes.1) {
            formes.0
        } else {
            formes.1
        };
        self.forme_change(pokemon, forme, FormeChangeKind::InBattle, source);
    }
}
