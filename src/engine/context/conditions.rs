//! `BattleContext`'s condition mutations: volatile conditions (Protect, Substitute, flinch,
//! confusion, Leech Seed), side conditions (entry hazards, Tailwind) and field conditions
//! (Trick Room, weather, terrain). Split out of `context/mod.rs`; every method here is still
//! `BattleContext`'s and traces what it changes.

use crate::dex::PokemonType;
use crate::ids::{AbilityId, ItemId, MoveId, PokemonId, SideId};
use crate::state::{
    FieldConditionKind, RandomRequestKind, SideConditionKind, TerrainKind, VolatileConditionKind,
    WeatherKind,
};

use super::BattleContext;
use crate::engine::{ConditionApplyError, EffectSource, TraceEvent};

/// Yawn's `duration`: the sleep comes at the end of the turn after the one it landed.
const YAWN_TURNS: u8 = 2;

/// Syrup Bomb's `duration`: the countdown ends on the fourth residual, after three drops.
const SYRUP_BOMB_TURNS: u8 = 4;

/// Throat Chop's `duration`: the rest of the turn it landed and the next one.
const THROAT_CHOP_TURNS: u8 = 2;

impl<'a> BattleContext<'a> {
    pub fn set_protect(
        &mut self,
        target: PokemonId,
        source: PokemonId,
        kind: crate::state::ProtectKind,
    ) -> Result<(), ConditionApplyError> {
        let turn = self.state.turn;
        let pokemon = self
            .state
            .pokemon_mut(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        pokemon.volatiles.set_protect(source, turn, kind);
        self.trace.push(TraceEvent::VolatileStarted {
            target,
            kind: VolatileConditionKind::Protect,
        });
        Ok(())
    }

    pub fn create_substitute(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        let divisor = self.rules.conditions.substitute_hp_divisor.max(1);
        let cost = {
            let pokemon = self
                .state
                .pokemon_mut(target)
                .ok_or(ConditionApplyError::InvalidTarget)?;
            if pokemon
                .volatiles
                .contains(VolatileConditionKind::Substitute)
            {
                return Err(ConditionApplyError::AlreadyActive);
            }
            let cost = (pokemon.max_hp / divisor).max(1);
            let gen1 = self.rules.conditions.substitute_gen1;
            // Gen 1 fails only below a quarter (`hp < maxhp / 4`), later at or below it.
            let too_weak = if gen1 {
                u32::from(pokemon.hp) * u32::from(divisor) < u32::from(pokemon.max_hp)
            } else {
                pokemon.hp <= cost
            };
            if too_weak {
                return Err(ConditionApplyError::NotEnoughHp);
            }
            pokemon
                .volatiles
                .set_substitute(source, if gen1 { cost + 1 } else { cost });
            cost
        };
        self.substitute_started(target);
        // The price (`onHit`'s `directDamage`), once the Substitute is up.
        self.apply_damage(crate::engine::DamageRequest::cost(
            target,
            cost,
            EffectSource::Move {
                pokemon: source,
                move_id: MoveId::SUBSTITUTE,
            },
        ));
        Ok(())
    }

    /// Shed Tail (`onTryHit`, `volatileStatus: 'substitute'`, `onHit`): with someone to come in,
    /// no Substitute up and more than half its max HP (rounded up) left, the user puts up a
    /// Substitute of a quarter of its max HP, then pays half of it (rounded up). The switch is
    /// the move's `selfSwitch` (DESIGN.md §30.7).
    pub fn shed_tail(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        let side = self
            .side_of(user)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if !self.can_switch(side) {
            return Err(ConditionApplyError::NothingToDo);
        }
        let divisor = self.rules.conditions.substitute_hp_divisor.max(1);
        let pokemon = self
            .state
            .pokemon_mut(user)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if pokemon
            .volatiles
            .contains(VolatileConditionKind::Substitute)
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        let cost = pokemon.max_hp.div_ceil(2);
        if pokemon.hp <= cost {
            return Err(ConditionApplyError::NotEnoughHp);
        }
        pokemon
            .volatiles
            .set_substitute(user, pokemon.max_hp / divisor);
        self.substitute_started(user);
        self.apply_damage(crate::engine::DamageRequest::cost(
            user,
            cost,
            EffectSource::Move {
                pokemon: user,
                move_id: MoveId::SHED_TAIL,
            },
        ));
        Ok(())
    }

    /// A Substitute went up. Showdown `substitute`'s `onStart` (every generation): a bind on its
    /// holder ends.
    fn substitute_started(&mut self, target: PokemonId) {
        self.trace.push(TraceEvent::VolatileStarted {
            target,
            kind: VolatileConditionKind::Substitute,
        });
        if self.state.pokemon_mut(target).is_some_and(|pokemon| {
            pokemon
                .volatiles
                .remove(VolatileConditionKind::PartiallyTrapped)
        }) {
            self.trace.push(TraceEvent::VolatileEnded {
                target,
                kind: VolatileConditionKind::PartiallyTrapped,
                seen: true,
            });
        }
    }

    pub fn set_flinch(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        // A Dynamaxed Pokémon cannot flinch (`dynamax.onTryAddVolatile`, DESIGN.md §44.5).
        if self
            .state
            .pokemon(target)
            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Dynamax))
        {
            return Err(ConditionApplyError::Immune);
        }
        if self.ability_blocks_volatile(target, VolatileConditionKind::Flinch) {
            return Err(ConditionApplyError::Immune);
        }
        // Focus Punch's volatile (`onTryAddVolatile`: `null` for a flinch).
        if self
            .state
            .pokemon(target)
            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::FocusPunch))
        {
            return Err(ConditionApplyError::Immune);
        }
        let pokemon = self
            .state
            .pokemon_mut(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        pokemon.volatiles.set_flinch(source);
        let recharge_cleared = self.rules.conditions.flinch_clears_recharge
            && pokemon
                .volatiles
                .remove(VolatileConditionKind::MustRecharge);
        self.trace.push(TraceEvent::VolatileStarted {
            target,
            kind: VolatileConditionKind::Flinch,
        });
        if recharge_cleared {
            self.trace.push(TraceEvent::VolatileEnded {
                target,
                kind: VolatileConditionKind::MustRecharge,
                seen: true,
            });
        }
        Ok(())
    }

    /// Showdown `TryAddVolatile`: an ability refuses the volatile — the target's own (Inner
    /// Focus: flinch; Purifying Salt, Insomnia, Vital Spirit: Yawn; Own Tempo: confusion; Leaf
    /// Guard: Yawn in sun; and, a step earlier in `runStatusImmunity`, Oblivious: infatuation)
    /// or, through `onAllyTryAddVolatile`, one on its side, its own included (Sweet Veil, Flower
    /// Veil, Aroma Veil).
    pub(super) fn ability_blocks_volatile(
        &self,
        target: PokemonId,
        kind: VolatileConditionKind,
    ) -> bool {
        self.volatile_refuser(target, kind).is_some()
    }

    /// `ability_blocks_volatile`'s refusal with the Pokémon whose ability it is (an ally's for
    /// `onAllyTryAddVolatile`), whether its line names it — an `onImmunity` (Oblivious's
    /// infatuation) does not: `runStatusImmunity`'s `-immune|target` — and whether it is such an
    /// ally guard (the target's own Aroma Veil among them).
    fn volatile_refuser(
        &self,
        target: PokemonId,
        kind: VolatileConditionKind,
    ) -> Option<(PokemonId, AbilityId, bool, bool)> {
        use crate::effects::abilities::{VolatileGuard, VolatileGuardWhen};
        let pokemon = self.state.pokemon(target)?;
        let own = crate::engine::ability_handlers(self.state, self.rules, pokemon);
        let covers = |guard: &VolatileGuard| {
            guard.kinds.contains(&kind)
                && match guard.when {
                    VolatileGuardWhen::Always => true,
                    VolatileGuardWhen::Sun => {
                        crate::engine::weather_for(self.state, self.rules, target)
                            == Some(crate::state::WeatherKind::Sun)
                    }
                    VolatileGuardWhen::GrassType => {
                        crate::engine::types_of(self.state, self.rules, pokemon)
                            .contains(crate::dex::PokemonType::Grass)
                    }
                }
        };
        let refused = |named| Some((target, crate::engine::raw_ability(pokemon), named, false));
        if own.blocks_volatiles.contains(&kind)
            || own.in_forme_blocks.is_some_and(|guard| {
                guard.volatiles.contains(&kind) && guard.holds(crate::engine::species_of(pokemon))
            })
            || own.volatile_guard.is_some_and(|g| !g.allies && covers(&g))
        {
            return refused(true);
        }
        if own.status_immunity == Some(crate::effects::abilities::StatusImmunity::Volatile(kind)) {
            return refused(false);
        }
        // `runEvent`'s `speedSort`: of two guards on the side (an Aroma Veil Entrainment gave the
        // target beside its partner's own), the faster holder's refuses and is named (R-M10-53).
        crate::engine::allied_ability_holders(self.state, self.rules, target)
            .into_iter()
            .filter(|(_, b)| b.volatile_guard.is_some_and(|g| g.allies && covers(&g)))
            .min_by_key(|(holder, _)| {
                std::cmp::Reverse(crate::engine::stats::handler_speed(self.state, *holder))
            })
            .and_then(|(holder, _)| {
                self.state
                    .pokemon(holder)
                    .map(|ally| (holder, crate::engine::raw_ability(ally), true, true))
            })
    }

    /// A status move's volatile an ability refused (`ConditionApplyError::Immune`): the refusal's
    /// line names it — `-immune|target|[from] ability: Insomnia` for Yawn, `-block|target|ability:
    /// Aroma Veil|[of] holder` for a move's Taunt, Encore, Disable, Heal Block, Attract, Torment
    /// (R-M10-40).
    pub fn announce_volatile_refusal(&mut self, target: PokemonId, kind: VolatileConditionKind) {
        if let Some((holder, ability, true, _)) = self.volatile_refuser(target, kind) {
            self.ability_activated(holder, ability);
        }
    }

    /// A move's secondary volatile an ability refused (Psychic Noise's Heal Block): an
    /// `onAllyTryAddVolatile` guard names itself for a move's effect as for a status move's
    /// (Aroma Veil's `-block|target|ability: Aroma Veil|[of] holder`, R-M10-59); the target's own
    /// refusal of a secondary (Inner Focus's flinch, Own Tempo's confusion) says nothing.
    pub fn announce_secondary_volatile_refusal(
        &mut self,
        target: PokemonId,
        kind: VolatileConditionKind,
    ) {
        if let Some((holder, ability, true, true)) = self.volatile_refuser(target, kind) {
            self.ability_activated(holder, ability);
        }
    }

    /// Confusion lasts a generation-resolved random `time` (RandomRequestKind::Duration) and
    /// does not stack: an already confused target draws nothing (Showdown `addVolatile` fails
    /// before the `onStart` that rolls the time).
    pub fn try_set_confusion(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        let lifecycle = self.rules.status.lifecycle;
        self.try_set_confusion_lasting(
            target,
            source,
            (lifecycle.confusion_time_min, lifecycle.confusion_time_max),
        )
    }

    /// Gen 2 Berserk Gene (`onUpdate`): the holder uses it up — its `boosts`, +2 Attack — and is
    /// confused by its own doing (`addVolatile('confusion')`, the event's target the source:
    /// Safeguard does not stop it).
    pub fn use_berserk_gene(&mut self, holder: PokemonId) {
        let item = crate::ids::ItemId::BERSERK_GENE;
        if self.consume_item(holder).is_none() {
            return;
        }
        self.try_change_stat_stage(crate::engine::StatChangeRequest {
            target: holder,
            stat: crate::state::Stat::Attack,
            stages: 2,
            source: super::EffectSource::Item {
                pokemon: holder,
                item,
            },
            cause: crate::engine::StatChangeCause::Item,
        });
        // Gen 2's `confusion.onStart`: `time = 256` from the gene, nothing drawn.
        let _ = self.start_confusion(holder, holder, ConfusionTime::Fixed(256));
    }

    /// The target's ability's `onHit` for a move whose `volatileStatus` is confusion (Own
    /// Tempo: `-immune|target|confusion`; the confusion was its `blocks_volatiles`).
    pub fn confusion_move_hit(&mut self, target: PokemonId) {
        if let Some(ability) = self.running_ability(target)
            && crate::engine::ability_behavior(self.rules, ability).announces_blocked_confusion
        {
            self.ability_activated(target, ability);
        }
    }

    /// `try_set_confusion` with its `time` drawn from `time` (Gen 2 Thrash's fatigue: 2-3).
    pub fn try_set_confusion_lasting(
        &mut self,
        target: PokemonId,
        source: PokemonId,
        time: (u8, u8),
    ) -> Result<(), ConditionApplyError> {
        self.start_confusion(target, source, ConfusionTime::Drawn(time.0, time.1))
    }

    fn start_confusion(
        &mut self,
        target: PokemonId,
        source: PokemonId,
        time: ConfusionTime,
    ) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon(target).filter(|p| !p.is_fainted()) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon.volatiles.contains(VolatileConditionKind::Confusion) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        // Own Tempo (`onTryAddVolatile`: `null`).
        if self.ability_blocks_volatile(target, VolatileConditionKind::Confusion) {
            return Err(ConditionApplyError::Immune);
        }
        // Safeguard (`onTryAddVolatile`), before the duration is drawn: someone else's move.
        let culprit =
            crate::engine::protections::Culprit::by_move(self.state, self.rules, source, target);
        if crate::engine::protections::safeguard_refuses(self.state, target, culprit) {
            return Err(ConditionApplyError::Immune);
        }
        // Misty Terrain (Gen 7+ `onTryAddVolatile`): no confusion for a grounded Pokémon that
        // is not semi-invulnerable.
        if self.rules.conditions.terrain.misty_blocks_confusion
            && self.state.field.terrain.map(|t| t.kind) == Some(TerrainKind::Misty)
            && self.terrain_covers(target)
        {
            return Err(ConditionApplyError::Immune);
        }
        let time = match time {
            ConfusionTime::Drawn(min, max) => self.state.rng.range_inclusive_for(
                RandomRequestKind::Duration,
                u16::from(min),
                u16::from(max),
            ),
            ConfusionTime::Fixed(time) => time,
        };
        let pokemon = self
            .state
            .pokemon_mut(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if !pokemon.volatiles.set_confusion(source, time) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target,
            kind: VolatileConditionKind::Confusion,
        });
        // Gen 2: confused by another Pokémon's move, the target eats its confusion berry at
        // once (pokecrystal `BattleCommand_FinishConfusingTarget`, I33).
        self.status_inflicted(target, Some(source));
        Ok(())
    }

    /// Yawn (Showdown `addVolatile('yawn')`, `duration: 2`): refused when the target already
    /// yawns, by Safeguard against someone else's move, and by Electric Terrain for a grounded
    /// target not semi-invulnerable (`onTryAddVolatile`); the sleep comes when the residual
    /// counts it out.
    pub fn set_yawn(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon(target).filter(|p| !p.is_fainted()) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon.volatiles.contains(VolatileConditionKind::Yawn) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        if self.ability_blocks_volatile(target, VolatileConditionKind::Yawn) {
            return Err(ConditionApplyError::Immune);
        }
        let culprit =
            crate::engine::protections::Culprit::by_move(self.state, self.rules, source, target);
        if crate::engine::protections::safeguard_refuses(self.state, target, culprit) {
            return Err(ConditionApplyError::Immune);
        }
        if self.state.field.terrain.map(|t| t.kind) == Some(TerrainKind::Electric)
            && self.terrain_covers(target)
        {
            return Err(ConditionApplyError::Immune);
        }
        let source_side = self.state.pokemon(source).map(|pokemon| pokemon.side);
        self.start_timed(target, VolatileConditionKind::Yawn, |v| {
            v.set_yawn(YAWN_TURNS, source_side)
        })
    }

    /// Nightmare (Showdown `onStart`): only on a sleeping target (or one with Comatose).
    pub fn set_nightmare(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let asleep = self
            .state
            .pokemon(target)
            .is_some_and(|p| crate::engine::counts_as_asleep(self.state, self.rules, p));
        if !asleep {
            return Err(ConditionApplyError::InvalidTarget);
        }
        self.start_timed(target, VolatileConditionKind::Nightmare, |v| {
            v.set_nightmare()
        })
    }

    /// Heal Block (Showdown `addVolatile('healblock')`) for `turns` (its `durationCallback`,
    /// `effects::moves::heal_block_turns`); refused when the target already has one
    /// (`onRestart`: no renewal).
    pub fn set_heal_block(
        &mut self,
        target: PokemonId,
        turns: u8,
    ) -> Result<(), ConditionApplyError> {
        if self
            .state
            .pokemon(target)
            .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::HealBlock))
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        if self.ability_blocks_volatile(target, VolatileConditionKind::HealBlock) {
            return Err(ConditionApplyError::Immune);
        }
        self.start_timed(target, VolatileConditionKind::HealBlock, |v| {
            v.set_heal_block(turns)
        })
    }

    /// Embargo (Showdown `addVolatile('embargo')`, `duration: 5`): the target's item has no
    /// effect while it lasts (`engine::held_item`); refused when already there. Its `onStart`
    /// sends `End` to the item, which no implemented item answers (a Choice item's lock stays,
    /// set aside while the item is).
    ///
    /// Gen 4 also fails it on a target whose Quick Claw or Custap Berry went off this turn and
    /// that has not moved yet (`ConditionRules::embargo_fails_after_priority_item`,
    /// `BattleState::priority_item_pending`, issues.md I52).
    pub fn set_embargo(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        if self.rules.conditions.embargo_fails_after_priority_item
            && self.state.priority_item_pending(target)
        {
            return Err(ConditionApplyError::Immune);
        }
        let turns = self.rules.conditions.embargo_turns;
        self.start_timed(target, VolatileConditionKind::Embargo, |v| {
            v.set_embargo(turns)
        })
    }

    /// Salt Cure, Tar Shot (Showdown `addVolatile`, no duration): refused when already there.
    /// Tar Shot's and Roost's `onStart` refuse a Terastallized Pokémon (`return false`): Tar
    /// Shot's Speed drop stands alone, Roost's user keeps its Flying type. Not for a volatile
    /// with guards of its own (`set_torment`, `set_gastro_acid`).
    pub fn set_marker_volatile(
        &mut self,
        target: PokemonId,
        kind: VolatileConditionKind,
    ) -> Result<(), ConditionApplyError> {
        debug_assert!(
            !matches!(
                kind,
                VolatileConditionKind::Torment | VolatileConditionKind::GastroAcid
            ),
            "{kind:?} starts through its own fn"
        );
        if matches!(
            kind,
            VolatileConditionKind::TarShot | VolatileConditionKind::Roost
        ) && self
            .state
            .pokemon(target)
            .is_some_and(|pokemon| pokemon.terastallized.is_some())
        {
            return Err(ConditionApplyError::Immune);
        }
        self.start_timed(target, kind, |v| v.set_marker(kind))
    }

    /// Gastro Acid (Showdown `addVolatile('gastroacid')`, no duration): refused when already
    /// there; its `onStart` announces it (`-endability`) and ends the ability (`End`, §41.2).
    /// From then on `engine::ignoring_ability` sets the ability aside.
    pub fn set_gastro_acid(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        // Its `onStart` refuses a holder of Ability Shield: Gastro Acid's own TryHit has blocked
        // it already, Core Enforcer's hit gets here.
        if self.state.pokemon(target).is_some_and(|pokemon| {
            crate::engine::has_item(
                self.state,
                self.rules,
                pokemon,
                crate::ids::ItemId::ABILITY_SHIELD,
            )
        }) {
            return Err(ConditionApplyError::Immune);
        }
        self.start_timed(target, VolatileConditionKind::GastroAcid, |v| {
            v.set_marker(VolatileConditionKind::GastroAcid)
        })?;
        self.end_ability(target, super::AbilityEndCause::Suppressed);
        Ok(())
    }

    /// Core Enforcer's `onHit` / `onAfterSubDamage`: a target that has already moved this turn —
    /// it neither just came in (`newlySwitched`: no turn boundary on the field since; a lead
    /// is past one by turn 1) nor still has to move (`queue.willMove`) — has its ability
    /// suppressed (`gastroacid`), unless it cannot be suppressed.
    pub fn core_enforcer(&mut self, target: PokemonId) {
        if crate::engine::core_enforcer_suppresses(self.state, self.rules, target) {
            let _ = self.set_gastro_acid(target);
        }
    }

    /// Flame Burst's `onHit` / `onAfterSubDamage`: each adjacent ally of the target still
    /// standing loses 1/16 of its max HP (`baseMaxhp / 16`, at least 1). The damage is the
    /// move's condition's, not the move's: Magic Guard stops it, Sturdy and Focus Sash do not
    /// hold, a Substitute does not take it.
    pub fn flame_burst(&mut self, user: PokemonId, target: PokemonId) {
        let Some(position) = self.state.active_position(target, &self.rules.topology) else {
            return;
        };
        let allies: Vec<PokemonId> = self
            .rules
            .topology
            .adjacent_allies(position)
            .into_iter()
            .filter_map(|position| {
                self.state
                    .pokemon_at_position(position, &self.rules.topology)
            })
            .filter(|ally| self.state.pokemon(*ally).is_some_and(|p| !p.is_fainted()))
            .collect();
        for ally in allies {
            let Some(max_hp) = self.base_max_hp(ally) else {
                continue;
            };
            self.apply_damage(crate::engine::DamageRequest {
                target: ally,
                amount: crate::engine::DamageAmount((max_hp / 16).max(1)),
                kind: crate::engine::DamageKind::Other,
                source: super::EffectSource::Move {
                    pokemon: user,
                    move_id: crate::ids::MoveId::FLAME_BURST,
                },
                flags: crate::engine::DamageFlags {
                    bypass_substitute: true,
                    ..crate::engine::DamageFlags::default()
                },
            });
        }
    }

    /// Syrup Bomb (Showdown `addVolatile('syrupbomb', source)`, `duration: 4`): refused when
    /// already there.
    pub fn set_syrup_bomb(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        self.start_timed(target, VolatileConditionKind::SyrupBomb, |v| {
            v.set_syrup_bomb(source, SYRUP_BOMB_TURNS)
        })
    }

    /// Throat Chop (Showdown `addVolatile('throatchop')`, `duration: 2`): refused when already
    /// there.
    pub fn set_throat_chop(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        self.start_timed(target, VolatileConditionKind::ThroatChop, |v| {
            v.set_throat_chop(THROAT_CHOP_TURNS)
        })
    }

    /// Attract, Cute Charm (Showdown `addVolatile('attract', source)`): the target becomes
    /// infatuated with `source`. Refused when it already is, and by the condition's `onStart`
    /// unless the two are one male and one female.
    pub fn set_attract(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        let (Some(pokemon), Some(by)) = (
            self.state.pokemon(target).filter(|p| !p.is_fainted()),
            self.state.pokemon(source),
        ) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon.volatiles.contains(VolatileConditionKind::Attract) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        if self.ability_blocks_volatile(target, VolatileConditionKind::Attract) {
            return Err(ConditionApplyError::Immune);
        }
        if !pokemon.gender.opposite(by.gender) {
            return Err(ConditionApplyError::Immune);
        }
        // Destiny Knot (`onAttract`, last in the `Attract` event, before the holder's own
        // infatuation starts): whoever caused it is infatuated with the holder in turn.
        if crate::engine::held_item(self.state, self.rules, target)
            .is_some_and(|(_, item)| item.reciprocates_infatuation)
            && self.state.pokemon(source).is_some_and(|p| {
                !p.volatiles.contains(VolatileConditionKind::Attract) && source != target
            })
        {
            let _ = self.set_attract(source, target);
        }
        self.start_timed(target, VolatileConditionKind::Attract, |v| {
            v.set_attract(source)
        })
    }

    /// Taunt (Showdown `onStart`): 3 turns, one more when the target has been on the field
    /// for a turn and already moved this turn.
    pub fn set_taunt(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let pokemon = self
            .state
            .pokemon(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if pokemon.is_fainted() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        // Already on: `addVolatile` fails before its `durationCallback` rolls anything.
        if pokemon.volatiles.contains(VolatileConditionKind::Taunt) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        if self.ability_blocks_volatile(target, VolatileConditionKind::Taunt) {
            return Err(ConditionApplyError::Immune);
        }
        let turns = self.timed_turns(self.rules.conditions.taunt, target);
        self.start_timed(target, VolatileConditionKind::Taunt, |v| v.set_taunt(turns))
    }

    /// Encore (Showdown `addVolatile('encore')`): locks the target into its last move for 3
    /// turns (one more when it already moved this turn). An encored target refuses it at once;
    /// otherwise the duration is drawn first (`durationCallback`, Gen 2-4) and `onStart` then
    /// refuses a target with no last move, with that move out of PP, or whose last move cannot
    /// be encored (`last_move_fails_encore`: its `failencore` flag, which the caller reads).
    pub fn set_encore(
        &mut self,
        target: PokemonId,
        last_move_fails_encore: bool,
    ) -> Result<(), ConditionApplyError> {
        let pokemon = self
            .state
            .pokemon(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if pokemon.is_fainted() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        if pokemon.volatiles.contains(VolatileConditionKind::Encore) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        if self.ability_blocks_volatile(target, VolatileConditionKind::Encore) {
            return Err(ConditionApplyError::Immune);
        }
        let turns = self.timed_turns(self.rules.conditions.encore, target);
        let pokemon = self
            .state
            .pokemon(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        // `encore`'s onStart: a Dynamaxed target is not encored (DESIGN.md §44.5).
        if pokemon.volatiles.contains(VolatileConditionKind::Dynamax) {
            return Err(ConditionApplyError::Immune);
        }
        // A Max Move reads as the move it was made from (`move.isMax && move.baseMove`).
        let Some(last) = pokemon.last_move_encore_resolved() else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if last_move_fails_encore || pokemon.move_slot_of(last).is_none_or(|slot| slot.pp == 0) {
            return Err(ConditionApplyError::InvalidTarget);
        }
        self.start_timed(target, VolatileConditionKind::Encore, |v| {
            v.set_encore(last, turns)
        })
    }

    /// Disable (Showdown `onStart`): the target's last move is unusable for 5 turns (one
    /// fewer when the target still moves this turn); fails without a last move or with that
    /// move out of PP.
    pub fn set_disable(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        self.disable(target, false)
    }

    /// Cursed Body's Disable on the Pokémon whose move just hit the holder: Disable's `onStart`
    /// counts the Pokémon moving a move of its own now (`pokemon === this.activePokemon &&
    /// !this.activeMove.isExternal`: not a dance Dancer copied) as one that still acts this turn,
    /// so a Gen 5+ duration is one short — once, an instructed target that still moves this turn
    /// included (DESIGN.md R-M8-93).
    pub fn set_disable_on_mover(
        &mut self,
        target: PokemonId,
        own_move: bool,
    ) -> Result<(), ConditionApplyError> {
        self.disable(target, own_move)
    }

    fn disable(&mut self, target: PokemonId, moving: bool) -> Result<(), ConditionApplyError> {
        let pokemon = self
            .state
            .pokemon(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if pokemon.is_fainted() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        if self.rules.conditions.disable_gen1 {
            return self.set_disable_gen1(target);
        }
        let Some(last) = pokemon.last_move else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        // Struggle, a Z-Move or a Max Move fails it, held or not (Disable's `onTryHit`: a Max
        // Move a Pokémon knows outright in a custom game, R-M8-123); then the last move's slot
        // out of PP; no slot holding it at all fails it in Gen 2-4
        // (`ConditionRules::disable_needs_the_move_held`) — a Gen 5+ Sketch that became the move
        // it copied still gets sealed (R-M8-115, R-M8-117).
        let unsealable = last == MoveId::STRUGGLE
            // `lastMove.isZOrMaxPowered`: a status move used as a Z-Move (R-M9-36).
            || pokemon.last_move_z
            || self
                .dex
                .and_then(|dex| dex.move_by_id(last))
                .is_some_and(|m| m.z_crystal.is_some() || m.is_max);
        let refused = unsealable
            || match pokemon.move_slot_of(last) {
                Some(slot) => slot.pp == 0,
                None => {
                    self.rules.conditions.disable_needs_the_move_held
                        || self.dex.is_none_or(|dex| dex.move_by_id(last).is_none())
                }
            };
        if refused {
            return Err(ConditionApplyError::InvalidTarget);
        }
        // Already on: `addVolatile` fails before its `durationCallback` rolls anything.
        if pokemon.volatiles.contains(VolatileConditionKind::Disable) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        if self.ability_blocks_volatile(target, VolatileConditionKind::Disable) {
            return Err(ConditionApplyError::Immune);
        }
        let rule = self.rules.conditions.disable;
        let mut turns = self.timed_turns(rule, target);
        // `willMove || (moving its own move)`: one turn off, which `timed_turns` already took
        // for a target that still moves (an instructed one does).
        if moving
            && rule.adjust == crate::rules::DurationAdjust::MinusIfWillMove
            && !self.state.will_move(target)
        {
            turns = turns.saturating_sub(1).max(1);
        }
        self.start_timed(target, VolatileConditionKind::Disable, |v| {
            v.set_disable(last, turns)
        })
    }

    /// Gen 1 Disable (Showdown gen1 `disable`): `onTryHit` wants a move with PP; the
    /// condition's `onStart` picks one of those moves (`DisableSlot`, drawn only when there is a
    /// choice) and a `time` of 1-8 (`Duration`) counted down at the holder's BeforeMove.
    fn set_disable_gen1(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let pokemon = self
            .state
            .pokemon(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        let candidates: Vec<(usize, MoveId)> = pokemon
            .move_slots()
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.pp > 0)
            .map(|(index, slot)| (index, slot.move_id))
            .collect();
        if candidates.is_empty() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        if pokemon.volatiles.contains(VolatileConditionKind::Disable) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        let index = if candidates.len() > 1 {
            usize::from(self.state.rng.range_inclusive_for(
                RandomRequestKind::DisableSlot,
                0,
                (candidates.len() - 1) as u16,
            ))
        } else {
            0
        };
        let time = self
            .state
            .rng
            .range_inclusive_for(RandomRequestKind::Duration, 1, 8) as u8;
        let (slot, move_id) = candidates[index];
        self.start_timed(target, VolatileConditionKind::Disable, |v| {
            v.set_disable_in(move_id, time, u8::try_from(slot).ok())
        })
    }

    /// A timed volatile's duration: fixed or a `Duration` decision, then the `onStart`
    /// adjustment (DESIGN.md §31.1).
    fn timed_turns(&mut self, rule: crate::rules::TimedVolatileRules, target: PokemonId) -> u8 {
        let base = match rule.duration {
            crate::rules::TimedDuration::Fixed(turns) => i16::from(turns),
            crate::rules::TimedDuration::Rolled { min, max } => self.state.rng.range_inclusive_for(
                RandomRequestKind::Duration,
                u16::from(min),
                u16::from(max),
            ) as i16,
        };
        let adjusted = base + crate::engine::duration_adjustment(self.state, rule.adjust, target);
        adjusted.clamp(1, i16::from(u8::MAX)) as u8
    }

    /// Follow Me for the rest of the turn: on the user (a volatile), or in Gen 3 on its slot.
    pub fn set_follow_me(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        if self.rules.conditions.follow_me_on_slot {
            let side = self
                .state
                .pokemon(user)
                .map(|p| p.side)
                .ok_or(ConditionApplyError::InvalidTarget)?;
            let slot = self
                .state
                .side(side)
                .active
                .iter()
                .position(|active| *active == Some(user))
                .ok_or(ConditionApplyError::InvalidTarget)?;
            let side_state = self.state.side_mut(side);
            if side_state.follow_me_slots.contains(&slot) {
                return Err(ConditionApplyError::AlreadyActive);
            }
            side_state.follow_me_slots.push(slot);
            // Showdown's `-singleturn|p|move: Follow Me` all the same: seen on the user.
            self.trace.push(TraceEvent::VolatileStarted {
                target: user,
                kind: VolatileConditionKind::FollowMe,
            });
            return Ok(());
        }
        self.start_timed(user, VolatileConditionKind::FollowMe, |v| v.set_follow_me())
    }

    pub fn set_rage_powder(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        self.start_timed(user, VolatileConditionKind::RagePowder, |v| {
            v.set_rage_powder()
        })
    }

    /// Showdown `queue.willAct()`: somebody still acts this turn.
    pub fn anyone_will_act(&self) -> bool {
        !self.state.pending_movers.is_empty()
    }

    /// Wide Guard / Quick Guard's `onTry` (`effects::moves::CustomPrimary::tries`): nobody else
    /// still acts this turn (`willAct`), or — Gen 5 (`ConditionRules::guards_roll_stall`, the
    /// gen5 mod's `StallMove`) — the Protect counter's roll fails it.
    pub fn guard_try(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        if !self.anyone_will_act()
            || (self.rules.conditions.guards_roll_stall && !self.stall_roll(user))
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        Ok(())
    }

    /// Wide Guard / Quick Guard, past their `onTry` (`guard_try`): the side is guarded for the
    /// turn (unless it already is) and the user's Protect counter advances
    /// (`addVolatile('stall')`) without a roll.
    pub fn set_guard(
        &mut self,
        side: SideId,
        kind: SideConditionKind,
        user: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        if self.state.side_mut(side).conditions.set_guard(kind, user) {
            self.side_condition_started(side, kind);
        }
        // Showdown `onHitSide` adds the stall volatile whether or not the guard was already up
        // (a partner's second Wide Guard still advances its own counter) — and its `true` is
        // what the move did: `runMoveEffects` combines the side condition's `false` with it,
        // so a guard already up is no failure (no `-fail`; Stomping Tantrum and Temper Flare
        // after it are not boosted, R-M8-146).
        self.advance_stall(user);
        Ok(())
    }

    /// Crafty Shield / Mat Block, past their `onTry` (`willAct`): the side guarded for the
    /// turn; fails when it already is.
    pub fn set_single_turn_guard(
        &mut self,
        side: SideId,
        kind: SideConditionKind,
        user: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        if !self.state.side_mut(side).conditions.set_guard(kind, user) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.side_condition_started(side, kind);
        Ok(())
    }

    /// Whether this is `pokemon`'s first move action since it came in (Showdown
    /// `activeMoveActions`, counted as the move starts).
    pub fn first_move_action(&self, pokemon: PokemonId) -> bool {
        self.state
            .pokemon(pokemon)
            .is_some_and(|p| p.active_move_actions <= 1)
    }

    /// Showdown `addVolatile('stall')` after a successful Protect / guard: a fresh counter or
    /// the next one, its duration reset.
    fn advance_stall(&mut self, user: PokemonId) {
        let stall = self.rules.conditions.protect_stall;
        let turns = self.rules.conditions.two_turn_move_turns;
        let fresh = self
            .state
            .pokemon(user)
            .is_some_and(|p| p.volatiles.stall_counter().is_none());
        if let Some(pokemon) = self.state.pokemon_mut(user) {
            pokemon
                .volatiles
                .bump_stall(stall.start(), |counter| stall.next(counter), turns);
        }
        if fresh {
            self.trace.push(TraceEvent::VolatileStarted {
                target: user,
                kind: VolatileConditionKind::Stall,
            });
        }
    }

    /// Hyper Beam's recharge turn.
    /// Bind / Wrap / Fire Spin … landed: the target is trapped for a `Duration` decision of
    /// turns (Showdown `partiallytrapped` `durationCallback`); a Pokémon already bound stays
    /// as it is.
    pub fn set_partially_trapped(
        &mut self,
        target: PokemonId,
        source: PokemonId,
        move_id: MoveId,
    ) -> Result<(), ConditionApplyError> {
        let rules = self.rules.conditions;
        // Gen 1: two turns, set again by every hit (DESIGN.md §30.9).
        if rules.switching.binding_locks_user {
            self.gen1_partially_trap(target, source, move_id);
            return Ok(());
        }
        // `addVolatile` refuses a fainted or already bound target before rolling the duration.
        let Some(pokemon) = self.state.pokemon(target).filter(|p| !p.is_fainted()) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon
            .volatiles
            .contains(VolatileConditionKind::PartiallyTrapped)
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        // Grip Claw fixes the turns, no roll; Binding Band's chip is set as the bind starts.
        let trapper_item = crate::engine::held_item_id(self.state, self.rules, source);
        let turns = if trapper_item == Some(ItemId::GRIP_CLAW) {
            rules.grip_claw_bind_turns
        } else {
            self.state.rng.range_inclusive_for(
                RandomRequestKind::Duration,
                u16::from(rules.bind_turns_min),
                u16::from(rules.bind_turns_max),
            ) as u8
        };
        let banded = trapper_item == Some(ItemId::BINDING_BAND);
        self.start_timed(target, VolatileConditionKind::PartiallyTrapped, |v| {
            v.set_partially_trapped_banded(source, move_id, turns, banded)
        })
    }

    /// Ally Switch's counter (Showdown `addVolatile('allyswitch')`, DESIGN.md §42.4): 3 when it
    /// starts, ×3 (at most 729) each consecutive use.
    pub fn bump_ally_switch(&mut self, user: PokemonId) {
        let turns = self.rules.conditions.two_turn_move_turns;
        if let Some(pokemon) = self.state.pokemon_mut(user) {
            let counter = pokemon.volatiles.bump_ally_switch(turns);
            if counter == 3 {
                self.trace.push(TraceEvent::VolatileStarted {
                    target: user,
                    kind: VolatileConditionKind::AllySwitch,
                });
            }
        }
    }

    /// The `allyswitch` counter's failed chance: the volatile goes.
    pub fn drop_ally_switch(&mut self, user: PokemonId) {
        if let Some(pokemon) = self.state.pokemon_mut(user)
            && pokemon.volatiles.remove_ally_switch()
        {
            self.trace.push(TraceEvent::VolatileEnded {
                target: user,
                kind: VolatileConditionKind::AllySwitch,
                seen: true,
            });
        }
    }

    /// A `randomChance(numerator, denominator)` decision of the engine's shared kind.
    pub fn chance_for(
        &mut self,
        kind: crate::state::RandomRequestKind,
        numerator: u16,
        denominator: u16,
    ) -> bool {
        self.state.rng.roll_ratio_for(kind, numerator, denominator)
    }

    /// Ally Switch's swap (Showdown `swapPosition`): the user and its partner trade slots, the
    /// `-swap` line names both. Fails when the partner is gone.
    pub fn swap_with_partner(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon(user) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        let side = pokemon.side;
        if self
            .state
            .active_position(user, &self.rules.topology)
            .is_none()
        {
            return Err(ConditionApplyError::InvalidTarget);
        }
        let other = self
            .state
            .side(side)
            .active
            .iter()
            .flatten()
            .copied()
            .find(|ally| *ally != user)
            .ok_or(ConditionApplyError::NothingToDo)?;
        if !self.state.swap_positions(side, user, other) {
            return Err(ConditionApplyError::NothingToDo);
        }
        self.trace.push(TraceEvent::PositionsSwapped { side });
        // Gen 8+ Healing Wish / Lunar Dance (`onSwap`, the partner's `Swap` first): a wish
        // waiting on a slot heals the Pokémon moved into it, if it needs it (DESIGN.md R-M8-86).
        if self.rules.conditions.self_faint.slot_heal == crate::rules::SlotHealRules::WhenNeeded {
            self.apply_slot_heals(other);
            self.apply_slot_heals(user);
        }
        Ok(())
    }

    /// Mimic (Showdown `source.moveSlots[mimicIndex] = …`, DESIGN.md §42.5): the copy goes into
    /// the slots the Pokémon battles with — an overlay over its own, thrown away on switch-out.
    /// `drawn_from`: Gen 1's draw among that Pokémon's moves.
    pub fn write_mimicked_slot(
        &mut self,
        user: PokemonId,
        slot: usize,
        entry: crate::state::MoveSlotState,
        drawn_from: Option<PokemonId>,
    ) {
        let Some(pokemon) = self.state.pokemon_mut(user) else {
            return;
        };
        pokemon.set_move_overlay(slot, entry);
        self.trace.push(TraceEvent::MoveSlotsRewritten {
            pokemon: user,
            slot,
            move_id: entry.move_id,
            by: MoveId::MIMIC,
            drawn_from,
        });
    }

    /// Sketch (Showdown writes `moveSlots` and `baseMoveSlots`, §42.5): learned for good.
    pub fn write_learned_slot(
        &mut self,
        user: PokemonId,
        slot: usize,
        entry: crate::state::MoveSlotState,
    ) {
        let Some(pokemon) = self.state.pokemon_mut(user) else {
            return;
        };
        pokemon.set_learned_move(slot, entry);
        self.trace.push(TraceEvent::MoveSlotsRewritten {
            pokemon: user,
            slot,
            move_id: entry.move_id,
            by: MoveId::SKETCH,
            drawn_from: None,
        });
    }

    /// Showdown `addVolatile('mefirst')` (DESIGN.md §42.8).
    pub fn set_me_first(&mut self, user: PokemonId) -> bool {
        let Some(pokemon) = self.state.pokemon_mut(user) else {
            return false;
        };
        pokemon.volatiles.set_me_first()
    }

    /// Showdown `addVolatile('snatch')` (DESIGN.md §42.8).
    pub fn set_snatch(&mut self, user: PokemonId) -> bool {
        let Some(pokemon) = self.state.pokemon_mut(user) else {
            return false;
        };
        pokemon.volatiles.set_snatch()
    }

    /// Showdown `addVolatile('magiccoat')` (DESIGN.md §42.8): the coat is a single-turn volatile
    /// that announces itself (`-singleturn|user|move: Magic Coat`). A second one is no failure.
    pub fn set_magic_coat(&mut self, user: PokemonId) -> bool {
        let boosted = self
            .state
            .active_move
            .is_some_and(|active| active.prankster_boosted);
        let Some(pokemon) = self.state.pokemon_mut(user) else {
            return false;
        };
        pokemon.volatiles.set_magic_coat(boosted)
    }

    /// The `-singleturn` announcement of a volatile that shows itself in the log.
    pub fn singleturn(&mut self, user: PokemonId, kind: crate::state::VolatileConditionKind) {
        self.trace
            .push(TraceEvent::VolatileStarted { target: user, kind });
    }

    /// Helping Hand on `ally`: fails when it already acted this turn and did not just come in
    /// (Showdown `onTryHit`: `newlySwitched || willMove`); a second one stacks.
    pub fn set_helping_hand(&mut self, ally: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon(ally) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon.is_fainted() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        if !self.state.will_move(ally) && pokemon.active_turns > 0 {
            return Err(ConditionApplyError::AlreadyActive);
        }
        let fresh = !pokemon
            .volatiles
            .contains(VolatileConditionKind::HelpingHand);
        if let Some(pokemon) = self.state.pokemon_mut(ally) {
            pokemon.volatiles.add_helping_hand();
        }
        if fresh {
            self.trace.push(TraceEvent::VolatileStarted {
                target: ally,
                kind: VolatileConditionKind::HelpingHand,
            });
        }
        Ok(())
    }

    pub fn set_ingrain(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        self.start_timed(target, VolatileConditionKind::Ingrain, |v| v.set_ingrain())
    }

    /// A stalling move (Showdown `stallingMove`): Protect and its variants, Endure, past their
    /// `onPrepareHit` (`stall_try`: somebody still to act, the counter's roll): it sets its
    /// volatile and advances the counter.
    pub fn stall_move(
        &mut self,
        user: PokemonId,
        effect: StallEffect,
    ) -> Result<(), ConditionApplyError> {
        // Gen 2: the user's own Substitute fails the stalling move (its `onTryPrimaryHit`),
        // after the roll and before the counter moves.
        if self.rules.conditions.substitute_gen2_quirks
            && self
                .state
                .pokemon(user)
                .is_some_and(|p| p.volatiles.contains(VolatileConditionKind::Substitute))
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        match effect {
            StallEffect::Protect(kind) => self.set_protect(user, user, kind)?,
            StallEffect::Endure => {
                let turn = self.state.turn;
                self.state
                    .pokemon_mut(user)
                    .ok_or(ConditionApplyError::InvalidTarget)?
                    .volatiles
                    .set_endure(turn);
                self.trace.push(TraceEvent::VolatileStarted {
                    target: user,
                    kind: VolatileConditionKind::Endure,
                });
            }
        }
        self.advance_stall(user);
        Ok(())
    }

    /// A stalling move's `onPrepareHit` (`effects::moves::CustomPrimary::tries`, before
    /// Protean's): somebody else still acts this turn (`willAct`) and the counter's roll
    /// (`StallMove`) succeeds.
    pub fn stall_try(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        if !self.anyone_will_act() || !self.stall_roll(user) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        Ok(())
    }

    /// Showdown `StallMove` (`stall` `onStallMove`): with a counter from a recent Protect or
    /// guard, 1 / counter to succeed; a failure ends the counter where the generation says.
    fn stall_roll(&mut self, user: PokemonId) -> bool {
        let stall = self.rules.conditions.protect_stall;
        let Some(counter) = self
            .state
            .pokemon(user)
            .and_then(|p| p.volatiles.stall_counter())
        else {
            return true;
        };
        let (numerator, denominator) = stall.chance(counter);
        let success = self.state.rng.roll_ratio_for(
            RandomRequestKind::ProtectSuccess,
            numerator,
            denominator,
        );
        self.trace.push(TraceEvent::ProtectChecked {
            actor: user,
            counter,
            success,
        });
        if !success
            && stall.failure_ends_stall()
            && let Some(pokemon) = self.state.pokemon_mut(user)
            && pokemon.volatiles.remove(VolatileConditionKind::Stall)
        {
            self.trace.push(TraceEvent::VolatileEnded {
                target: user,
                kind: VolatileConditionKind::Stall,
                seen: true,
            });
        }
        success
    }

    pub fn set_must_recharge(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let turns = self.rules.conditions.must_recharge_turns;
        self.start_timed(target, VolatileConditionKind::MustRecharge, |v| {
            v.set_must_recharge(turns)
        })
    }

    /// A Choice item locks the holder into `move_id` (no-op while a lock is live).
    pub fn set_choice_lock(&mut self, holder: PokemonId, move_id: MoveId) -> bool {
        self.start_timed(holder, VolatileConditionKind::ChoiceLock, |v| {
            v.set_choice_lock(move_id)
        })
        .is_ok()
    }

    pub fn clear_choice_lock(&mut self, holder: PokemonId) {
        self.end_volatile(holder, VolatileConditionKind::ChoiceLock);
    }

    /// Removes a volatile condition and traces its end; false when it was not there. Not for
    /// one whose `End` changes the battle: it ends through its own fn (`end_stockpile`,
    /// `end_rampage`, `end_uproar`, `free_trapped`, the engine's `end_dynamax`).
    pub fn end_volatile(&mut self, target: PokemonId, kind: VolatileConditionKind) -> bool {
        debug_assert!(
            !matches!(
                kind,
                VolatileConditionKind::Dynamax
                    | VolatileConditionKind::Stockpile
                    | VolatileConditionKind::Rampage
                    | VolatileConditionKind::Uproar
                    | VolatileConditionKind::Trapped
            ),
            "{kind:?} ends through its own fn"
        );
        self.drop_volatile(target, kind)
    }

    /// `end_volatile` with no question asked: the removal under the fns a volatile with an
    /// `End` of its own ends through (`free_trapped` …).
    pub(super) fn drop_volatile(&mut self, target: PokemonId, kind: VolatileConditionKind) -> bool {
        self.remove_volatile(target, kind, true)
    }

    /// `end_volatile` where Showdown removes it with no word (`VolatileEnded::seen`, DESIGN.md
    /// R-M10-26): the observers still see it on.
    pub fn end_volatile_unseen(&mut self, target: PokemonId, kind: VolatileConditionKind) -> bool {
        self.remove_volatile(target, kind, false)
    }

    fn remove_volatile(
        &mut self,
        target: PokemonId,
        kind: VolatileConditionKind,
        seen: bool,
    ) -> bool {
        let ended = self
            .state
            .pokemon_mut(target)
            .is_some_and(|pokemon| pokemon.volatiles.remove(kind));
        if ended {
            self.trace
                .push(TraceEvent::VolatileEnded { target, kind, seen });
        }
        ended
    }

    /// Showdown `removeVolatile('Protect')`: the `protect` volatile (Protect, Detect), not King's
    /// Shield's or the other protections'.
    pub fn end_plain_protect(&mut self, target: PokemonId) -> bool {
        let plain = self.state.pokemon(target).is_some_and(|pokemon| {
            pokemon.volatiles.protect_kind() == Some(crate::state::ProtectKind::Protect)
        });
        plain && self.end_volatile(target, VolatileConditionKind::Protect)
    }

    pub(super) fn start_timed(
        &mut self,
        target: PokemonId,
        kind: VolatileConditionKind,
        set: impl FnOnce(&mut crate::state::VolatileState) -> bool,
    ) -> Result<(), ConditionApplyError> {
        let pokemon = self
            .state
            .pokemon_mut(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if pokemon.is_fainted() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        if !set(&mut pokemon.volatiles) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.trace
            .push(TraceEvent::VolatileStarted { target, kind });
        Ok(())
    }

    /// Lays a layer of Spikes (up to the generation's maximum); returns the new layer count.
    pub fn add_spikes(
        &mut self,
        side: SideId,
        source: PokemonId,
    ) -> Result<u8, ConditionApplyError> {
        let max = self.rules.conditions.spikes_max_layers;
        let layers = self
            .state
            .side_mut(side)
            .conditions
            .add_spikes(source, max)
            .ok_or(ConditionApplyError::AlreadyActive)?;
        self.side_condition_layered(side, SideConditionKind::Spikes, layers);
        Ok(layers)
    }

    /// Lays a layer of Toxic Spikes (two at most); returns the new layer count.
    pub fn add_toxic_spikes(
        &mut self,
        side: SideId,
        source: PokemonId,
    ) -> Result<u8, ConditionApplyError> {
        let max = self.rules.conditions.toxic_spikes_max_layers;
        let layers = self
            .state
            .side_mut(side)
            .conditions
            .add_toxic_spikes(source, max)
            .ok_or(ConditionApplyError::AlreadyActive)?;
        self.side_condition_layered(side, SideConditionKind::ToxicSpikes, layers);
        Ok(layers)
    }

    fn side_condition_layered(&mut self, side: SideId, kind: SideConditionKind, layers: u8) {
        if layers == 1 {
            self.side_condition_started(side, kind);
        }
        self.trace
            .push(TraceEvent::SideConditionLayered { side, kind, layers });
    }

    pub fn set_sticky_web(
        &mut self,
        side: SideId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        if !self.state.side_mut(side).conditions.set_sticky_web(source) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.side_condition_started(side, SideConditionKind::StickyWeb);
        Ok(())
    }

    pub fn remove_side_condition(&mut self, side: SideId, kind: SideConditionKind) -> bool {
        let removed = self.state.side_mut(side).conditions.remove(kind);
        if removed {
            self.trace
                .push(TraceEvent::SideConditionEnded { side, kind });
        }
        removed
    }

    /// Leech Seed on `target`, draining towards `source`'s current position. Grass types are
    /// immune; a seeded target cannot be seeded again.
    pub fn set_leech_seed(
        &mut self,
        target: PokemonId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        if self.has_type(target, PokemonType::Grass) {
            return Err(ConditionApplyError::Immune);
        }
        let position = self
            .state
            .active_position(source, &self.rules.topology)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        let pokemon = self
            .state
            .pokemon_mut(target)
            .ok_or(ConditionApplyError::InvalidTarget)?;
        if pokemon.is_fainted() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        if !pokemon.volatiles.set_leech_seed(source, position) {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.trace.push(TraceEvent::VolatileStarted {
            target,
            kind: VolatileConditionKind::LeechSeed,
        });
        Ok(())
    }

    pub fn set_stealth_rock(
        &mut self,
        side: SideId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        if !self
            .state
            .side_mut(side)
            .conditions
            .set_stealth_rock(source)
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.side_condition_started(side, SideConditionKind::StealthRock);
        Ok(())
    }

    pub fn set_tailwind(
        &mut self,
        side: SideId,
        source: PokemonId,
    ) -> Result<(), ConditionApplyError> {
        let turns = self.rules.conditions.tailwind_turns;
        if !self
            .state
            .side_mut(side)
            .conditions
            .set_tailwind(source, turns)
        {
            return Err(ConditionApplyError::AlreadyActive);
        }
        self.side_condition_started(side, SideConditionKind::Tailwind);
        Ok(())
    }

    /// Showdown `runEvent('PseudoWeatherChange')` as a pseudo-weather starts: Room Service
    /// (`onAnyPseudoWeatherChange`) on every active Pokémon, fastest first. Only Trick Room's
    /// start can find a holder still holding it (one that came in under Trick Room used it as
    /// it entered).
    fn pseudo_weather_changed(&mut self) {
        let mut active: Vec<(PokemonId, u32)> = [SideId::P1, SideId::P2]
            .into_iter()
            .flat_map(|side| self.state.side(side).active.iter().flatten().copied())
            .map(|pokemon| {
                let speed =
                    crate::engine::stats::action_speed(self.state, self.rules, pokemon, None);
                (pokemon, speed)
            })
            .collect();
        active.sort_by_key(|(_, speed)| std::cmp::Reverse(*speed));
        for (pokemon, _) in active {
            if self.held_item(pokemon).is_some_and(|(_, behavior)| {
                matches!(
                    behavior.used_on_field,
                    Some(crate::effects::items::UsedOnField {
                        when: crate::effects::items::FieldItemWhen::TrickRoom,
                        ..
                    })
                )
            }) {
                crate::effects::items::use_if_the_field_has_it(self, pokemon);
            }
        }
    }

    /// Returns whether Trick Room is active after the toggle.
    pub fn toggle_trick_room(&mut self, source: PokemonId) -> bool {
        let active = self
            .state
            .field
            .toggle_trick_room(source, self.rules.conditions.trick_room_turns);
        if active {
            self.trace.push(TraceEvent::FieldConditionStarted {
                kind: FieldConditionKind::TrickRoom,
            });
            self.pseudo_weather_changed();
        } else {
            self.trace.push(TraceEvent::FieldConditionEnded {
                kind: FieldConditionKind::TrickRoom,
            });
        }
        active
    }

    /// A weather move (Sunny Day, Rain Dance, Sandstorm, Hail / Snowscape): five turns; fails
    /// under the same weather from Gen 3 (Showdown `setWeather` → false → `-fail`).
    pub fn set_weather_by_move(&mut self, kind: WeatherKind, source: PokemonId) -> bool {
        // A strong weather keeps it off (`onAnySetWeather`, §23.5).
        if self.strong_weather_up() {
            return false;
        }
        let same = self.state.field.weather.is_some_and(|w| w.kind == kind);
        if same && self.rules.weather.same_weather_move.fails(kind) {
            return false;
        }
        let turns = self.weather_turns(kind, source);
        self.start_weather(kind, source, turns);
        true
    }

    /// Showdown `durationCallback`: five turns, eight when the setter holds the weather's rock.
    fn weather_turns(&self, kind: WeatherKind, source: PokemonId) -> u8 {
        let rock = match kind {
            WeatherKind::Sun => Some(ItemId::HEAT_ROCK),
            WeatherKind::Rain => Some(ItemId::DAMP_ROCK),
            WeatherKind::Sandstorm => Some(ItemId::SMOOTH_ROCK),
            WeatherKind::Snow => Some(ItemId::ICY_ROCK),
            WeatherKind::StrongWinds => None,
        };
        if rock.is_some() && crate::engine::held_item_id(self.state, self.rules, source) == rock {
            self.rules.weather.rock_turns
        } else {
            self.rules.conditions.weather_turns
        }
    }

    /// A weather ability on entry (Drought, Drizzle, Sand Stream, Snow Warning): permanent
    /// before Gen 6, five turns from Gen 6 (Showdown `duration 0`). From Gen 6 it never starts
    /// under the same weather; before, only a permanent one stops it, so the ability makes a
    /// move's timed weather permanent (Showdown `setWeather`).
    pub fn set_weather_by_ability(
        &mut self,
        kind: WeatherKind,
        source: PokemonId,
        ability: AbilityId,
    ) -> bool {
        // A strong weather keeps it off (`onAnySetWeather` → `-ability|holder|Drought|[from]
        // Desolate Land|[fail]`, §23.5): the line names the ability (R-M10-39).
        if self.strong_weather_up() {
            self.ability_activated(source, ability);
            return false;
        }
        if let Some(current) = self.state.field.weather.filter(|w| w.kind == kind)
            && (!self.rules.weather.ability_weather_permanent || current.turns_remaining == 0)
        {
            return false;
        }
        self.ability_activated(source, ability);
        let turns = if self.rules.weather.ability_weather_permanent {
            0
        } else {
            self.weather_turns(kind, source)
        };
        self.start_weather(kind, source, turns);
        true
    }

    fn start_weather(&mut self, kind: WeatherKind, source: PokemonId, turns: u8) {
        let previous = self
            .state
            .field
            .set_weather(kind, source, turns)
            .map(|weather| weather.kind);
        self.trace.push(TraceEvent::WeatherChanged {
            from: previous,
            to: Some(kind),
        });
        // Showdown `setWeather` → `eachEvent('WeatherChange')` (Protosynthesis).
        self.weather_changed();
    }

    /// Starts a terrain for `TerrainRules::turns` — `extender_turns` when the setter holds
    /// Terrain Extender (`durationCallback`) — and lets the active Pokémon react
    /// (`TerrainChange`); the same terrain already up makes it fail (Showdown `setTerrain`).
    pub fn set_terrain(&mut self, kind: TerrainKind, source: PokemonId) -> bool {
        if self.state.field.terrain.is_some_and(|t| t.kind == kind) {
            return false;
        }
        let rules = self.rules.conditions.terrain;
        let turns = if crate::engine::held_item_id(self.state, self.rules, source)
            == Some(ItemId::TERRAIN_EXTENDER)
        {
            rules.extender_turns
        } else {
            rules.turns
        };
        let previous = self
            .state
            .field
            .set_terrain(kind, source, turns)
            .map(|terrain| terrain.kind);
        self.trace.push(TraceEvent::TerrainChanged {
            from: previous,
            to: Some(kind),
        });
        self.terrain_changed();
        true
    }
}

/// What a stalling move sets (`BattleContext::stall_move`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallEffect {
    Protect(crate::state::ProtectKind),
    Endure,
}

/// How long a confusion lasts (Showdown `effectState.time`).
#[derive(Debug, Clone, Copy)]
enum ConfusionTime {
    /// Drawn between the two (`RandomRequestKind::Duration`).
    Drawn(u8, u8),
    /// Set, nothing drawn (Gen 2 Berserk Gene's 256).
    Fixed(u16),
}
