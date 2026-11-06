//! The move as it starts and as it will hit (DESIGN.md §41.2): `ModifyType` / `ModifyMove`
//! (`modified_move_cow`), PrepareHit's power (`prepared_move`: Fling, Natural Gift, Parental
//! Bond), Shell Side Arm's category by estimate, Beat Up's members, Terrain Pulse's type.

use super::*;

/// The move as its `onPrepareHit` handlers leave it, once the targets are known. The move's
/// own first: Fling's power from the item it throws (`fling_item`) and the item's status or
/// flinch as a secondary without a chance — unless Gen 4 Embargo on the target keeps it away
/// (`ItemTransferRules::embargo_stops_flung_effects`, issues.md I92); Natural Gift's power from
/// its berry (`natural_gift_berry`). Then Parental Bond's: the move strikes twice, the
/// second hit weakened (`MoveFlags::parental_bond`, `DamageRules::parental_bond_second_hit`) —
/// a damaging single-hit move that is neither a spread hit (`targets > 1`) nor exempt
/// (`MoveFlags::parental_bond_exempt`). `target`: the move's (first) target. Shared by the
/// Shell and the replay driver.
pub fn prepared_move(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    mut move_def: crate::dex::ResolvedMove,
    targets: usize,
    target: Option<crate::ids::PokemonId>,
) -> crate::dex::ResolvedMove {
    if let Some(pokemon) = state.pokemon(user) {
        if move_def.behavior.fling
            && let Some((_, item, power)) = fling_item(state, rules, pokemon)
        {
            move_def.base_power = power;
            let embargoed = rules.conditions.items.embargo_stops_flung_effects
                && target
                    .and_then(|target| state.pokemon(target))
                    .is_some_and(|target| {
                        target
                            .volatiles
                            .contains(crate::state::VolatileConditionKind::Embargo)
                    });
            if let Some(effect) = item.fling_effect
                && !embargoed
            {
                use crate::dex::{SecondaryEffect, SecondaryEffectSpec, SecondaryTarget};
                use crate::effects::items::FlingEffect;
                let secondary = match effect {
                    FlingEffect::Status(status) => Some(SecondaryEffect::Status(status)),
                    FlingEffect::Flinch => Some(SecondaryEffect::Volatile(
                        crate::state::VolatileConditionKind::Flinch,
                    )),
                    // `fling.effect` is the move's `onHit`, not a secondary.
                    FlingEffect::ClearLoweredStages | FlingEffect::CureMentalHerbVolatiles => {
                        move_def.behavior.flung_on_hit = Some(effect);
                        None
                    }
                };
                if let Some(effect) = secondary {
                    move_def.secondaries.push(SecondaryEffectSpec {
                        chance: crate::dex::EffectChance::CERTAIN,
                        target: SecondaryTarget::Target,
                        trigger: crate::dex::SecondaryTrigger::PerTargetHit,
                        effect,
                        requires: None,
                        shares_roll: false,
                        in_secondaries: true,
                        from_item: false,
                    });
                }
            }
        }
        if move_def.behavior.natural_gift
            && let Some((_, gift)) = natural_gift_berry(state, rules, pokemon)
        {
            move_def.base_power = gift.power;
        }
    }
    let bond = state
        .pokemon(user)
        .is_some_and(|pokemon| ability_handler(state, rules, pokemon, |b| b.parental_bond))
        // `!(move.isZ || move.isMax)`: a Z-powered move strikes once (DESIGN.md §44.4), and so
        // does a Z-Move or a Max Move a custom game put in a slot (R-M8-80).
        && !state.active_move.is_some_and(|active| active.z_powered)
        && move_def.z_crystal.is_none()
        && !move_def.is_max
        && move_def.category != crate::dex::MoveCategory::Status
        && move_def.multi_hit == crate::dex::MultiHit::Single
        && !move_def.behavior.beat_up
        && !move_def.flags.parental_bond_exempt
        && targets <= 1;
    if bond {
        move_def.multi_hit = crate::dex::MultiHit::Fixed(2);
        move_def.flags.parental_bond = true;
    }
    move_def
}

/// The move as `user` uses it right now (Showdown `ModifyType` / `ModifyMove` as it starts):
/// the move's own type and category (Weather Ball, Revelation Dance, Hidden Power, Gen 2-4
/// Beat Up), then the user's ability retyping it (Aerilate …, Liquid Voice, Normalize) — the
/// move's own events run before the Pokémon's; in Gen 1-3 a category the move does not set
/// itself follows the new type; Infiltrator's moves bypass a Substitute from Gen 6. `physical`:
/// the move's own `onModifyMove` made it physical against its target (Shell Side Arm — the
/// caller decides, `physical_by_estimate` or a tie's coin), before the ability edits the move.
/// Returns the ability that changed the type when it also boosts the move
/// (`typeChangerBoosted`). Shared by the Shell and the replay driver.
pub(crate) fn modified_move_cow<'a>(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    move_def: &'a crate::dex::ResolvedMove,
    physical: bool,
) -> (
    std::borrow::Cow<'a, crate::dex::ResolvedMove>,
    Option<crate::ids::AbilityId>,
) {
    use crate::dex::PokemonType;
    use crate::effects::abilities::TypeChangeScope;
    use crate::effects::moves::OwnType;
    let mut modified = std::borrow::Cow::Borrowed(move_def);
    let Some(pokemon) = state.pokemon(user) else {
        return (modified, None);
    };
    match move_def.behavior.own_type {
        None => {}
        Some(OwnType::Weather) => {
            // Delta Stream's winds change nothing (Weather Ball's lists do not name them).
            let weather = weather_for(state, rules, user)
                .filter(|kind| *kind != crate::state::WeatherKind::StrongWinds);
            if let Some(weather) = weather {
                modified.to_mut().move_type = match weather {
                    crate::state::WeatherKind::Sun => PokemonType::Fire,
                    crate::state::WeatherKind::Rain => PokemonType::Water,
                    crate::state::WeatherKind::Sandstorm => PokemonType::Rock,
                    crate::state::WeatherKind::Snow | crate::state::WeatherKind::StrongWinds => {
                        PokemonType::Ice
                    }
                };
                modified.to_mut().base_power = modified.base_power.saturating_mul(2);
            }
        }
        Some(OwnType::UserFirstType) => {
            modified.to_mut().move_type = crate::engine::types_of(state, rules, pokemon).primary
        }
        Some(OwnType::UserSpecies(types)) => {
            if let Some((_, move_type)) = types
                .iter()
                .find(|(species, _)| species_of(pokemon) == Some(*species))
            {
                modified.to_mut().move_type = *move_type;
            }
        }
        Some(OwnType::HeldItemType(gift)) => {
            use crate::effects::moves::ItemGift;
            let gift_type = match gift {
                // Judgment (`onModifyType`): `if (item.id && item.onPlate && !item.zMove)` — a
                // Z-Crystal types Arceus (Multitype reads its `plate` like a Plate's) but never
                // the move itself.
                ItemGift::Plate => {
                    let z_crystal = crate::engine::held_item(state, rules, pokemon.id())
                        .is_some_and(|(item, _)| {
                            crate::engine::item_behavior(rules, item).take_refusal
                                == Some(crate::effects::items::TakeRefusal::ZCrystal)
                        });
                    (!z_crystal)
                        .then(|| crate::engine::plate_type(state, rules, pokemon))
                        .flatten()
                }
                // Multi-Attack's Memory event (or Techno Blast's Drive event): the held item's
                // own `onMemory` / `onDrive` (a suppressed item has none: `held_item`). The
                // event's 'Normal' default leaves the move's own type, which is Normal anyway.
                ItemGift::Memory => crate::engine::memory_type(state, rules, pokemon),
                ItemGift::Drive => crate::engine::drive_type(state, rules, pokemon),
            };
            if let Some(type_) = gift_type {
                modified.to_mut().move_type = type_;
            }
        }
        Some(OwnType::HiddenPower) => modified.to_mut().move_type = pokemon.hidden_power_of().0,
        Some(OwnType::Terrain) => {
            if let Some(terrain) = state.field.terrain.map(|t| t.kind)
                && environment::grounded(state, rules, pokemon)
            {
                modified.to_mut().move_type = terrain_type(terrain);
                modified.to_mut().base_power = modified.base_power.saturating_mul(2);
            }
        }
        Some(OwnType::NaturalGift) => {
            if let Some((_, gift)) = natural_gift_berry(state, rules, pokemon) {
                modified.to_mut().move_type = gift.move_type;
            }
        }
    }
    // Sky Drop (`onModifyMove`): the turn that carries the target up is sure to hit and makes
    // no contact.
    if move_def.behavior.sky_drop
        && !pokemon
            .volatiles
            .contains(crate::state::VolatileConditionKind::SkyDrop)
    {
        modified.to_mut().accuracy = None;
        modified.to_mut().flags.contact = false;
    }
    // Spit Up on the Gen 3 cartridge (`Cmd_stockpiletobasedamage`, then `adjustsetdamage`; the
    // oracle's `cartridgeGen3GetDamage`, issues.md I218): never a critical hit, no random factor
    // (`CartridgeProfile::stockpile_after_formula`).
    if move_def.behavior.power_formula == Some(crate::effects::moves::PowerFormula::StockpileLayers)
        && matches!(rules.damage.algorithm,
            crate::rules::DamageAlgorithm::Cartridge(profile) if profile.stockpile_after_formula)
    {
        let spit_up = modified.to_mut();
        spit_up.will_crit = Some(false);
        spit_up.no_damage_variance = true;
    }
    // Curse (`onModifyMove`): a user that is not a Ghost type uses the other form, aimed where
    // `nonGhostTarget` says (itself).
    if let Some(pattern) = move_def.non_ghost_target
        && !crate::engine::has_type(state, rules, pokemon, PokemonType::Ghost)
    {
        modified.to_mut().target = pattern;
    }
    // Expanding Force: all adjacent foes, for a grounded user in its terrain.
    if let Some(kind) = move_def.behavior.terrain_spread
        && state.field.terrain.map(|t| t.kind) == Some(kind)
        && environment::grounded(state, rules, pokemon)
    {
        modified.to_mut().target = crate::rules::TargetPattern::AllAdjacentFoes;
        modified.to_mut().is_spread = true;
    }
    // A combined Water Pledge's side (`onModifyMove`): the side's moves double their
    // secondaries' chances (a Serene Grace flinch is left alone, DESIGN.md §42.9).
    if state
        .side(pokemon.side)
        .conditions
        .contains(crate::state::SideConditionKind::WaterPledge)
    {
        let serene =
            ability_handler(state, rules, pokemon, |b| b.secondary_chance_multiplier).is_some();
        for secondary in &mut modified.to_mut().secondaries {
            if serene
                && secondary.effect
                    == crate::dex::SecondaryEffect::Volatile(
                        crate::state::VolatileConditionKind::Flinch,
                    )
            {
                continue;
            }
            secondary.chance = secondary.chance.doubled();
        }
    }
    // Secret Power: the terrain's secondary in place of its own.
    if move_def.behavior.terrain_secondaries
        && let Some(terrain) = state.field.terrain.map(|t| t.kind)
    {
        use crate::dex::{SecondaryEffect, SecondaryEffectSpec, SecondaryTarget, SecondaryTrigger};
        let effect = match terrain {
            crate::state::TerrainKind::Electric => {
                SecondaryEffect::Status(crate::state::NonVolatileStatus::Paralysis)
            }
            crate::state::TerrainKind::Grassy => {
                SecondaryEffect::Status(crate::state::NonVolatileStatus::Sleep)
            }
            crate::state::TerrainKind::Misty => SecondaryEffect::StatStage {
                stat: crate::state::Stat::SpecialAttack,
                delta: -1,
            },
            crate::state::TerrainKind::Psychic => SecondaryEffect::StatStage {
                stat: crate::state::Stat::Speed,
                delta: -1,
            },
        };
        modified.to_mut().secondaries = vec![SecondaryEffectSpec {
            chance: crate::dex::EffectChance::percent(30),
            target: SecondaryTarget::Target,
            trigger: SecondaryTrigger::PerTargetHit,
            effect,
            requires: None,
            shares_roll: false,
            in_secondaries: true,
            from_item: false,
        }];
    }
    // Gen 2-4 Beat Up (its own `onModifyMove`): typeless, special until Gen 4 makes it
    // physical — a category it sets itself.
    let own_category = if move_def.behavior.beat_up {
        use crate::rules::BeatUpStyle;
        match rules.damage.beat_up {
            BeatUpStyle::Gen2 | BeatUpStyle::Gen3 => Some(crate::dex::MoveCategory::Special),
            BeatUpStyle::Gen4 => Some(crate::dex::MoveCategory::Physical),
            BeatUpStyle::Modern => None,
        }
    } else {
        None
    };
    if let Some(category) = own_category {
        modified.to_mut().move_type = PokemonType::Typeless;
        modified.to_mut().category = category;
    }
    // Gen 4 on the cartridge (issues.md I20): Beat Up's effect script sets
    // `SYSCTL_IGNORE_IMMUNITIES`, so no type immunity stops it — a Normalize user's Beat Up hits
    // a Ghost type.
    if move_def.behavior.beat_up && rules.damage.beat_up == crate::rules::BeatUpStyle::Gen4 {
        modified.to_mut().flags.type_immunity = false;
    }
    // Tera Blast and Tera Starstorm (`onModifyType` / `onModifyMove`, DESIGN.md §44.6): a
    // Terastallized Tera Blast takes the Tera type; a Terapagos-Stellar's Tera Starstorm is
    // Stellar and hits all adjacent foes. Both are physical when the user's Attack beats its
    // Special Attack at their stages without modifiers. Tera Blast re-reads its type below,
    // where a Normal-type ability would otherwise change it.
    // `onModifyType` / `onModifyMove`: the Stellar type and the all-adjacent-foes target belong
    // to Terapagos-Stellar itself; only the category needs it Terastallized.
    let stellar_species = move_def.behavior.tera_starstorm
        && species_of(pokemon) == Some(crate::ids::SpeciesId::TERAPAGOS_STELLAR);
    let stellar_user = stellar_species && pokemon.terastallized.is_some();
    if move_def.behavior.tera_blast {
        if let Some(tera) = pokemon.terastallized {
            modified.to_mut().move_type = tera;
        }
    } else if stellar_species {
        modified.to_mut().move_type = PokemonType::Stellar;
        modified.to_mut().target = crate::rules::TargetPattern::AllAdjacentFoes;
        modified.to_mut().is_spread = true;
    }
    if (move_def.behavior.tera_blast && pokemon.terastallized.is_some() || stellar_user)
        && stats::apply_stage(rules, pokemon.battle_stats().attack, pokemon.stages.attack)
            > stats::apply_stage(
                rules,
                pokemon.battle_stats().special_attack,
                pokemon.stages.special_attack,
            )
    {
        modified.to_mut().category = crate::dex::MoveCategory::Physical;
    }
    // Tera Blast's Stellar form: 100 power, and the user's Attack and Special Attack drop by
    // one once the move is over (the move data's `self`, added here as `onModifyMove` does).
    if move_def.behavior.tera_blast && pokemon.terastallized == Some(PokemonType::Stellar) {
        modified.to_mut().base_power = 100;
        for stat in [
            crate::state::Stat::Attack,
            crate::state::Stat::SpecialAttack,
        ] {
            modified
                .to_mut()
                .secondaries
                .push(crate::dex::SecondaryEffectSpec {
                    chance: crate::dex::EffectChance::percent(100),
                    target: crate::dex::SecondaryTarget::User,
                    trigger: crate::dex::SecondaryTrigger::OnceAfterMove,
                    effect: crate::dex::SecondaryEffect::StatStage { stat, delta: -1 },
                    requires: None,
                    shares_roll: false,
                    in_secondaries: false,
                    from_item: false,
                });
        }
    }
    // Photon Geyser (`onModifyMove`): physical when the user's Attack beats its Special Attack,
    // both at their stages and without modifiers (`getStat(.., false, true)`).
    if move_def.behavior.category_by_stats
        && stats::apply_stage(rules, pokemon.battle_stats().attack, pokemon.stages.attack)
            > stats::apply_stage(
                rules,
                pokemon.battle_stats().special_attack,
                pokemon.stages.special_attack,
            )
    {
        modified.to_mut().category = crate::dex::MoveCategory::Physical;
    }
    if physical {
        modified = std::borrow::Cow::Owned(made_physical(modified.into_owned()));
    }
    let mut changer = None;
    if let Some(change) = ability_handler(state, rules, pokemon, |b| b.modify_type) {
        let own_type = move_def
            .behavior
            .own_type
            .is_some_and(|own| own.spares_type_changers());
        // A Terastallized Tera Blast keeps its Tera type (every Normal-type ability's
        // `onModifyType` names it).
        let tera_blast_terastallized =
            move_def.behavior.tera_blast && pokemon.terastallized.is_some();
        // A Z-Move keeps its type (`!(move.isZ && move.category !== 'Status')`, every
        // type-changing ability's `onModifyType`, DESIGN.md §44.4).
        let z_move =
            move_def.z_crystal.is_some() && move_def.category != crate::dex::MoveCategory::Status;
        let applies = match change.scope {
            TypeChangeScope::NormalMoves => {
                modified.move_type == PokemonType::Normal
                    && !own_type
                    && !tera_blast_terastallized
                    && !z_move
            }
            // Liquid Voice (the only sound-scoped changer): a Dynamaxed holder's sound move keeps
            // its type (`!pokemon.volatiles['dynamax']` in its `onModifyType`, DESIGN.md §44.5) —
            // the Max Move it becomes follows the base move's own type.
            TypeChangeScope::SoundMoves => {
                move_def.flags.sound
                    && !pokemon
                        .volatiles
                        .contains(crate::state::VolatileConditionKind::Dynamax)
            }
            TypeChangeScope::AllMoves => {
                move_def.id != crate::ids::MoveId::STRUGGLE
                    && !(rules.damage.all_moves_type_change_spares_own_type && own_type)
                    && !tera_blast_terastallized
                    && !z_move
            }
        };
        if applies {
            modified.to_mut().move_type = change.to;
            if ability_handler(state, rules, pokemon, |b| b.type_change_boost) {
                changer = Some(raw_ability(pokemon));
            }
        }
    }
    // Ion Deluge (`onModifyType`, priority −2, after the ability's): a Normal move is Electric
    // for the rest of the turn.
    if modified.move_type == PokemonType::Normal
        && state
            .field
            .contains(crate::state::FieldConditionKind::IonDeluge)
    {
        modified.to_mut().move_type = PokemonType::Electric;
    }
    // Electrify (`onModifyType`, priority −2 as Ion Deluge's): the electrified Pokémon's move,
    // Struggle aside, is Electric.
    if pokemon
        .volatiles
        .contains(crate::state::VolatileConditionKind::Electrify)
        && move_def.id != crate::ids::MoveId::STRUGGLE
    {
        modified.to_mut().move_type = PokemonType::Electric;
    }
    if rules.damage.category_follows_type
        && own_category.is_none()
        && modified.category != crate::dex::MoveCategory::Status
        && modified.move_type != move_def.move_type
    {
        modified.to_mut().category = match modified.move_type {
            PokemonType::Fire
            | PokemonType::Water
            | PokemonType::Grass
            | PokemonType::Ice
            | PokemonType::Electric
            | PokemonType::Dark
            | PokemonType::Psychic
            | PokemonType::Dragon => crate::dex::MoveCategory::Special,
            _ => crate::dex::MoveCategory::Physical,
        };
    }
    // The user's ability editing the move (`onModifyMove` as data), on its final type.
    if let Some(edit) = ability_handler(state, rules, pokemon, |b| b.move_edit) {
        if edit.ignore_evasion {
            modified.to_mut().stat_sources.ignore_evasion = true;
        }
        if edit.normal_fighting_ignore_immunity
            && matches!(
                modified.move_type,
                PokemonType::Normal | PokemonType::Fighting
            )
        {
            modified.to_mut().flags.immunity_ignored = true;
        }
        if edit.no_contact {
            modified.to_mut().flags.contact = false;
        }
        if edit.tracks_target {
            modified.to_mut().tracks_target = true;
        }
        if edit.contact_ignores_protect && modified.flags.contact {
            modified.to_mut().flags.protectable = false;
        }
        if edit.max_hits {
            if let crate::dex::MultiHit::Range { max, .. } = modified.multi_hit {
                modified.to_mut().multi_hit = crate::dex::MultiHit::Fixed(max);
            }
            modified.to_mut().multi_accuracy = false;
        }
        // (`!move.hasSheerForceBoost`: a move Sheer Force only boosts keeps everything.) The
        // move's `self` goes with the secondaries, and so does its `selfBoost` (Showdown names
        // Clangorous Soulblaze, the one move with both `selfBoost` and secondaries; DESIGN.md
        // R-M8-84).
        if edit.sheer_force && modified.has_secondaries && !modified.flags.sheer_force_boost {
            modified.to_mut().secondaries.retain(|secondary| {
                !secondary.in_secondaries
                    && secondary.trigger == crate::dex::SecondaryTrigger::PerTargetHit
            });
            modified.to_mut().has_secondaries = false;
            modified.to_mut().flags.sheer_force = true;
        }
        if edit.ignores_abilities
            || (edit.status_ignores_abilities
                && modified.category == crate::dex::MoveCategory::Status)
        {
            modified.to_mut().ignores_ability = true;
        }
        if let Some(percent) = edit.flinch_chance
            && modified.category != crate::dex::MoveCategory::Status
            && !modified.secondaries.iter().any(|secondary| {
                secondary.effect
                    == crate::dex::SecondaryEffect::Volatile(
                        crate::state::VolatileConditionKind::Flinch,
                    )
            })
        {
            modified
                .to_mut()
                .secondaries
                .push(crate::dex::SecondaryEffectSpec {
                    chance: crate::dex::EffectChance::percent(percent),
                    target: crate::dex::SecondaryTarget::Target,
                    trigger: crate::dex::SecondaryTrigger::PerTargetHit,
                    effect: crate::dex::SecondaryEffect::Volatile(
                        crate::state::VolatileConditionKind::Flinch,
                    ),
                    requires: None,
                    shares_roll: false,
                    in_secondaries: true,
                    from_item: false,
                });
            modified.to_mut().has_secondaries = true;
        }
    }
    // The user's item editing the move (`onModifyMove` as data), after the ability: Loaded
    // Dice works on the hits Skill Link left (Showdown counts them in `hitStepMoveHitLoop`).
    if let Some(edit) = held_item(state, rules, user).and_then(|(_, item)| item.move_edit) {
        if edit.punch_no_contact && modified.flags.punch {
            modified.to_mut().flags.contact = false;
        }
        if edit.adds_flinch && modified.category != crate::dex::MoveCategory::Status {
            let flinch =
                crate::dex::SecondaryEffect::Volatile(crate::state::VolatileConditionKind::Flinch);
            let item_flinch = rules.hits.item_flinch;
            let adds = match item_flinch.moves {
                Some(moves) => moves.contains(&modified.id),
                None => !modified
                    .secondaries
                    .iter()
                    .any(|secondary| secondary.in_secondaries && secondary.effect == flinch),
            };
            if adds {
                modified
                    .to_mut()
                    .secondaries
                    .push(crate::dex::SecondaryEffectSpec {
                        chance: crate::dex::EffectChance::percent(item_flinch.chance_percent),
                        target: crate::dex::SecondaryTarget::Target,
                        trigger: crate::dex::SecondaryTrigger::PerTargetHit,
                        effect: flinch,
                        requires: None,
                        shares_roll: false,
                        in_secondaries: true,
                        from_item: true,
                    });
            }
        }
        if edit.loaded_dice {
            modified.to_mut().multi_accuracy = false;
            modified.to_mut().multi_hit = match modified.multi_hit {
                crate::dex::MultiHit::Range { min: 2, max: 5 } => {
                    crate::dex::MultiHit::Range { min: 4, max: 5 }
                }
                crate::dex::MultiHit::Fixed(10) => crate::dex::MultiHit::Range { min: 4, max: 10 },
                other => other,
            };
        }
    }
    // Infiltrator (`move.infiltrates`): from Gen 6 a Substitute lets the move through like a
    // `bypasssub` one (Showdown `substitute`'s `onTryPrimaryHit`).
    if ability_handler(state, rules, pokemon, |b| b.infiltrates)
        && rules.conditions.substitute_honors_infiltrates
    {
        modified.to_mut().flags.bypass_substitute = true;
    }
    (modified, changer)
}

/// `modified_move_cow`, owned: the move as its user's edits leave it. A caller that only reads
/// the result (the action's order) takes the `Cow` instead and copies nothing for a move no edit
/// touches (R-M10-10).
pub fn modified_move(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
    physical: bool,
) -> (crate::dex::ResolvedMove, Option<crate::ids::AbilityId>) {
    let (modified, changer) = modified_move_cow(state, rules, user, move_def, physical);
    (modified.into_owned(), changer)
}

/// A terrain's type (Terrain Pulse).
pub(crate) const fn terrain_type(terrain: crate::state::TerrainKind) -> crate::dex::PokemonType {
    match terrain {
        crate::state::TerrainKind::Electric => crate::dex::PokemonType::Electric,
        crate::state::TerrainKind::Grassy => crate::dex::PokemonType::Grass,
        crate::state::TerrainKind::Misty => crate::dex::PokemonType::Fairy,
        crate::state::TerrainKind::Psychic => crate::dex::PokemonType::Psychic,
    }
}

/// The first active Pokémon — side 1's positions, then side 2's — whose ability `pick`
/// accepts: the holder of an `onAny` handler (Showdown runs those from every active Pokémon).
pub(crate) fn active_holder(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    pick: impl Fn(&crate::effects::abilities::AbilityBehavior) -> bool,
) -> Option<(crate::ids::PokemonId, crate::ids::AbilityId)> {
    [crate::ids::SideId::P1, crate::ids::SideId::P2]
        .into_iter()
        .flat_map(|side| state.side(side).active.iter().flatten().copied())
        .filter_map(|id| {
            state
                .pokemon(id)
                .filter(|pokemon| !pokemon.is_fainted())
                .and_then(|pokemon| ability_running(state, rules, pokemon))
                .map(|ability| (id, ability))
        })
        .find(|(_, ability)| pick(ability_behavior(rules, *ability)))
}

/// Beat Up's members, one hit each, in party order (Showdown `side.pokemon`): neither fainted nor
/// statused — from Gen 5 the user whatever its status. Shared with the replay.
pub fn beat_up_members(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
) -> Vec<crate::ids::PokemonId> {
    let Some(side) = state.pokemon(user).map(|pokemon| pokemon.side) else {
        return Vec::new();
    };
    // Gen 5+, and Gen 4 on the cartridge (`BeatUpEligibleMon`: the user's own slot counts
    // whatever its status, issues.md I20): the user always takes part.
    let user_always = matches!(
        rules.damage.beat_up,
        crate::rules::BeatUpStyle::Gen4 | crate::rules::BeatUpStyle::Modern
    );
    state
        .side(side)
        .battle_order
        .iter()
        .copied()
        .filter(|member| {
            (user_always && *member == user)
                || state
                    .pokemon(*member)
                    .is_some_and(|pokemon| !pokemon.is_fainted() && pokemon.status.is_none())
        })
        .collect()
}

/// Shell Side Arm (`onModifyMove`): whether its physical estimate against `target` beats the
/// special one, `None` on a tie (a coin decides). Each estimate is
/// `⌊⌊⌊⌊2L/5 + 2⌋ × power × attack⌋ / defense⌋ / 50⌋` with the stats' stages and none of their
/// modifiers (Showdown `getStat(stat, false, true)`). Shared with the replay.
pub fn physical_by_estimate(
    state: &crate::state::BattleState,
    rules: &crate::rules::ResolvedRules,
    user: crate::ids::PokemonId,
    target: crate::ids::PokemonId,
    move_def: &crate::dex::ResolvedMove,
) -> Option<bool> {
    let (Some(user), Some(foe)) = (state.pokemon(user), state.pokemon(target)) else {
        return Some(false);
    };
    let staged = |value, stage| stats::stage_value(rules.stats.stage_table, value, stage);
    let estimate = |attack: u32, defense: u32| {
        (2 * u32::from(user.level) / 5 + 2) * u32::from(move_def.base_power) * attack
            / defense.max(1)
            / 50
    };
    let physical = estimate(
        staged(user.battle_stats().attack, user.stages.attack),
        staged(foe.battle_stats().defense, foe.stages.defense),
    );
    let special = estimate(
        staged(
            user.battle_stats().special_attack,
            user.stages.special_attack,
        ),
        staged(
            foe.battle_stats().special_defense,
            foe.stages.special_defense,
        ),
    );
    (physical != special).then_some(physical > special)
}

/// The move made physical, with contact (Shell Side Arm's `onModifyMove`).
fn made_physical(mut move_def: crate::dex::ResolvedMove) -> crate::dex::ResolvedMove {
    move_def.category = crate::dex::MoveCategory::Physical;
    move_def.flags.contact = true;
    move_def
}
