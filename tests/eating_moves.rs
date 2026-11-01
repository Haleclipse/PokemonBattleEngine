//! L1 rule tests for R221 where the replay cannot tell: Belch and Stuff Cheeks keep themselves
//! off the request (`onDisableMove`: `ChoiceError::MoveDisabledByItself`) — Showdown never offers
//! them then, so the replay never submits them; a berry whose `onEat` is `false` taken by Bug Bite
//! does not count as eaten, which only Belch's request shows; with nothing else left to choose the
//! request holds Struggle.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, ChoiceError, EngineStep, SideChoice, SlotChoice, must_struggle,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{BattleState, SideState};

const USER: PokemonId = PokemonId(1);
const FOE: PokemonId = PokemonId(2);

fn set(species: &str, moves: &[&str], ability: &str, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.item = item.map(Into::into);
    set
}

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

/// Gen 9 singles, `user` against `foe`, the battle's own random numbers.
fn arena(user: PokemonSet, foe: PokemonSet) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen9,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen9));
    let pokemon = vec![
        build_pokemon(&dex, USER, SideId::P1, &user).unwrap(),
        build_pokemon(&dex, FOE, SideId::P2, &foe).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(USER), side(FOE)], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

impl Arena {
    /// A turn: the user's move in `slot` at the foe, the foe's first move.
    fn turn(&mut self, slot: u8) -> Result<(), BattleError> {
        let choice = |side, actor, slot, target| SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        };
        let foe = Some(DeclaredTarget(
            self.rules.topology().position(SideId::P2, 0).unwrap(),
        ));
        self.engine.submit_turn_choices([
            choice(SideId::P1, USER, slot, foe),
            choice(SideId::P2, FOE, 0, None),
        ])?;
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        Ok(())
    }

    fn refused_by_itself(&mut self, slot: u8) -> bool {
        let move_id = self.engine.state().pokemon(USER).unwrap().moves[usize::from(slot)].move_id;
        matches!(
            self.turn(slot),
            Err(BattleError::Choice(ChoiceError::MoveDisabledByItself { actor: USER, move_id: m }))
                if m == move_id
        )
    }

    fn must_struggle(&self) -> bool {
        must_struggle(self.engine.state(), &self.rules, &self.dex, USER)
    }
}

#[test]
fn belch_waits_for_a_berry_eaten() {
    // Showdown `onDisableMove`: `!pokemon.ateBerry`. A Sitrus Berry Bug Bite takes and eats
    // counts.
    let mut arena = arena(
        set("Scizor", &["Belch", "Bug Bite"], "Light Metal", None),
        set("Blissey", &["Harden"], "Natural Cure", Some("Sitrus Berry")),
    );
    assert!(arena.refused_by_itself(0));
    arena.turn(1).unwrap();
    assert!(
        arena
            .engine
            .state()
            .pokemon(FOE)
            .unwrap()
            .held_item
            .is_none()
    );
    arena.turn(0).unwrap();
}

#[test]
fn a_pomeg_berry_taken_by_bug_bite_does_not_open_belch() {
    // Pomeg Berry's `onEat` is `false`: Bug Bite takes it, but `if (item.onEat)
    // source.ateBerry = true` does not count it.
    let mut arena = arena(
        set("Scizor", &["Belch", "Bug Bite"], "Light Metal", None),
        set("Blissey", &["Harden"], "Natural Cure", Some("Pomeg Berry")),
    );
    arena.turn(1).unwrap();
    assert!(
        arena
            .engine
            .state()
            .pokemon(FOE)
            .unwrap()
            .held_item
            .is_none()
    );
    assert!(arena.refused_by_itself(0));
}

#[test]
fn stuff_cheeks_needs_a_berry_held_set_aside_or_not() {
    // Showdown `onDisableMove`: `!pokemon.getItem().isBerry` — the held item itself, so Klutz
    // setting a berry aside leaves the move open.
    for (item, open) in [
        (None, false),
        (Some("Leftovers"), false),
        (Some("Sitrus Berry"), true),
    ] {
        let mut arena = arena(
            set("Greedent", &["Stuff Cheeks", "Harden"], "Klutz", item),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        );
        assert_eq!(!arena.refused_by_itself(0), open, "{item:?}");
    }
}

#[test]
fn only_belch_left_means_struggle() {
    // Showdown `getMoves`: every move kept off the request by its own `onDisableMove` leaves
    // Struggle alone; a berry eaten opens Belch.
    let mut arena = arena(
        set("Scizor", &["Belch"], "Light Metal", None),
        set("Snorlax", &["Harden"], "Thick Fat", None),
    );
    assert!(arena.must_struggle());
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.pokemon_mut(USER).unwrap().history.ate_berry = true;
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    assert!(!arena.must_struggle());
}
