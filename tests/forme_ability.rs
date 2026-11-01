//! L1 tests for the abilities tied to one species' formes (DESIGN.md §43.4, R-N.3): what the
//! replay cannot see — the generation's values, a whole disguise's neutral effectiveness, and
//! the once-a-battle flags that outlive a switch.
use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId, SpeciesId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const USER: PokemonId = PokemonId(1);
const FOE: PokemonId = PokemonId(2);

fn rules(generation: Generation) -> Arc<ResolvedRules> {
    Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    )
}

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = 50;
    set
}

fn engine_for(
    generation: Generation,
    rules: &Arc<ResolvedRules>,
    user: &PokemonSet,
    foe: &PokemonSet,
) -> BattleEngine {
    build_engine(generation, rules, user, foe, None)
}

/// The same with a second Pokémon on the foe's side: a knock-out then leaves a foe standing,
/// which Battle Bond asks for (`source.side.foePokemonLeft()`).
fn engine_with_bench(
    generation: Generation,
    rules: &Arc<ResolvedRules>,
    user: &PokemonSet,
    foe: &PokemonSet,
) -> BattleEngine {
    build_engine(
        generation,
        rules,
        user,
        foe,
        Some(&set("Snorlax", &["Harden"])),
    )
}

fn build_engine(
    generation: Generation,
    rules: &Arc<ResolvedRules>,
    user: &PokemonSet,
    foe: &PokemonSet,
    bench: Option<&PokemonSet>,
) -> BattleEngine {
    let dex = Arc::new(DexResolver::resolve(generation));
    let bench_id = PokemonId(3);
    let mut pokemon = vec![
        build_pokemon(&dex, USER, SideId::P1, user).unwrap(),
        build_pokemon(&dex, FOE, SideId::P2, foe).unwrap(),
    ];
    let mut foe_team = vec![FOE];
    if let Some(bench) = bench {
        pokemon.push(build_pokemon(&dex, bench_id, SideId::P2, bench).unwrap());
        foe_team.push(bench_id);
    }
    let side = |id: PokemonId, team: Vec<PokemonId>| {
        let mut side = SideState::new(team, 1);
        side.active[0] = Some(id);
        side
    };
    BattleEngine::try_new(
        rules.clone(),
        dex,
        BattleState::new(pokemon, [side(USER, vec![USER]), side(FOE, foe_team)], 1),
    )
    .unwrap()
}

/// Both sides use their first move at each other, to the end of the turn.
fn play(engine: &mut BattleEngine, user_slot: MoveSlot, foe_slot: MoveSlot) {
    let topology = engine.rules().topology();
    let choice = |side: SideId, actor: PokemonId, target: SideId, slot: MoveSlot| SideChoice {
        side,
        actions: vec![SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot,
                target: Some(DeclaredTarget(topology.position(target, 0).unwrap())),
                mechanic: None,
            },
        }],
    };
    engine
        .submit_turn_choices([
            choice(SideId::P1, USER, SideId::P2, user_slot),
            choice(SideId::P2, FOE, SideId::P1, foe_slot),
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
}

/// The generation's values (DESIGN.md §43.4): Gen 7-8 Battle Bond becomes Ash-Greninja,
/// Gen 9 raises three stats; breaking Disguise costs an eighth of the max HP from Gen 8.
#[test]
fn the_forme_rules_follow_the_generation() {
    assert!(
        rules(Generation::Gen7)
            .conditions()
            .forme
            .battle_bond_becomes_ash_greninja
    );
    assert!(
        rules(Generation::Gen8)
            .conditions()
            .forme
            .battle_bond_becomes_ash_greninja
    );
    assert!(
        !rules(Generation::Gen9)
            .conditions()
            .forme
            .battle_bond_becomes_ash_greninja
    );
    assert!(!rules(Generation::Gen7).conditions().forme.disguise_costs_hp);
    assert!(rules(Generation::Gen8).conditions().forme.disguise_costs_hp);
    assert!(rules(Generation::Gen9).conditions().forme.disguise_costs_hp);
}

/// A whole disguise makes the move neutral (`onEffectiveness`), but not one the chart already
/// makes immune (`if (!target.runImmunity(move)) return;`): a Normal move still misses a
/// whole-disguise Mimikyu's Ghost type.
#[test]
fn a_whole_disguise_makes_the_hit_neutral_but_not_immune() {
    let rules = rules(Generation::Gen9);
    let dex = DexResolver::resolve(Generation::Gen9);
    let engine = engine_for(
        Generation::Gen9,
        &rules,
        &set("Mimikyu", &["Harden", "Tackle"]),
        &set("Snorlax", &["Harden"]),
    );
    let tackle = dex.move_by_name("Tackle").unwrap();
    let shadow_ball = dex.move_by_name("Shadow Ball").unwrap();
    let (_, neutral) =
        pokemon_battle_engine::engine::type_immunity(engine.state(), &rules, USER, tackle);
    // Tackle is Normal: Ghost is immune to it, and the disguise does not lift that.
    assert!(
        neutral.is_some(),
        "a Normal move still misses a whole-disguise Mimikyu"
    );
    // Shadow Ball is Ghost: normally 2× on Mimikyu's Ghost half, neutral while the disguise
    // holds.
    let (effectiveness, reason) =
        pokemon_battle_engine::engine::type_immunity(engine.state(), &rules, USER, shadow_ball);
    assert!(reason.is_none());
    assert_eq!(
        effectiveness,
        pokemon_battle_engine::rules::Fraction::ONE,
        "a whole disguise makes the hit neutral"
    );
}

/// Battle Bond fires on a knock-out the holder's move caused, and marks the Pokémon
/// (Showdown `bondTriggered`), which is what keeps it from firing twice in a battle.
#[test]
fn battle_bond_fires_on_a_knock_out_and_marks_the_holder() {
    let rules = rules(Generation::Gen9);
    let engine = engine_for(
        Generation::Gen9,
        &rules,
        &set("Greninja-Bond", &["Water Shuriken", "Harden"]),
        &set("Blissey", &["Harden"]),
    );
    let user = engine.state().pokemon(USER).unwrap();
    assert!(!user.bond_triggered);
    assert!(
        pokemon_battle_engine::engine::ability_behavior(
            &rules,
            pokemon_battle_engine::engine::raw_ability(user)
        )
        .battle_bond
    );
    // A level 5 Blissey falls to one Water Shuriken (Gen 9's rule: the stats rise, the forme
    // does not change), which sets the flag.
    let mut weak = set("Blissey", &["Harden"]);
    weak.level = 5;
    let mut engine = engine_with_bench(
        Generation::Gen9,
        &rules,
        &set("Greninja-Bond", &["Water Shuriken"]),
        &weak,
    );
    play(
        &mut engine,
        MoveSlot::new(0).unwrap(),
        MoveSlot::new(0).unwrap(),
    );
    assert!(
        engine.state().pokemon(USER).unwrap().bond_triggered,
        "a knock-out marks the holder"
    );
    assert_eq!(
        engine.state().pokemon(USER).unwrap().stages.attack,
        1,
        "Gen 9 raises its Attack, Special Attack and Speed instead of changing forme"
    );
}

/// Ice Face's own data: only a physical move breaks it, and nothing is charged for the break.
#[test]
fn ice_face_takes_physical_moves_only() {
    let rules = rules(Generation::Gen9);
    let dex = DexResolver::resolve(Generation::Gen9);
    let engine = engine_for(
        Generation::Gen9,
        &rules,
        &set("Eiscue", &["Harden"]),
        &set("Snorlax", &["Harden"]),
    );
    let absorbing = pokemon_battle_engine::engine::ability_behavior(
        &rules,
        pokemon_battle_engine::engine::raw_ability(engine.state().pokemon(USER).unwrap()),
    )
    .absorbing
    .expect("Ice Face absorbs");
    assert_eq!(absorbing.whole, SpeciesId::EISCUE);
    assert_eq!(absorbing.broken, SpeciesId::EISCUE_NOICE);
    assert!(absorbing.physical_only);
    assert!(absorbing.break_cost.is_none());
    assert_eq!(dex.species_by_name("Eiscue").unwrap().id, SpeciesId::EISCUE);
}

/// A whole disguise turns the critical hit back (`onCriticalHit` false), which the frames do
/// not show: Wicked Blow always crits, and no critical hit is registered against the Mimikyu.
#[test]
fn a_whole_disguise_blocks_the_critical_hit() {
    let rules = rules(Generation::Gen9);
    let mut engine = engine_for(
        Generation::Gen9,
        &rules,
        &set("Mimikyu", &["Harden", "Tackle"]),
        &set("Snorlax", &["Wicked Blow"]),
    );
    play(
        &mut engine,
        MoveSlot::new(0).unwrap(),
        MoveSlot::new(0).unwrap(),
    );
    let critical = engine.trace().events().iter().any(|event| {
        matches!(
            event,
            pokemon_battle_engine::engine::TraceEvent::CriticalChecked {
                target,
                critical: true,
                ..
            } if *target == USER
        )
    });
    assert!(
        !critical,
        "the disguise takes the hit whole: no critical hit is registered"
    );
}

/// Battle Bond asks for a foe still standing (`source.side.foePokemonLeft()`): with none, the
/// knock-out changes nothing.
#[test]
fn battle_bond_needs_a_foe_left_standing() {
    let rules = rules(Generation::Gen9);
    let mut weak = set("Blissey", &["Harden"]);
    weak.level = 5;
    let mut engine = engine_for(
        Generation::Gen9,
        &rules,
        &set("Greninja-Bond", &["Water Shuriken"]),
        &weak,
    );
    play(
        &mut engine,
        MoveSlot::new(0).unwrap(),
        MoveSlot::new(0).unwrap(),
    );
    let user = engine.state().pokemon(USER).unwrap();
    assert!(!user.bond_triggered);
    assert_eq!(user.stages.attack, 0, "no foe is left standing");
}

/// Ice Face takes physical moves whole only (`onEffectiveness` / `onCriticalHit`:
/// `move.category !== 'Physical'` returns): a special move against a whole Eiscue keeps its
/// effectiveness — Flamethrower is super effective on the Ice type — and its critical hits
/// (Showdown's `-supereffective` and `-crit` on a whole Eiscue, R-M9-14).
#[test]
fn ice_face_leaves_a_special_move_as_it_is() {
    use pokemon_battle_engine::engine::TraceEvent;
    let rules = rules(Generation::Gen9);
    let mut engine = engine_for(
        Generation::Gen9,
        &rules,
        &set("Eiscue", &["Harden"]),
        &set("Snorlax", &["Flamethrower"]),
    );
    play(
        &mut engine,
        MoveSlot::new(0).unwrap(),
        MoveSlot::new(0).unwrap(),
    );
    let effectiveness = engine
        .trace()
        .events()
        .iter()
        .find_map(|event| match event {
            TraceEvent::TypeEffectivenessResolved {
                target, modifier, ..
            } if *target == USER => Some(*modifier),
            _ => None,
        });
    assert_eq!(
        effectiveness,
        Some(pokemon_battle_engine::rules::Fraction::new(2, 1)),
        "a special move is not neutralised"
    );
    let eiscue = engine.state().pokemon(USER).unwrap();
    assert!(eiscue.hp < eiscue.max_hp, "a special move reaches the HP");
    assert_eq!(
        eiscue.species,
        Some(SpeciesId::EISCUE),
        "and breaks nothing"
    );
}

/// A Substitute in front of the whole forme takes the hit, which the forme never meets (`hitSub`
/// in Disguise's and Ice Face's `onCriticalHit` / `onEffectiveness`, R-M10-78): Wicked Blow's
/// critical hit lands on Mimikyu's Substitute, and a Rock move is super effective on Eiscue's —
/// save for a move that goes past the Substitute (`bypasssub`, Infiltrator's from Gen 6).
#[test]
fn a_substitute_in_front_of_the_whole_forme_takes_the_hit_as_it_is() {
    let rules = rules(Generation::Gen9);
    let dex = DexResolver::resolve(Generation::Gen9);
    let mut engine = engine_for(
        Generation::Gen9,
        &rules,
        &set("Mimikyu", &["Substitute"]),
        &set("Snorlax", &["Wicked Blow"]),
    );
    play(
        &mut engine,
        MoveSlot::new(0).unwrap(),
        MoveSlot::new(0).unwrap(),
    );
    let critical = engine.trace().events().iter().any(|event| {
        matches!(
            event,
            pokemon_battle_engine::engine::TraceEvent::CriticalChecked {
                target,
                critical: true,
                ..
            } if *target == USER
        )
    });
    assert!(critical, "the Substitute takes Wicked Blow's critical hit");
    let mut engine = engine_for(
        Generation::Gen9,
        &rules,
        &set("Eiscue", &["Substitute"]),
        &set("Snorlax", &["Harden"]),
    );
    play(
        &mut engine,
        MoveSlot::new(0).unwrap(),
        MoveSlot::new(0).unwrap(),
    );
    let rock_slide = dex.move_by_name("Rock Slide").unwrap();
    let (effectiveness, _) =
        pokemon_battle_engine::engine::type_immunity(engine.state(), &rules, USER, rock_slide);
    assert_eq!(
        effectiveness,
        pokemon_battle_engine::rules::Fraction::new(2, 1),
        "the Substitute takes the Rock move as it is"
    );
    let mut past_the_substitute = rock_slide.clone();
    past_the_substitute.flags.bypass_substitute = true;
    let (effectiveness, _) = pokemon_battle_engine::engine::type_immunity(
        engine.state(),
        &rules,
        USER,
        &past_the_substitute,
    );
    assert_eq!(
        effectiveness,
        pokemon_battle_engine::rules::Fraction::ONE,
        "past the Substitute the whole forme makes it neutral"
    );
}

/// Ice Face answers a fresh weather change, not the hail that counts again as a Cloud Nine
/// holder faints (its `End`'s `WeatherChange`, `suppressWeather`, R-M10-82): Golduck's Quick
/// Attack breaks the face first, Eiscue's Hail under Cloud Nine changes nothing, and once Headbutt knocks
/// Golduck out Eiscue stays Noice. Without Cloud Nine the Hail itself brings the face back.
#[test]
fn ice_face_does_not_answer_the_hail_a_fainting_cloud_nine_lets_back() {
    let rules = rules(Generation::Gen8);
    for (ability, whole_after) in [("Cloud Nine", false), ("Damp", true)] {
        let mut golduck = set("Golduck", &["Quick Attack"]);
        golduck.level = 5;
        golduck.ability = Some(ability.into());
        let mut eiscue = set("Eiscue", &["Hail", "Headbutt"]);
        eiscue.ability = Some("Ice Face".into());
        let mut engine = engine_for(Generation::Gen8, &rules, &eiscue, &golduck);
        play(
            &mut engine,
            MoveSlot::new(0).unwrap(),
            MoveSlot::new(0).unwrap(),
        );
        if whole_after {
            let forme =
                pokemon_battle_engine::engine::species_of(engine.state().pokemon(USER).unwrap());
            assert_eq!(
                forme,
                Some(SpeciesId::EISCUE),
                "a fresh Hail brings it back"
            );
            continue;
        }
        play(
            &mut engine,
            MoveSlot::new(1).unwrap(),
            MoveSlot::new(0).unwrap(),
        );
        assert!(engine.state().pokemon(FOE).unwrap().is_fainted());
        let forme =
            pokemon_battle_engine::engine::species_of(engine.state().pokemon(USER).unwrap());
        assert_eq!(forme, Some(SpeciesId::EISCUE_NOICE), "{ability}");
    }
}
