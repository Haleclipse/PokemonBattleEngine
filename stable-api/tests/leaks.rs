//! What an observer may not know does not show (DESIGN.md §47.0 item 11, constraint 7): two
//! battles alike but for what one of P2's members keeps hidden — its item, nature, EVs and Tera
//! type — look the same to P1 and to a spectator, step by step, as long as that member has not
//! come in; to P2 and to the omniscient eye they do not. Through the stable API alone, next to
//! the replay's comparison with Showdown's per-side protocol (`tests/support/views.rs`).

mod common;

use common::{Policy, library, next_request, teams};
use pokemon_battle_engine::view::{BattleView, Observer, PokemonView};
use pokemon_battle_engine::{
    BattleEngine, BattleRequest, BattleSession, EngineStep, ForcedReplacement, ForcedSwitchChoice,
    ReviveChoice, SessionFormat, SessionRequest, SideId, Stat, TeamChoice, TeamSet,
};

/// Items for the other set, the first the format admits taken (another than the set's own, none
/// another member holds where Item Clause asks so). The member never comes in while compared:
/// its item does nothing.
const OTHER_ITEMS: [&str; 6] = [
    "Leftovers",
    "Life Orb",
    "Choice Scarf",
    "Focus Sash",
    "Sitrus Berry",
    "Expert Belt",
];

/// The member as another hidden set: another item (the `nth` of `OTHER_ITEMS`), the EVs of
/// Attack and Special Attack swapped, another nature, another Tera type.
fn hidden_variant(set: &TeamSet, nth: usize) -> Option<TeamSet> {
    let mut other = set.clone();
    if OTHER_ITEMS[nth] == set.item {
        return None;
    }
    other.item = OTHER_ITEMS[nth].to_string();
    if let Some(evs) = &mut other.evs {
        std::mem::swap(&mut evs.atk, &mut evs.spa);
    }
    other.nature = if set.nature == "Adamant" {
        "Modest"
    } else {
        "Adamant"
    }
    .into();
    if !set.tera_type.is_empty() {
        other.tera_type = if set.tera_type == "Ghost" {
            "Steel"
        } else {
            "Ghost"
        }
        .into();
    }
    Some(other)
}

/// P2's last member holding an item (one that holds none stays so: a Gen 4-7 preview shows
/// whether it holds one).
fn hidden_member(format_id: &str) -> usize {
    teams(format_id)[1]
        .iter()
        .rposition(|set| !set.item.is_empty())
        .expect("a member holding an item")
}

/// The battle a session of `format_id` starts, P2's `member` as `change` makes it; `None` when
/// the format refuses that set.
fn started(
    format_id: &str,
    member: usize,
    change: impl Fn(&TeamSet) -> Option<TeamSet>,
) -> Option<BattleEngine> {
    let mut session = BattleSession::new(SessionFormat::new(library(), format_id).unwrap(), 11);
    let mut sides = teams(format_id);
    let p2 = sides.pop().unwrap();
    let p1 = sides.pop().unwrap();
    let mut p2_changed = p2.clone();
    p2_changed[member] = change(&p2[member])?;
    session.register(SideId::P1, p1).unwrap();
    session.register(SideId::P2, p2_changed).ok()?;
    if let SessionRequest::Choose { sides, .. } = session.request() {
        for side in sides {
            session.choose(side, TeamChoice::default()).ok()?;
        }
    }
    session.engine().cloned()
}

/// Everything the view shows, written out.
fn shown(view: &BattleView<'_>) -> String {
    let mut out = format!("turn {} {:?}\n", view.turn(), view.phase());
    let field = view.field();
    out += &format!(
        "field {:?} {:?} {:?}\n",
        field.weather(),
        field.terrain(),
        field.conditions().collect::<Vec<_>>()
    );
    for side in [SideId::P1, SideId::P2] {
        let side = view.side(side);
        out += &format!(
            "{:?} left {} conditions {:?}\n",
            side.id(),
            side.pokemon_left(),
            side.conditions().collect::<Vec<_>>()
        );
        for (position, standing) in side.active() {
            out += &format!("  at {position:?}: {:?}\n", standing.map(|p| p.id()));
        }
        for member in side.team() {
            out += &pokemon(&member);
        }
        for member in side.previewed() {
            out += &format!(
                "  previewed {:?} {} {} {:?} {:?} {:?} {:?} {:?}\n",
                member.species(),
                member.forme_hidden(),
                member.level(),
                member.gender(),
                member.item(),
                member.ability(),
                member.moves(),
                member.tera_type()
            );
        }
    }
    out
}

fn pokemon(pokemon: &PokemonView<'_>) -> String {
    let stats = [
        Stat::Attack,
        Stat::Defense,
        Stat::SpecialAttack,
        Stat::SpecialDefense,
        Stat::Speed,
        Stat::Accuracy,
        Stat::Evasion,
    ];
    let moves = pokemon.moves();
    format!(
        "  {:?} {:?} L{} {:?} {:?} fainted {} {:?} {:?} tera {:?} item {:?} ability {:?} moves {:?} complete {} volatiles {:?}\n",
        pokemon.id(),
        pokemon.species(),
        pokemon.level(),
        pokemon.gender(),
        pokemon.hp(),
        pokemon.fainted(),
        pokemon.status(),
        stats.map(|stat| pokemon.boost(stat)),
        pokemon.terastallized(),
        pokemon.item(),
        pokemon.ability(),
        moves
            .revealed()
            .map(|m| (m.move_id(), m.pp(), m.observed_uses()))
            .collect::<Vec<_>>(),
        moves.complete(),
        pokemon.volatiles().iter().collect::<Vec<_>>()
    )
}

/// One step of both battles with the same choices (a seeded policy reading the requests and
/// `choice_options`, which the hidden set does not change): false once either is over.
fn step_both(engines: &mut [BattleEngine; 2], policy: &mut [Policy; 2]) -> bool {
    let mut requests = Vec::new();
    for engine in engines.iter_mut() {
        requests.push(next_request(engine));
    }
    assert_eq!(
        format!("{:?}", requests[0]),
        format!("{:?}", requests[1]),
        "the same request"
    );
    for (engine, policy) in engines.iter_mut().zip(policy.iter_mut()) {
        match &requests[0] {
            EngineStep::Finished(_) => return false,
            EngineStep::Awaiting(BattleRequest::TurnChoices { .. }) => {
                let choices = [
                    policy.turn(engine, SideId::P1),
                    policy.turn(engine, SideId::P2),
                ];
                engine.submit_turn_choices(choices).unwrap();
            }
            EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) => {
                // The first Pokémon it may send in: the last member stays out while another can.
                let choices = replacements.clone().map(|requirement| ForcedSwitchChoice {
                    side: requirement.side,
                    replacements: requirement
                        .positions
                        .iter()
                        .take(requirement.required_count)
                        .zip(&requirement.available_pokemon)
                        .map(|(slot, pokemon)| ForcedReplacement {
                            position: slot.position,
                            pokemon: *pokemon,
                        })
                        .collect(),
                });
                engine.resolve_forced_switches(choices).unwrap();
            }
            EngineStep::Awaiting(BattleRequest::Revive { side, candidates }) => {
                engine
                    .resolve_revive(ReviveChoice {
                        side: *side,
                        pokemon: candidates[0],
                    })
                    .unwrap();
            }
            other => panic!("{other:?}"),
        }
    }
    true
}

#[test]
fn what_a_member_keeps_hidden_does_not_show() {
    for format in ["gen9ou", "gen9vgc2025regi", "gen8ou", "gen7ou", "gen4ou"] {
        let member = hidden_member(format);
        let variant = (0..OTHER_ITEMS.len())
            .find_map(|nth| started(format, member, |set| hidden_variant(set, nth)))
            .unwrap_or_else(|| panic!("{format}: no other set admitted"));
        let mut engines = [
            started(format, member, |set| Some(set.clone())).unwrap(),
            variant,
        ];
        let mut policy = [Policy(5), Policy(5)];
        // The member in the battle, if its side brought it (each side's team in the order
        // brought; the team's own order brings the first ones).
        let hidden = engines[0]
            .omniscient_view()
            .side(SideId::P2)
            .team()
            .nth(member)
            .map(|pokemon| pokemon.id());
        let mut compared = 0;
        for _ in 0..40 {
            for observer in [Observer::Side(SideId::P1), Observer::Spectator] {
                assert_eq!(
                    shown(&engines[0].view(observer)),
                    shown(&engines[1].view(observer)),
                    "{format}: {observer:?} after {compared} steps"
                );
            }
            compared += 1;
            // The member has come in: what it shows of itself is seen from now on.
            let appeared = hidden.is_some_and(|id| {
                engines[0]
                    .view(Observer::Side(SideId::P1))
                    .side(SideId::P2)
                    .team()
                    .any(|member| member.id() == id)
            });
            if appeared || !step_both(&mut engines, &mut policy) {
                break;
            }
        }
        assert!(compared >= 3, "{format}: {compared} steps compared");
        // The side that keeps it and the omniscient eye see the difference: its item.
        for observer in [Some(Observer::Side(SideId::P2)), None] {
            let seen = |engine: &BattleEngine| match observer {
                Some(observer) => shown(&engine.view(observer)),
                None => shown(&engine.omniscient_view()),
            };
            assert_ne!(
                seen(&engines[0]),
                seen(&engines[1]),
                "{format}: {observer:?}"
            );
        }
    }
}
