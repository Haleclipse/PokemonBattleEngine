//! L1 rule tests for Heal Block (DESIGN.md §15, R114) and Throat Chop (R121): what they keep
//! off the request — Showdown refuses such a choice itself, so no recorded battle can contain
//! one — and where their BeforeMove check stands (priority 6, before confusion: the replay reads
//! `cant … Heal Block` / `cant … Throat Chop` as a refusal that drew nothing, whichever order
//! the engine used).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{
    ActionChoice, BeforeMoveBlockReason, ChoiceError, EngineStep, MoveFailReason, SideChoice,
    SlotChoice, TraceEvent, validate_side_choice,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, PokemonState, RandomOverride, RandomRequestKind, SideState,
    VolatileConditionKind,
};

const P1A: PokemonId = PokemonId(1);
const P1B: PokemonId = PokemonId(2);
const P2A: PokemonId = PokemonId(3);
const P2B: PokemonId = PokemonId(4);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some("Run Away".into());
    set
}

/// A battle of `sets` (one per side in singles, two in doubles), `prepare`d before it starts.
fn arena(
    generation: Generation,
    mode: BattleMode,
    sets: &[(PokemonId, SideId, PokemonSet)],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules =
        Arc::new(RulesResolver::resolve(&BattleSpec::new(generation, mode, "custom")).unwrap());
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon: Vec<PokemonState> = sets
        .iter()
        .map(|(id, side, set)| build_pokemon(&dex, *id, *side, set).unwrap())
        .collect();
    prepare(&mut pokemon);
    let team = |side: SideId| -> Vec<PokemonId> {
        sets.iter()
            .filter(|(_, s, _)| *s == side)
            .map(|(id, _, _)| *id)
            .collect()
    };
    let side = |ids: Vec<PokemonId>| {
        let mut side = SideState::new(ids.clone(), ids.len());
        for (slot, id) in ids.into_iter().enumerate() {
            side.active[slot] = Some(id);
        }
        side
    };
    let state = BattleState::new(pokemon, [side(team(SideId::P1)), side(team(SideId::P2))], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn action(&self, actor: PokemonId, slot: u8, target: Option<(SideId, u8)>) -> SlotChoice {
        SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::new(slot).unwrap(),
                target: target
                    .map(|(s, i)| DeclaredTarget(self.rules.topology().position(s, i).unwrap())),
                mechanic: None,
            },
        }
    }

    fn validate(&self, side: SideId, actions: Vec<SlotChoice>) -> Result<(), ChoiceError> {
        validate_side_choice(
            self.engine.state(),
            &self.rules,
            &self.dex,
            side,
            &SideChoice { side, actions },
        )
    }
}

#[test]
fn heal_block_takes_moves_that_heal_off_the_request() {
    // Gen 7 doubles: the heal-blocked Mew cannot choose Recover, and its Pollen Puff may aim
    // only at a foe (Showdown `getMoves` makes its target `adjacentFoe`); its partner's may
    // still aim at Mew.
    let arena = arena(
        Generation::Gen7,
        BattleMode::Doubles,
        &[
            (P1A, SideId::P1, set("Mew", &["Recover", "Pollen Puff"])),
            (P1B, SideId::P1, set("Chansey", &["Pollen Puff"])),
            (P2A, SideId::P2, set("Snorlax", &["Harden"])),
            (P2B, SideId::P2, set("Snorlax", &["Harden"])),
        ],
        |pokemon| assert!(pokemon[0].volatiles.set_heal_block(5)),
    );
    let partner = arena.action(P1B, 0, Some((SideId::P1, 0)));
    assert!(matches!(
        arena.validate(
            SideId::P1,
            vec![arena.action(P1A, 0, None), partner.clone()]
        ),
        Err(ChoiceError::MoveDisabled {
            by: VolatileConditionKind::HealBlock,
            ..
        })
    ));
    assert!(matches!(
        arena.validate(
            SideId::P1,
            vec![arena.action(P1A, 1, Some((SideId::P1, 1))), partner.clone()]
        ),
        Err(ChoiceError::InvalidTarget(_))
    ));
    assert!(
        arena
            .validate(
                SideId::P1,
                vec![arena.action(P1A, 1, Some((SideId::P2, 0))), partner]
            )
            .is_ok()
    );
}

#[test]
fn heal_block_holds_a_heal_move_before_confusion() {
    // Gen 5: the confused Snorlax chose Recover before the faster Mew's Heal Block landed.
    // Heal Block's BeforeMove (priority 6) refuses it before confusion (3) counts down or rolls.
    let mut arena = arena(
        Generation::Gen5,
        BattleMode::Singles,
        &[
            (P1A, SideId::P1, set("Mew", &["Heal Block"])),
            (P2A, SideId::P2, set("Snorlax", &["Recover"])),
        ],
        |pokemon| assert!(pokemon[1].volatiles.set_confusion(P1A, 3)),
    );
    let mut snapshot = arena.engine.snapshot();
    // Heal Block (accuracy 100) draws nothing; an unplanned roll would fall back to the seed.
    snapshot.state.rng = BattleRng::with_semantic_tape(1, Vec::new());
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.action(P1A, 0, None)],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.action(P2A, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(
        arena.engine.trace().events()[start..]
            .iter()
            .any(|e| matches!(
                e,
                TraceEvent::MoveFailed {
                    actor: P2A,
                    reason: MoveFailReason::BeforeMove(BeforeMoveBlockReason::HealBlock),
                    ..
                }
            ))
    );
    let snorlax = arena.engine.state().pokemon(P2A).unwrap();
    assert!(snorlax.volatiles.contains(VolatileConditionKind::HealBlock));
    assert_eq!(snorlax.volatiles.confusion_time(), Some(3));
}

#[test]
fn throat_chop_takes_sound_moves_off_the_request() {
    // Gen 9: the chopped Snorlax cannot choose Hyper Voice; Tackle is fine.
    let arena = arena(
        Generation::Gen9,
        BattleMode::Singles,
        &[
            (P1A, SideId::P1, set("Mew", &["Harden"])),
            (P2A, SideId::P2, set("Snorlax", &["Hyper Voice", "Tackle"])),
        ],
        |pokemon| assert!(pokemon[1].volatiles.set_throat_chop(2)),
    );
    assert!(matches!(
        arena.validate(SideId::P2, vec![arena.action(P2A, 0, None)]),
        Err(ChoiceError::MoveDisabled {
            by: VolatileConditionKind::ThroatChop,
            ..
        })
    ));
    assert!(
        arena
            .validate(
                SideId::P2,
                vec![arena.action(P2A, 1, Some((SideId::P1, 0)))]
            )
            .is_ok()
    );
}

#[test]
fn throat_chop_holds_a_sound_move_before_confusion() {
    // Gen 9: the confused Snorlax chose Hyper Voice before the faster Mew's Throat Chop landed.
    // Throat Chop's BeforeMove (priority 6) refuses it before confusion (3) counts down or rolls.
    let mut arena = arena(
        Generation::Gen9,
        BattleMode::Singles,
        &[
            (P1A, SideId::P1, set("Mew", &["Throat Chop"])),
            (P2A, SideId::P2, set("Snorlax", &["Hyper Voice"])),
        ],
        |pokemon| assert!(pokemon[1].volatiles.set_confusion(P1A, 3)),
    );
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.rng = BattleRng::with_semantic_tape(
        1,
        vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(RandomRequestKind::DamageRoll, 100),
        ],
    );
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let start = arena.engine.trace().events().len();
    let choices = [
        SideChoice {
            side: SideId::P1,
            actions: vec![arena.action(P1A, 0, Some((SideId::P2, 0)))],
        },
        SideChoice {
            side: SideId::P2,
            actions: vec![arena.action(P2A, 0, None)],
        },
    ];
    arena.engine.submit_turn_choices(choices).unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(arena.engine.state().rng.semantic_tape_exhausted());
    assert!(
        arena.engine.trace().events()[start..]
            .iter()
            .any(|e| matches!(
                e,
                TraceEvent::MoveFailed {
                    actor: P2A,
                    reason: MoveFailReason::BeforeMove(BeforeMoveBlockReason::ThroatChop),
                    ..
                }
            ))
    );
    let snorlax = arena.engine.state().pokemon(P2A).unwrap();
    assert_eq!(snorlax.volatiles.confusion_time(), Some(3));
}
