//! L1 rule tests for the abilities that keep statuses away or take them off (DESIGN.md §20.2,
//! R115), where the replay cannot tell: the chance Healer rolls with (the replay plans the
//! decision's outcome, not its odds), and Cute Charm's roll going ahead when Aroma Veil then
//! keeps the infatuation off (a refusal Showdown does not print for an ability).

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, Gender, NonVolatileStatus, PokemonState, RandomOverride,
    RandomRequestKind, SideState, StatusState, VolatileConditionKind,
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

fn set(species: &str, moves: &[&str], ability: &str, gender: Option<Gender>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.ability = Some(ability.into());
    set.gender = gender;
    set
}

/// A doubles battle, `prepare`d before it starts. Speeds all differ: no speed tie is drawn.
fn doubles(
    generation: Generation,
    sets: [PokemonSet; 4],
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Doubles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let ids = [
        (P1A, SideId::P1),
        (P1B, SideId::P1),
        (P2A, SideId::P2),
        (P2B, SideId::P2),
    ];
    let mut pokemon: Vec<PokemonState> = ids
        .iter()
        .zip(&sets)
        .map(|((id, side), set)| build_pokemon(&dex, *id, *side, set).unwrap())
        .collect();
    prepare(&mut pokemon);
    let side = |team: [PokemonId; 2]| {
        let mut side = SideState::new(team.to_vec(), 2);
        side.active[0] = Some(team[0]);
        side.active[1] = Some(team[1]);
        side
    };
    let state = BattleState::new(pokemon, [side([P1A, P1B]), side([P2A, P2B])], 1);
    let engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    Arena { rules, dex, engine }
}

impl Arena {
    /// One turn with `rng`; every Pokémon uses its first move (at `targets`' entry, if any).
    fn turn(&mut self, rng: BattleRng, targets: [Option<(SideId, u8)>; 4]) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = rng;
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let action = |actor: PokemonId, target: Option<(SideId, u8)>| SlotChoice {
            actor,
            action: ActionChoice::Move {
                slot: MoveSlot::FIRST,
                target: target
                    .map(|(s, i)| DeclaredTarget(self.rules.topology().position(s, i).unwrap())),
                mechanic: None,
            },
        };
        let choices = [
            SideChoice {
                side: SideId::P1,
                actions: vec![action(P1A, targets[0]), action(P1B, targets[1])],
            },
            SideChoice {
                side: SideId::P2,
                actions: vec![action(P2A, targets[2]), action(P2B, targets[3])],
            },
        ];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
    }

    fn pokemon(&self, id: PokemonId) -> &PokemonState {
        self.engine.state().pokemon(id).unwrap()
    }
}

#[test]
fn healer_cures_an_adjacent_partner_three_times_in_ten() {
    // Gen 5: Audino's Healer rolls `random(10) < 3` for its burned partner at the end of the
    // turn (before the burn's own residual); a raw draw of 2 cures, 4 does not.
    for (draw, cured) in [(2, true), (4, false)] {
        let mut arena = doubles(
            Generation::Gen5,
            [
                set("Audino", &["Harden"], "Healer", None),
                set("Blissey", &["Harden"], "Run Away", None),
                set("Mew", &["Harden"], "Run Away", None),
                set("Snorlax", &["Harden"], "Run Away", None),
            ],
            |pokemon| pokemon[1].status = Some(StatusState::new(NonVolatileStatus::Burn)),
        );
        arena.turn(BattleRng::with_tape(1, vec![draw]), [None; 4]);
        assert_eq!(arena.pokemon(P1B).status.is_none(), cured, "draw {draw}");
    }
}

#[test]
fn cute_charm_rolls_before_aroma_veil_keeps_the_infatuation_off() {
    // Gen 7: the male Aromatisse's Tackle on the female Clefable draws Cute Charm's roll; it
    // takes, and Aroma Veil then refuses the infatuation (`onAllyTryAddVolatile`).
    let mut arena = doubles(
        Generation::Gen7,
        [
            set("Aromatisse", &["Tackle"], "Aroma Veil", Some(Gender::Male)),
            set("Snorlax", &["Harden"], "Run Away", None),
            set("Clefable", &["Harden"], "Cute Charm", Some(Gender::Female)),
            set("Chansey", &["Harden"], "Run Away", None),
        ],
        |_| {},
    );
    arena.turn(
        BattleRng::with_semantic_tape(
            1,
            vec![
                RandomOverride::bool(RandomRequestKind::Critical, false),
                RandomOverride::range(RandomRequestKind::DamageRoll, 100),
                RandomOverride::bool(RandomRequestKind::HitReaction, true),
            ],
        ),
        [Some((SideId::P2, 0)), None, None, None],
    );
    assert!(arena.engine.state().rng.semantic_tape_exhausted());
    assert!(
        !arena
            .pokemon(P1A)
            .volatiles
            .contains(VolatileConditionKind::Attract)
    );
}
