//! What an observer sees (DESIGN.md §47.0 items 8, 9 and 11, M10.4b): the view borrows the
//! battle, shows a player their own side as it is, the omniscient view everything, and of the
//! other side — a spectator of both — only what is public; changing what is hidden changes
//! nothing an observer sees (§47.0 item 11, constraint 7).

#[path = "support/policy.rs"]
mod policy;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::ids::SideId;
use pokemon_battle_engine::spec::Generation;
use pokemon_battle_engine::state::{BattleState, Knowledge, PokemonState, Stat};
use pokemon_battle_engine::view::{BattleView, Hp, Observer, PokemonView, Seen};
use policy::{Policy, answer, start_battle};

const STATS: [Stat; 7] = [
    Stat::Attack,
    Stat::Defense,
    Stat::SpecialAttack,
    Stat::SpecialDefense,
    Stat::Speed,
    Stat::Accuracy,
    Stat::Evasion,
];

/// The moves a view shows — each move, its PP, the uses seen — and whether that is all of them.
type MovesShown = (Vec<(u16, Seen<(u8, u8)>, u16)>, bool);

/// Everything a view shows of one Pokémon, comparable.
#[derive(Debug, PartialEq)]
struct Shown {
    id: u8,
    species: Option<(u16, u8)>,
    level: u8,
    hp: Hp,
    fainted: bool,
    status: Option<String>,
    boosts: Vec<i8>,
    tera: Option<String>,
    item: Seen<u16>,
    ability: Seen<u16>,
    moves: MovesShown,
    volatiles: Vec<String>,
}

fn shown(pokemon: PokemonView<'_>) -> Shown {
    Shown {
        id: pokemon.id().0,
        species: pokemon.species().map(|s| (s.dex, s.form)),
        level: pokemon.level(),
        hp: pokemon.hp(),
        fainted: pokemon.fainted(),
        status: pokemon.status().map(|s| format!("{s:?}")),
        boosts: STATS.iter().map(|stat| pokemon.boost(*stat)).collect(),
        tera: pokemon.terastallized().map(|t| format!("{t:?}")),
        item: match pokemon.item() {
            Seen::Known(item) => Seen::Known(item.0),
            Seen::KnownNone => Seen::KnownNone,
            Seen::Unknown => Seen::Unknown,
        },
        ability: match pokemon.ability() {
            Seen::Known(ability) => Seen::Known(ability.0),
            Seen::KnownNone => Seen::KnownNone,
            Seen::Unknown => Seen::Unknown,
        },
        moves: {
            let moves = pokemon.moves();
            let revealed = moves
                .revealed()
                .map(|m| (m.move_id().0, m.pp(), m.observed_uses()))
                .collect();
            (revealed, moves.complete())
        },
        volatiles: pokemon
            .volatiles()
            .iter()
            .map(|v| format!("{v:?}"))
            .collect(),
    }
}

/// A side as a view shows it: who stands where, and the team.
fn side_shown(view: BattleView<'_>, side: SideId) -> (Vec<Option<Shown>>, Vec<Shown>, usize) {
    let side_view = view.side(side);
    (
        side_view
            .active()
            .map(|(_, standing)| standing.map(shown))
            .collect(),
        side_view.team().map(shown).collect(),
        side_view.pokemon_left(),
    )
}

/// What the battle itself holds of `pokemon`, as an all-knowing eye shows it: its slots, with the
/// uses everyone saw.
fn truth(knowledge: &Knowledge, pokemon: &PokemonState) -> Shown {
    Shown {
        id: pokemon.id().0,
        species: pokemon_battle_engine::engine::species_of(pokemon).map(|s| (s.dex, s.form)),
        level: pokemon.level,
        hp: Hp::Exact {
            hp: pokemon.hp,
            max: pokemon.max_hp,
        },
        fainted: pokemon.is_fainted(),
        status: pokemon.status.as_ref().map(|s| format!("{:?}", s.kind)),
        boosts: STATS.iter().map(|stat| pokemon.stages.get(*stat)).collect(),
        tera: pokemon.terastallized.map(|t| format!("{t:?}")),
        item: pokemon
            .held_item
            .map_or(Seen::KnownNone, |item| Seen::Known(item.0)),
        ability: Seen::Known(pokemon.battle_ability().0),
        moves: (
            pokemon
                .move_slots()
                .iter()
                .map(|m| {
                    (
                        m.move_id.0,
                        Seen::Known((m.pp, m.max_pp)),
                        knowledge.observed_uses(pokemon.id(), m.move_id),
                    )
                })
                .collect(),
            true,
        ),
        volatiles: pokemon
            .volatiles
            .entries()
            .iter()
            .map(|v| format!("{:?}", v.kind()))
            .collect(),
    }
}

/// The moves an observer that does not know `pokemon`'s side sees (DESIGN.md §47.0 item 13), from
/// the knowledge (no Illusion in these battles: the one seen is itself): standing transformed into
/// one of the observer's own, its slots; else the moves seen and those one of its own took from it
/// by Transform, all of them once four were seen or one of its own took them.
fn public_moves(
    state: &BattleState,
    observer: Option<Observer>,
    pokemon: &PokemonState,
) -> MovesShown {
    let knowledge = &state.knowledge;
    let id = pokemon.id();
    if let Some(into) = knowledge.transformed_into(id)
        && let Some(Observer::Side(own)) = observer
        && state.pokemon(into).is_some_and(|into| into.side == own)
    {
        let slots = pokemon
            .move_slots()
            .iter()
            .map(|m| {
                (
                    m.move_id.0,
                    Seen::Unknown,
                    knowledge.observed_uses(id, m.move_id),
                )
            })
            .collect();
        return (slots, true);
    }
    let sees = |by: pokemon_battle_engine::ids::PokemonId| matches!(observer, Some(Observer::Side(own)) if state.pokemon(by).is_some_and(|by| by.side == own));
    let seen: Vec<_> = knowledge
        .moves_seen(id, id, sees)
        .map(|(move_id, uses)| (move_id.0, Seen::Unknown, uses))
        .collect();
    let owner = knowledge.transformed_into(id).unwrap_or(id);
    let complete = seen.len() >= 4 || knowledge.moves_copied_by(owner, sees);
    (seen, complete)
}

/// How a format shares the other side's HP (Showdown `getHealth`).
#[derive(Debug, Clone, Copy)]
enum Sharing {
    Percent,
    Pixels,
}

/// Showdown's shared HP of `hp` out of `max`: a percentage rounded up, 99 while not full; or
/// 48ths rounded down, 1 at the least while it stands.
fn shared(sharing: Sharing, hp: u16, max: u16) -> Hp {
    let (hp, max) = (u32::from(hp), u32::from(max));
    match sharing {
        Sharing::Percent if hp == 0 => Hp::Percent(0),
        Sharing::Pixels if hp == 0 => Hp::Pixels(0),
        Sharing::Percent => {
            let percent = (hp * 100).div_ceil(max);
            Hp::Percent(if percent == 100 && hp < max {
                99
            } else {
                percent as u8
            })
        }
        Sharing::Pixels => Hp::Pixels((hp * 48 / max).max(1) as u8),
    }
}

/// Checks every view of `engine` at this moment against its state.
fn check(engine: &BattleEngine, sharing: Sharing) {
    let state = engine.state();
    for side in [SideId::P1, SideId::P2] {
        let team: Vec<&PokemonState> = state
            .side(side)
            .team
            .iter()
            .filter_map(|id| state.pokemon(*id))
            .collect();
        // Its own player, and the omniscient eye, see the side as it is (no Illusion fools them).
        for view in [engine.view(Observer::Side(side)), engine.omniscient_view()] {
            let (_, members, left) = side_shown(view, side);
            let expected: Vec<Shown> = team.iter().map(|p| truth(&state.knowledge, p)).collect();
            assert_eq!(members, expected, "{side:?} as it is");
            assert_eq!(left, state.living_team_members(side).len());
        }
        // The other player and a spectator see what is public.
        for view in [
            engine.view(Observer::Side(side.other())),
            engine.view(Observer::Spectator),
        ] {
            let (active, members, _) = side_shown(view, side);
            for shown in members.iter().chain(active.iter().flatten()) {
                // What was seen named or lost, else nothing yet (no Illusion in these battles:
                // the one seen is itself) — never what it holds unseen.
                let revealed = state
                    .knowledge
                    .revealed(pokemon_battle_engine::ids::PokemonId(shown.id));
                let item = match revealed.item() {
                    Some(Some(item)) => Seen::Known(item.0),
                    Some(None) => Seen::KnownNone,
                    None => Seen::Unknown,
                };
                let ability = revealed
                    .ability()
                    .map_or(Seen::Unknown, |ability| Seen::Known(ability.0));
                assert_eq!((&shown.item, &shown.ability), (&item, &ability));
            }
            // The members that came out — those on the bench since too.
            let appeared: Vec<&&PokemonState> = team
                .iter()
                .filter(|p| state.knowledge.has_appeared(p.id()))
                .collect();
            assert_eq!(members.len(), appeared.len(), "the members that came out");
            // Independently of the knowledge: every Pokémon on the field, and every one that
            // fainted (it was on it), has come out.
            for pokemon in &team {
                if pokemon.is_fainted() || state.side(side).is_active(pokemon.id()) {
                    assert!(
                        state.knowledge.has_appeared(pokemon.id()),
                        "{:?} on the field, not shown",
                        pokemon.id()
                    );
                }
            }
            for (member, pokemon) in members.iter().zip(appeared) {
                // No Illusion in these battles: what it appears to be is what it is.
                let real = truth(&state.knowledge, pokemon);
                assert_eq!(member.id, real.id);
                assert_eq!(member.moves, public_moves(state, view.observer(), pokemon));
                // Independently of the knowledge: a move revealed is one it has — standing, one it
                // battles with (Transform's, Mimic's); off the field, its own.
                let has: Vec<u16> = if state.side(side).is_active(pokemon.id()) {
                    pokemon.move_slots().iter().map(|m| m.move_id.0).collect()
                } else {
                    pokemon.base_slots().map(|m| m.move_id.0).collect()
                };
                for (move_id, _, _) in &member.moves.0 {
                    assert!(
                        has.contains(move_id),
                        "{:?}: move {move_id} revealed, not held",
                        pokemon.id()
                    );
                }
                // Its volatiles: standing, those seen on it — each one it has; off the field, none.
                let standing = state.side(side).is_active(pokemon.id());
                let seen: Vec<String> = if standing {
                    state
                        .knowledge
                        .volatiles_seen(pokemon.id())
                        .map(|v| format!("{v:?}"))
                        .collect()
                } else {
                    Vec::new()
                };
                assert_eq!(member.volatiles, seen);
                // Each one it has — but one whose end can show nothing, seen on until it leaves,
                // acts or the turn ends (Gen 1 Bide's to a full paralysis too).
                let visibility = &engine.rules().conditions().volatile_visibility;
                for kind in state.knowledge.volatiles_seen(pokemon.id()) {
                    let may_linger = visibility.ending_unseen.contains(kind)
                        || visibility.until_it_acts.contains(kind)
                        || visibility.for_the_turn.contains(kind)
                        || kind == pokemon_battle_engine::state::VolatileConditionKind::Bide;
                    assert!(
                        !standing || real.volatiles.contains(&format!("{kind:?}")) || may_linger,
                        "{:?}: {kind:?} shown, not on",
                        pokemon.id()
                    );
                }
                // On the field as it is; off it as it was seen leaving (a heal it took as it
                // left, Regenerator's, unseen) — a faint is seen.
                let off_field = !state.side(side).is_active(pokemon.id()) && !pokemon.is_fainted();
                let (hp, status) = match state.knowledge.sighting(pokemon.id()) {
                    Some(seen) if off_field => (
                        shared(sharing, seen.hp, seen.max_hp),
                        seen.status.map(|s| format!("{s:?}")),
                    ),
                    _ => (
                        shared(sharing, pokemon.hp, pokemon.max_hp),
                        real.status.clone(),
                    ),
                };
                assert_eq!(member.hp, hp);
                assert_eq!(
                    (
                        &member.status,
                        &member.boosts,
                        &member.fainted,
                        &member.tera
                    ),
                    (&status, &real.boosts, &real.fainted, &real.tera)
                );
            }
        }
    }
}

/// Changes what P2's Pokémon hide — their items, abilities, moves' PP, volatiles — and checks
/// that P1 and a spectator see exactly what they saw.
fn hidden_changes_nothing(engine: &BattleEngine) {
    let before: Vec<_> = [Observer::Side(SideId::P1), Observer::Spectator]
        .iter()
        .map(|observer| side_shown(engine.view(*observer), SideId::P2))
        .collect();
    let mut snapshot = engine.snapshot();
    let team = snapshot.state.side(SideId::P2).team.clone();
    for id in team {
        let pokemon = snapshot.state.pokemon_mut(id).expect("a member");
        pokemon.held_item = None;
        pokemon.ability = pokemon_battle_engine::ids::AbilityId(1);
        for slot in pokemon.move_slots_mut() {
            slot.pp = slot.pp.saturating_sub(1);
        }
        pokemon.volatiles.set_taunt(3);
    }
    let changed = BattleEngine::restore(
        std::sync::Arc::new(engine.rules().clone()),
        std::sync::Arc::new(engine.dex().clone()),
        snapshot,
    )
    .expect("the changed battle");
    let after: Vec<_> = [Observer::Side(SideId::P1), Observer::Spectator]
        .iter()
        .map(|observer| side_shown(changed.view(*observer), SideId::P2))
        .collect();
    assert_eq!(before, after, "a hidden change showed");
}

#[test]
fn each_observer_sees_what_it_knows() {
    // HP Percentage Mod (Standard has it), or any format from Gen 7 (Showdown `getHealth`'s
    // `gen >= 7`; Gen 7's VGC has no such mod); Gen 6's VGC shares 48ths.
    for (format_id, sharing) in [
        ("gen9ou", Sharing::Percent),
        ("gen9vgc2025regi", Sharing::Percent),
        ("gen7vgc2018", Sharing::Percent),
        ("gen6vgc2016", Sharing::Pixels),
        ("gen4ou", Sharing::Percent),
        ("gen1ou", Sharing::Percent),
    ] {
        let (rules, dex, start) = start_battle(format_id);
        let mut engine = BattleEngine::restore(rules, dex, start).expect("the battle's start");
        let mut policy = Policy(2);
        let mut turns = 0;
        check(&engine, sharing);
        while answer(&mut engine, &mut policy).is_some() {
            check(&engine, sharing);
            if turns % 5 == 0 {
                hidden_changes_nothing(&engine);
            }
            turns += 1;
        }
        check(&engine, sharing);
        assert!(turns > 3, "{format_id}");
    }
}

#[test]
fn a_view_borrows_the_battle() {
    let (rules, dex, start) = start_battle("gen9ou");
    let engine = BattleEngine::restore(rules, dex, start).expect("the battle's start");
    let view = engine.view(Observer::Side(SideId::P1));
    assert_eq!(view.observer(), Some(Observer::Side(SideId::P1)));
    assert_eq!(engine.omniscient_view().observer(), None);
    assert_eq!(view.turn(), engine.state().turn);
    assert_eq!(view.generation(), Generation::Gen9);
}

/// The events `push_with` may leave unbuilt show nothing (DESIGN.md §47.0 item 12): the trace's
/// mode never changes what an observer learns.
#[test]
fn the_events_an_off_trace_skips_disclose_nothing() {
    use pokemon_battle_engine::engine::TraceEvent;
    use pokemon_battle_engine::ids::{MoveId, PokemonId};
    let a = PokemonId(0);
    for event in [
        TraceEvent::TargetsResolved {
            actor: a,
            move_id: MoveId(1),
            targets: Vec::new(),
        },
        TraceEvent::SwitchInTiesDrawn { tied: Vec::new() },
    ] {
        assert_eq!(event.disclosures(), [None, None], "{event:?}");
    }
}

/// The other side's HP as the format shares it (Showdown `getHealth`, DESIGN.md M10.4c): exactly,
/// as a percentage rounded up (99 short of full), or in 48ths rounded down (1 at the least
/// while it stands).
#[test]
fn the_other_sides_hp_is_shared_as_the_format_says() {
    use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
    use pokemon_battle_engine::ids::PokemonId;
    use pokemon_battle_engine::rules::{FormatRules, HpReport, RulesResolver};
    use pokemon_battle_engine::spec::{BattleMode, BattleSpec};
    use pokemon_battle_engine::state::{BattleState, SideState};
    use std::sync::Arc;

    let battle = |report: HpReport, foe_hp: u16| {
        let spec = BattleSpec::new(Generation::Gen9, BattleMode::Singles, "custom");
        let format = FormatRules {
            hp_report: report,
            ..FormatRules::custom()
        };
        let rules = Arc::new(RulesResolver::resolve_format(&spec, &format).unwrap());
        let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
        let set = |species: &str| {
            let mut set = PokemonSet::new(species, ["Harden"]);
            set.level = 50;
            set
        };
        let mut pokemon = vec![
            build_pokemon(&dex, PokemonId(0), SideId::P1, &set("Snorlax")).unwrap(),
            build_pokemon(&dex, PokemonId(1), SideId::P2, &set("Chansey")).unwrap(),
        ];
        pokemon[1].hp = foe_hp;
        let max = pokemon[1].max_hp;
        let mut p1 = SideState::new(vec![PokemonId(0)], 1);
        p1.active[0] = Some(PokemonId(0));
        let mut p2 = SideState::new(vec![PokemonId(1)], 1);
        p2.active[0] = Some(PokemonId(1));
        let mut engine =
            BattleEngine::try_new(rules, dex, BattleState::new(pokemon, [p1, p2], 1)).unwrap();
        engine.run_lead_switch_ins();
        let shown = engine
            .view(Observer::Side(SideId::P1))
            .side(SideId::P2)
            .active()
            .find_map(|(_, standing)| standing)
            .unwrap()
            .hp();
        (shown, max)
    };
    let (_, max) = battle(HpReport::Exact, 1);
    assert_eq!(battle(HpReport::Exact, 100).0, Hp::Exact { hp: 100, max });
    // Percentages: rounded up, 99 one short of full.
    assert_eq!(battle(HpReport::Percentage, max).0, Hp::Percent(100));
    assert_eq!(battle(HpReport::Percentage, max - 1).0, Hp::Percent(99));
    assert_eq!(
        battle(HpReport::Percentage, 100).0,
        Hp::Percent(((100 * 100) as u32).div_ceil(u32::from(max)) as u8)
    );
    assert_eq!(battle(HpReport::Percentage, 1).0, Hp::Percent(1));
    // 48ths: rounded down, 1 at the least.
    assert_eq!(battle(HpReport::Pixels, max).0, Hp::Pixels(48));
    assert_eq!(
        battle(HpReport::Pixels, 100).0,
        Hp::Pixels((100 * 48 / u32::from(max)) as u8)
    );
    assert_eq!(battle(HpReport::Pixels, 1).0, Hp::Pixels(1));
}
