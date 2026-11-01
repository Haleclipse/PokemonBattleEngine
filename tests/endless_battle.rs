//! L1 tests for the rules that end a battle at a turn's end (DESIGN.md §45.7), on the parts the
//! replays do not reach.
//! - The staleness writers: none shows in a battle until Endless Battle Clause reads it, and
//!   the two replayed battles (`m9-endless-battle-clause*`) only use Heal Pulse.
//! - The clause's other branches: the turn-100 wait, the switch check, doubles, berry-cycling
//!   in either order.
//! - Gen 1's other no-progress cases, and the turn limit, which a fixture would need 1000 turns
//!   to reach.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleEndRule, EngineStep, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{BattleClauses, DeclaredTarget, FormatRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleOutcome, BattleState, NonVolatileStatus, SideState, Staleness, StatusState, WeatherKind,
};

const P1: PokemonId = PokemonId(1);

fn set(species: &str, moves: &[&str], ability: Option<&str>, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = ability.map(Into::into);
    set.item = item.map(Into::into);
    set
}

/// A battle between `p1` and `p2` (P1's Pokémon numbered from 1, then P2's), Endless Battle
/// Clause on when `clause` says, `prepare` run on the state before it starts, the leads in.
fn battle(
    generation: Generation,
    mode: BattleMode,
    teams: [Vec<PokemonSet>; 2],
    clause: bool,
    prepare: impl FnOnce(&mut BattleState),
) -> BattleEngine {
    let format = FormatRules {
        clauses: BattleClauses {
            endless_battle_clause: clause,
            ..BattleClauses::default()
        },
        ..FormatRules::custom()
    };
    let rules = Arc::new(
        RulesResolver::resolve_format(&BattleSpec::new(generation, mode, "custom"), &format)
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let slots = if mode == BattleMode::Doubles { 2 } else { 1 };
    let mut pokemon = Vec::new();
    let mut next = 1u8;
    let sides = [SideId::P1, SideId::P2].map(|side| {
        let sets = &teams[side.index()];
        let ids: Vec<PokemonId> = sets
            .iter()
            .map(|set| {
                let id = PokemonId(next);
                next += 1;
                pokemon.push(build_pokemon(&dex, id, side, set).unwrap());
                id
            })
            .collect();
        let mut state = SideState::new(ids.clone(), slots);
        for (slot, id) in ids.iter().take(slots).enumerate() {
            state.active[slot] = Some(*id);
        }
        state
    });
    let mut state = BattleState::new(pokemon, sides, 1);
    prepare(&mut state);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine.run_lead_switch_ins();
    engine
}

/// `actor`'s move in `slot` (from 0) at the Pokémon in `target`'s first slot, or at no one.
fn uses(engine: &BattleEngine, actor: u8, slot: u8, target: Option<SideId>) -> SlotChoice {
    SlotChoice {
        actor: PokemonId(actor),
        action: ActionChoice::Move {
            slot: MoveSlot::new(slot).unwrap(),
            target: target
                .map(|side| DeclaredTarget(engine.rules().topology().position(side, 0).unwrap())),
            mechanic: None,
        },
    }
}

/// One turn with these choices, run to the next request or the battle's end.
fn turn(engine: &mut BattleEngine, p1: Vec<SlotChoice>, p2: Vec<SlotChoice>) {
    engine
        .submit_turn_choices([
            SideChoice {
                side: SideId::P1,
                actions: p1,
            },
            SideChoice {
                side: SideId::P2,
                actions: p2,
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = engine.step().unwrap() {}
}

fn staleness(engine: &BattleEngine, id: u8) -> (Option<Staleness>, Option<Staleness>, bool) {
    let pokemon = engine.state().pokemon(PokemonId(id)).unwrap();
    (
        pokemon.staleness,
        pokemon.volatile_staleness,
        pokemon.pending_staleness,
    )
}

const NONE: (Option<Staleness>, Option<Staleness>, bool) = (None, None, false);
const EXTERNAL: (Option<Staleness>, Option<Staleness>, bool) =
    (Some(Staleness::External), None, false);

/// The rule that ended the battle and the outcome that follows it, from the trace.
fn ended_by(engine: &BattleEngine) -> Option<(BattleEndRule, BattleOutcome)> {
    let events = engine.trace().events();
    let at = events
        .iter()
        .position(|event| matches!(event, TraceEvent::BattleEndedByRule { .. }))?;
    let TraceEvent::BattleEndedByRule { rule } = events[at] else {
        unreachable!()
    };
    events[at..].iter().find_map(|event| match event {
        TraceEvent::BattleFinished { outcome } => Some((rule, *outcome)),
        _ => None,
    })
}

// --- The staleness writers --------------------------------------------------------------------

#[test]
fn heal_pulse_leaves_a_healed_foe_stale_but_not_a_full_one_or_an_ally() {
    let teams = || {
        [
            vec![
                set("Chansey", &["Heal Pulse", "Splash"], None, None),
                set("Snorlax", &["Splash"], None, None),
            ],
            vec![
                set("Snorlax", &["Splash"], None, None),
                set("Snorlax", &["Splash"], None, None),
            ],
        ]
    };
    let hurt = |state: &mut BattleState, id: u8| {
        let pokemon = state.pokemon_mut(PokemonId(id)).unwrap();
        pokemon.hp = pokemon.max_hp / 2;
    };
    // A hurt foe: stale by a foe's doing.
    let mut engine = battle(Generation::Gen9, BattleMode::Doubles, teams(), false, |s| {
        hurt(s, 3)
    });
    let foe = Some(SideId::P2);
    let p1 = vec![uses(&engine, 1, 0, foe), uses(&engine, 2, 0, None)];
    let p2 = vec![uses(&engine, 3, 0, None), uses(&engine, 4, 0, None)];
    turn(&mut engine, p1, p2);
    assert_eq!(staleness(&engine, 3), EXTERNAL);
    assert_eq!(staleness(&engine, 1), NONE);
    // A foe at full HP: the heal fails, nothing changes.
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Doubles,
        teams(),
        false,
        |_| {},
    );
    let p1 = vec![uses(&engine, 1, 0, foe), uses(&engine, 2, 0, None)];
    let p2 = vec![uses(&engine, 3, 0, None), uses(&engine, 4, 0, None)];
    turn(&mut engine, p1, p2);
    assert_eq!(staleness(&engine, 3), NONE);
    // A hurt ally: healed, not stale.
    let mut engine = battle(Generation::Gen9, BattleMode::Doubles, teams(), false, |s| {
        hurt(s, 2)
    });
    let ally = DeclaredTarget(engine.rules().topology().position(SideId::P1, 1).unwrap());
    let mut heal_ally = uses(&engine, 1, 0, None);
    if let ActionChoice::Move { target, .. } = &mut heal_ally.action {
        *target = Some(ally);
    }
    let p1 = vec![heal_ally, uses(&engine, 2, 0, None)];
    let p2 = vec![uses(&engine, 3, 0, None), uses(&engine, 4, 0, None)];
    turn(&mut engine, p1, p2);
    let ally = engine.state().pokemon(PokemonId(2)).unwrap();
    assert_eq!(ally.hp, ally.max_hp);
    assert_eq!(staleness(&engine, 2), NONE);
}

#[test]
fn a_leppa_berry_eaten_off_another_leaves_its_owner_or_its_eater_stale() {
    let singles = |p1: PokemonSet, p2: PokemonSet| {
        battle(
            Generation::Gen9,
            BattleMode::Singles,
            [vec![p1], vec![p2]],
            false,
            |_| {},
        )
    };
    // Bug Bite takes the target's Leppa Berry and eats it: the target is stale.
    for (berry, expected) in [("Leppa Berry", EXTERNAL), ("Sitrus Berry", NONE)] {
        let mut engine = singles(
            set("Snorlax", &["Bug Bite"], None, None),
            set("Blissey", &["Splash"], None, Some(berry)),
        );
        let (p1, p2) = (
            uses(&engine, 1, 0, Some(SideId::P2)),
            uses(&engine, 2, 0, None),
        );
        turn(&mut engine, vec![p1], vec![p2]);
        assert_eq!(
            engine.state().pokemon(PokemonId(2)).unwrap().held_item,
            None
        );
        assert_eq!(staleness(&engine, 2), expected, "{berry}");
        assert_eq!(staleness(&engine, 1), NONE, "{berry}");
    }
    // Fling throws a Leppa Berry, and the target eats it: the target is stale.
    let mut engine = singles(
        set("Snorlax", &["Fling"], None, Some("Leppa Berry")),
        set("Blissey", &["Splash"], None, None),
    );
    let (p1, p2) = (
        uses(&engine, 1, 0, Some(SideId::P2)),
        uses(&engine, 2, 0, None),
    );
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(staleness(&engine, 2), EXTERNAL);
    // The user is not stale. (Showdown's `setItem('')` leaves it a pending mark that nothing
    // reads: its next item comes through `setItem`, which writes the mark again.)
    assert_eq!(staleness(&engine, 1).0, None);
}

#[test]
fn an_ability_a_foe_gives_leaves_the_taker_stale_while_it_stays_in() {
    let teams = |p1: PokemonSet| {
        [
            vec![p1],
            vec![
                set("Snorlax", &["Splash"], Some("Thick Fat"), None),
                set("Snorlax", &["Splash"], Some("Thick Fat"), None),
            ],
        ]
    };
    // Skill Swap: the target is stale, the user's own volatile staleness goes.
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        teams(set(
            "Slowbro",
            &["Skill Swap", "Splash"],
            Some("Oblivious"),
            None,
        )),
        false,
        |state| {
            state.pokemon_mut(P1).unwrap().volatile_staleness = Some(Staleness::External);
        },
    );
    let (p1, p2) = (
        uses(&engine, 1, 0, Some(SideId::P2)),
        uses(&engine, 2, 0, None),
    );
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(
        staleness(&engine, 2),
        (None, Some(Staleness::External), false)
    );
    assert_eq!(staleness(&engine, 1), NONE);
    // Leaving the field ends it.
    let p1 = uses(&engine, 1, 1, None);
    let p2 = SlotChoice {
        actor: PokemonId(2),
        action: ActionChoice::Switch {
            pokemon: PokemonId(3),
        },
    };
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(staleness(&engine, 2), NONE);
    // Entrainment.
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        teams(set("Snorlax", &["Entrainment"], Some("Immunity"), None)),
        false,
        |_| {},
    );
    let (p1, p2) = (
        uses(&engine, 1, 0, Some(SideId::P2)),
        uses(&engine, 2, 0, None),
    );
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(engine.state().pokemon(PokemonId(2)).unwrap().ability.0, {
        engine.state().pokemon(P1).unwrap().ability.0
    });
    assert_eq!(
        staleness(&engine, 2),
        (None, Some(Staleness::External), false)
    );
    // Wandering Spirit trades with a contact move's user: the holder is the one left stale.
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        [
            vec![set("Snorlax", &["Tackle"], Some("Thick Fat"), None)],
            vec![set("Snorlax", &["Splash"], Some("Wandering Spirit"), None)],
        ],
        false,
        |_| {},
    );
    let (p1, p2) = (
        uses(&engine, 1, 0, Some(SideId::P2)),
        uses(&engine, 2, 0, None),
    );
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(
        staleness(&engine, 2),
        (None, Some(Staleness::External), false)
    );
    assert_eq!(staleness(&engine, 1), NONE);
}

#[test]
fn a_restorative_berry_an_item_change_gave_makes_its_eater_stale_by_its_own_doing() {
    let half = |state: &mut BattleState, id: u8| {
        let pokemon = state.pokemon_mut(PokemonId(id)).unwrap();
        pokemon.hp = pokemon.max_hp / 2 - 1;
    };
    // A berry it started with: nothing.
    let engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        [
            vec![set("Snorlax", &["Splash"], None, Some("Sitrus Berry"))],
            vec![set("Snorlax", &["Splash"], None, None)],
        ],
        false,
        |state| half(state, 1),
    );
    assert_eq!(engine.state().pokemon(P1).unwrap().held_item, None);
    assert_eq!(staleness(&engine, 1), NONE);
    // Recycle gives it back (`setItem`), and eating it again makes it stale.
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        [
            vec![set("Snorlax", &["Recycle"], None, Some("Sitrus Berry"))],
            vec![set("Snorlax", &["Splash"], None, None)],
        ],
        false,
        |state| half(state, 1),
    );
    let pokemon = engine.state().pokemon(P1).unwrap();
    assert_eq!(pokemon.history.last_item, Some(ItemId::SITRUS_BERRY));
    let (p1, p2) = (uses(&engine, 1, 0, None), uses(&engine, 2, 0, None));
    // At 3/4 HP after the first Sitrus Berry, the second waits: it is held, pending.
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(staleness(&engine, 1), (None, None, true));
    // Trick swaps a Sitrus Berry onto a hurt foe, which eats it: its own doing in Showdown's
    // `setItem`, which no caller gives a source.
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        [
            vec![set("Slowbro", &["Trick"], None, Some("Sitrus Berry"))],
            vec![set("Snorlax", &["Splash"], None, None)],
        ],
        false,
        |state| half(state, 2),
    );
    let (p1, p2) = (
        uses(&engine, 1, 0, Some(SideId::P2)),
        uses(&engine, 2, 0, None),
    );
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(
        engine.state().pokemon(PokemonId(2)).unwrap().held_item,
        None
    );
    assert_eq!(
        staleness(&engine, 2),
        (Some(Staleness::Internal), None, false)
    );
    // A foe's doing stays: eating such a berry does not overwrite it.
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        [
            vec![set("Slowbro", &["Trick"], None, Some("Sitrus Berry"))],
            vec![set("Snorlax", &["Splash"], None, None)],
        ],
        false,
        |state| {
            half(state, 2);
            state.pokemon_mut(PokemonId(2)).unwrap().staleness = Some(Staleness::External);
        },
    );
    let (p1, p2) = (
        uses(&engine, 1, 0, Some(SideId::P2)),
        uses(&engine, 2, 0, None),
    );
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(staleness(&engine, 2), EXTERNAL);
    // Harvest grows the berry back (`setItem`): eaten, the holder is stale.
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        [
            vec![set("Exeggutor", &["Splash"], Some("Harvest"), None)],
            vec![set("Snorlax", &["Splash"], None, None)],
        ],
        false,
        |state| {
            half(state, 1);
            state.pokemon_mut(P1).unwrap().history.last_item = Some(ItemId::SITRUS_BERRY);
            state.field.set_weather(WeatherKind::Sun, PokemonId(2), 5);
        },
    );
    let (p1, p2) = (uses(&engine, 1, 0, None), uses(&engine, 2, 0, None));
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(
        staleness(&engine, 1),
        (Some(Staleness::Internal), None, false)
    );
}

#[test]
fn an_item_taken_away_takes_its_pending_staleness_with_it() {
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        [
            vec![set("Snorlax", &["Knock Off"], None, None)],
            vec![set("Blissey", &["Splash"], None, Some("Leftovers"))],
        ],
        false,
        |state| state.pokemon_mut(PokemonId(2)).unwrap().pending_staleness = true,
    );
    let (p1, p2) = (
        uses(&engine, 1, 0, Some(SideId::P2)),
        uses(&engine, 2, 0, None),
    );
    turn(&mut engine, vec![p1], vec![p2]);
    assert_eq!(
        engine.state().pokemon(PokemonId(2)).unwrap().held_item,
        None
    );
    assert_eq!(staleness(&engine, 2), NONE);
}

// --- Endless Battle Clause ---------------------------------------------------------------------

/// Two sides of `p1` and `p2` (Splash first), the clause on, at the end of turn `turn - 1`
/// with each Pokémon given the staleness `stale` names (by 1-based id).
fn stale_battle(
    mode: BattleMode,
    teams: [Vec<PokemonSet>; 2],
    turn: u32,
    stale: &[(u8, Staleness)],
    clause: bool,
) -> BattleEngine {
    battle(Generation::Gen9, mode, teams, clause, |state| {
        state.turn = turn;
        for &(id, staleness) in stale {
            state.pokemon_mut(PokemonId(id)).unwrap().staleness = Some(staleness);
        }
    })
}

/// A turn of Splash (each Pokémon's first move) from these Pokémon.
fn splash_turn(engine: &mut BattleEngine, p1: &[u8], p2: &[u8]) {
    let splash = |ids: &[u8]| -> Vec<SlotChoice> {
        ids.iter().map(|&id| uses(engine, id, 0, None)).collect()
    };
    let (p1, p2) = (splash(p1), splash(p2));
    turn(engine, p1, p2);
}

fn snorlax() -> PokemonSet {
    set("Snorlax", &["Splash"], Some("Thick Fat"), Some("Leftovers"))
}

use Staleness::{External, Internal};

#[test]
fn endless_battle_clause_ends_a_stale_battle_once_turn_100_is_over() {
    let teams = || [vec![snorlax()], vec![snorlax()]];
    let stale = [(1, External), (2, External)];
    let mut engine = stale_battle(BattleMode::Singles, teams(), 99, &stale, true);
    splash_turn(&mut engine, &[1], &[2]);
    assert_eq!(engine.state().outcome, None, "turn 100 is about to start");
    splash_turn(&mut engine, &[1], &[2]);
    assert_eq!(engine.state().turn, 101);
    assert_eq!(
        ended_by(&engine),
        Some((
            BattleEndRule::EndlessBattle {
                berry_cycling: [false, false]
            },
            BattleOutcome::Draw
        ))
    );
    // Without the clause the battle goes on.
    let mut engine = stale_battle(BattleMode::Singles, teams(), 100, &stale, false);
    splash_turn(&mut engine, &[1], &[2]);
    assert_eq!(engine.state().outcome, None);
}

#[test]
fn endless_battle_clause_needs_every_side_stale_and_one_by_a_foe_s_doing() {
    let teams = || [vec![snorlax()], vec![snorlax()]];
    for (stale, ends) in [
        (&[(1, Internal), (2, Internal)][..], false),
        (&[(1, External)][..], false),
        (&[(1, Internal), (2, External)][..], true),
    ] {
        let mut engine = stale_battle(BattleMode::Singles, teams(), 100, stale, true);
        splash_turn(&mut engine, &[1], &[2]);
        assert_eq!(engine.state().outcome.is_some(), ends, "{stale:?}");
    }
    // A foe's ability counts while the Pokémon stays in (`volatileStaleness`).
    let mut engine = battle(
        Generation::Gen9,
        BattleMode::Singles,
        teams(),
        true,
        |state| {
            state.turn = 100;
            state.pokemon_mut(PokemonId(1)).unwrap().volatile_staleness = Some(External);
            state.pokemon_mut(PokemonId(2)).unwrap().staleness = Some(Internal);
        },
    );
    splash_turn(&mut engine, &[1], &[2]);
    assert_eq!(engine.state().outcome, Some(BattleOutcome::Draw));
}

#[test]
fn endless_battle_clause_waits_while_every_side_can_switch_to_a_pokemon_not_stale() {
    let stale = [(1, External), (3, External)];
    // Both sides have one on the bench: the battle goes on.
    let teams = || [vec![snorlax(), snorlax()], vec![snorlax(), snorlax()]];
    let mut engine = stale_battle(BattleMode::Singles, teams(), 100, &stale, true);
    splash_turn(&mut engine, &[1], &[3]);
    assert_eq!(engine.state().outcome, None);
    // A Pokémon that must recharge is held to its action, not trapped (Showdown's
    // `mustrecharge` has no `TrapPokemon`): its side can still switch, and the battle goes on.
    let teams = || {
        [
            vec![
                set("Snorlax", &["Hyper Beam"], Some("Thick Fat"), None),
                snorlax(),
            ],
            vec![snorlax(), snorlax()],
        ]
    };
    let mut engine = stale_battle(BattleMode::Singles, teams(), 100, &stale, true);
    let (p1, p2) = (
        uses(&engine, 1, 0, Some(SideId::P2)),
        uses(&engine, 3, 0, None),
    );
    turn(&mut engine, vec![p1], vec![p2]);
    assert!(
        engine
            .state()
            .pokemon(P1)
            .unwrap()
            .volatiles
            .contains(pokemon_battle_engine::state::VolatileConditionKind::MustRecharge)
    );
    assert_eq!(engine.state().outcome, None);
    // One side's bench Pokémon has fainted: that side cannot switch.
    let mut engine = battle(Generation::Gen9, BattleMode::Singles, teams(), true, |s| {
        s.turn = 100;
        s.pokemon_mut(PokemonId(1)).unwrap().staleness = Some(External);
        s.pokemon_mut(PokemonId(3)).unwrap().staleness = Some(External);
        s.pokemon_mut(PokemonId(4)).unwrap().hp = 0;
    });
    splash_turn(&mut engine, &[1], &[3]);
    assert_eq!(engine.state().outcome, Some(BattleOutcome::Draw));
    // A side held by a foe's Shadow Tag cannot switch either.
    let teams = [
        vec![snorlax(), snorlax()],
        vec![
            set("Snorlax", &["Splash"], Some("Shadow Tag"), None),
            snorlax(),
        ],
    ];
    let mut engine = stale_battle(BattleMode::Singles, teams, 100, &stale, true);
    splash_turn(&mut engine, &[1], &[3]);
    assert_eq!(engine.state().outcome, Some(BattleOutcome::Draw));
}

#[test]
fn a_doubles_side_is_stale_with_one_stale_pokemon_on_the_field() {
    let teams = || [vec![snorlax(), snorlax()], vec![snorlax(), snorlax()]];
    // P1's second Pokémon is not stale and could stay; P2's are both stale, so P2 cannot switch
    // to one that is not.
    let stale = [(1, External), (3, External), (4, External)];
    let mut engine = stale_battle(BattleMode::Doubles, teams(), 100, &stale, true);
    splash_turn(&mut engine, &[1, 2], &[3, 4]);
    assert_eq!(engine.state().outcome, Some(BattleOutcome::Draw));
}

#[test]
fn a_side_that_started_with_berry_cycling_loses_in_showdown_s_order() {
    let sitrus = || {
        set(
            "Snorlax",
            &["Splash"],
            Some("Thick Fat"),
            Some("Sitrus Berry"),
        )
    };
    let recycler = || set("Snorlax", &["Splash", "Recycle"], Some("Thick Fat"), None);
    let harvester = || {
        set(
            "Exeggutor",
            &["Splash"],
            Some("Harvest"),
            Some("Sitrus Berry"),
        )
    };
    // P1 and P2 each lead one Pokémon; the other is fainted on the bench.
    let outcome = |p1: Vec<PokemonSet>, p2: Vec<PokemonSet>| {
        let bench = [2, 4];
        let mut engine = battle(
            Generation::Gen9,
            BattleMode::Singles,
            [p1, p2],
            true,
            |state| {
                state.turn = 100;
                state.pokemon_mut(PokemonId(1)).unwrap().staleness = Some(External);
                state.pokemon_mut(PokemonId(3)).unwrap().staleness = Some(External);
                for id in bench {
                    state.pokemon_mut(PokemonId(id)).unwrap().hp = 0;
                }
            },
        );
        splash_turn(&mut engine, &[1], &[3]);
        ended_by(&engine)
    };
    let cycling = |p1, p2, outcome| {
        Some((
            BattleEndRule::EndlessBattle {
                berry_cycling: [p1, p2],
            },
            outcome,
        ))
    };
    // A Recycle user before the berry: P1 cycles berries and loses.
    assert_eq!(
        outcome(vec![recycler(), sitrus()], vec![snorlax(), snorlax()]),
        cycling(true, false, BattleOutcome::Winner(SideId::P2))
    );
    // The berry before the Recycle user: P1 does not count.
    assert_eq!(
        outcome(vec![sitrus(), recycler()], vec![snorlax(), snorlax()]),
        cycling(false, false, BattleOutcome::Draw)
    );
    // Harvest with the berry on one Pokémon: P2 loses.
    assert_eq!(
        outcome(vec![snorlax(), snorlax()], vec![harvester(), snorlax()]),
        cycling(false, true, BattleOutcome::Winner(SideId::P1))
    );
    // Both sides: a tie.
    assert_eq!(
        outcome(vec![harvester(), snorlax()], vec![harvester(), snorlax()]),
        cycling(true, true, BattleOutcome::Draw)
    );
}

// --- The turn limit and Gen 1's no-progress tie --------------------------------------------------

#[test]
fn turn_1000_s_end_is_a_tie_with_or_without_the_clause() {
    for clause in [false, true] {
        let mut engine = stale_battle(
            BattleMode::Singles,
            [vec![snorlax()], vec![snorlax()]],
            999,
            &[],
            clause,
        );
        splash_turn(&mut engine, &[1], &[2]);
        assert_eq!(engine.state().outcome, None, "turn 1000 is about to start");
        splash_turn(&mut engine, &[1], &[2]);
        assert_eq!(
            ended_by(&engine),
            Some((BattleEndRule::TurnLimit, BattleOutcome::Draw))
        );
    }
}

#[test]
fn gen_1_battles_that_cannot_progress_are_a_tie_from_the_start() {
    let frozen = |state: &mut BattleState, ids: &[u8]| {
        for &id in ids {
            state.pokemon_mut(PokemonId(id)).unwrap().status =
                Some(StatusState::new(NonVolatileStatus::Freeze));
        }
    };
    let gen1 = |teams: [Vec<PokemonSet>; 2], prepare: &dyn Fn(&mut BattleState)| {
        battle(Generation::Gen1, BattleMode::Singles, teams, false, prepare)
    };
    let mon = |species: &str, moves: &[&str]| set(species, moves, None, None);
    // Every Pokémon frozen (the bench too) or fainted.
    let engine = gen1(
        [
            vec![
                mon("Snorlax", &["Body Slam"]),
                mon("Chansey", &["Seismic Toss"]),
            ],
            vec![mon("Snorlax", &["Body Slam"])],
        ],
        &|state| {
            frozen(state, &[1, 3]);
            state.pokemon_mut(PokemonId(2)).unwrap().hp = 0;
        },
    );
    assert_eq!(
        ended_by(&engine),
        Some((BattleEndRule::NoProgress, BattleOutcome::Draw))
    );
    // A Pokémon that is not, on either side: the battle goes on.
    let engine = gen1(
        [
            vec![
                mon("Snorlax", &["Body Slam"]),
                mon("Chansey", &["Seismic Toss"]),
            ],
            vec![mon("Snorlax", &["Body Slam"])],
        ],
        &|state| frozen(state, &[1, 3]),
    );
    assert_eq!(engine.state().outcome, None);
    // Only Transform on both sides.
    let engine = gen1(
        [
            vec![mon("Ditto", &["Transform"])],
            vec![mon("Mew", &["Transform"])],
        ],
        &|_| {},
    );
    assert_eq!(
        ended_by(&engine),
        Some((BattleEndRule::NoProgress, BattleOutcome::Draw))
    );
    // Only Transform against a foe that is frozen but knows more: the Transform user can still
    // copy it and move.
    let engine = gen1(
        [
            vec![mon("Ditto", &["Transform"])],
            vec![mon("Snorlax", &["Body Slam"])],
        ],
        &|state| frozen(state, &[2]),
    );
    assert_eq!(engine.state().outcome, None);
    // One side that can do more than Transform.
    let engine = gen1(
        [
            vec![mon("Ditto", &["Transform"])],
            vec![mon("Mew", &["Transform", "Pound"])],
        ],
        &|_| {},
    );
    assert_eq!(engine.state().outcome, None);
    // Out of PP against Ghosts, on one side only: the other can still hurt it.
    let engine = gen1(
        [
            vec![mon("Gengar", &["Night Shade"])],
            vec![mon("Gengar", &["Night Shade"])],
        ],
        &|state| {
            for slot in state.pokemon_mut(P1).unwrap().moves.iter_mut() {
                slot.pp = 0;
            }
        },
    );
    assert_eq!(engine.state().outcome, None);
    // From Gen 2 no such check runs.
    let engine = battle(
        Generation::Gen2,
        BattleMode::Singles,
        [
            vec![mon("Snorlax", &["Body Slam"])],
            vec![mon("Snorlax", &["Body Slam"])],
        ],
        false,
        |state| frozen(state, &[1, 2]),
    );
    assert_eq!(engine.state().outcome, None);
}
