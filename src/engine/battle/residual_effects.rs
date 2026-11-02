//! Each condition's own end-of-turn handler, run by the residual scheduler (`residual.rs`):
//! Wish, Perish Song, Nightmare, Curse, Salt Cure, Syrup Bomb, Yawn, the weather, a partial
//! trap, Leech Seed, Gen 2's thaw, the major statuses and Gen 1-2's after-own-action residual.

use super::BattleEngine;
use super::residual::ResidualRun;
use crate::engine::{BattleContext, DamageApplication, EffectSource, PendingFaints, TraceEvent};
use crate::ids::{PokemonId, SideId};
use crate::rules::{StatusResidualTiming, ToxicModel};
use crate::state::{NonVolatileStatus, RandomRequestKind, VolatileConditionKind, WeatherKind};

impl BattleEngine {
    /// Every Wish whose slot holds no standing Pokémon: one residual off, gone at the last.
    pub(super) fn tick_unattended_wishes(&mut self) {
        for side in [SideId::P1, SideId::P2] {
            let unattended: Vec<usize> = self
                .state
                .side(side)
                .wishes
                .iter()
                .enumerate()
                .filter(|(_, wish)| {
                    self.state
                        .side(side)
                        .active
                        .get(wish.slot)
                        .copied()
                        .flatten()
                        .and_then(|id| self.state.pokemon(id))
                        .is_none_or(|pokemon| pokemon.is_fainted())
                })
                .map(|(index, _)| index)
                .collect();
            let wishes = &mut self.state.side_mut(side).wishes;
            for index in unattended.into_iter().rev() {
                wishes[index].turns_remaining = wishes[index].turns_remaining.saturating_sub(1);
                if wishes[index].turns_remaining == 0 {
                    wishes.remove(index);
                }
            }
        }
    }

    /// The side and index of the Wish pending on `holder`'s slot, if any.
    pub(super) fn wish_slot(&self, holder: PokemonId) -> Option<(SideId, usize)> {
        let position = self.state.active_position(holder, &self.rules.topology)?;
        let spec = self.rules.topology.spec(position)?;
        let slot = usize::from(spec.slot);
        let index = self
            .state
            .side(spec.side)
            .wishes
            .iter()
            .position(|wish| wish.slot == slot)?;
        Some((spec.side, index))
    }

    /// Wish's residual (Showdown `onResidual`, Gen 3-4 its `duration`): counted down the turn it
    /// was made; the next turn it ends and heals whoever stands in the slot — half the wisher's
    /// max HP taken as it wished (Gen 5+) or half the recipient's (Gen 3-4:
    /// `FieldRules::wish_heals_wisher_share`), `[from] move: Wish`.
    pub(super) fn run_wish(&mut self, holder: PokemonId) {
        let Some((side, index)) = self.wish_slot(holder) else {
            return;
        };
        let wishes = &mut self.state.side_mut(side).wishes;
        wishes[index].turns_remaining = wishes[index].turns_remaining.saturating_sub(1);
        if wishes[index].turns_remaining > 0 {
            return;
        }
        let wish = wishes.remove(index);
        let Some(max_hp) = self
            .state
            .pokemon(holder)
            .filter(|pokemon| !pokemon.is_fainted())
            .map(|pokemon| pokemon.max_hp)
        else {
            return;
        };
        let amount = if self.rules.conditions.field.wish_heals_wisher_share {
            wish.amount
        } else {
            max_hp / 2
        };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_heal(crate::engine::HealRequest {
            target: holder,
            amount,
            source: EffectSource::Move {
                pokemon: wish.source,
                move_id: crate::ids::MoveId::WISH,
            },
            direct: false,
        });
    }

    /// Perish Song (Showdown: the count ticks, and at 0 `onEnd` faints the holder).
    pub(super) fn run_perish_song(&mut self, holder: PokemonId) -> ResidualRun {
        let left = self
            .state
            .pokemon_mut(holder)
            .filter(|p| !p.is_fainted())
            .and_then(|p| p.volatiles.tick_perish_song());
        if left != Some(0) {
            return ResidualRun::Ran;
        }
        self.trace.push(TraceEvent::VolatileEnded {
            target: holder,
            kind: VolatileConditionKind::PerishSong,
            seen: true,
        });
        BattleContext::new(&mut self.state, &self.rules, &mut self.trace).faint_directly(
            holder,
            EffectSource::Volatile(VolatileConditionKind::PerishSong),
        );
        ResidualRun::Ended
    }

    /// Nightmare (Showdown `onResidual`, Gen 2 `onAfterMoveSelf` while asleep): the holder
    /// loses a quarter of its max HP.
    pub(super) fn run_nightmare(&mut self, holder: PokemonId) {
        let Some(max_hp) = self.state.pokemon(holder).map(|p| p.base_max_hp()) else {
            return;
        };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount((max_hp / 4).max(1)),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Volatile(VolatileConditionKind::Nightmare),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// Curse (Showdown `onResidual`, Gen 2 `onAfterMoveSelf`): the holder loses a quarter of its
    /// max HP (`baseMaxhp`).
    pub(super) fn run_curse(&mut self, holder: PokemonId) {
        let Some(max_hp) = self.state.pokemon(holder).map(|p| p.base_max_hp()) else {
            return;
        };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount((max_hp / 4).max(1)),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Volatile(VolatileConditionKind::Curse),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// Salt Cure (Showdown `onResidual`): the holder loses an eighth of its max HP, a quarter
    /// for a Water or Steel type.
    pub(super) fn run_salt_cure(&mut self, holder: PokemonId) {
        let Some((max_hp, soaked)) = self.state.pokemon(holder).map(|p| {
            (
                p.base_max_hp(),
                crate::engine::types_of(&self.state, &self.rules, p)
                    .contains(crate::dex::PokemonType::Water)
                    || crate::engine::types_of(&self.state, &self.rules, p)
                        .contains(crate::dex::PokemonType::Steel),
            )
        }) else {
            return;
        };
        let divisor = if soaked { 4 } else { 8 };
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount((max_hp / divisor).max(1)),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Volatile(VolatileConditionKind::SaltCure),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// Syrup Bomb (Showdown `duration: 4`, `onResidual`): the countdown ticks first (ending on
    /// the fourth residual), then the holder's Speed drops by one — a drop the thrower causes,
    /// which Clear Body refuses and Mirror Armor sends back.
    pub(super) fn run_syrup_bomb(&mut self, holder: PokemonId) {
        let Some(source) = self
            .state
            .pokemon(holder)
            .and_then(|p| p.volatiles.syrup_bomb_source())
        else {
            return;
        };
        let left = self.state.pokemon_mut(holder).and_then(|pokemon| {
            pokemon
                .volatiles
                .tick_timed(VolatileConditionKind::SyrupBomb)
        });
        if left == Some(0) {
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind: VolatileConditionKind::SyrupBomb,
                seen: true,
            });
            return;
        }
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
            target: holder,
            stat: crate::state::Stat::Speed,
            stages: -1,
            source: EffectSource::Move {
                pokemon: source,
                move_id: crate::ids::MoveId::SYRUP_BOMB,
            },
            cause: crate::engine::StatChangeCause::Move,
        });
    }

    /// Yawn (Showdown `duration: 2`, `onEnd`): the countdown ticks, and when it runs out the
    /// holder falls asleep — through the usual status checks, but Safeguard lets Yawn's own
    /// sleep through (`effect.id === 'yawn'`).
    pub(super) fn run_yawn(&mut self, holder: PokemonId) {
        let source = self
            .state
            .pokemon(holder)
            .and_then(|pokemon| pokemon.volatiles.yawn_source());
        let left = self
            .state
            .pokemon_mut(holder)
            .and_then(|pokemon| pokemon.volatiles.tick_timed(VolatileConditionKind::Yawn));
        if left != Some(0) {
            return;
        }
        self.trace.push(TraceEvent::VolatileEnded {
            target: holder,
            kind: VolatileConditionKind::Yawn,
            seen: true,
        });
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.try_set_status_inflicted(
            holder,
            NonVolatileStatus::Sleep,
            EffectSource::Volatile(VolatileConditionKind::Yawn),
            source,
        );
    }

    /// The weather's residual (Showdown `onFieldResidual`): the duration ticks first (a weather
    /// set for 5 turns chips 4 times and ends on the fifth upkeep), then sandstorm / hail hit
    /// every Pokémon on the field, fastest first, unless a type or being underground shields
    /// it. The chip is a `DamageRequest` (Magic Guard refuses it like any non-move damage).
    pub(super) fn run_weather_residual(&mut self) {
        if let Some(ended) = self.state.field.tick_weather() {
            self.trace.push(TraceEvent::WeatherChanged {
                from: Some(ended),
                to: None,
            });
            // Showdown `clearWeather` → `eachEvent('WeatherChange')` (Protosynthesis).
            BattleContext::new(&mut self.state, &self.rules, &mut self.trace).weather_changed();
            return;
        }
        let Some(weather) = self.state.field.weather else {
            return;
        };
        self.trace.push(TraceEvent::WeatherUpkeep {
            weather: weather.kind,
            turns_remaining: weather.turns_remaining,
        });
        // Showdown's upkeep runs `eachEvent('Weather')` — each active Pokémon's `onWeather`
        // (Dry Skin, Rain Dish, Ice Body, Solar Power …), then the weather's own chip — whose
        // handlers Air Lock / Cloud Nine suppress, every one (`runEvent` skips the `Weather`
        // event while `suppressingWeather`; sandstorm, hail and snow skip the event itself).
        // The `Update` that follows it from Gen 7 then has nothing to act on either: the
        // weather is the residual's first step.
        if crate::engine::weather::suppressed(&self.state, &self.rules) {
            return;
        }
        self.run_weather_event(weather);
        // Showdown `eachEvent`: from Gen 7 an `Update` follows `eachEvent('Weather')`; the
        // chip's faints are still pending.
        if self.rules.update.after_weather {
            self.run_update(PendingFaints::Pending);
        }
    }

    /// Showdown `eachEvent('Weather')`, the weather in effect.
    fn run_weather_event(&mut self, weather: crate::state::WeatherState) {
        let rules = self.rules.weather;
        let immune_types: Option<&[crate::dex::PokemonType]> = match weather.kind {
            WeatherKind::Sandstorm => Some(rules.sandstorm_immune_types),
            WeatherKind::Snow if rules.hail_damages => Some(rules.hail_immune_types),
            WeatherKind::Sun | WeatherKind::Rain | WeatherKind::Snow | WeatherKind::StrongWinds => {
                None
            }
        };
        // Showdown `eachEvent('Weather')`: every active Pokémon fastest first, ties shuffled (a
        // `SpeedTie` decision each when the weather chips; otherwise only the order of the
        // messages would show it).
        let mut targets: Vec<(PokemonId, u32, u64)> = self
            .active_pokemon()
            .into_iter()
            .map(|id| (id, self.residual_speed(id), 0))
            .collect();
        if immune_types.is_some() {
            for (index, _) in self.weather_tie_groups() {
                targets[index].2 = self.state.rng.next_u64_for(RandomRequestKind::SpeedTie);
            }
        }
        targets.sort_by_key(|&(_, speed, tie_break)| {
            (std::cmp::Reverse(speed), std::cmp::Reverse(tie_break))
        });
        for (target, ..) in targets {
            if self.state.pokemon(target).is_none_or(|p| p.is_fainted()) {
                continue;
            }
            // The ability's `onWeather` first: the weather's own handler has no speed.
            self.run_weather_reactions(target);
            let Some(immune_types) = immune_types else {
                continue;
            };
            let Some(pokemon) = self.state.pokemon(target) else {
                continue;
            };
            if pokemon.is_fainted() {
                continue;
            }
            let ability = crate::engine::ability_handlers(&self.state, &self.rules, pokemon);
            let weather_proof = ability.weather_and_powder_immune
                || ability.weather_immunity == Some(weather.kind)
                || crate::engine::held_item(&self.state, &self.rules, target)
                    .is_some_and(|(_, item)| item.weather_and_powder_immune);
            let shielded = immune_types
                .iter()
                .any(|t| crate::engine::types_of(&self.state, &self.rules, pokemon).contains(*t))
                || weather_proof
                || crate::engine::two_turn::hiding(pokemon)
                    .is_some_and(|kind| kind.sheltered_from_weather());
            if shielded {
                self.trace.push(TraceEvent::WeatherImmune {
                    target,
                    weather: weather.kind,
                });
                continue;
            }
            // Showdown's sandstorm / hail chip is `pokemon.baseMaxhp / 16`: under a Dynamax it
            // reads the HP before the doubling (DESIGN.md §44.5).
            let damage = (pokemon.base_max_hp() / rules.damage_divisor.max(1)).max(1);
            self.trace.push(TraceEvent::WeatherDamage {
                target,
                weather: weather.kind,
                damage,
            });
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            ctx.apply_damage(crate::engine::DamageRequest {
                target,
                amount: crate::engine::DamageAmount(damage),
                kind: crate::engine::DamageKind::Weather,
                source: EffectSource::Weather(weather.kind),
                flags: crate::engine::DamageFlags::default(),
            });
        }
    }

    /// The holder's ability's `onWeather` (`AbilityBehavior::weather_reactions`) in the
    /// weather's upkeep: a heal, a loss or a cure, as the holder sees the weather.
    fn run_weather_reactions(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon(holder) else {
            return;
        };
        let (ability, max_hp) = (crate::engine::raw_ability(pokemon), pokemon.base_max_hp());
        let reactions = crate::engine::ability_handler(&self.state, &self.rules, pokemon, |b| {
            b.weather_reactions
        });
        for reaction in reactions {
            if crate::engine::weather_for(&self.state, &self.rules, holder)
                != Some(reaction.weather)
            {
                continue;
            }
            let source = EffectSource::Ability {
                pokemon: holder,
                ability,
            };
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            match reaction.effect {
                crate::effects::abilities::WeatherEffect::Heal(divisor) => {
                    ctx.apply_heal(crate::engine::HealRequest {
                        target: holder,
                        amount: (max_hp / divisor.max(1)).max(1),
                        source,
                        direct: false,
                    });
                }
                crate::effects::abilities::WeatherEffect::Damage(divisor) => {
                    ctx.apply_damage(crate::engine::DamageRequest {
                        target: holder,
                        amount: crate::engine::DamageAmount((max_hp / divisor.max(1)).max(1)),
                        kind: crate::engine::DamageKind::Residual,
                        source,
                        flags: crate::engine::DamageFlags::default(),
                    });
                }
                crate::effects::abilities::WeatherEffect::CureStatus => {
                    if ctx.has_status(holder) {
                        ctx.ability_activated(holder, ability);
                        ctx.cure_status(holder, crate::engine::CureReason::Ability(ability));
                    }
                }
            }
        }
    }

    /// Bind / Wrap / Fire Spin … (Showdown `partiallytrapped`): the duration ticks first (the
    /// last turn ends it without a chip); then, if the trapper has left the field, fainted or
    /// just came back (`activeTurns` 0), the bind ends silently; otherwise the chip
    /// (`ConditionRules::bind_damage_divisor`) is a residual `DamageRequest`.
    pub(super) fn run_partially_trapped(&mut self, holder: PokemonId) {
        let left = self.state.pokemon_mut(holder).and_then(|p| {
            p.volatiles
                .tick_timed(VolatileConditionKind::PartiallyTrapped)
        });
        if left == Some(0) {
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind: VolatileConditionKind::PartiallyTrapped,
                seen: true,
            });
            return;
        }
        let Some((source, _)) = self
            .state
            .pokemon(holder)
            .and_then(|p| p.volatiles.partially_trapped_by())
        else {
            return;
        };
        let trapper_gone = self
            .state
            .active_position(source, &self.rules.topology)
            .is_none()
            || self
                .state
                .pokemon(source)
                .is_none_or(|p| p.is_fainted() || p.active_turns == 0);
        if trapper_gone {
            if let Some(pokemon) = self.state.pokemon_mut(holder) {
                pokemon
                    .volatiles
                    .remove(VolatileConditionKind::PartiallyTrapped);
            }
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind: VolatileConditionKind::PartiallyTrapped,
                seen: true,
            });
            return;
        }
        let Some(max_hp) = self.state.pokemon(holder).map(|p| p.base_max_hp()) else {
            return;
        };
        let banded = self
            .state
            .pokemon(holder)
            .is_some_and(|p| p.volatiles.partially_trapped_banded());
        let divisor = if banded {
            self.rules.conditions.binding_band_damage_divisor
        } else {
            self.rules.conditions.bind_damage_divisor
        };
        let amount = (max_hp / divisor.max(1)).max(1);
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount(amount),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Volatile(VolatileConditionKind::PartiallyTrapped),
            flags: crate::engine::DamageFlags::default(),
        });
    }

    /// One residual tick of a timed volatile (Taunt / Encore / Disable / Heal Block): Showdown's
    /// `duration--`, ending the condition at 0.
    pub(super) fn tick_volatile(&mut self, holder: PokemonId, kind: VolatileConditionKind) {
        let left = self
            .state
            .pokemon_mut(holder)
            .and_then(|pokemon| pokemon.volatiles.tick_timed(kind));
        if left == Some(0) {
            self.trace.push(TraceEvent::VolatileEnded {
                target: holder,
                kind,
                seen: true,
            });
        }
    }

    /// Leech Seed (Showdown `leechseed` `onResidual`, Gen 2 `onAfterMoveSelf`): nothing
    /// happens unless someone stands at the seeder's position; then the holder loses
    /// `max HP / 8` and that Pokémon heals what was taken.
    pub(super) fn run_leech_seed(&mut self, holder: PokemonId) {
        let Some((max_hp, source_position)) = self.state.pokemon(holder).and_then(|pokemon| {
            pokemon
                .volatiles
                .leech_seed_source_position()
                .map(|position| (pokemon.base_max_hp(), position))
        }) else {
            return;
        };
        let receiver = self
            .state
            .pokemon_at_position(source_position, &self.rules.topology)
            .filter(|receiver| {
                self.state
                    .pokemon(*receiver)
                    .is_some_and(|pokemon| !pokemon.is_fainted())
            });
        let Some(receiver) = receiver else {
            return;
        };
        let mut amount = (max_hp / self.rules.conditions.leech_seed_hp_divisor.max(1)).max(1);
        // Gen 1: the `residualdmg` counter advances and multiplies the drain.
        let gen1 = self.rules.conditions.leech_seed_gen1;
        if gen1
            && let Some(pokemon) = self.state.pokemon_mut(holder)
            && pokemon.volatiles.residual_damage_counter().is_some()
        {
            pokemon.volatiles.advance_residual_damage();
            let counter = pokemon.volatiles.residual_damage_counter().unwrap_or(1);
            amount = amount.saturating_mul(u16::from(counter));
        }
        let holder_down = self.state.pokemon(holder).is_none_or(|p| p.is_fainted());
        let drained = if holder_down {
            0
        } else {
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            match ctx.apply_damage(crate::engine::DamageRequest {
                target: holder,
                amount: crate::engine::DamageAmount(amount),
                kind: crate::engine::DamageKind::Residual,
                source: EffectSource::Volatile(VolatileConditionKind::LeechSeed),
                flags: crate::engine::DamageFlags::default(),
            }) {
                DamageApplication::Hp { amount, .. } => amount.0,
                _ => 0,
            }
        };
        // Gen 1 heals the full amount, however little HP the seeded Pokémon had left.
        let healed = if gen1 { amount } else { drained };
        if healed == 0 {
            return;
        }
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.drain_heal(
            crate::engine::HealRequest {
                target: receiver,
                amount: healed,
                source: EffectSource::Volatile(VolatileConditionKind::LeechSeed),
                direct: false,
            },
            holder,
            crate::engine::HealDrain::LeechSeed,
        );
    }

    /// Gen 2's thaw (`frz` `onResidualOrder` 7): a frozen Pokémon thaws with the generation's
    /// chance at the end of the turn.
    pub(super) fn run_freeze_thaw(&mut self, holder: PokemonId) {
        if !self
            .state
            .pokemon(holder)
            .is_some_and(|p| p.has_status(NonVolatileStatus::Freeze))
        {
            return;
        }
        let chance = self.rules.status.lifecycle.thaw_chance;
        if self.state.rng.roll_ratio_for(
            RandomRequestKind::Thaw,
            chance.numerator,
            chance.denominator,
        ) {
            let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
            ctx.cure_status(holder, crate::engine::CureReason::Thawed);
        }
    }

    /// Gen 1-2 (`StatusResidualTiming::AfterOwnAction`, Showdown `onAfterMoveSelf`): after the
    /// Pokémon's own action — also one stopped at BeforeMove (`stopped`), but not a move after
    /// which the foe in front of it has fainted — the `residualdmg` counter of a statused
    /// holder advances, then poison / burn / bad poison take their HP, then Leech Seed, then
    /// Gen 2's Nightmare. The actor's own HP does not matter (its confusion, recoil or
    /// Explosion may have left it at 0): damage to it then does nothing, but Gen 1's Leech Seed
    /// still heals.
    pub(super) fn run_after_own_action(
        &mut self,
        actor: PokemonId,
        stopped: bool,
        aimed_at_self: bool,
    ) {
        if self.rules.status.residual_timing != StatusResidualTiming::AfterOwnAction {
            return;
        }
        let Some((side, down)) = self.state.pokemon(actor).map(|p| (p.side, p.is_fainted())) else {
            return;
        };
        // Gen 1: a move aimed at its user asks the user itself to stand
        // (`ConditionRules::after_own_action_reads_the_target`).
        if !stopped
            && aimed_at_self
            && down
            && self.rules.conditions.after_own_action_reads_the_target
        {
            return;
        }
        let foe_standing = self.state.side(side.other()).active[0]
            .and_then(|foe| self.state.pokemon(foe))
            .is_some_and(|foe| !foe.is_fainted());
        if !stopped && !foe_standing {
            return;
        }
        self.advance_residual_damage(actor);
        if !down {
            self.run_status_residual(actor);
        }
        // Gen 1's Leech Seed still runs on a holder at 0 HP (Showdown gen1 `onAfterMoveSelf`
        // has no HP check; Gen 2's returns at 0 HP): the seeder heals in full.
        if self.state.pokemon(actor).is_some_and(|p| {
            (!p.is_fainted() || self.rules.conditions.leech_seed_gen1)
                && p.volatiles.contains(VolatileConditionKind::LeechSeed)
        }) {
            self.run_leech_seed(actor);
        }
        // Gen 2 Nightmare (`onAfterMoveSelfPriority` 1, after Leech Seed's 2); it ends when
        // the holder wakes, so a holder that still has it is asleep.
        if self.state.pokemon(actor).is_some_and(|p| {
            !p.is_fainted() && p.volatiles.contains(VolatileConditionKind::Nightmare)
        }) {
            self.run_nightmare(actor);
        }
        // Gen 2 Curse (`onAfterMoveSelfPriority` 0: last; pokecrystal `ResidualDamage` too).
        if self
            .state
            .pokemon(actor)
            .is_some_and(|p| !p.is_fainted() && p.volatiles.contains(VolatileConditionKind::Curse))
        {
            self.run_curse(actor);
        }
    }

    /// Gen 2 `residualdmg` `onAfterMoveSelf` / `onAfterSwitchInSelf`: a burned, poisoned or
    /// badly poisoned holder's counter advances.
    pub(super) fn advance_residual_damage(&mut self, pokemon_id: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
            && pokemon.status.is_some_and(|status| {
                matches!(
                    status.kind,
                    NonVolatileStatus::Burn | NonVolatileStatus::Poison | NonVolatileStatus::Toxic
                )
            })
        {
            pokemon.volatiles.advance_residual_damage();
        }
    }

    pub(super) fn run_status_residual(&mut self, pokemon_id: PokemonId) {
        let Some((status_state, max_hp)) = self
            .state
            .pokemon(pokemon_id)
            .and_then(|pokemon| pokemon.status.map(|status| (status, pokemon.base_max_hp())))
        else {
            return;
        };
        let status = status_state.kind;
        // Gen 2: the `residualdmg` counter, if any, scales every status's damage.
        let residual_counter = self
            .state
            .pokemon(pokemon_id)
            .and_then(|pokemon| pokemon.volatiles.residual_damage_counter())
            .filter(|_| self.rules.status.toxic_model == ToxicModel::ResidualVolatile);
        let damage = match (status, residual_counter) {
            (
                NonVolatileStatus::Burn | NonVolatileStatus::Poison | NonVolatileStatus::Toxic,
                Some(counter),
            ) => {
                let sixteenth = max_hp / self.rules.status.toxic_residual_divisor.max(1);
                let damage = if status == NonVolatileStatus::Toxic {
                    u32::from(sixteenth.max(1)) * u32::from(counter)
                } else {
                    (u32::from(sixteenth) * u32::from(counter)).max(1)
                };
                damage.min(u32::from(u16::MAX)) as u16
            }
            (NonVolatileStatus::Burn, _) => {
                let damage = (max_hp / self.rules.status.burn_residual_divisor.max(1)).max(1);
                // Heatproof (`onDamage`): half of it, still at least 1.
                let halved = self.state.pokemon(pokemon_id).is_some_and(|p| {
                    crate::engine::ability_handler(&self.state, &self.rules, p, |b| {
                        b.halves_status_damage
                    }) == Some(NonVolatileStatus::Burn)
                });
                if halved { (damage / 2).max(1) } else { damage }
            }
            (NonVolatileStatus::Poison, _) => {
                (max_hp / self.rules.status.poison_residual_divisor.max(1)).max(1)
            }
            (NonVolatileStatus::Toxic, _) => {
                // Showdown: `stage++` (capped at 15), then `clampIntRange(maxhp / 16, 1) *
                // stage` — the sixteenth is truncated first, then multiplied.
                let stage = status_state.toxic_counter.saturating_add(1).min(15);
                if let Some(pokemon) = self.state.pokemon_mut(pokemon_id)
                    && let Some(status) = &mut pokemon.status
                {
                    status.toxic_counter = stage;
                }
                let divisor = self.rules.status.toxic_residual_divisor.max(1);
                let sixteenth = u32::from((max_hp / divisor).max(1));
                sixteenth
                    .saturating_mul(u32::from(stage))
                    .min(u32::from(u16::MAX)) as u16
            }
            _ => return,
        };
        self.trace.push(TraceEvent::StatusResidual {
            target: pokemon_id,
            status,
            damage,
        });
        let mut ctx = BattleContext::new(&mut self.state, &self.rules, &mut self.trace);
        ctx.apply_damage(crate::engine::DamageRequest {
            target: pokemon_id,
            amount: crate::engine::DamageAmount(damage),
            kind: crate::engine::DamageKind::Residual,
            source: EffectSource::Status(status),
            flags: crate::engine::DamageFlags::default(),
        });
    }
}
