//! What a side can choose (DESIGN.md §47.0 items 10-11, M10.4b): `choice_options` lists exactly
//! what `validate_choice` accepts. Real formats' battles, played to the end by the benches'
//! random policy; at every turn's request, for each Pokémon on the field, each move with each
//! target it could name and each team member, alone (the others pass, which is always taken):
//! accepted if and only if listed.

#[path = "support/policy.rs"]
mod policy;

use pokemon_battle_engine::engine::{
    ActionChoice, ChoiceError, ChoiceOptions, SideChoice, SlotChoice, must_struggle,
};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::DeclaredTarget;
use pokemon_battle_engine::state::VolatileConditionKind;
use pokemon_battle_engine::{BattleEngine, Trapped};
use policy::{Policy, advance, start_battle};

const FORMATS: [&str; 5] = [
    "gen9ou",
    "gen9vgc2025regi",
    "gen8doublesou",
    "gen4ou",
    "gen1ou",
];

/// `action` for `actor`, every other Pokémon of the side passing.
fn alone(options: &ChoiceOptions, actor: PokemonId, action: ActionChoice) -> SideChoice {
    SideChoice {
        side: options.side,
        actions: options
            .actors
            .iter()
            .map(|other| SlotChoice {
                actor: other.actor,
                action: if other.actor == actor {
                    action.clone()
                } else {
                    ActionChoice::Pass
                },
            })
            .collect(),
    }
}

/// Checks `side`'s options against `validate_choice`; the number of actions checked.
fn check(engine: &BattleEngine, side: SideId) -> usize {
    let options = engine.choice_options(side).expect("a turn's request");
    let accepts = |actor, action| engine.validate_choice(side, &alone(&options, actor, action));
    let positions: Vec<Option<DeclaredTarget>> = std::iter::once(None)
        .chain(
            engine
                .rules()
                .topology()
                .positions()
                .iter()
                .map(|p| Some(DeclaredTarget(p.id))),
        )
        .collect();
    let mut checked = 0;
    assert_eq!(
        options.actors.len(),
        engine.state().side(side).active.iter().flatten().count(),
        "every Pokémon on the field"
    );
    for actor in &options.actors {
        let pokemon = engine.state().pokemon(actor.actor).expect("the actor");
        assert_eq!(actor.moves.len(), pokemon.move_slots().len(), "every slot");
        for option in &actor.moves {
            assert_eq!(
                option.refused.is_some(),
                option.targets.is_empty(),
                "{:?}: refused exactly when no target is accepted",
                option.move_id
            );
            for target in &positions {
                let move_action = ActionChoice::Move {
                    slot: option.slot,
                    target: *target,
                    mechanic: None,
                };
                let listed = option.targets.contains(target);
                let accepted = accepts(actor.actor, move_action).is_ok();
                assert_eq!(
                    listed, accepted,
                    "{:?} slot {:?} at {target:?}: listed {listed}, accepted {accepted}",
                    actor.actor, option.slot
                );
                checked += 1;
            }
            if let Some(reason) = &option.refused {
                let first = ActionChoice::Move {
                    slot: option.slot,
                    target: None,
                    mechanic: None,
                };
                assert!(accepts(actor.actor, first).is_err(), "{reason:?}");
            }
        }
        for member in &engine.state().side(side).team {
            let listed = actor.switches.contains(member);
            let outcome = accepts(actor.actor, ActionChoice::Switch { pokemon: *member });
            // A trap for all to see refuses every switch and lists none; one an ability sets is
            // the player's to find: listed, refused as it is tried, and shown as maybe.
            let seen_trap = matches!(
                outcome,
                Err(ChoiceError::Trapped { .. } | ChoiceError::TrappedByField { .. })
            );
            let ability_trap = matches!(outcome, Err(ChoiceError::TrappedByAbility { .. }));
            assert_eq!(
                listed,
                outcome.is_ok() || ability_trap,
                "{:?} to {member:?}: {outcome:?}",
                actor.actor
            );
            assert_eq!(
                seen_trap,
                actor.trapped == Trapped::Yes,
                "{:?} to {member:?}: {outcome:?}, trapped {:?}",
                actor.actor,
                actor.trapped
            );
            assert!(
                !ability_trap || actor.trapped == Trapped::Maybe,
                "{:?} to {member:?}: trapped unseen, shown {:?}",
                actor.actor,
                actor.trapped
            );
            checked += 1;
        }
        // Held to one action: recharging, locked into a move, out of moves.
        let held = pokemon.hard_lock().is_some()
            || pokemon.semi_locked_move().is_some()
            || pokemon
                .volatiles
                .contains(VolatileConditionKind::MustRecharge)
            || must_struggle(engine.state(), engine.rules(), engine.dex(), actor.actor);
        assert!(
            !held || actor.locked.is_some(),
            "{:?} is held to one action",
            actor.actor
        );
        if let Some(locked) = &actor.locked {
            assert!(
                accepts(actor.actor, locked.clone()).is_ok(),
                "{locked:?} refused"
            );
        }
        // No slot past the Pokémon's own.
        let past = MoveSlot::new(u8::try_from(actor.moves.len()).unwrap_or(4));
        if let Some(slot) = past {
            let action = ActionChoice::Move {
                slot,
                target: None,
                mechanic: None,
            };
            assert!(accepts(actor.actor, action).is_err());
        }
    }
    checked
}

#[test]
fn the_options_are_what_validate_choice_accepts() {
    for format_id in FORMATS {
        let (rules, dex, start) = start_battle(format_id);
        let mut engine = BattleEngine::restore(rules, dex, start).expect("the battle's start");
        let mut policy = Policy(3);
        let mut checked = 0;
        while advance(&mut engine, &mut policy).is_some() {
            checked += check(&engine, SideId::P1) + check(&engine, SideId::P2);
            let choices = [
                policy.turn_choice(&engine, SideId::P1),
                policy.turn_choice(&engine, SideId::P2),
            ];
            engine
                .submit_turn_choices(choices)
                .expect("valid turn choices");
            // Mid-turn, nothing to choose.
            assert!(engine.choice_options(SideId::P1).is_none());
        }
        assert!(checked > 100, "{format_id}: {checked} actions checked");
    }
}
