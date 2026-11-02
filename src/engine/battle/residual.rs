//! `BattleEngine`'s end-of-turn residual scheduler (ROADMAP §7.3, DESIGN.md §30.1): every
//! residual instance on the field collected with its generation's order key, sorted (order,
//! faster first, sub-order, speed ties drawn), run one by one (`run_residual`); the turn's
//! expiry. The one owner of the order: each condition's own handler is in
//! `residual_effects.rs`, run from here.

use super::BattleEngine;
use crate::engine::{BattleContext, EffectSource, TraceEvent};
use crate::ids::{PokemonId, SideId};
use crate::rules::ResidualKind;
use crate::state::{
    BattlePhase, NonVolatileStatus, RandomRequestKind, VolatileConditionKind, WeatherKind,
};

impl BattleEngine {
    /// End-of-turn residual scheduler (ROADMAP §7.3): every residual instance on the field is
    /// collected with its generation order key, sorted (order, faster first, sub-order), and
    /// run in that order; instances tied on all three are shuffled (Showdown `speedSort`: one
    /// `SpeedTie` decision each, `residual_tie_groups`). An instance whose holder has left the
    /// field or fainted since collection is skipped, as Showdown skips handlers of fainted
    /// holders.
    ///
    /// Showdown's `residual` action starts with `updateSpeed`: every sort in it — the handlers
    /// here, the weather's `eachEvent` — reads the speeds of that moment (`residual_speeds`), so
    /// the Gen 4 Tailwind that ends first still doubles its side's place (R-M9-46).
    pub(super) fn run_end_turn_residual(&mut self) {
        self.residual_speeds = self
            .active_pokemon()
            .into_iter()
            .map(|id| {
                (
                    id,
                    crate::engine::residual_speed(&self.state, &self.rules, id),
                )
            })
            .collect();
        self.run_residual_handlers();
        self.residual_speeds.clear();
    }

    fn run_residual_handlers(&mut self) {
        self.set_phase(BattlePhase::EndTurn);
        self.state.turn_actions_done = true;

        // Gen 4 Tailwind counts down before anything else in the residual.
        if self.rules.conditions.tailwind_ticks_first {
            for side in [SideId::P1, SideId::P2] {
                if self.state.side_mut(side).conditions.tick_tailwind() {
                    self.trace.push(TraceEvent::SideConditionEnded {
                        side,
                        kind: crate::state::SideConditionKind::Tailwind,
                    });
                }
            }
        }
        // So do Gen 3-4 screens, Mist, Safeguard and Lucky Chant (`onSideResidualOrder` 1-6),
        // before the weather can decide the battle.
        if self.rules.conditions.side_protections.tick_first {
            for side in [SideId::P1, SideId::P2] {
                for kind in self.state.side_mut(side).conditions.tick_timed() {
                    self.trace
                        .push(TraceEvent::SideConditionEnded { side, kind });
                }
            }
        }

        // Showdown's `residual` action keeps every active Pokémon's HP for the Emergency Exit
        // check once the whole residual is over — `getUndynamaxedHP()`, the space the residual's
        // own check reads (DESIGN.md §44.5).
        let hp_before: Vec<(PokemonId, u16)> = self
            .active_pokemon()
            .into_iter()
            .filter_map(|id| self.state.pokemon(id).map(|p| (id, p.undynamaxed_hp(None))))
            .collect();

        // Showdown's slot conditions run for a fainted occupant too (`isSlotCondition`): a Wish
        // whose slot has nobody standing counts down all the same, and heals no one if it runs
        // out there.
        self.tick_unattended_wishes();
        self.tick_unattended_future_attacks();
        let collected = self.collect_residuals();
        // Gen 2+: the duration-only handlers take their place among the others (§31.1).
        let positioned = self.rules.residual.durations.is_some();
        let durations = self.collect_durations();
        let mut tie_breaks = vec![0; collected.len() + durations.len()];
        for (index, _) in residual_tie_groups(&collected, &durations) {
            tie_breaks[index] = self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
        }
        let mut handlers: Vec<(ResidualHandler, u64)> = collected
            .into_iter()
            .map(ResidualHandler::Instance)
            .chain(durations.into_iter().map(ResidualHandler::Duration))
            .zip(tie_breaks)
            .collect();
        // A stable sort: handlers equal on every key keep their collection order.
        handlers.sort_by_key(|(handler, tie_break)| {
            let (order, speed, sub_order, slot) = handler.key();
            (
                // Showdown `comparePriority`: order, then speed, then sub-order — so in Gen 3-4
                // (everything at order 10) the faster Pokémon finishes all its residuals first.
                order,
                std::cmp::Reverse(speed),
                sub_order,
                std::cmp::Reverse(*tie_break),
                slot,
            )
        });

        // A handler whose count ran out (Showdown `fieldEvent` calls its `end` and moves on
        // without `faintMessages()`) leaves its faint to the next handler that runs: Perish Song
        // taking both last Pokémon in one residual is a draw, and a duration that goes on after
        // it (a terrain, a charge) settles those faints before the later ones count down.
        let mut faints_waiting = false;
        for (handler, _) in handlers {
            // Showdown `fieldEvent`: `faintMessages()` after every handler, and the residual
            // stops the moment the battle is decided.
            if self.state.outcome.is_some() {
                break;
            }
            let instance = match handler {
                ResidualHandler::Duration(duration) => {
                    match self.run_duration(duration.handler) {
                        None | Some(ResidualRun::Ended) => {}
                        Some(ResidualRun::Ran) => {
                            self.resolve_faints();
                            faints_waiting = false;
                        }
                    }
                    continue;
                }
                ResidualHandler::Instance(instance) => instance,
            };
            let Some(holder) = instance.holder else {
                // The field's residual: the weather (its tick, then its chip on everyone).
                if self.state.field.weather.is_some() {
                    self.run_weather_residual();
                    self.resolve_faints();
                    faints_waiting = false;
                }
                continue;
            };
            // Showdown `fieldEvent`: the handlers of a Pokémon that has fainted are skipped, and
            // so is one whose condition a prior handler removed (a Healer's cure ending
            // Nightmare).
            let on_field = self
                .state
                .active_position(holder, &self.rules.topology)
                .is_some();
            // A slot condition whose holder has fallen since collection still counts down,
            // striking or healing no one (its `onEnd` asks for a standing target).
            if let Some((side, slot)) = instance.position
                && (!on_field || self.state.pokemon(holder).is_some_and(|p| p.is_fainted()))
            {
                self.count_down_slot_condition(instance.kind, side, slot);
                self.resolve_faints();
                faints_waiting = false;
                continue;
            }
            if !on_field || !self.residual_condition_held(holder, instance.kind) {
                continue;
            }
            // One whose faint is still waiting is not yet `fainted`: its handler runs (to no
            // effect at 0 HP) and the faints settle after it — unless its count runs out, which
            // leaves them waiting (the Roost of a Pokémon Perish Song took, R-M10-66).
            if self
                .state
                .pokemon(holder)
                .is_some_and(|pokemon| pokemon.is_fainted())
            {
                if self.count_runs_out(holder, instance.kind) {
                    faints_waiting = true;
                } else {
                    self.resolve_faints();
                    faints_waiting = false;
                }
                continue;
            }
            let counted_out = self.count_runs_out(holder, instance.kind);
            self.trace.push(TraceEvent::ResidualEffect {
                holder,
                kind: instance.kind,
                order: instance.order,
                sub_order: instance.sub_order,
            });
            if self.run_residual(holder, instance.kind) == ResidualRun::Ended || counted_out {
                faints_waiting = true;
            } else {
                self.resolve_faints();
                faints_waiting = false;
            }
        }
        // No handler ran after that count (Showdown's next `faintMessages`). Gen 1, whose
        // durations are not handlers of the residual, ticks every duration-only effect before
        // those faints settle.
        let durations_ticked = faints_waiting && !positioned;
        if faints_waiting {
            if !positioned {
                self.expire_conditions_end_turn(false);
            }
            self.resolve_faints();
        }

        // Status counters advance once per turn, after the ticks; the turn's declared moves are
        // all spent. Time on the field (`active_turns`) advances when the next turn starts
        // (`BattleEngine::next_turn`), as Showdown's `activeTurns` does.
        let active = self.active_pokemon();
        for pokemon_id in active {
            if let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
                && let Some(status) = &mut pokemon.status
            {
                status.advance_turn();
            }
        }
        self.state.pending_movers.clear();
        self.state.struggling.clear();

        // The durations (Tailwind 26, Trick Room / terrain 27, and every duration-only effect
        // after them) tick inside Showdown's residual, after every handler that can knock a
        // Pokémon out: a battle decided in the residual never gets there.
        if self.state.outcome.is_some() {
            return;
        }
        if !durations_ticked {
            self.expire_conditions_end_turn(positioned);
        }
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        let mut exited = false;
        for (pokemon, before) in hp_before {
            // The residual's own check reads through `getUndynamaxedHP` (DESIGN.md §44.5).
            exited |= ctx.emergency_exit_residual(pokemon, before);
        }
        // Showdown runs this `EmergencyExit` after the residual's `checkFainted`, whose switch
        // flags on the fainted the exit clears: their slots are asked for after the exit's. One
        // a hit in the residual already ordered out (a Future Sight setting off Wimp Out) clears
        // nothing (`target.switchFlag` returns first): one request for all (R-M8-130).
        self.state.faint_refills_held = exited;
    }

    /// The speed the residual and weather schedules order by: Showdown's residual starts with
    /// `updateSpeed` and sorts its handlers once, so it is the effective speed (stages,
    /// paralysis, Tailwind, Choice Scarf; Gen 2-3's Quick Claw) of that moment, inverted under
    /// Trick Room — which is also what a replay previewing the residual's ties reads.
    pub(super) fn residual_speed(&self, pokemon: PokemonId) -> u32 {
        self.residual_speeds
            .iter()
            .find(|(id, _)| *id == pokemon)
            .map_or_else(
                || crate::engine::residual_speed(&self.state, &self.rules, pokemon),
                |(_, speed)| *speed,
            )
    }

    /// The active Pokémon (in `active_pokemon` order) whose Speed ties another's as the weather
    /// chips them, by index, each with its tie group.
    pub(super) fn weather_tie_groups(&self) -> Vec<(usize, usize)> {
        let speeds: Vec<u32> = self
            .active_pokemon()
            .into_iter()
            .map(|id| self.residual_speed(id))
            .collect();
        tie_groups(&speeds)
    }

    internal!(
        /// The end-of-turn decisions a replay has to plan before the residual runs (DESIGN.md
        /// §12.4): the residual handlers that tie, in the order their `SpeedTie` decisions are drawn
        /// with their tie groups, then — when the weather will chip this turn — the Pokémon that
        /// tie for its order.
        pub fn pending_residual_ties(&self) -> super::ResidualTies {
            let instances = self.collect_residuals();
            let durations = self.collect_durations();
            let tied = residual_tie_groups(&instances, &durations);
            let handlers = tied
                .iter()
                .filter_map(|&(index, group)| {
                    let instance = instances.get(index)?;
                    Some((instance.holder?, instance.kind, group))
                })
                .collect();
            let durations = tied
                .iter()
                .filter_map(|&(index, group)| {
                    Some((
                        durations.get(index.checked_sub(instances.len())?)?.handler,
                        group,
                    ))
                })
                .collect();
            let suppressed = crate::engine::weather::suppressed(&self.state, &self.rules);
            let chips = self.state.field.weather.is_some_and(|weather| {
                weather.turns_remaining != 1
                    && !suppressed
                    && match weather.kind {
                        WeatherKind::Sandstorm => true,
                        WeatherKind::Snow => self.rules.weather.hail_damages,
                        _ => false,
                    }
            });
            let active = self.active_pokemon();
            let weather = if chips {
                self.weather_tie_groups()
                    .into_iter()
                    .map(|(index, group)| (active[index], group))
                    .collect()
            } else {
                Vec::new()
            };
            super::ResidualTies {
                handlers,
                durations,
                weather,
            }
        }
    );

    pub(super) fn active_pokemon(&self) -> Vec<PokemonId> {
        let mut active = Vec::new();
        for side in [SideId::P1, SideId::P2] {
            active.extend(self.state.side(side).active.iter().copied().flatten());
        }
        active
    }

    pub(super) fn collect_residuals(&self) -> Vec<ResidualInstance> {
        let rules = self.rules.residual;
        let mut instances = Vec::new();
        if self.state.field.weather.is_some()
            && let Some((order, sub_order)) = rules.key(ResidualKind::Weather)
        {
            instances.push(ResidualInstance {
                holder: None,
                kind: ResidualKind::Weather,
                order,
                sub_order,
                speed: 0,
                slot: 0,
                position: None,
            });
        }
        for (slot, holder) in self.active_pokemon().into_iter().enumerate() {
            let Some(pokemon) = self.state.pokemon(holder) else {
                continue;
            };
            let speed = self.residual_speed(holder);
            // Gen 3 on the cartridge (issues.md I111): a strike goes by battler (pokeemerald
            // `HandleWishPerishSongOnTurnEnd`), not by speed — distinct keys, no ties.
            let future_battler = self.future_attack_battler(holder);
            // A kind the generation's table does not list has no end-of-turn residual there.
            let mut push = |kind: ResidualKind| {
                let Some((order, sub_order)) = rules.key(kind) else {
                    return;
                };
                let speed = match (kind, future_battler) {
                    (ResidualKind::FutureAttack, Some(battler)) => u32::MAX - battler,
                    _ => speed,
                };
                let position = match kind {
                    ResidualKind::Wish | ResidualKind::FutureAttack => self
                        .state
                        .active_position(holder, &self.rules.topology)
                        .and_then(|position| self.rules.topology.spec(position))
                        .map(|spec| (spec.side, usize::from(spec.slot))),
                    _ => None,
                };
                instances.push(ResidualInstance {
                    holder: Some(holder),
                    kind,
                    order,
                    sub_order,
                    speed,
                    slot,
                    position,
                });
            };
            // The held item as it is: Showdown collects a set-aside item's handler too and
            // skips it as it runs (`singleEvent` asks `ignoringItem()` then), so an item Magic
            // Room stops acts once the room ends earlier in the same residual.
            if let Some(behavior) = pokemon
                .held_item
                .map(|item| crate::engine::item_behavior(&self.rules, item))
            {
                if let Some(participant) = behavior.residual {
                    push(participant.kind);
                } else if matches!(behavior.update, Some(crate::effects::items::ItemUpdate::Eat(trigger)) if trigger.at_turn_end(&self.rules.conditions.berries))
                {
                    // Gen 2's status berries (pokecrystal `HandleHealingItems`, issues.md I33)
                    // and Gen 2-3's PP berries (`HandleMysteryberry`, `ENDTURN_ITEMS1`, I49) at
                    // the end of the turn.
                    push(ResidualKind::Berry);
                }
            }
            // The ability as it is: Showdown collects a suppressed ability's handler too and skips
            // it as it runs (`fieldEvent` asks `suppressingAbility` then), so an ability a
            // Neutralizing Gas stops acts once its holder faints earlier in the same residual.
            if let Some(participant) =
                crate::engine::ability_behavior(&self.rules, crate::engine::raw_ability(pokemon))
                    .residual
            {
                push(participant.kind);
            }
            if pokemon.volatiles.contains(VolatileConditionKind::Ingrain) {
                push(ResidualKind::Ingrain);
            }
            // Dynamax's counter (its own `onResidualPriority: -100`): last among the handlers.
            if pokemon.volatiles.contains(VolatileConditionKind::Dynamax) {
                push(ResidualKind::DynamaxCounter);
            }
            if self.wish_slot(holder).is_some() {
                push(ResidualKind::Wish);
            }
            if self.future_attack_slot(holder).is_some() {
                push(ResidualKind::FutureAttack);
            }
            if pokemon
                .volatiles
                .contains(VolatileConditionKind::PerishSong)
            {
                push(ResidualKind::PerishSong);
            }
            if pokemon.volatiles.contains(VolatileConditionKind::AquaRing) {
                push(ResidualKind::AquaRing);
            }
            if self.state.field.terrain.map(|t| t.kind) == Some(crate::state::TerrainKind::Grassy) {
                push(ResidualKind::GrassyTerrain);
            }
            // A combined Fire Pledge on this Pokémon's side chips it (a Fire type is spared,
            // in the instance's own check): the same side condition, per active Pokémon
            // (Showdown's `onResidual`, DESIGN.md §42.9).
            if self
                .state
                .side(pokemon.side)
                .conditions
                .contains(crate::state::SideConditionKind::FirePledge)
            {
                push(ResidualKind::FirePledge);
            }
            if pokemon.volatiles.contains(VolatileConditionKind::LeechSeed) {
                push(ResidualKind::LeechSeed);
            }
            if pokemon
                .volatiles
                .contains(VolatileConditionKind::PartiallyTrapped)
            {
                push(ResidualKind::PartiallyTrapped);
            }
            if pokemon.volatiles.contains(VolatileConditionKind::Octolock) {
                push(ResidualKind::Octolock);
            }
            match pokemon.status.map(|status| status.kind) {
                Some(NonVolatileStatus::Poison) => push(ResidualKind::Poison),
                Some(NonVolatileStatus::Toxic) => push(ResidualKind::Toxic),
                Some(NonVolatileStatus::Burn) => push(ResidualKind::Burn),
                Some(NonVolatileStatus::Freeze) => push(ResidualKind::FreezeThaw),
                _ => {}
            }
            for (kind, residual) in [
                (VolatileConditionKind::Taunt, ResidualKind::Taunt),
                (VolatileConditionKind::Encore, ResidualKind::Encore),
                (VolatileConditionKind::Disable, ResidualKind::Disable),
                (VolatileConditionKind::Nightmare, ResidualKind::Nightmare),
                (VolatileConditionKind::Curse, ResidualKind::Curse),
                (VolatileConditionKind::Yawn, ResidualKind::Yawn),
                (VolatileConditionKind::HealBlock, ResidualKind::HealBlock),
                (VolatileConditionKind::Embargo, ResidualKind::Embargo),
                (VolatileConditionKind::MagnetRise, ResidualKind::MagnetRise),
                (
                    VolatileConditionKind::Telekinesis,
                    ResidualKind::Telekinesis,
                ),
                (VolatileConditionKind::SaltCure, ResidualKind::SaltCure),
                (VolatileConditionKind::SyrupBomb, ResidualKind::SyrupBomb),
                (VolatileConditionKind::ThroatChop, ResidualKind::ThroatChop),
                (VolatileConditionKind::Roost, ResidualKind::Roost),
                (VolatileConditionKind::Electrify, ResidualKind::Electrify),
                (VolatileConditionKind::Powder, ResidualKind::Powder),
                (VolatileConditionKind::LaserFocus, ResidualKind::LaserFocus),
                (VolatileConditionKind::MicleBerry, ResidualKind::MicleBoost),
                (VolatileConditionKind::Rampage, ResidualKind::Rampage),
                (VolatileConditionKind::Uproar, ResidualKind::Uproar),
            ] {
                if pokemon.volatiles.contains(kind) {
                    push(residual);
                }
            }
        }
        instances
    }

    /// Whether `kind`'s residual on `holder` is a count running out (Showdown `fieldEvent`: the
    /// handler's `duration` reaches 0, `handler.end` runs and the loop goes on with no
    /// `faintMessages()`, R-M10-66): Roost, Electrify and Powder last the turn, the others have
    /// one turn left. Gen 2 on, where the durations are handlers of the residual. Uproar and a
    /// rampage are left out: a callback of theirs may end them too.
    fn count_runs_out(&self, holder: PokemonId, kind: ResidualKind) -> bool {
        if self.rules.residual.durations.is_none() {
            return false;
        }
        let volatile = match kind {
            ResidualKind::Roost | ResidualKind::Electrify | ResidualKind::Powder => return true,
            ResidualKind::Taunt => VolatileConditionKind::Taunt,
            ResidualKind::Encore => VolatileConditionKind::Encore,
            ResidualKind::Disable => VolatileConditionKind::Disable,
            ResidualKind::Yawn => VolatileConditionKind::Yawn,
            ResidualKind::HealBlock => VolatileConditionKind::HealBlock,
            ResidualKind::Embargo => VolatileConditionKind::Embargo,
            ResidualKind::MagnetRise => VolatileConditionKind::MagnetRise,
            ResidualKind::Telekinesis => VolatileConditionKind::Telekinesis,
            ResidualKind::ThroatChop => VolatileConditionKind::ThroatChop,
            ResidualKind::LaserFocus => VolatileConditionKind::LaserFocus,
            ResidualKind::SyrupBomb => VolatileConditionKind::SyrupBomb,
            ResidualKind::MicleBoost => VolatileConditionKind::MicleBerry,
            ResidualKind::PartiallyTrapped => VolatileConditionKind::PartiallyTrapped,
            _ => return false,
        };
        self.state
            .pokemon(holder)
            .and_then(|pokemon| pokemon.volatiles.timed_turns_left(volatile))
            == Some(1)
    }

    /// Whether the volatile or status a residual belongs to is still on its holder.
    fn residual_condition_held(&self, holder: PokemonId, kind: ResidualKind) -> bool {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return false;
        };
        let volatile = match kind {
            ResidualKind::Ingrain => VolatileConditionKind::Ingrain,
            ResidualKind::LeechSeed => VolatileConditionKind::LeechSeed,
            ResidualKind::PartiallyTrapped => VolatileConditionKind::PartiallyTrapped,
            ResidualKind::Octolock => VolatileConditionKind::Octolock,
            ResidualKind::PerishSong => VolatileConditionKind::PerishSong,
            ResidualKind::Taunt => VolatileConditionKind::Taunt,
            ResidualKind::Encore => VolatileConditionKind::Encore,
            ResidualKind::Disable => VolatileConditionKind::Disable,
            ResidualKind::Nightmare => VolatileConditionKind::Nightmare,
            ResidualKind::Curse => VolatileConditionKind::Curse,
            ResidualKind::Yawn => VolatileConditionKind::Yawn,
            ResidualKind::HealBlock => VolatileConditionKind::HealBlock,
            ResidualKind::Embargo => VolatileConditionKind::Embargo,
            ResidualKind::MagnetRise => VolatileConditionKind::MagnetRise,
            ResidualKind::Telekinesis => VolatileConditionKind::Telekinesis,
            ResidualKind::SaltCure => VolatileConditionKind::SaltCure,
            ResidualKind::SyrupBomb => VolatileConditionKind::SyrupBomb,
            ResidualKind::ThroatChop => VolatileConditionKind::ThroatChop,
            ResidualKind::Roost => VolatileConditionKind::Roost,
            ResidualKind::Electrify => VolatileConditionKind::Electrify,
            ResidualKind::Powder => VolatileConditionKind::Powder,
            ResidualKind::MicleBoost => VolatileConditionKind::MicleBerry,
            ResidualKind::Rampage => VolatileConditionKind::Rampage,
            ResidualKind::Uproar => VolatileConditionKind::Uproar,
            ResidualKind::Poison | ResidualKind::Toxic => {
                return pokemon.has_status(NonVolatileStatus::Poison)
                    || pokemon.has_status(NonVolatileStatus::Toxic);
            }
            ResidualKind::Burn => return pokemon.has_status(NonVolatileStatus::Burn),
            ResidualKind::FreezeThaw => return pokemon.has_status(NonVolatileStatus::Freeze),
            _ => return true,
        };
        pokemon.volatiles.contains(volatile)
    }

    /// A Wish's or a foreseen attack's count on `side`'s `slot`, its holder down: one residual
    /// off, gone at the last without its effect.
    fn count_down_slot_condition(&mut self, kind: ResidualKind, side: SideId, slot: usize) {
        let side = self.state.side_mut(side);
        match kind {
            ResidualKind::Wish => {
                if let Some(index) = side.wishes.iter().position(|wish| wish.slot == slot) {
                    let wish = &mut side.wishes[index];
                    wish.turns_remaining = wish.turns_remaining.saturating_sub(1);
                    if wish.turns_remaining == 0 {
                        side.wishes.remove(index);
                    }
                }
            }
            _ => {
                if let Some(index) = side.future_attacks.iter().position(|a| a.slot == slot) {
                    let attack = &mut side.future_attacks[index];
                    attack.turns_remaining = attack.turns_remaining.saturating_sub(1);
                    if attack.turns_remaining == 0 {
                        side.future_attacks.remove(index);
                    }
                }
            }
        }
    }

    /// Runs one residual handler; `Ended` when its count ran out (Showdown's `handler.end`).
    pub(super) fn run_residual(&mut self, holder: PokemonId, kind: ResidualKind) -> ResidualRun {
        match kind {
            // Dynamax's `onResidual`: one turn older; the third turn's end is the `endTurn`
            // sweep (`expire_dynamax`, after the whole residual).
            ResidualKind::DynamaxCounter => {
                if let Some(pokemon) = self.state.pokemon_mut(holder) {
                    pokemon.volatiles.count_a_dynamax_turn();
                }
                return ResidualRun::Ran;
            }
            ResidualKind::Wish => self.run_wish(holder),
            ResidualKind::FutureAttack => return self.run_future_attack(holder),
            ResidualKind::PerishSong => return self.run_perish_song(holder),
            // Field-scoped; dispatched from `run_end_turn_residual` without a holder.
            ResidualKind::Weather => {}
            // A berry at the end of the turn: its own `onResidual` (Gen 2-3 HP and confusion
            // berries, Gen 2 Berry Juice), or a Gen 2 status berry's check.
            ResidualKind::Berry => {
                let Some((_, item)) = crate::engine::held_item(&self.state, &self.rules, holder)
                else {
                    return ResidualRun::Ran;
                };
                let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                match item.residual {
                    Some(participant) if participant.kind == kind => {
                        (participant.handler)(&mut ctx, holder);
                    }
                    Some(_) => {}
                    None => ctx.feed_turn_end_berry(holder),
                }
            }
            // Item residuals: the holder's item, if it still has the one that was scheduled.
            ResidualKind::Leftovers
            | ResidualKind::BlackSludge
            | ResidualKind::StatusOrb
            | ResidualKind::MicleBerry
            | ResidualKind::WhiteHerb
            | ResidualKind::MirrorHerb
            | ResidualKind::EjectPack
            | ResidualKind::StickyBarb => {
                let Some((_, item)) = crate::engine::held_item(&self.state, &self.rules, holder)
                else {
                    return ResidualRun::Ran;
                };
                if let Some(participant) = item.residual
                    && participant.kind == kind
                {
                    let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                    (participant.handler)(&mut ctx, holder);
                }
            }
            // Ability residuals (Shed Skin, Speed Boost, Slow Start, Gen 3 Truant, Rain Dish,
            // Hydration, Healer, Bad Dreams, Moody, Harvest, Cud Chew, Pickup); Harvest asks the dex whether
            // the last item is a berry.
            ResidualKind::ShedSkin
            | ResidualKind::SpeedBoost
            | ResidualKind::SlowStart
            | ResidualKind::Truant
            | ResidualKind::RainDish
            | ResidualKind::Hydration
            | ResidualKind::Healer
            | ResidualKind::BadDreams
            | ResidualKind::Moody
            | ResidualKind::Harvest
            | ResidualKind::CudChew
            | ResidualKind::Pickup
            | ResidualKind::FormeChange
            | ResidualKind::Opportunist => {
                let Some(ability) = self
                    .state
                    .pokemon(holder)
                    .and_then(|p| crate::engine::ability_running(&self.state, &self.rules, p))
                else {
                    return ResidualRun::Ran;
                };
                if let Some(participant) =
                    crate::engine::ability_behavior(&self.rules, ability).residual
                    && participant.kind == kind
                {
                    let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                        .with_dex(&self.dex);
                    (participant.handler)(&mut ctx, holder);
                }
            }
            ResidualKind::LeechSeed => self.run_leech_seed(holder),
            ResidualKind::Rampage => return self.run_rampage_residual(holder),
            ResidualKind::Uproar => return self.run_uproar(holder),
            ResidualKind::FreezeThaw => self.run_freeze_thaw(holder),
            ResidualKind::Ingrain | ResidualKind::AquaRing => {
                let Some(max_hp) = self.state.pokemon(holder).map(|p| p.base_max_hp()) else {
                    return ResidualRun::Ran;
                };
                let volatile = if kind == ResidualKind::Ingrain {
                    VolatileConditionKind::Ingrain
                } else {
                    VolatileConditionKind::AquaRing
                };
                let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                let amount = ctx.big_root(holder, (max_hp / 16).max(1));
                ctx.apply_heal(crate::engine::HealRequest {
                    target: holder,
                    amount,
                    source: EffectSource::Volatile(volatile),
                    direct: false,
                });
            }
            ResidualKind::PartiallyTrapped => self.run_partially_trapped(holder),
            ResidualKind::FirePledge => {
                let Some(pokemon) = self.state.pokemon(holder) else {
                    return ResidualRun::Ran;
                };
                // Fire types are spared (Showdown's `onResidual`: `!pokemon.hasType('Fire')`).
                if crate::engine::types_of(&self.state, &self.rules, pokemon)
                    .contains(crate::dex::PokemonType::Fire)
                {
                    return ResidualRun::Ran;
                }
                let amount = (pokemon.base_max_hp() / 8).max(1);
                let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                ctx.apply_damage(crate::engine::DamageRequest {
                    target: holder,
                    amount: crate::engine::DamageAmount(amount),
                    kind: crate::engine::DamageKind::Residual,
                    source: EffectSource::SideCondition(
                        crate::state::SideConditionKind::FirePledge,
                    ),
                    flags: crate::engine::DamageFlags::default(),
                });
            }
            ResidualKind::Octolock => {
                BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                    .octolock_residual(holder);
            }
            ResidualKind::GrassyTerrain => {
                let Some(pokemon) = self.state.pokemon(holder) else {
                    return ResidualRun::Ran;
                };
                if !crate::engine::environment::grounded(&self.state, &self.rules, pokemon)
                    || crate::engine::two_turn::hiding(pokemon).is_some()
                {
                    return ResidualRun::Ran;
                }
                let amount = (pokemon.base_max_hp() / 16).max(1);
                let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
                ctx.apply_heal(crate::engine::HealRequest {
                    target: holder,
                    amount,
                    source: EffectSource::Terrain(crate::state::TerrainKind::Grassy),
                    direct: false,
                });
            }
            ResidualKind::Poison | ResidualKind::Toxic | ResidualKind::Burn => {
                self.run_status_residual(holder);
            }
            ResidualKind::Taunt => self.tick_volatile(holder, VolatileConditionKind::Taunt),
            ResidualKind::Encore => {
                // Showdown: `duration--` first (ending at 0), then `onResidual` ends it early
                // when the encored move has no PP left.
                self.tick_volatile(holder, VolatileConditionKind::Encore);
                let out_of_pp = self.state.pokemon(holder).is_some_and(|pokemon| {
                    pokemon.volatiles.encored_move().is_some_and(|encored| {
                        pokemon
                            .move_slot_of(encored)
                            .is_none_or(|slot| slot.pp == 0)
                    })
                });
                if out_of_pp
                    && let Some(pokemon) = self.state.pokemon_mut(holder)
                    && pokemon.volatiles.remove(VolatileConditionKind::Encore)
                {
                    self.trace.push(TraceEvent::VolatileEnded {
                        target: holder,
                        kind: VolatileConditionKind::Encore,
                        seen: true,
                    });
                }
            }
            ResidualKind::Disable => self.tick_volatile(holder, VolatileConditionKind::Disable),
            ResidualKind::Nightmare => self.run_nightmare(holder),
            ResidualKind::Curse => self.run_curse(holder),
            ResidualKind::Yawn => self.run_yawn(holder),
            ResidualKind::HealBlock => {
                self.tick_volatile(holder, VolatileConditionKind::HealBlock);
            }
            ResidualKind::Embargo => self.tick_volatile(holder, VolatileConditionKind::Embargo),
            ResidualKind::MagnetRise => {
                self.tick_volatile(holder, VolatileConditionKind::MagnetRise)
            }
            ResidualKind::Telekinesis => {
                self.tick_volatile(holder, VolatileConditionKind::Telekinesis)
            }
            ResidualKind::SaltCure => self.run_salt_cure(holder),
            ResidualKind::SyrupBomb => self.run_syrup_bomb(holder),
            ResidualKind::ThroatChop => {
                self.tick_volatile(holder, VolatileConditionKind::ThroatChop);
            }
            ResidualKind::LaserFocus => {
                self.tick_volatile(holder, VolatileConditionKind::LaserFocus);
            }
            ResidualKind::Roost | ResidualKind::Electrify | ResidualKind::Powder => {
                let kind = match kind {
                    ResidualKind::Roost => VolatileConditionKind::Roost,
                    ResidualKind::Electrify => VolatileConditionKind::Electrify,
                    _ => VolatileConditionKind::Powder,
                };
                BattleContext::new(&mut self.state, &self.rules, &mut self.trace)
                    .end_volatile(holder, kind);
            }
            ResidualKind::MicleBoost => {
                self.tick_volatile(holder, VolatileConditionKind::MicleBerry);
            }
        }
        ResidualRun::Ran
    }

    /// Gen 3 Follow Me's slot condition lasts the turn: it ends on whoever stands there (seen on
    /// the user, `VolatileStarted`).
    fn clear_follow_me_slots(&mut self, side: SideId) {
        for slot in std::mem::take(&mut self.state.side_mut(side).follow_me_slots) {
            if let Some(target) = self.state.side(side).active.get(slot).copied().flatten() {
                self.trace.push(TraceEvent::VolatileEnded {
                    target,
                    kind: VolatileConditionKind::FollowMe,
                    seen: true,
                });
            }
        }
    }

    /// The end of the turn's expiry; `positioned` (Gen 2+) leaves the volatiles', sides' and
    /// field's durations to their residual handlers (`collect_durations`).
    pub(super) fn expire_conditions_end_turn(&mut self, positioned: bool) {
        // Gen 1 binding's `duration`s (§30.9).
        self.tick_binding();
        if positioned {
            for side in [SideId::P1, SideId::P2] {
                self.clear_follow_me_slots(side);
            }
            return;
        }
        let turn = self.state.turn;
        let pokemon_ids = self.state.pokemon_ids().collect::<Vec<_>>();
        // Showdown `twoturnmove` (duration 2): ticked at the end of the charging turn and of
        // the attacking turn, ending after the latter.
        for pokemon_id in pokemon_ids.iter().copied() {
            // Gen 1's charge has no duration (`ChargeModel::Gen1`).
            if !self.rules.two_turn.gen1() {
                self.tick_volatile(pokemon_id, VolatileConditionKind::TwoTurnMove);
            }
            // Showdown `stall` (duration 2): Protect's counter survives exactly one turn.
            self.tick_volatile(pokemon_id, VolatileConditionKind::Stall);
            // Showdown `mustrecharge` (duration 2): gone at BeforeMove, or after the next turn.
            self.tick_volatile(pokemon_id, VolatileConditionKind::MustRecharge);
            // Showdown `toxic` (duration 1): a Poison type's sure-hit Toxic, for the turn.
            self.tick_volatile(pokemon_id, VolatileConditionKind::SureHit);
            // Showdown `magiccoat` (duration 1): the coat lasts the turn (Gen 4 ended it as it
            // reflected, §42.8).
            self.tick_volatile(pokemon_id, VolatileConditionKind::MagicCoat);
            // Showdown `snatch` and `mefirst` (duration 1) too.
            self.tick_volatile(pokemon_id, VolatileConditionKind::Snatch);
            self.tick_volatile(pokemon_id, VolatileConditionKind::MeFirst);
            // Showdown `allyswitch` (duration 2): the counter survives a turn.
            self.tick_volatile(pokemon_id, VolatileConditionKind::AllySwitch);
            // Chilly Reception's announcement (duration 1).
            self.tick_volatile(pokemon_id, VolatileConditionKind::ChillyReception);
            // Focus Punch's, Beak Blast's, Shell Trap's (duration 1).
            self.tick_volatile(pokemon_id, VolatileConditionKind::FocusPunch);
            self.tick_volatile(pokemon_id, VolatileConditionKind::BeakBlast);
            self.tick_volatile(pokemon_id, VolatileConditionKind::ShellTrap);
            self.tick_volatile(pokemon_id, VolatileConditionKind::Counter);
            self.tick_volatile(pokemon_id, VolatileConditionKind::MirrorCoat);
            // Gen 2+ Bide's `duration`: past its release turn it just ends.
            self.tick_volatile(pokemon_id, VolatileConditionKind::Bide);
            // Rollout / Ice Ball (`duration` 1, back to 2 by each of the first four hits).
            self.tick_volatile(pokemon_id, VolatileConditionKind::Rolling);
        }
        for pokemon_id in pokemon_ids {
            let expired = self
                .state
                .pokemon_mut(pokemon_id)
                .map(|pokemon| pokemon.volatiles.expire_end_turn(turn))
                .unwrap_or_default();
            for kind in expired {
                self.trace.push(TraceEvent::VolatileEnded {
                    target: pokemon_id,
                    kind,
                    seen: true,
                });
            }
        }

        for side in [SideId::P1, SideId::P2] {
            self.clear_follow_me_slots(side);
            let expired = self.state.side_mut(side).conditions.tick_end_turn(
                !self.rules.conditions.tailwind_ticks_first,
                !self.rules.conditions.side_protections.tick_first,
            );
            for kind in expired {
                self.trace
                    .push(TraceEvent::SideConditionEnded { side, kind });
            }
        }

        let expiry = self.state.field.tick_end_turn();
        for kind in expiry.conditions {
            self.trace.push(TraceEvent::FieldConditionEnded { kind });
        }
        if let Some(terrain) = expiry.terrain {
            self.trace.push(TraceEvent::TerrainChanged {
                from: Some(terrain),
                to: None,
            });
            // Showdown `clearTerrain` → `eachEvent('TerrainChange')` (Quark Drive).
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace).terrain_changed();
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ResidualInstance {
    /// `None` for a field-scoped residual (the weather).
    holder: Option<PokemonId>,
    kind: ResidualKind,
    order: u8,
    sub_order: u8,
    speed: u32,
    slot: usize,
    /// A slot condition's place (Wish, Future Sight): it counts down there even once its holder
    /// has fainted (Showdown `fieldEvent` keeps a fainted holder's `isSlotCondition` handlers).
    position: Option<(SideId, usize)>,
}

/// A handler of the residual: one with a callback, or one that only counts an effect down.
#[derive(Debug, Clone, Copy)]
enum ResidualHandler {
    Instance(ResidualInstance),
    Duration(super::durations::DurationInstance),
}

impl ResidualHandler {
    /// `(order, speed, sub-order, slot)`.
    fn key(&self) -> (u8, u32, u8, usize) {
        match self {
            Self::Instance(instance) => (
                instance.order,
                instance.speed,
                instance.sub_order,
                instance.slot,
            ),
            Self::Duration(duration) => (
                duration.order,
                duration.speed,
                duration.sub_order,
                usize::MAX,
            ),
        }
    }
}

/// Showdown `speedSort` on the residual handlers: the ones equal in order, speed and sub-order
/// (the field's weather ties nothing) are shuffled — the duration-only ones too, among
/// themselves and with the others (two sides' Safeguards, a Pokémon's Protect counter and its
/// Endure). Returns (index, tie group) in collection order — the instances, then the durations
/// past them — the order their `SpeedTie` decisions are drawn.
fn residual_tie_groups(
    instances: &[ResidualInstance],
    durations: &[super::durations::DurationInstance],
) -> Vec<(usize, usize)> {
    let keys: Vec<Option<(u8, u32, u8)>> = instances
        .iter()
        .map(|instance| {
            instance
                .holder
                .map(|_| (instance.order, instance.speed, instance.sub_order))
        })
        .chain(
            durations
                .iter()
                .map(|duration| Some((duration.order, duration.speed, duration.sub_order))),
        )
        .collect();
    let mut groups: Vec<(u8, u32, u8)> = Vec::new();
    let mut tied = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let Some(key) = key else {
            continue;
        };
        if !keys
            .iter()
            .enumerate()
            .any(|(other, k)| other != index && k.as_ref() == Some(key))
        {
            continue;
        }
        let group = groups.iter().position(|g| g == key).unwrap_or_else(|| {
            groups.push(*key);
            groups.len() - 1
        });
        tied.push((index, group));
    }
    tied
}

/// Indices of equal values, each with its group (first-seen order).
fn tie_groups(speeds: &[u32]) -> Vec<(usize, usize)> {
    let mut groups: Vec<u32> = Vec::new();
    let mut tied = Vec::new();
    for (index, speed) in speeds.iter().enumerate() {
        if speeds.iter().filter(|s| *s == speed).count() < 2 {
            continue;
        }
        let group = groups.iter().position(|g| g == speed).unwrap_or_else(|| {
            groups.push(*speed);
            groups.len() - 1
        });
        tied.push((index, group));
    }
    tied
}

/// How a residual handler ran (Showdown `fieldEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResidualRun {
    /// Its callback ran; the faints it caused are settled right after it.
    Ran,
    /// Its count ran out and its `end` ran instead: the faints it caused wait for the next
    /// handler's settling (or the residual's end).
    Ended,
}
