//! L1 rule tests for phazing and the switch-triggering items (ROADMAP Batch F 6-7, DESIGN.md
//! §30.2): Roar / Whirlwind and Dragon Tail drag the target out for a random bench Pokémon at
//! the end of the action, Red Card drags the attacker, Eject Button orders the holder out
//! through the mid-turn ForcedSwitch request.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PrimaryEffect, ResolvedDex, StatusMoveEffect, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, EngineStep, MoveFailReason, SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{ItemId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, PokemonState, RandomOverride, RandomRequestKind, SideState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);
const C: PokemonId = PokemonId(3);
const D: PokemonId = PokemonId(4);
const E: PokemonId = PokemonId(5);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

/// P1 = [A, C]; P2 = [B, D, E]. The first of each side leads.
fn setup(generation: Generation, p1: [PokemonSet; 2], p2: [PokemonSet; 3]) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &p1[0]).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &p2[0]).unwrap(),
        build_pokemon(&dex, C, SideId::P1, &p1[1]).unwrap(),
        build_pokemon(&dex, D, SideId::P2, &p2[1]).unwrap(),
        build_pokemon(&dex, E, SideId::P2, &p2[2]).unwrap(),
    ];
    let side = |team: Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), 1);
        side.active[0] = Some(team[0]);
        side
    };
    let state = BattleState::new(pokemon, [side(vec![A, C]), side(vec![B, D, E])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    fn choice(&self, side: SideId, slot: usize) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
        let move_id = self.engine.state().pokemon(actor).unwrap().moves[slot].move_id;
        let target = match self.dex.move_by_id(move_id).unwrap().target {
            TargetPattern::SelectedAny
            | TargetPattern::SelectedAdjacent
            | TargetPattern::SelectedFoe
            | TargetPattern::SelectedAdjacentFoe
            | TargetPattern::Ally
            | TargetPattern::AllyOrSelf => Some(DeclaredTarget(
                self.rules.topology().position(side.other(), 0).unwrap(),
            )),
            _ => None,
        };
        SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(slot as u8).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        }
    }

    fn submit(&mut self, p1_slot: usize, p2_slot: usize, tape: Vec<RandomOverride>) -> usize {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [
            self.choice(SideId::P1, p1_slot),
            self.choice(SideId::P2, p2_slot),
        ];
        self.engine.submit_turn_choices(choices).unwrap();
        start
    }

    /// One whole turn; the tape must be consumed exactly.
    fn turn(&mut self, p1_slot: usize, p2_slot: usize, tape: Vec<RandomOverride>) -> usize {
        let start = self.submit(p1_slot, p2_slot, tape);
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        assert!(
            self.engine.state().rng.semantic_tape_exhausted(),
            "tape not fully consumed"
        );
        start
    }

    fn events(&self, from: usize) -> &[TraceEvent] {
        &self.engine.trace().events()[from..]
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }

    fn active(&self, side: SideId) -> Option<PokemonId> {
        self.engine.state().side(side).active[0]
    }
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

fn drag(index: u16) -> RandomOverride {
    RandomOverride::range(RandomRequestKind::RandomTarget, index)
}

#[test]
fn phazing_moves_classify_from_snapshot_data() {
    let dex = DexResolver::resolve(Generation::Gen9);
    for name in ["Roar", "Whirlwind"] {
        let mv = dex.move_by_name(name).unwrap();
        assert_eq!(
            mv.primary_effect,
            PrimaryEffect::Status(StatusMoveEffect::ForceSwitch),
            "{name}"
        );
        assert_eq!(mv.priority, -6);
    }
    for name in ["Dragon Tail", "Circle Throw"] {
        let mv = dex.move_by_name(name).unwrap();
        assert_eq!(mv.primary_effect, PrimaryEffect::Damage, "{name}");
        assert!(mv.force_switch && mv.priority == -6);
    }
    // Gen 2 Roar fails unless the user moves last (its `onTryHit`, §15.17); Gen 1's does nothing.
    let gen2 = DexResolver::resolve(Generation::Gen2);
    let roar = gen2.move_by_name("Roar").unwrap();
    assert_eq!(
        roar.primary_effect,
        PrimaryEffect::Status(StatusMoveEffect::ForceSwitch)
    );
    assert_eq!(
        roar.behavior.target_rule,
        Some(pokemon_battle_engine::effects::moves::TargetRule::UserMovesLast)
    );
    assert_eq!(dex.move_by_name("Roar").unwrap().behavior.target_rule, None);
    let gen1 = DexResolver::resolve(Generation::Gen1);
    assert_eq!(
        gen1.move_by_name("Whirlwind").unwrap().primary_effect,
        PrimaryEffect::Status(StatusMoveEffect::Inert)
    );
    for name in ["Red Card", "Eject Button"] {
        assert!(dex.item_by_name(name).unwrap().implemented, "{name}");
    }
}

#[test]
fn roar_drags_a_random_bench_pokemon_in_at_the_end_of_the_action() {
    // Snorlax (30 Spe) Roars last anyway (-6); Garchomp's Tackle lands first. The bench is
    // [Pikachu, Clefable]; the RandomTarget decision picks index 1 → Clefable.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Roar", "Tackle"], None),
            set("Vaporeon", &["Tackle"], None),
        ],
        [
            set("Garchomp", &["Tackle"], None),
            set("Pikachu", &["Tackle"], None),
            set("Clefable", &["Tackle"], None),
        ],
    );
    let mut tape = hit(100);
    tape.push(drag(1));
    let start = arena.turn(0, 0, tape);
    assert_eq!(arena.active(SideId::P2), Some(E), "Clefable was dragged in");
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::DraggedOut {
            target: B,
            incoming: E
        }
    )));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::Switched {
            outgoing: Some(B),
            incoming: E,
            ..
        }
    )));
    // Garchomp's stages are cleared like any switch-out; the drag is not a chosen switch.
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::SwitchOutRequested { .. }))
    );
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
}

#[test]
fn roar_cancels_the_dragged_targets_pending_action() {
    // Garchomp's Roar (-6) still goes before Snorlax's Tackle? No: Snorlax (30) vs Garchomp
    // Roar at -6 priority — Roar is last. Use Whirlwind from the faster side with Trick Room
    // absent: give P1 (Snorlax) a Quick Attack so it acts first, then P2's Roar drags Snorlax
    // out before... The simplest observable: P1 Roars first because P2 chose a -7 nothing.
    // Instead: P2 (Garchomp) uses Roar, P1 (Snorlax) uses Tackle; Tackle (priority 0) goes
    // before Roar (-6) regardless of speed, so nothing is cancelled. To observe cancellation
    // the target must not have acted yet: make P1's move a -7-priority one… none exists, so
    // check the queue directly: Roar from a Pokémon whose target still has its action pending
    // requires the phazer to move first, i.e. the target's own move must be slower than -6.
    // Trick Room does not change priority. So cancellation is exercised through Dragon Tail
    // vs a target using a -6 move: Snorlax (30 Spe) uses Roar, Garchomp (102 Spe) uses Dragon
    // Tail: both -6, Garchomp faster → Dragon Tail drags Snorlax out, Snorlax's Roar is gone.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Roar"], None),
            set("Vaporeon", &["Tackle"], None),
        ],
        [
            set("Garchomp", &["Dragon Tail"], None),
            set("Pikachu", &["Tackle"], None),
            set("Clefable", &["Tackle"], None),
        ],
    );
    // Dragon Tail: 90 % accuracy, then the hit. The bench is [Vaporeon] alone, so the drag
    // asks no RandomTarget decision.
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    let start = arena.turn(0, 0, tape);
    assert_eq!(arena.active(SideId::P1), Some(C));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::DraggedOut {
            target: A,
            incoming: C
        }
    )));
    // Snorlax never used Roar.
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveUsed { actor: A, .. }))
    );
    assert_eq!(arena.active(SideId::P2), Some(B), "Garchomp stays");
}

#[test]
fn roar_fails_without_a_bench_and_a_missed_dragon_tail_drags_nobody() {
    // P1 has a bench but P2 does not: Snorlax's Roar fails.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Roar"], None),
            set("Vaporeon", &["Tackle"], None),
        ],
        [
            set("Garchomp", &["Tackle"], None),
            set("Pikachu", &["Tackle"], None),
            set("Clefable", &["Tackle"], None),
        ],
    );
    {
        let mut snapshot = arena.engine.snapshot();
        snapshot.state.pokemon_mut(D).unwrap().hp = 0;
        snapshot.state.pokemon_mut(E).unwrap().hp = 0;
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    }
    let start = arena.turn(0, 0, hit(100));
    assert_eq!(arena.active(SideId::P2), Some(B));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::MoveFailed {
            actor: A,
            reason: MoveFailReason::NoEffect,
            ..
        }
    )));

    // A Dragon Tail that misses drags nobody and asks no RandomTarget decision.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Tackle"], None),
            set("Vaporeon", &["Tackle"], None),
        ],
        [
            set("Garchomp", &["Dragon Tail"], None),
            set("Pikachu", &["Tackle"], None),
            set("Clefable", &["Tackle"], None),
        ],
    );
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, false));
    arena.turn(0, 0, tape);
    assert_eq!(arena.active(SideId::P1), Some(A));
}

#[test]
fn red_card_drags_the_attacker_out_and_is_used_up() {
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Tackle"], None),
            set("Vaporeon", &["Tackle"], None),
        ],
        [
            set("Garchomp", &["Swords Dance"], Some("Red Card")),
            set("Pikachu", &["Tackle"], None),
            set("Clefable", &["Tackle"], None),
        ],
    );
    // Garchomp Swords Dances first, Snorlax's Tackle hits the card: Snorlax is dragged out
    // for Vaporeon (the only bench Pokémon: no RandomTarget decision).
    let start = arena.turn(0, 0, hit(100));
    assert_eq!(arena.active(SideId::P1), Some(C));
    assert_eq!(arena.pokemon(B).held_item, None);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::ItemConsumed {
            holder: B,
            item: ItemId::RED_CARD,
            seen: true,
        }
    )));
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::DraggedOut {
            target: A,
            incoming: C
        }
    )));

    // No bench on the attacker's side: the card stays.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Tackle"], None),
            set("Vaporeon", &["Tackle"], None),
        ],
        [
            set("Garchomp", &["Swords Dance"], Some("Red Card")),
            set("Pikachu", &["Tackle"], None),
            set("Clefable", &["Tackle"], None),
        ],
    );
    {
        let mut snapshot = arena.engine.snapshot();
        snapshot.state.pokemon_mut(C).unwrap().hp = 0;
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    }
    arena.turn(0, 0, hit(100));
    assert_eq!(arena.active(SideId::P1), Some(A));
    assert_eq!(arena.pokemon(B).held_item, Some(ItemId::RED_CARD));
}

#[test]
fn eject_button_suspends_the_turn_for_a_chosen_replacement_and_cancels_the_attackers_u_turn() {
    // Jolteon's U-turn hits the Eject Button: Garchomp is ordered out (the trainer chooses),
    // Jolteon's own switch is cancelled, the button is used up.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Jolteon", &["U-turn"], None),
            set("Vaporeon", &["Tackle"], None),
        ],
        [
            set("Garchomp", &["Tackle"], Some("Eject Button")),
            set("Pikachu", &["Tackle"], None),
            set("Clefable", &["Tackle"], None),
        ],
    );
    let mut tape = hit(100);
    tape.extend(hit(100));
    let start = arena.submit(0, 0, tape);
    let step = arena.engine.step().unwrap();
    let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) = step else {
        panic!("expected a mid-turn ForcedSwitch, got {step:?}");
    };
    assert_eq!(
        replacements[0].required_count, 0,
        "Jolteon's U-turn was cancelled"
    );
    assert_eq!(replacements[1].required_count, 1);
    assert_eq!(replacements[1].positions[0].outgoing, Some(B));
    assert_eq!(arena.pokemon(B).held_item, None);
    assert!(arena.events(start).iter().any(|e| matches!(
        e,
        TraceEvent::ItemConsumed {
            holder: B,
            item: ItemId::EJECT_BUTTON,
            seen: true,
        }
    )));
    assert!(arena.engine.state().side(SideId::P1).switch_out.is_none());

    let position = arena.rules.topology().position(SideId::P2, 0).unwrap();
    arena
        .engine
        .resolve_forced_switches([
            pokemon_battle_engine::engine::ForcedSwitchChoice {
                side: SideId::P1,
                replacements: vec![],
            },
            pokemon_battle_engine::engine::ForcedSwitchChoice {
                side: SideId::P2,
                replacements: vec![pokemon_battle_engine::engine::ForcedReplacement {
                    position,
                    pokemon: E,
                }],
            },
        ])
        .unwrap();
    assert_eq!(arena.active(SideId::P2), Some(E));
    // Garchomp's pending Tackle is gone with it; the turn ends normally.
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert!(
        !arena
            .events(start)
            .iter()
            .any(|e| matches!(e, TraceEvent::MoveUsed { actor: B, .. }))
    );
    assert_eq!(arena.engine.state().turn, 2);
}

#[test]
fn dragon_tail_drags_out_nobody_behind_a_substitute() {
    // Showdown `tryPrimaryHitEvent` drops a target whose Substitute took the hit before
    // `forceSwitch`: no drag, no RandomTarget decision.
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Tackle"], None),
            set("Vaporeon", &["Tackle"], None),
        ],
        [
            set("Garchomp", &["Dragon Tail"], None),
            set("Pikachu", &["Tackle"], None),
            set("Clefable", &["Tackle"], None),
        ],
    );
    {
        let mut snapshot = arena.engine.snapshot();
        snapshot
            .state
            .pokemon_mut(A)
            .unwrap()
            .volatiles
            .set_substitute(A, 200);
        arena.engine =
            BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    }
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, true));
    tape.extend(hit(100));
    arena.turn(0, 0, tape);
    assert_eq!(arena.active(SideId::P1), Some(A));
    assert!(
        arena
            .pokemon(A)
            .volatiles
            .substitute_hp()
            .is_some_and(|hp| hp < 200)
    );
}

fn with_ability(mut set: PokemonSet, ability: &str) -> PokemonSet {
    set.ability = Some(ability.into());
    set
}

/// Eject Pack as the battle starts (DESIGN.md R247): Incineroar's Intimidate lowers Gyarados's
/// Attack and the pack orders it out before the first turn's choices — still turn 0 — and the
/// replacement starts the first turn as a lead does (Showdown's start-of-battle `endTurn` comes
/// after it, and as the first turn's it leaves the turn's history alone: Incineroar, lowered by
/// Gyarados's Intimidate, still had a stat lowered this turn).
#[test]
fn eject_pack_replaces_a_lead_before_the_first_turn() {
    let mut arena = setup(
        Generation::Gen9,
        [
            with_ability(set("Incineroar", &["Harden"], None), "Intimidate"),
            set("Snorlax", &["Harden"], None),
        ],
        [
            with_ability(
                set("Gyarados", &["Harden"], Some("Eject Pack")),
                "Intimidate",
            ),
            set("Snorlax", &["Harden"], None),
            set("Corviknight", &["Harden"], None),
        ],
    );
    arena.engine.run_lead_switch_ins();
    let EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) =
        arena.engine.step().unwrap()
    else {
        panic!("a replacement before the first turn");
    };
    assert_eq!(replacements[0].required_count, 0);
    assert_eq!(replacements[1].required_count, 1);
    assert_eq!(replacements[1].positions[0].outgoing, Some(B));
    assert_eq!(arena.pokemon(B).held_item, None);
    assert_eq!(arena.engine.state().turn, 0);

    let position = arena.rules.topology().position(SideId::P2, 0).unwrap();
    arena
        .engine
        .resolve_forced_switches([
            pokemon_battle_engine::engine::ForcedSwitchChoice {
                side: SideId::P1,
                replacements: vec![],
            },
            pokemon_battle_engine::engine::ForcedSwitchChoice {
                side: SideId::P2,
                replacements: vec![pokemon_battle_engine::engine::ForcedReplacement {
                    position,
                    pokemon: D,
                }],
            },
        ])
        .unwrap();
    assert_eq!(arena.active(SideId::P2), Some(D));
    assert_eq!(arena.engine.state().turn, 1);
    assert_eq!(arena.pokemon(D).active_turns, 1);
    assert!(!arena.pokemon(D).history.newly_switched);
    assert!(arena.pokemon(A).history.stats_lowered_this_turn);
    assert!(matches!(
        arena.engine.step().unwrap(),
        EngineStep::Awaiting(BattleRequest::TurnChoices { .. })
    ));
    arena.turn(0, 0, vec![]);
    assert_eq!(arena.engine.state().turn, 2);
}

/// Eject Pack's mark (`itemState.eject`) starts over as the holder comes in (`switchIn`): a
/// Pokémon marked before it left does not leave again on its own entry.
#[test]
fn eject_packs_mark_starts_over_on_entry() {
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Harden"], None),
            set("Snorlax", &["Harden"], None),
        ],
        [
            set("Snorlax", &["Harden"], None),
            set("Gyarados", &["Harden"], Some("Eject Pack")),
            set("Corviknight", &["Harden"], None),
        ],
    );
    let mut snapshot = arena.engine.snapshot();
    snapshot.state.pokemon_mut(D).unwrap().item_state.eject = true;
    snapshot.state.rng = BattleRng::with_semantic_tape(1, vec![]);
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let start = arena.engine.trace().events().len();
    arena
        .engine
        .submit_turn_choices([
            arena.choice(SideId::P1, 0),
            SideChoice {
                side: SideId::P2,
                actions: vec![SlotChoice {
                    actor: B,
                    action: ActionChoice::Switch { pokemon: D },
                }],
            },
        ])
        .unwrap();
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {}
    assert_eq!(arena.active(SideId::P2), Some(D));
    assert_eq!(arena.pokemon(D).held_item, Some(ItemId::EJECT_PACK));
    assert!(!arena.pokemon(D).item_state.eject);
    assert!(arena.events(start).iter().all(|e| !matches!(
        e,
        TraceEvent::ItemConsumed {
            item: ItemId::EJECT_PACK,
            ..
        }
    )));
}

/// Eject Pack waits while nobody can come in (`onUseItem`: `canSwitch`): Hydreigon's Draco
/// Meteor marks it, the pack stays held, and the mark with it.
#[test]
fn eject_pack_waits_while_nobody_can_come_in() {
    let mut arena = setup(
        Generation::Gen9,
        [
            set("Snorlax", &["Harden"], None),
            set("Snorlax", &["Harden"], None),
        ],
        [
            set("Hydreigon", &["Draco Meteor"], Some("Eject Pack")),
            set("Snorlax", &["Harden"], None),
            set("Corviknight", &["Harden"], None),
        ],
    );
    let mut snapshot = arena.engine.snapshot();
    for id in [D, E] {
        snapshot.state.pokemon_mut(id).unwrap().hp = 0;
    }
    arena.engine = BattleEngine::restore(arena.rules.clone(), arena.dex.clone(), snapshot).unwrap();
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(0, 0, tape);
    assert_eq!(arena.pokemon(B).stages.special_attack, -2);
    assert_eq!(arena.pokemon(B).held_item, Some(ItemId::EJECT_PACK));
    assert!(arena.pokemon(B).item_state.eject);
}
