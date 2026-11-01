//! L1 rule tests for R194 where the replay cannot tell: Frisk only announces (the frame holds
//! no trace of it), and the replay plans Gen 4-5 Frisk's foe draw only to keep the tape in step.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, build_pokemon};
use pokemon_battle_engine::engine::TraceEvent;
use pokemon_battle_engine::ids::{ItemId, PokemonId, SideId};
use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, RandomOverride, RandomRequestKind, SideState,
};

const FRISKER: PokemonId = PokemonId(1);
const PARTNER: PokemonId = PokemonId(2);
const HOLDER: PokemonId = PokemonId(3);
const EMPTY_HANDED: PokemonId = PokemonId(4);

fn set(species: &str, ability: &str, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, ["Harden"]);
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set
}

/// A Furret with Frisk and its partner against Snorlax holding Leftovers and Chansey holding
/// nothing (singles: Furret against Snorlax); `edit` changes the state, then the leads come in
/// with `tape`. What they traced and how many planned draws they took.
fn leads(
    generation: Generation,
    mode: BattleMode,
    tape: Vec<RandomOverride>,
    edit: impl FnOnce(&mut BattleState),
) -> (Vec<TraceEvent>, usize) {
    let rules =
        Arc::new(RulesResolver::resolve(&BattleSpec::new(generation, mode, "custom")).unwrap());
    let dex = Arc::new(DexResolver::resolve(generation));
    let build = |id, side, set: PokemonSet| build_pokemon(&dex, id, side, &set).unwrap();
    let doubles = mode == BattleMode::Doubles;
    let mut pokemon = vec![build(FRISKER, SideId::P1, set("Furret", "Frisk", None))];
    let mut foes = vec![build(
        HOLDER,
        SideId::P2,
        set("Snorlax", "Thick Fat", Some("Leftovers")),
    )];
    if doubles {
        pokemon.push(build(PARTNER, SideId::P1, set("Mew", "Synchronize", None)));
        foes.push(build(
            EMPTY_HANDED,
            SideId::P2,
            set("Chansey", "Natural Cure", None),
        ));
    }
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), team.len());
        for (slot, id) in team.into_iter().enumerate() {
            side.active[slot] = Some(id);
        }
        side
    };
    let sides = if doubles {
        [
            side(vec![FRISKER, PARTNER]),
            side(vec![HOLDER, EMPTY_HANDED]),
        ]
    } else {
        [side(vec![FRISKER]), side(vec![HOLDER])]
    };
    pokemon.extend(foes);
    let mut state = BattleState::new(pokemon, sides, 1);
    edit(&mut state);
    state.rng = BattleRng::with_semantic_tape(1, tape);
    let mut engine = BattleEngine::try_new(rules, dex, state).unwrap();
    engine.run_lead_switch_ins();
    (
        engine.trace().events().to_vec(),
        engine.state().rng.semantic_tape_cursor(),
    )
}

fn revealed(events: &[TraceEvent]) -> Vec<(PokemonId, ItemId, PokemonId)> {
    events
        .iter()
        .filter_map(|event| match event {
            TraceEvent::ItemRevealed {
                holder, item, by, ..
            } => Some((*holder, *item, *by)),
            _ => None,
        })
        .collect()
}

fn sample(index: u16) -> RandomOverride {
    RandomOverride::range(RandomRequestKind::Sample, index)
}

#[test]
fn gen_6_plus_frisk_names_every_foe_holding_an_item_and_draws_nothing() {
    let (events, drawn) = leads(
        Generation::Gen9,
        BattleMode::Doubles,
        vec![sample(1)],
        |_| {},
    );
    assert_eq!(
        revealed(&events),
        vec![(HOLDER, ItemId::LEFTOVERS, FRISKER)]
    );
    assert_eq!(drawn, 0);
}

#[test]
fn frisk_names_an_item_whatever_sets_it_aside() {
    // Showdown reads `target.item`: an Embargo on Snorlax hides nothing.
    let (events, _) = leads(Generation::Gen7, BattleMode::Doubles, vec![], |state| {
        state.pokemon_mut(HOLDER).unwrap().volatiles.set_embargo(3);
    });
    assert_eq!(
        revealed(&events),
        vec![(HOLDER, ItemId::LEFTOVERS, FRISKER)]
    );
}

#[test]
fn gen_4_5_frisk_draws_one_foe_and_names_its_item_if_it_has_one() {
    // The first foe drawn: Snorlax's Leftovers.
    let (events, drawn) = leads(
        Generation::Gen5,
        BattleMode::Doubles,
        vec![sample(0)],
        |_| {},
    );
    assert_eq!(
        revealed(&events),
        vec![(HOLDER, ItemId::LEFTOVERS, FRISKER)]
    );
    assert_eq!(drawn, 1);
    // The second: Chansey holds nothing, so nothing is named.
    let (events, drawn) = leads(
        Generation::Gen4,
        BattleMode::Doubles,
        vec![sample(1)],
        |_| {},
    );
    assert_eq!(revealed(&events), vec![]);
    assert_eq!(drawn, 1);
}

#[test]
fn gen_4_5_frisk_against_one_foe_draws_nothing() {
    let (events, drawn) = leads(
        Generation::Gen5,
        BattleMode::Singles,
        vec![sample(0)],
        |_| {},
    );
    assert_eq!(
        revealed(&events),
        vec![(HOLDER, ItemId::LEFTOVERS, FRISKER)]
    );
    assert_eq!(drawn, 0);
}
