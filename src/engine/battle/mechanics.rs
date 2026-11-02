//! The gimmicks as they run (DESIGN.md §44): Mega Evolution and its reinsertion in the queue,
//! Dynamax and its end, Terastallization.

use crate::engine::{
    ActionChoice, ActionOrder, ActionStep, BattleError, EffectSource, MechanicChoice,
    PendingExecution, TraceEvent,
};
use crate::ids::PokemonId;
use crate::state::{FieldConditionKind, RandomRequestKind, VolatileConditionKind};

use super::*;

impl BattleEngine {
    /// The gimmick itself: it is used up for its side (`SideMechanicState`) and takes effect.
    pub(super) fn apply_mechanic(
        &mut self,
        pokemon: PokemonId,
        mechanic: MechanicChoice,
    ) -> Result<(), BattleError> {
        match mechanic {
            MechanicChoice::Terastallize => {
                // A refused Terastallization (an Ogerpon of another type) spends nothing.
                if !self.terastallize(pokemon)? {
                    return Ok(());
                }
            }
            MechanicChoice::Dynamax => self.dynamax(pokemon)?,
            MechanicChoice::MegaEvolve => self.mega_evolve(pokemon, false)?,
            MechanicChoice::UltraBurst => self.mega_evolve(pokemon, true)?,
            // A Z-Move is spent as its move is used (the Z power).
            MechanicChoice::ZMove => {}
        }
        crate::engine::spend_mechanic(&mut self.state, &mut self.trace, pokemon, mechanic)
    }

    /// Showdown `runMegaEvo` (DESIGN.md §44.3): the Pokémon takes its mega forme — or Ultra
    /// Necrozma with the Ultranecrozium Z — through the forme-change primitive, sourced at the
    /// item it holds, so the forme's own ability starts with it (`-mega` / `-burst` are the
    /// item-sourced forme change's log, which the frames' species carry here). The side has no
    /// Mega Evolution left; Gen 7 recalculates the Pokémon's move action afterwards.
    fn mega_evolve(&mut self, pokemon: PokemonId, ultra: bool) -> Result<(), BattleError> {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return Err(BattleError::UnknownPokemon(pokemon));
        };
        let target = if ultra {
            holder.ultra_bursts_into
        } else {
            holder.mega_evolves_into
        };
        let Some(target) = target else {
            return Ok(());
        };
        let item = holder.held_item;
        // The line names the forme's `requiredItem` (`-mega|p|species|item`), the stone or the
        // Ultranecrozium Z held — none for Mega Rayquaza, whose forme asks a move instead
        // (`requiredMove`): the item it holds stays unseen (R-M10-62).
        let named = item.filter(|_| {
            !holder
                .formes
                .iter()
                .any(|forme| forme.species == target && forme.required_move.is_some())
        });
        let source = item.map_or(EffectSource::Rule, |item| EffectSource::Item {
            pokemon,
            item,
        });
        crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
            .forme_change(
                pokemon,
                target,
                crate::engine::FormeChangeKind::Permanent,
                source,
            );
        // Showdown sets `moveThisTurnResult = true`: the action counts as the Pokémon's for
        // Truant and Stomping Tantrum.
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.history.move_this_turn = Some(crate::state::MoveResult::Succeeded);
        }
        self.trace.push(TraceEvent::MegaEvolved {
            pokemon,
            species: target,
            ultra,
            item: named,
        });
        // Gen 7: the Pokémon's move action is taken out of the queue and put back at its new
        // speed (`updateSpeed`, then `insertChoice`), among the actions its order ties with.
        if self.rules.action_order.mega_reorders_move {
            crate::engine::stats::update_handler_speeds(&mut self.state, &self.rules);
            self.reinsert_after_mega(pokemon);
        }
        self.run_after_mega();
        Ok(())
    }

    /// Gen 7 `runAction`'s `insertChoice(queuedAction, true)` after a Mega Evolution: the
    /// actor's pending move action goes back into the queue at the draw's place among the
    /// actions its order ties with (one `QueueInsert` decision).
    fn reinsert_after_mega(&mut self, actor: PokemonId) {
        let PendingExecution::Turn { actions, .. } = &self.pending else {
            return;
        };
        let Some(index) = actions.iter().position(|action| {
            action.actor == actor
                && action.step == ActionStep::Act
                && matches!(action.choice, ActionChoice::Move { .. })
        }) else {
            return;
        };
        let mut action = actions[index].clone();
        action.order = self.refreshed_order(&action);
        let trick_room = self.state.field.contains(FieldConditionKind::TrickRoom);
        let earlier = |left: &ActionOrder, right: &ActionOrder| {
            crate::engine::action::order_before(left, right, trick_room)
        };
        let PendingExecution::Turn { actions, .. } = &mut self.pending else {
            return;
        };
        actions.remove(index);
        // Showdown's `insertChoice`: the equal range is [first, last] and the index is drawn
        // from it (`battle.random(firstIndex, lastIndex + 1)`).
        let mut first = actions.len();
        let mut last = actions.len();
        let mut seen_first = false;
        for (i, current) in actions.iter().enumerate() {
            if earlier(&current.order, &action.order) {
                if seen_first {
                    last = i;
                    break;
                }
                continue;
            }
            if !seen_first {
                first = i;
                seen_first = true;
            }
            if earlier(&action.order, &current.order) {
                last = i;
                break;
            }
        }
        let index = if first >= last {
            first
        } else {
            self.state.rng.range_inclusive_for(
                RandomRequestKind::QueueInsert,
                first as u16,
                (last - 1) as u16,
            ) as usize
        };
        actions.insert(index, action);
    }

    /// Showdown `runEvent('AfterMega')` of the Pokémon on the field: the abilities' and items'
    /// `onAnyAfterMega` (Opportunist, White Herb, Eject Pack, Mirror Herb) — the same handlers
    /// their `onAnyAfterMove` runs — fastest first, a holder's ability before its item.
    fn run_after_mega(&mut self) {
        let mut holders: Vec<(PokemonId, u32, u8, crate::effects::SwitchHandler)> = Vec::new();
        for pokemon in self.active_pokemon() {
            let speed = crate::engine::stats::handler_speed(&self.state, pokemon);
            if let Some(handler) = self.state.pokemon(pokemon).and_then(|p| {
                crate::engine::ability_handler(&self.state, &self.rules, p, |b| b.any_after_mega)
            }) {
                holders.push((pokemon, speed, 0, handler));
            }
            if let Some(handler) = crate::engine::held_item(&self.state, &self.rules, pokemon)
                .and_then(|(_, behavior)| behavior.any_after_mega)
            {
                holders.push((pokemon, speed, 1, handler));
            }
        }
        holders.sort_by_key(|(_, speed, sub, _)| (std::cmp::Reverse(*speed), *sub));
        for (pokemon, _, _, handler) in holders {
            let mut ctx =
                crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            handler(&mut ctx, pokemon);
        }
    }

    /// Showdown `runAction`'s `runDynamax` + `addVolatile('dynamax')` (DESIGN.md §44.5): the
    /// Pokémon Dynamaxes — Minimize and Substitute go, Torment ends silently, and the HP is
    /// multiplied by `1.5 + 0.05 × its Dynamax level` (Shedinja excepted).
    fn dynamax(&mut self, pokemon: PokemonId) -> Result<(), BattleError> {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return Err(BattleError::UnknownPokemon(pokemon));
        };
        let level = holder.dynamax_level;
        let base_max_hp = holder.max_hp;
        // `pokemon.baseSpecies.name === 'Shedinja'`: its own species, whatever it transformed into
        // (DESIGN.md R-M9-25: the check named national dex number 514, which is Simisear).
        let shedinja = holder.species == Some(crate::ids::SpeciesId::SHEDINJA);
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.volatiles.set_dynamax(base_max_hp);
            holder.volatiles.remove(VolatileConditionKind::Minimize);
            holder.volatiles.remove(VolatileConditionKind::Substitute);
            if holder.volatiles.remove(VolatileConditionKind::Torment) {
                self.trace.push(TraceEvent::VolatileEnded {
                    target: pokemon,
                    kind: VolatileConditionKind::Torment,
                    seen: true,
                });
            }
        }
        self.trace.push(TraceEvent::DynamaxStarted { pokemon });
        // Changes based on dynamax level, 2 is max (at LVL 10). Shedinja keeps its one point.
        if !shedinja {
            let ratio = 1.5 + f64::from(level.min(10)) * 0.05;
            if let Some(holder) = self.state.pokemon_mut(pokemon) {
                let max_hp = (f64::from(holder.max_hp) * ratio).floor() as u16;
                let hp = (f64::from(holder.hp) * ratio).floor() as u16;
                holder.max_hp = max_hp.max(1);
                holder.hp = hp.min(holder.max_hp);
            }
            self.trace.push(TraceEvent::HealApplied {
                target: pokemon,
                amount: 0,
                hp_before: 0,
                hp_after: self.state.pokemon(pokemon).map_or(0, |p| p.hp),
                source: EffectSource::Rule,
            });
        }
        Ok(())
    }

    /// Showdown `dynamax.onEnd` (DESIGN.md §44.5): the HP comes back to its undynamaxed value
    /// and the max HP returns.
    pub(super) fn end_dynamax(&mut self, pokemon: PokemonId) {
        let Some(holder) = self.state.pokemon(pokemon) else {
            return;
        };
        let Some(base_max_hp) = holder.volatiles.dynamax_base_max_hp() else {
            return;
        };
        let hp = holder.undynamaxed_hp(None);
        self.trace.push(TraceEvent::DynamaxEnded { pokemon });
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.volatiles.remove(VolatileConditionKind::Dynamax);
            holder.max_hp = base_max_hp;
            holder.hp = hp.min(base_max_hp);
        }
        self.trace.push(TraceEvent::HealApplied {
            target: pokemon,
            amount: 0,
            hp_before: 0,
            hp_after: self.state.pokemon(pokemon).map_or(0, |p| p.hp),
            source: EffectSource::Rule,
        });
    }

    /// Showdown `actions.terastallize` (DESIGN.md §44.6): the Pokémon takes its Tera type, its
    /// added type goes, Ogerpon's `-Tera` forme and Terapagos's Stellar one follow, and the
    /// `AfterTerastallization` handlers run. False when nothing happened at all: an Ogerpon
    /// whose Tera type is none of Fire, Grass, Rock and Water (Showdown's hint and return) —
    /// its side's Terastallization is not spent then.
    fn terastallize(&mut self, pokemon: PokemonId) -> Result<bool, BattleError> {
        let Some((tera_type, own, battling)) = self
            .state
            .pokemon(pokemon)
            .map(|p| (p.tera_type, p.species, crate::engine::species_of(p)))
        else {
            return Err(BattleError::UnknownPokemon(pokemon));
        };
        // Ogerpon's Tera type is one of its masks' (Showdown's `hint`): anything else does
        // nothing at all (44.10 的 S2).
        if own.is_some_and(|own| own.same_species(crate::ids::SpeciesId::OGERPON))
            && !matches!(
                tera_type,
                crate::dex::PokemonType::Fire
                    | crate::dex::PokemonType::Grass
                    | crate::dex::PokemonType::Rock
                    | crate::dex::PokemonType::Water
            )
        {
            return Ok(false);
        }
        // An Illusion that wears an Ogerpon's or Terapagos's face ends first (Showdown ends the
        // ability as it terastallizes): the disguise would otherwise be the form the log names.
        let disguised = self
            .state
            .pokemon(pokemon)
            .and_then(|holder| holder.illusion)
            .and_then(|illusion| self.state.pokemon(illusion))
            .and_then(|illusion| illusion.species);
        if disguised.is_some_and(|species| {
            species.same_species(crate::ids::SpeciesId::OGERPON)
                || species.same_species(crate::ids::SpeciesId::TERAPAGOS)
        }) {
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .end_illusion(pokemon);
        }
        if let Some(holder) = self.state.pokemon_mut(pokemon) {
            holder.terastallized = Some(tera_type);
            holder.added_type = None;
        }
        self.trace
            .push(TraceEvent::Terastallized { pokemon, tera_type });
        // Ogerpon's `-Tera` forme (its masks' forms, permanent and source-less): its ability
        // becomes the matching Embody Aspect, which starts and raises its stat.
        if let Some(forme) = battling.and_then(crate::effects::abilities::tera::ogerpon_tera_forme)
            && let Some(holder) = self.state.pokemon(pokemon)
            && holder.forme_of(forme).is_some()
        {
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .forme_change(
                    pokemon,
                    forme,
                    crate::engine::FormeChangeKind::Permanent,
                    EffectSource::Rule,
                );
        }
        // Morpeko keeps the forme it is in (Showdown marks it `formeRegression` and takes it
        // as the base): a faint puts the set's Morpeko back (DESIGN.md §44.6).
        if battling.is_some_and(|species| species.same_species(crate::ids::SpeciesId::MORPEKO))
            && let Some(holder) = self.state.pokemon(pokemon)
            && holder.transform.is_none()
            && holder.species != battling
            && let Some(forme) = battling.and_then(|species| holder.forme_of(species).copied())
            && let Some(holder) = self.state.pokemon_mut(pokemon)
        {
            holder.take_forme(&forme);
            holder.forme_regression = true;
        }
        // Terapagos-Terastal becomes its Stellar forme (Teraform Zero then runs below).
        if battling == Some(crate::ids::SpeciesId::TERAPAGOS_TERASTAL) {
            crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                .forme_change(
                    pokemon,
                    crate::ids::SpeciesId::TERAPAGOS_STELLAR,
                    crate::engine::FormeChangeKind::Permanent,
                    EffectSource::Rule,
                );
        }
        self.run_after_terastallization();
        Ok(true)
    }

    /// Showdown `runEvent('AfterTerastallization')` of the Pokémon on the field: the abilities'
    /// (Opportunist) and items' (Mirror Herb) `onAnyAfterTerastallization`, fastest first, a
    /// holder's ability before its item.
    fn run_after_terastallization(&mut self) {
        let mut holders: Vec<(PokemonId, u32, u8, crate::effects::SwitchHandler)> = Vec::new();
        for pokemon in self.active_pokemon() {
            let speed = crate::engine::stats::handler_speed(&self.state, pokemon);
            // The Terastallized Pokémon's own `onAfterTerastallization` (Teraform Zero) runs
            // with the others'.
            if let Some(handler) = self.state.pokemon(pokemon).and_then(|p| {
                let behavior = crate::engine::ability_handlers(&self.state, &self.rules, p);
                behavior
                    .after_terastallization
                    .or(behavior.any_after_terastallization)
            }) {
                holders.push((pokemon, speed, 0, handler));
            }
            if let Some(handler) = crate::engine::held_item(&self.state, &self.rules, pokemon)
                .and_then(|(_, behavior)| behavior.any_after_terastallization)
            {
                holders.push((pokemon, speed, 1, handler));
            }
        }
        holders.sort_by_key(|(_, speed, sub, _)| (std::cmp::Reverse(*speed), *sub));
        for (pokemon, _, _, handler) in holders {
            let mut ctx =
                crate::engine::BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            handler(&mut ctx, pokemon);
        }
    }
}
