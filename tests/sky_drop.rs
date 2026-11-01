//! L1 rule tests for Sky Drop (DESIGN.md §15.23): what no scenario can show — the carried
//! Pokémon's request refuses a switch (a choice the oracle's script cannot send), and the carrier
//! fainting at the end of the carrying turn lets its catch down.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleError, ChoiceError, EngineStep, SideChoice, SlotChoice,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, SideState, StatusState,
    VolatileConditionKind,
};

const LIFTER: PokemonId = PokemonId(1);
const PREY: PokemonId = PokemonId(2);
const BENCH: PokemonId = PokemonId(3);
const SPARE: PokemonId = PokemonId(4);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], level: u8) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.level = level;
    set
}

/// Gen 5 singles: P1 = [Braviary with Sky Drop, Chansey], P2 = [Machamp, Snorlax].
fn arena(prepare: impl FnOnce(&mut Vec<PokemonState>)) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(
            Generation::Gen5,
            BattleMode::Singles,
            "custom",
        ))
        .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(Generation::Gen5));
    let mut pokemon = vec![
        build_pokemon(
            &dex,
            LIFTER,
            SideId::P1,
            &set("Braviary", &["Sky Drop"], 50),
        )
        .unwrap(),
        build_pokemon(&dex, PREY, SideId::P2, &set("Machamp", &["Harden"], 40)).unwrap(),
        build_pokemon(&dex, BENCH, SideId::P1, &set("Chansey", &["Harden"], 50)).unwrap(),
        build_pokemon(&dex, SPARE, SideId::P2, &set("Snorlax", &["Harden"], 50)).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(
        pokemon,
        [side(vec![LIFTER, BENCH]), side(vec![PREY, SPARE])],
        1,
    );
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn choices(&self, prey: ActionChoice) -> [SideChoice; 2] {
        let foe = DeclaredTarget(self.rules.topology().position(SideId::P2, 0).unwrap());
        [
            SideChoice {
                side: SideId::P1,
                actions: vec![SlotChoice {
                    actor: LIFTER,
                    action: ActionChoice::Move {
                        slot: MoveSlot::new(0).unwrap(),
                        target: Some(foe),
                        mechanic: None,
                    },
                }],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: PREY,
                    action: prey,
                }],
            },
        ]
    }

    fn harden() -> ActionChoice {
        ActionChoice::Move {
            slot: MoveSlot::new(0).unwrap(),
            target: None,
            mechanic: None,
        }
    }

    fn turn(&mut self, prey: ActionChoice) -> Result<(), BattleError> {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, Vec::new());
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        self.engine.submit_turn_choices(self.choices(prey))?;
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        Ok(())
    }

    fn carried(&self) -> bool {
        self.engine
            .state()
            .pokemon(PREY)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::SkyDropped)
    }
}

#[test]
fn the_carried_pokemon_cannot_switch_out() {
    let mut arena = arena(|_| {});
    arena.turn(Arena::harden()).unwrap();
    assert!(arena.carried());
    let refused = arena.turn(ActionChoice::Switch { pokemon: SPARE });
    assert!(
        matches!(
            refused,
            Err(BattleError::Choice(ChoiceError::Trapped {
                by: VolatileConditionKind::SkyDropped,
                ..
            }))
        ),
        "{refused:?}"
    );
}

#[test]
fn a_carrier_that_faints_lets_its_catch_down() {
    // The carrier, at 1 HP and poisoned, falls to the poison at the end of the carrying turn.
    let mut arena = arena(|pokemon| {
        pokemon[0].hp = 1;
        pokemon[0].status = Some(StatusState::new(NonVolatileStatus::Poison));
    });
    arena.turn(Arena::harden()).unwrap();
    assert!(arena.engine.state().pokemon(LIFTER).unwrap().is_fainted());
    assert!(!arena.carried());
}
