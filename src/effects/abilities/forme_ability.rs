//! The abilities tied to one species' formes (DESIGN.md §43.4, N.3): Disguise and Ice Face (the
//! forme that takes a hit whole), Gulp Missile (the forme it dives into and spits out), Battle
//! Bond and Zero to Hero (the forme it earns).

use crate::engine::{BattleContext, DamageDecision, DamageRequest, EffectSource};
use crate::ids::{AbilityId, PokemonId, SpeciesId};

/// Which forme of its own a hit-absorbing ability answers, and what it breaks into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbsorbingForme {
    /// The whole forme (Mimikyu, Eiscue).
    pub whole: SpeciesId,
    /// The broken one (Mimikyu-Busted, Eiscue-Noice).
    pub broken: SpeciesId,
    /// A Totem forme that answers as well, with the broken forme it takes (Showdown's
    /// `['mimikyu', 'mimikyutotem']`: Mimikyu-Totem breaks into Mimikyu-Busted-Totem, DESIGN.md
    /// R-M9-21).
    pub totem: Option<(SpeciesId, SpeciesId)>,
    /// Only a physical move breaks it (Ice Face; Disguise takes any damaging move).
    pub physical_only: bool,
    /// What breaking costs, where the generation charges one (Disguise from Gen 8: an eighth of
    /// the max HP, `FormeRules::disguise_costs_hp`); `None` for the abilities that cost
    /// nothing.
    pub break_cost: Option<crate::rules::Fraction>,
}

impl AbsorbingForme {
    /// The broken forme for a Pokémon battling as `forme`, when `forme` is a whole one.
    fn broken_for(self, forme: Option<SpeciesId>) -> Option<SpeciesId> {
        let forme = forme?;
        if forme == self.whole {
            return Some(self.broken);
        }
        self.totem
            .filter(|(whole, _)| *whole == forme)
            .map(|(_, broken)| broken)
    }

    /// Whether `holder`'s forme takes `request` whole: a move that reached its HP, of the right
    /// category, while it wears the whole forme and is not transformed (`target.species.id`).
    pub fn absorbs(&self, ctx: &BattleContext<'_>, request: &DamageRequest) -> bool {
        // `effect?.effectType === 'Move'` (`DamageRequest::from_a_move`): a confusion self-hit's
        // `confused` pseudo-move too, which Disguise takes whole. That pseudo-move has no
        // category, so Ice Face (`effect.category === 'Physical'`) lets it through (DESIGN.md
        // R-M9-21).
        self.whole_now(ctx.state(), ctx.resolved_rules(), request.target)
            && request.from_a_move(&ctx.rules())
            && (!self.physical_only
                || (matches!(request.kind, crate::engine::DamageKind::Move)
                    && ctx.active_move_category() == Some(crate::dex::MoveCategory::Physical)))
    }

    /// Whether the whole forme answers `move_def` aimed at `holder` before the hit — its
    /// `onCriticalHit` (no critical hit) and `onEffectiveness` (neutral): a damaging move, a
    /// physical one for Ice Face, while the forme is whole — not one that strikes a Substitute in
    /// front of it (`hitSub`: a `substitute` and no `bypasssub`; Infiltrator's moves have it from
    /// Gen 6, `modified_move`), which the forme never meets (R-M10-78).
    pub fn answers_before_the_hit(
        self,
        state: &crate::state::BattleState,
        rules: &crate::rules::ResolvedRules,
        holder: PokemonId,
        move_def: &crate::dex::ResolvedMove,
    ) -> bool {
        move_def.category != crate::dex::MoveCategory::Status
            && (!self.physical_only || move_def.category == crate::dex::MoveCategory::Physical)
            && self.whole_now(state, rules, holder)
            && !state.pokemon(holder).is_some_and(|pokemon| {
                pokemon
                    .volatiles
                    .contains(crate::state::VolatileConditionKind::Substitute)
                    && !move_def.flags.bypass_substitute
            })
    }

    /// Whether `holder` is wearing the whole forme now: every callback of these abilities asks
    /// that first.
    pub fn whole_now(
        self,
        state: &crate::state::BattleState,
        rules: &crate::rules::ResolvedRules,
        holder: PokemonId,
    ) -> bool {
        state.pokemon(holder).is_some_and(|holder| {
            self.broken_for(crate::engine::species_of(holder)).is_some()
                && holder.transform.is_none()
                && crate::engine::ability_handler(state, rules, holder, |b| b.absorbing)
                    == Some(self)
        })
    }

    /// The hit is taken whole: the forme announces itself (`onDamage` priority 1, `-activate`),
    /// notes that it is broken, and the damage is zero — Showdown returns 0 from `onDamage`, so
    /// the hit commits no HP and still logs the target's (`-damage … 150/150`).
    /// Gen 7 Disguise takes a Rollout / Ice Ball hit as a hit of the chain, not a doubling
    /// (`source.volatiles[effect.id].contactHitCount--`, issues.md I212).
    pub fn absorb(&self, ctx: &mut BattleContext<'_>, request: &DamageRequest) -> DamageDecision {
        if let EffectSource::Move { pokemon, move_id } = request.source
            && (move_id == crate::ids::MoveId::ROLLOUT || move_id == crate::ids::MoveId::ICE_BALL)
            && ctx.rules().conditions().forme.disguise_delays_rolling
        {
            ctx.delay_rolling(pokemon, move_id);
        }
        ctx.ability_activated(request.target, ctx.raw_ability(request.target));
        ctx.set_busted(request.target, true);
        DamageDecision::Reduce(crate::engine::DamageAmount(0))
    }

    /// `onUpdate`: the holder of the whole forme whose disguise is broken takes its broken
    /// forme, and the generation's cost is paid once it is in it.
    pub fn update(&self, ctx: &mut BattleContext<'_>, holder: PokemonId) {
        let Some(broken) = self.broken_for(ctx.species_of(holder)) else {
            return;
        };
        if !ctx.busted(holder) {
            return;
        }
        let ability = ctx.raw_ability(holder);
        let source = EffectSource::Ability {
            pokemon: holder,
            ability,
        };
        if !ctx.forme_change(
            holder,
            broken,
            crate::engine::FormeChangeKind::PermanentKeepingAbility,
            source,
        ) {
            return;
        }
        ctx.set_busted(holder, false);
        let Some(cost) = self
            .break_cost
            .filter(|_| ctx.rules().conditions().forme.disguise_costs_hp)
        else {
            return;
        };
        // `this.damage(pokemon.baseMaxhp / 8, …)`: a damage of the `Damage` event (not a
        // `directDamage`), so it sets `hurtThisTurn` like any other (DESIGN.md R-M8-91).
        let Some(max_hp) = ctx.base_max_hp(holder) else {
            return;
        };
        ctx.apply_damage(DamageRequest {
            target: holder,
            amount: crate::engine::DamageAmount(cost.apply_u16(max_hp).max(1)),
            kind: crate::engine::DamageKind::Other,
            source,
            flags: crate::engine::DamageFlags {
                bypass_substitute: true,
                ..crate::engine::DamageFlags::default()
            },
        });
    }
}

/// Showdown `onDamage` priority 1 for the absorbing formes: the hit is taken whole (an
/// `-activate` and no damage at all), and the forme is marked broken for the `Update` to take.
pub fn absorb_damage(ctx: &mut BattleContext<'_>, request: &DamageRequest) -> DamageDecision {
    let Some(absorbing) = ctx.absorbing(request.target) else {
        return DamageDecision::Allow;
    };
    if absorbing.absorbs(ctx, request) {
        absorbing.absorb(ctx, request)
    } else {
        DamageDecision::Allow
    }
}

/// Ice Face's `onStart` / `onWeatherChange`: a Noice Eiscue in the hail or snow freezes its face
/// back over. The weather resuming because Air Lock / Cloud Nine ended is not a change Ice Face
/// answers (`sourceEffect.suppressWeather`), which is why this runs only for a fresh change
/// (`leaving` is `None`).
pub fn ice_face_weather(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.species_of(holder) != Some(SpeciesId::EISCUE_NOICE)
        || ctx.has_transformed(holder)
        || ctx.hp(holder).is_none_or(|hp| hp == 0)
        || !matches!(ctx.field_weather(), Some(crate::state::WeatherKind::Snow))
    {
        return;
    }
    ctx.ability_activated(holder, AbilityId::ICE_FACE);
    ctx.set_busted(holder, false);
    ctx.forme_change(
        holder,
        SpeciesId::EISCUE,
        crate::engine::FormeChangeKind::PermanentKeepingAbility,
        EffectSource::Ability {
            pokemon: holder,
            ability: AbilityId::ICE_FACE,
        },
    );
}

/// Gulp Missile's `onSourceTryPrimaryHit` (`Surf`, priority 0, before the hit lands): a
/// Cramorant of its own comes up with its catch.
pub fn gulp_missile_surf(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    _target: PokemonId,
    move_def: &crate::dex::ResolvedMove,
) {
    if move_def.id == crate::ids::MoveId::SURF {
        gulp_missile_dive(ctx, holder);
    }
}

/// Dive's `onTryMove` (DESIGN.md §43.4): the same, as the charge starts.
pub fn gulp_missile_dive(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.species_of(holder) != Some(SpeciesId::CRAMORANT) || ctx.has_transformed(holder) {
        return;
    }
    let (Some(hp), Some(max_hp)) = (ctx.hp(holder), ctx.max_hp(holder)) else {
        return;
    };
    // At or below half its HP it comes up Gorging, above it Gulping (integer division).
    let forme = if hp <= max_hp / 2 {
        SpeciesId::CRAMORANT_GORGING
    } else {
        SpeciesId::CRAMORANT_GULPING
    };
    ctx.forme_change(
        holder,
        forme,
        crate::engine::FormeChangeKind::InBattle,
        EffectSource::Ability {
            pokemon: holder,
            ability: AbilityId::GULP_MISSILE,
        },
    );
}

/// Gulp Missile's `onDamagingHit`: the catch is spat at whoever hurt it — a quarter of the
/// attacker's max HP, and a Defense drop when the catch was Gulping, paralysis when it was
/// Gorging — and the Cramorant is plain again.
pub fn gulp_missile_spit(ctx: &mut BattleContext<'_>, hit: crate::engine::DamagingHit) {
    let Some(forme) = ctx.species_of(hit.holder) else {
        return;
    };
    let gulping = forme == SpeciesId::CRAMORANT_GULPING;
    if !gulping && forme != SpeciesId::CRAMORANT_GORGING {
        return;
    }
    // A fainted or benched attacker is spared, and nothing is spat while the Cramorant is itself
    // semi-invulnerable (`target.isSemiInvulnerable()`: a No Guard hit reaching it underwater
    // mid-Dive, R-M11-67).
    if ctx.hp(hit.attacker).is_none_or(|hp| hp == 0)
        || !ctx.is_active(hit.attacker)
        || ctx.semi_invulnerable(hit.holder)
    {
        return;
    }
    let source = EffectSource::Ability {
        pokemon: hit.holder,
        ability: AbilityId::GULP_MISSILE,
    };
    if let Some(max_hp) = ctx.base_max_hp(hit.attacker) {
        ctx.apply_damage(crate::engine::DamageRequest {
            target: hit.attacker,
            amount: crate::engine::DamageAmount((max_hp / 4).max(1)),
            kind: crate::engine::DamageKind::Reaction,
            source,
            flags: crate::engine::DamageFlags {
                bypass_substitute: true,
                ..crate::engine::DamageFlags::default()
            },
        });
    }
    if gulping {
        ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
            target: hit.attacker,
            stat: crate::state::Stat::Defense,
            stages: -1,
            source,
            cause: crate::engine::StatChangeCause::Ability,
        });
    } else {
        ctx.try_set_status(
            hit.attacker,
            crate::state::NonVolatileStatus::Paralysis,
            source,
        );
    }
    ctx.forme_change(
        hit.holder,
        SpeciesId::CRAMORANT,
        crate::engine::FormeChangeKind::InBattle,
        source,
    );
}

/// Battle Bond: the holder knocked a Pokémon out with a move and still stands — once a battle.
/// Gen 7-8 it becomes Ash-Greninja for good; Gen 9 raises its Attack, Special Attack and Speed
/// one stage each.
pub fn battle_bond(
    ctx: &mut BattleContext<'_>,
    holder: PokemonId,
    cause: crate::state::FaintCause,
    _length: i8,
) {
    if !cause.by_move
        || ctx.bond_triggered(holder)
        || ctx.species_of(holder) != Some(SpeciesId::GRENINJA_BOND)
        || ctx.has_transformed(holder)
        || ctx.hp(holder).is_none_or(|hp| hp == 0)
        || !ctx.foe_pokemon_left(holder)
    {
        return;
    }
    ctx.mark_bond_triggered(holder);
    let source = EffectSource::Ability {
        pokemon: holder,
        ability: AbilityId::BATTLE_BOND,
    };
    if ctx
        .rules()
        .conditions()
        .forme
        .battle_bond_becomes_ash_greninja
    {
        ctx.forme_change(
            holder,
            SpeciesId::GRENINJA_ASH,
            crate::engine::FormeChangeKind::Permanent,
            source,
        );
        return;
    }
    ctx.ability_activated(holder, AbilityId::BATTLE_BOND);
    for stat in [
        crate::state::Stat::Attack,
        crate::state::Stat::SpecialAttack,
        crate::state::Stat::Speed,
    ] {
        ctx.try_change_stat_stage(crate::engine::StatChangeRequest {
            target: holder,
            stat,
            stages: 1,
            source,
            cause: crate::engine::StatChangeCause::Ability,
        });
    }
}

/// Zero to Hero's `onSwitchOut`: Palafin becomes its Hero forme for good as it leaves.
pub fn zero_to_hero_out(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.species_of(holder) == Some(SpeciesId::PALAFIN) {
        ctx.forme_change(
            holder,
            SpeciesId::PALAFIN_HERO,
            crate::engine::FormeChangeKind::Permanent,
            EffectSource::Ability {
                pokemon: holder,
                ability: AbilityId::ZERO_TO_HERO,
            },
        );
    }
}

/// The same ability's `onSwitchIn`: the Hero forme announces the change once since it took it.
pub fn zero_to_hero_in(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.species_of(holder) == Some(SpeciesId::PALAFIN_HERO) && !ctx.hero_announced(holder) {
        ctx.ability_activated(holder, AbilityId::ZERO_TO_HERO);
        ctx.mark_hero_announced(holder);
    }
}

/// Commander (DESIGN.md §43.4): a Tatsugiri beside a Dondozo takes it into its mouth. Showdown
/// runs this as the ability's `onStart`, at every `onAnySwitchIn` (priority −2) and at every
/// `Update`; the queue guard is the engine's own Update timing.
pub fn commander(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if ctx.rules().topology().active_slots_per_side() < 2 {
        return;
    }
    let Some(own) = ctx.own_species(holder) else {
        return;
    };
    let ally = ctx
        .adjacent_allies(holder)
        .into_iter()
        .next()
        .and_then(|ally| ctx.own_species(ally).map(|species| (ally, species)));
    let paired = own.same_species(SpeciesId::TATSUGIRI)
        && ally.is_some_and(|(_, species)| species.same_species(SpeciesId::DONDOZO));
    if !paired {
        // A Tatsugiri that lost its Dondozo (or the wrong species) is let go.
        if ctx.has_volatile(holder, crate::state::VolatileConditionKind::Commanding) {
            ctx.end_volatile(holder, crate::state::VolatileConditionKind::Commanding);
        }
        return;
    }
    let (ally, _) = ally.expect("paired");
    if ctx.has_volatile(holder, crate::state::VolatileConditionKind::Commanding) {
        // Already in its mouth: only a fainted Dondozo releases it.
        if ctx.hp(ally).is_none_or(|hp| hp > 0) {
            return;
        }
        ctx.end_volatile(holder, crate::state::VolatileConditionKind::Commanding);
        return;
    }
    // The Dondozo can hold one commander only.
    if ctx.has_volatile(ally, crate::state::VolatileConditionKind::Commanded) {
        return;
    }
    ctx.ability_activated(holder, AbilityId::COMMANDER);
    ctx.start_commanding(holder);
    ctx.start_commanded(ally);
}
