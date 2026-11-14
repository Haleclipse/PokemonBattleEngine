//! L1 tests for what each observer learns of the other side's items, abilities and moves
//! (DESIGN.md §47.0 items 11 and 13, R-M10-22, R-M10-23): the disclosure table's rules one by one
//! — what shows an item, an ability or a move, what changes them unseen, what counts as a use,
//! what an Illusion's holder takes back. The replay against @pkmn/client (`tests/support/views.rs`)
//! checks that no view claims more than Showdown shows; these check that the views know what it
//! does show.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{
    DexResolver, PokemonSet, PokemonType, ResolvedDex, build_pokemon,
};
use pokemon_battle_engine::engine::{
    ActionChoice, BattleRequest, EngineStep, ForcedReplacement, ForcedSwitchChoice, MechanicChoice,
    SideChoice, SlotChoice, TraceEvent,
};
use pokemon_battle_engine::ids::{AbilityId, ItemId, MoveId, MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, NonVolatileStatus, PokemonState, RandomOverride, RandomRequestKind,
    SideState, VolatileConditionKind,
};
use pokemon_battle_engine::view::{Observer, PokemonView, Seen};

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], ability: &str, item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    // None before Gen 3.
    set.ability = (!ability.is_empty()).then(|| ability.into());
    set.item = item.map(Into::into);
    set.level = 50;
    set
}

/// P1's team first, ids from 1, then P2's; the first of each team leads (the first two in a
/// double battle). `prepare` edits the built Pokémon.
fn battle(
    generation: Generation,
    mode: BattleMode,
    p1: Vec<PokemonSet>,
    p2: Vec<PokemonSet>,
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules =
        Arc::new(RulesResolver::resolve(&BattleSpec::new(generation, mode, "custom")).unwrap());
    let dex = Arc::new(DexResolver::resolve(generation));
    let slots = if mode == BattleMode::Doubles { 2 } else { 1 };
    let mut pokemon = Vec::new();
    let mut teams: [Vec<PokemonId>; 2] = [Vec::new(), Vec::new()];
    let mut next = 1u8;
    for (index, (side, sets)) in [(SideId::P1, &p1), (SideId::P2, &p2)]
        .into_iter()
        .enumerate()
    {
        for set in sets.iter() {
            let id = PokemonId(next);
            next += 1;
            pokemon.push(build_pokemon(&dex, id, side, set).unwrap());
            teams[index].push(id);
        }
    }
    prepare(&mut pokemon);
    let side = |team: &Vec<PokemonId>| {
        let mut side = SideState::new(team.clone(), slots);
        for (slot, active) in side.active.iter_mut().enumerate().take(slots) {
            *active = team.get(slot).copied();
        }
        side
    };
    let state = BattleState::new(pokemon, [side(&teams[0]), side(&teams[1])], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

#[derive(Clone, Copy)]
enum Act {
    /// The move in this slot, at this side's slot (none: no target named).
    Move(usize, Option<(SideId, u8)>),
    /// The move in this slot with a mechanic (a Z-Move, Dynamax), no target named.
    With(usize, MechanicChoice),
    Switch(PokemonId),
}

impl Arena {
    fn choice(&self, side: SideId, acts: &[Act]) -> SideChoice {
        let state = self.engine.state();
        SideChoice {
            side,
            actions: acts
                .iter()
                .zip(state.side(side).active.iter())
                .filter_map(|(act, actor)| {
                    let actor = (*actor)?;
                    let action = match *act {
                        Act::Move(slot, target) => ActionChoice::Move {
                            slot: MoveSlot::new(slot as u8).unwrap(),
                            target: target.map(|(s, i)| {
                                DeclaredTarget(self.rules.topology().position(s, i).unwrap())
                            }),
                            mechanic: None,
                        },
                        Act::With(slot, mechanic) => ActionChoice::Move {
                            slot: MoveSlot::new(slot as u8).unwrap(),
                            target: None,
                            mechanic: Some(mechanic),
                        },
                        Act::Switch(pokemon) => ActionChoice::Switch { pokemon },
                    };
                    Some(SlotChoice { actor, action })
                })
                .collect(),
        }
    }

    /// One turn, the decisions `tape` plans drawn as planned (the rest from the seed).
    fn turn(&mut self, p1: &[Act], p2: &[Act], tape: Vec<RandomOverride>) {
        let mut snapshot = self.engine.snapshot();
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let choices = [self.choice(SideId::P1, p1), self.choice(SideId::P2, p2)];
        self.engine.submit_turn_choices(choices).unwrap();
        loop {
            match self.engine.step().unwrap() {
                EngineStep::Progressed { .. } => {}
                // A replacement (Baton Pass's): the first that can come in.
                EngineStep::Awaiting(BattleRequest::ForcedSwitch { replacements }) => {
                    let choices = replacements.map(|requirement| ForcedSwitchChoice {
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
                    self.engine.resolve_forced_switches(choices).unwrap();
                }
                _ => break,
            }
        }
    }

    /// `pokemon` of the side `observer` does not play as it sees it: standing, or a member it
    /// has seen.
    fn seen<T>(
        &self,
        observer: SideId,
        pokemon: PokemonId,
        read: impl Fn(PokemonView<'_>) -> T,
    ) -> T {
        self.seen_by(Observer::Side(observer), observer.other(), pokemon, read)
    }

    /// `pokemon` of `side` as `observer` sees it.
    fn seen_by<T>(
        &self,
        observer: Observer,
        side: SideId,
        pokemon: PokemonId,
        read: impl Fn(PokemonView<'_>) -> T,
    ) -> T {
        let view = self.engine.view(observer);
        let side = view.side(side);
        let found = side
            .active()
            .filter_map(|(_, standing)| standing)
            .chain(side.team())
            .find(|shown| shown.id() == pokemon)
            .unwrap_or_else(|| panic!("{pokemon:?} unseen"));
        read(found)
    }

    fn item(&self, observer: SideId, pokemon: PokemonId) -> Seen<ItemId> {
        self.seen(observer, pokemon, |p| p.item())
    }

    fn ability(&self, observer: SideId, pokemon: PokemonId) -> Seen<AbilityId> {
        self.seen(observer, pokemon, |p| p.ability())
    }

    /// The volatiles `observer` sees on `pokemon` (of the side it does not play).
    fn volatiles(&self, observer: SideId, pokemon: PokemonId) -> Vec<VolatileConditionKind> {
        self.seen(observer, pokemon, |p| p.volatiles().iter().collect())
    }

    /// The moves `observer` sees `pokemon` has, each with the uses seen, and whether that is all.
    fn moves(&self, observer: SideId, pokemon: PokemonId) -> (Vec<(MoveId, u16)>, bool) {
        self.moves_by(Observer::Side(observer), observer.other(), pokemon)
    }

    fn moves_by(
        &self,
        observer: Observer,
        side: SideId,
        pokemon: PokemonId,
    ) -> (Vec<(MoveId, u16)>, bool) {
        self.seen_by(observer, side, pokemon, |p| {
            let moves = p.moves();
            let revealed = moves
                .revealed()
                .map(|m| {
                    if !matches!(observer, Observer::Side(own) if own == side) {
                        assert_eq!(m.pp(), Seen::Unknown, "another side's PP");
                    }
                    (m.move_id(), m.observed_uses())
                })
                .collect();
            (revealed, moves.complete())
        })
    }
}

fn sorted(mut moves: Vec<(MoveId, u16)>) -> Vec<(MoveId, u16)> {
    moves.sort();
    moves
}

fn hit(roll: u16) -> Vec<RandomOverride> {
    vec![
        RandomOverride::bool(RandomRequestKind::Critical, false),
        RandomOverride::range(RandomRequestKind::DamageRoll, roll),
    ]
}

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

#[test]
fn an_item_at_work_is_known_and_one_used_up_known_gone() {
    // Life Orb's recoil names it (`-damage|…|[from] item: Life Orb`); a Sitrus Berry eaten is
    // gone (`-enditem|…|[eat]`), its heal after naming the berry it no longer holds.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Garchomp", &["Tackle"], "Rough Skin", Some("Life Orb"))],
        vec![set(
            "Blissey",
            &["Splash"],
            "Natural Cure",
            Some("Sitrus Berry"),
        )],
        |pokemon| pokemon[1].hp = pokemon[1].max_hp / 2 + 1,
    );
    assert_eq!(arena.item(SideId::P2, A), Seen::Unknown);
    assert_eq!(arena.item(SideId::P1, B), Seen::Unknown);
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::LIFE_ORB));
    assert_eq!(arena.item(SideId::P1, B), Seen::KnownNone);
}

#[test]
fn a_sticky_barb_passing_on_shows_nothing_of_either() {
    // Turn 1: the barb's residual names it on its holder. Turn 2: a contact move's user that
    // holds nothing takes it, with no message — the holder is still believed to hold it, as last
    // seen (R-M10-40) — and the barb's residual names it on the taker.
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set(
            "Ferrothorn",
            &["Splash"],
            "Iron Barbs",
            Some("Sticky Barb"),
        )],
        vec![set("Snorlax", &["Splash", "Tackle"], "Thick Fat", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::STICKY_BARB));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(1, None)], hit(100));
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::STICKY_BARB));
    assert_eq!(arena.item(SideId::P1, B), Seen::Known(ItemId::STICKY_BARB));
}

#[test]
fn trick_shows_what_each_holds_after() {
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set(
            "Alakazam",
            &["Trick"],
            "Synchronize",
            Some("Choice Scarf"),
        )],
        vec![set("Snorlax", &["Splash"], "Thick Fat", Some("Leftovers"))],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::LEFTOVERS));
    assert_eq!(arena.item(SideId::P1, B), Seen::Known(ItemId::CHOICE_SCARF));
}

#[test]
fn natural_gift_sets_a_known_berry_aside_unseen() {
    // Frisk names the berry as its holder's foe comes in; Natural Gift uses it up with no
    // message (`setItem('')`): it is still believed held, as last seen (R-M10-40).
    let mut arena = battle(
        Generation::Gen6,
        BattleMode::Singles,
        vec![set(
            "Snorlax",
            &["Natural Gift"],
            "Thick Fat",
            Some("Sitrus Berry"),
        )],
        vec![set("Gothitelle", &["Splash"], "Frisk", None)],
        |_| {},
    );
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::SITRUS_BERRY));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::SITRUS_BERRY));
}

#[test]
fn gen_5_frisk_names_no_holder_among_two_foes() {
    // Gen 4-5 Frisk names one foe's item, not the foe (`-item||item|[from] ability: Frisk`):
    // with one foe standing everyone can tell, with two nobody.
    let foes = || {
        vec![
            set("Snorlax", &["Splash"], "Thick Fat", Some("Leftovers")),
            set("Blissey", &["Splash"], "Natural Cure", Some("Leftovers")),
        ]
    };
    let doubles = battle(
        Generation::Gen5,
        BattleMode::Doubles,
        vec![
            set("Dusknoir", &["Splash"], "Frisk", None),
            set("Pikachu", &["Splash"], "Static", None),
        ],
        foes(),
        |_| {},
    );
    for foe in [PokemonId(3), PokemonId(4)] {
        assert_eq!(doubles.item(SideId::P1, foe), Seen::Unknown);
    }
    let singles = battle(
        Generation::Gen5,
        BattleMode::Singles,
        vec![set("Dusknoir", &["Splash"], "Frisk", None)],
        vec![set("Snorlax", &["Splash"], "Thick Fat", Some("Leftovers"))],
        |_| {},
    );
    assert_eq!(singles.item(SideId::P1, B), Seen::Known(ItemId::LEFTOVERS));
}

#[test]
fn an_announced_ability_is_known_one_at_work_unseen_is_not() {
    // Intimidate announces itself; Magic Guard refusing the poison's damage says nothing; Gen 3
    // Pressure is shown to its holder's side alone, Gen 4 to everyone.
    let arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Gyarados", &["Splash"], "Intimidate", None)],
        vec![set("Clefable", &["Splash"], "Magic Guard", None)],
        |_| {},
    );
    assert_eq!(
        arena.ability(SideId::P2, A),
        Seen::Known(AbilityId::INTIMIDATE)
    );
    let mut guarded = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set(
            "Clefable",
            &["Splash"],
            "Magic Guard",
            Some("Toxic Orb"),
        )],
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        |_| {},
    );
    guarded.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    // The orb's poison names it (`-status|…|[from] item: Toxic Orb`).
    assert_eq!(guarded.item(SideId::P2, A), Seen::Known(ItemId::TOXIC_ORB));
    guarded.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(guarded.ability(SideId::P2, A), Seen::Unknown);
    for (generation, shown) in [(Generation::Gen3, false), (Generation::Gen4, true)] {
        let arena = battle(
            generation,
            BattleMode::Singles,
            vec![set("Dusclops", &["Splash"], "Pressure", None)],
            vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
            |_| {},
        );
        let expected = if shown {
            Seen::Known(AbilityId::PRESSURE)
        } else {
            Seen::Unknown
        };
        assert_eq!(arena.ability(SideId::P2, A), expected, "{generation:?}");
    }
}

#[test]
fn a_status_an_ability_inflicts_names_it_but_a_gen_3_4_sleep() {
    // `-status|…|[from] ability: Static|[of] holder`: the base conditions' `onStart`, which the
    // gen4 mod's paralysis inherits. Its sleep's (Effect Spore's) names a move's alone; from Gen 5
    // the ability's too (R-M10-40).
    let reaction = |kind| RandomOverride::bool(kind, true);
    for (generation, species, ability, id, draws, status, named) in [
        (
            Generation::Gen4,
            "Pikachu",
            "Static",
            AbilityId::STATIC,
            vec![reaction(RandomRequestKind::ContactReaction)],
            NonVolatileStatus::Paralysis,
            true,
        ),
        (
            Generation::Gen4,
            "Breloom",
            "Effect Spore",
            AbilityId::EFFECT_SPORE,
            vec![
                reaction(RandomRequestKind::HitReaction),
                RandomOverride::range(RandomRequestKind::Sample, 0),
            ],
            NonVolatileStatus::Sleep,
            false,
        ),
        (
            Generation::Gen5,
            "Breloom",
            "Effect Spore",
            AbilityId::EFFECT_SPORE,
            vec![RandomOverride::range(RandomRequestKind::EffectSpore, 5)],
            NonVolatileStatus::Sleep,
            true,
        ),
    ] {
        let mut arena = battle(
            generation,
            BattleMode::Singles,
            vec![set("Snorlax", &["Tackle"], "Thick Fat", None)],
            vec![set(species, &["Splash"], ability, None)],
            |_| {},
        );
        let mut tape = Vec::new();
        // Gen 4 Tackle's 95.
        if generation == Generation::Gen4 {
            tape.push(RandomOverride::bool(RandomRequestKind::Accuracy, true));
        }
        tape.extend(hit(100));
        tape.extend(draws);
        arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], tape);
        assert_eq!(
            arena
                .engine
                .state()
                .pokemon(A)
                .and_then(|pokemon| pokemon.status)
                .map(|status| status.kind),
            Some(status),
            "{generation:?} {ability}"
        );
        assert_eq!(
            known_ability(&arena, SideId::P1, B),
            named.then_some(id),
            "{generation:?} {ability}"
        );
    }
}

#[test]
fn an_ability_swapped_unnamed_is_unknown_and_its_own_comes_back_as_it_leaves() {
    // Gyarados's Intimidate is known; Skill Swap gives it another, which the Gen 4 line does not
    // name; switched out and back, it has its own again.
    let mut arena = battle(
        Generation::Gen4,
        BattleMode::Singles,
        vec![
            set("Gyarados", &["Splash"], "Intimidate", None),
            set("Snorlax", &["Splash"], "Thick Fat", None),
        ],
        vec![set(
            "Alakazam",
            &["Skill Swap", "Splash"],
            "Synchronize",
            None,
        )],
        |_| {},
    );
    let gyarados = A;
    assert_eq!(
        arena.ability(SideId::P2, gyarados),
        Seen::Known(AbilityId::INTIMIDATE)
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.ability(SideId::P2, gyarados), Seen::Unknown);
    arena.turn(&[Act::Switch(B)], &[Act::Move(1, None)], Vec::new());
    assert_eq!(
        arena.ability(SideId::P2, gyarados),
        Seen::Known(AbilityId::INTIMIDATE)
    );
}

#[test]
fn an_illusion_broken_gives_its_holder_back_what_it_showed_in_the_disguise() {
    // Zoroark leads as Snorlax (its team's last member); its Life Orb's recoil is believed
    // Snorlax's. A hit breaks the Illusion: the Life Orb is Zoroark's, Snorlax's item unknown
    // again (DESIGN.md §47.0 item 11).
    let mut arena = battle(
        Generation::Gen5,
        BattleMode::Singles,
        vec![
            set(
                "Zoroark",
                &["Tackle", "Splash"],
                "Illusion",
                Some("Life Orb"),
            ),
            set("Snorlax", &["Splash"], "Thick Fat", None),
        ],
        vec![set("Machamp", &["Splash", "Tackle"], "Guts", None)],
        |_| {},
    );
    let (zoroark, snorlax) = (PokemonId(1), PokemonId(2));
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        hit(100),
    );
    assert_eq!(
        arena.item(SideId::P2, snorlax),
        Seen::Known(ItemId::LIFE_ORB)
    );
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], hit(100));
    assert_eq!(
        arena.item(SideId::P2, zoroark),
        Seen::Known(ItemId::LIFE_ORB)
    );
    assert_eq!(arena.item(SideId::P2, snorlax), Seen::Unknown);
}

#[test]
fn forme_changes_a_move_or_a_volatile_makes_name_no_ability() {
    // Zen Mode's forme comes through its `zenmode` volatile (`-formechange` with no `[from]`).
    let mut arena = battle(
        Generation::Gen5,
        BattleMode::Singles,
        vec![set("Darmanitan", &["Splash"], "Zen Mode", None)],
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        |pokemon| pokemon[0].hp = pokemon[0].max_hp / 3,
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert!(
        arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(pokemon_battle_engine::state::VolatileConditionKind::ZenMode),
        "Zen Mode went on"
    );
    assert_eq!(arena.ability(SideId::P2, A), Seen::Unknown);
}

#[test]
fn a_galar_darmanitan_back_above_the_line_drops_zen_mode() {
    // Above half its HP again, Zen Mode removes its volatile (`removeVolatile('zenmode')`), whose
    // `onEnd` puts the holder back into its own forme — a Galar Darmanitan's Galar one; the
    // engine took only plain Darmanitan's for "back" and kept the volatile (R-M10-68).
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set(
            "Darmanitan-Galar",
            &["Splash", "Recover"],
            "Zen Mode",
            None,
        )],
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        |pokemon| pokemon[0].hp = pokemon[0].max_hp / 3,
    );
    let species = |arena: &Arena, name: &str| {
        pokemon_battle_engine::engine::species_of(arena.engine.state().pokemon(A).unwrap())
            == arena.dex.species_by_name(name).map(|species| species.id)
    };
    let zen = |arena: &Arena| {
        arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::ZenMode)
    };
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert!(species(&arena, "Darmanitan-Galar-Zen") && zen(&arena));
    arena.turn(&[Act::Move(1, None)], &[Act::Move(0, None)], Vec::new());
    assert!(species(&arena, "Darmanitan-Galar"));
    assert!(!zen(&arena));
}

#[test]
fn natural_cure_cures_silently_and_as_one_shows_itself_not_unnerve() {
    // Natural Cure's line is `[silent]`: the cure is seen, the ability is not.
    let mut arena = battle(
        Generation::Gen5,
        BattleMode::Singles,
        vec![
            set("Chansey", &["Splash"], "Natural Cure", None),
            set("Snorlax", &["Splash"], "Thick Fat", None),
        ],
        vec![set("Machamp", &["Splash"], "Guts", None)],
        |pokemon| {
            pokemon[0].status = Some(pokemon_battle_engine::state::StatusState::new(
                pokemon_battle_engine::state::NonVolatileStatus::Poison,
            ))
        },
    );
    arena.turn(&[Act::Switch(B)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.engine.state().pokemon(A).unwrap().status, None);
    assert_eq!(arena.ability(SideId::P2, A), Seen::Unknown);
    // As One announces itself, then its Unnerve: the ability is As One.
    let arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Calyrex-Ice", &["Splash"], "As One (Glastrier)", None)],
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        |_| {},
    );
    assert_eq!(
        arena.ability(SideId::P2, A),
        Seen::Known(AbilityId::AS_ONE_GLASTRIER)
    );
}

#[test]
fn an_item_going_off_as_it_is_used_up_stays_gone() {
    // Eject Button is used up (`-enditem`) as it goes off: what is seen is that it is gone.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Garchomp", &["Tackle"], "Rough Skin", None)],
        vec![
            set("Blissey", &["Splash"], "Natural Cure", Some("Eject Button")),
            set("Snorlax", &["Splash"], "Thick Fat", None),
        ],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(arena.item(SideId::P1, B), Seen::KnownNone);
}

#[test]
fn a_move_is_seen_with_the_uses_seen_not_the_pp_it_paid() {
    // Pressure takes a second PP a use, unseen: the uses seen are one a use, the PP its own side's
    // to know. Four different moves seen are all of them.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set(
            "Snorlax",
            &["Tackle", "Splash", "Harden", "Defense Curl"],
            "Thick Fat",
            None,
        )],
        vec![set("Dusknoir", &["Splash"], "Pressure", None)],
        |_| {},
    );
    for _ in 0..2 {
        arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    }
    assert_eq!(
        arena.moves(SideId::P2, A),
        (vec![(MoveId::TACKLE, 2)], false)
    );
    assert_eq!(
        arena.moves(SideId::P1, B),
        (vec![(MoveId::SPLASH, 2)], false)
    );
    let own = arena.engine.view(Observer::Side(SideId::P1));
    let snorlax = own.side(SideId::P1).team().next().unwrap();
    let tackle = snorlax.moves().revealed().next().unwrap();
    assert_eq!(tackle.observed_uses(), 2);
    let Seen::Known((pp, max)) = tackle.pp() else {
        panic!("its own side knows its PP");
    };
    assert_eq!(max - pp, 4, "Pressure's PP");
    assert!(snorlax.moves().complete());
    for slot in 1..3 {
        arena.turn(&[Act::Move(slot, None)], &[Act::Move(0, None)], Vec::new());
    }
    assert!(!arena.moves(SideId::P2, A).1, "three of them");
    arena.turn(&[Act::Move(3, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        arena.moves(SideId::P2, A),
        (
            vec![
                (MoveId::TACKLE, 2),
                (MoveId::SPLASH, 1),
                (MoveId::HARDEN, 1),
                (MoveId::DEFENSE_CURL, 1)
            ],
            true
        )
    );
}

#[test]
fn a_two_turn_move_counts_once_as_it_starts() {
    // Solar Beam's charge is its use (`[still]`, `-prepare`); the attack is the lock's
    // (`[from] lockedmove`) — Gen 1's too, though it pays the PP then (`[from] Solar Beam`).
    for (generation, ability, foe) in [
        (Generation::Gen9, "Overgrow", "Blissey"),
        (Generation::Gen1, "", "Chansey"),
    ] {
        let mut arena = battle(
            generation,
            BattleMode::Singles,
            vec![set("Venusaur", &["Solar Beam"], ability, None)],
            vec![set(foe, &["Splash"], ability, None)],
            |_| {},
        );
        arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
        assert_eq!(arena.moves(SideId::P2, A).0, vec![(MoveId::SOLAR_BEAM, 1)]);
        arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
        assert_eq!(
            arena.moves(SideId::P2, A).0,
            vec![(MoveId::SOLAR_BEAM, 1)],
            "{generation:?}"
        );
    }
}

#[test]
fn a_call_shows_the_caller_and_sleep_talk_its_own_move() {
    // Copycat calls the foe's Splash: Splash is not Copycat's user's. Sleep Talk calls one of
    // its user's own moves: shown, not used by its action.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Copycat"], "Thick Fat", None)],
        vec![set("Jolteon", &["Splash"], "Volt Absorb", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.moves(SideId::P2, A).0, vec![(MoveId::COPYCAT, 1)]);
    let mut asleep = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Sleep Talk", "Harden"], "Thick Fat", None)],
        vec![set("Jolteon", &["Splash"], "Volt Absorb", None)],
        |pokemon| {
            let mut sleep = pokemon_battle_engine::state::StatusState::new(
                pokemon_battle_engine::state::NonVolatileStatus::Sleep,
            );
            sleep.sleep_turns_remaining = Some(3);
            sleep.sleep_turns_rolled = Some(3);
            pokemon[0].status = Some(sleep);
        },
    );
    asleep.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        asleep.moves(SideId::P2, A).0,
        vec![(MoveId::SLEEP_TALK, 1), (MoveId::HARDEN, 0)]
    );
}

#[test]
fn a_cant_names_the_move_it_stopped_and_focus_punch_shows_itself_as_the_turn_starts() {
    // Taunt lands before the chosen Swords Dance: `cant|p|move: Taunt|Swords Dance`. Focus Punch
    // is named as the turn starts (`-singleturn`): a flinch (`cant|p|flinch`) names nothing
    // after; hit first, it is lost (`cant|p|Focus Punch|Focus Punch`), no use either.
    let mut taunted = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Swords Dance"], "Thick Fat", None)],
        vec![set("Jolteon", &["Taunt"], "Volt Absorb", None)],
        |_| {},
    );
    taunted.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        taunted.moves(SideId::P2, A).0,
        vec![(MoveId::SWORDS_DANCE, 0)]
    );
    for (slot, flinch) in [(0, true), (1, false)] {
        let mut focused = battle(
            Generation::Gen9,
            BattleMode::Singles,
            vec![set("Conkeldurr", &["Focus Punch"], "Guts", None)],
            vec![set("Jolteon", &["Fake Out", "Tackle"], "Volt Absorb", None)],
            |_| {},
        );
        let mut tape = hit(100);
        if flinch {
            tape.push(RandomOverride::bool(RandomRequestKind::Secondary, true));
        }
        focused.turn(&[Act::Move(0, None)], &[Act::Move(slot, None)], tape);
        assert_eq!(
            focused.moves(SideId::P2, A).0,
            vec![(MoveId::FOCUS_PUNCH, 0)],
            "flinched: {flinch}"
        );
    }
}

#[test]
fn forewarn_names_a_foes_move_and_gen_4_no_foe_among_two() {
    let arena = battle(
        Generation::Gen5,
        BattleMode::Singles,
        vec![set("Hypno", &["Splash"], "Forewarn", None)],
        vec![set("Snorlax", &["Body Slam", "Splash"], "Thick Fat", None)],
        |_| {},
    );
    assert_eq!(arena.moves(SideId::P1, B).0, vec![(MoveId::BODY_SLAM, 0)]);
    let doubles = battle(
        Generation::Gen4,
        BattleMode::Doubles,
        vec![
            set("Hypno", &["Splash"], "Forewarn", None),
            set("Pikachu", &["Splash"], "Static", None),
        ],
        vec![
            set("Snorlax", &["Body Slam"], "Thick Fat", None),
            set("Blissey", &["Body Slam"], "Natural Cure", None),
        ],
        |_| {},
    );
    for foe in [PokemonId(3), PokemonId(4)] {
        assert_eq!(doubles.moves(SideId::P1, foe).0, Vec::new());
    }
}

#[test]
fn a_leppa_berry_names_the_move_it_restores() {
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set(
            "Snorlax",
            &["Splash", "Tackle"],
            "Thick Fat",
            Some("Leppa Berry"),
        )],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |pokemon| pokemon[0].move_slots_mut()[1].pp = 0,
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        sorted(arena.moves(SideId::P2, A).0),
        sorted(vec![(MoveId::SPLASH, 1), (MoveId::TACKLE, 0)])
    );
}

#[test]
fn a_z_move_or_a_max_move_shows_its_base_only_where_its_line_tells() {
    // A status Z-Move is its base (`Z-Swords Dance`); a signature one comes from one move
    // (Catastropika, Volt Tackle); a generic one (Breakneck Blitz) and a Max Move hide theirs.
    let z = |moves: &[&str], crystal: &str, species: &str| {
        let mut arena = battle(
            Generation::Gen7,
            BattleMode::Singles,
            vec![set(species, moves, "Static", Some(crystal))],
            vec![set("Garchomp", &["Splash"], "Rough Skin", None)],
            |_| {},
        );
        arena.turn(
            &[Act::With(0, MechanicChoice::ZMove)],
            &[Act::Move(0, None)],
            hit(100),
        );
        arena.moves(SideId::P2, A).0
    };
    assert_eq!(
        z(&["Swords Dance"], "Normalium Z", "Pikachu"),
        vec![(MoveId::SWORDS_DANCE, 1)]
    );
    assert_eq!(
        z(&["Volt Tackle"], "Pikanium Z", "Pikachu"),
        vec![(MoveId::VOLT_TACKLE, 1)]
    );
    assert_eq!(z(&["Tackle"], "Normalium Z", "Pikachu"), Vec::new());
    let mut dynamaxed = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Snorlax", &["Tackle"], "Thick Fat", None)],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |_| {},
    );
    dynamaxed.turn(
        &[Act::With(0, MechanicChoice::Dynamax)],
        &[Act::Move(0, None)],
        hit(100),
    );
    assert_eq!(dynamaxed.moves(SideId::P2, A).0, Vec::new());
}

#[test]
fn a_transformed_pokemon_has_the_moves_of_the_one_it_became() {
    // Imposter's Ditto takes Blissey's moves: Blissey's side knows them all, and Ditto's side too,
    // from its requests — for good; a spectator, those seen of Blissey and those Ditto used —
    // each Blissey's too. Gone, Ditto's own are known again (none), and Imposter, which named
    // itself.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![
            set("Ditto", &["Transform"], "Imposter", None),
            set("Snorlax", &["Splash"], "Thick Fat", None),
        ],
        // Three moves: Ditto's side knows they are all, not four seen.
        vec![set(
            "Blissey",
            &["Splash", "Soft-Boiled", "Seismic Toss"],
            "Natural Cure",
            None,
        )],
        |_| {},
    );
    let (ditto, blissey) = (PokemonId(1), PokemonId(3));
    let (known, complete) = arena.moves(SideId::P2, ditto);
    assert!(complete);
    assert_eq!(known.len(), 3);
    let (copied, complete) = arena.moves(SideId::P1, blissey);
    assert!(complete);
    assert_eq!(copied.len(), 3);
    assert_eq!(
        arena.moves_by(Observer::Spectator, SideId::P1, ditto),
        (Vec::new(), false)
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(1, None)], Vec::new());
    assert_eq!(
        sorted(arena.moves_by(Observer::Spectator, SideId::P1, ditto).0),
        sorted(vec![(MoveId::SPLASH, 1), (MoveId::SOFT_BOILED, 0)])
    );
    assert_eq!(
        sorted(arena.moves_by(Observer::Spectator, SideId::P2, blissey).0),
        sorted(vec![(MoveId::SPLASH, 0), (MoveId::SOFT_BOILED, 1)])
    );
    arena.turn(
        &[Act::Switch(PokemonId(2))],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(arena.moves(SideId::P2, ditto), (Vec::new(), false));
    assert!(
        arena.moves(SideId::P1, blissey).1,
        "Ditto's side saw them all"
    );
    assert!(!arena.moves_by(Observer::Spectator, SideId::P2, blissey).1);
    assert_eq!(
        arena.ability(SideId::P2, ditto),
        Seen::Known(AbilityId::IMPOSTER)
    );
}

#[test]
fn mimic_stands_in_its_place_for_the_stay_and_sketch_for_good() {
    let mut arena = battle(
        Generation::Gen4,
        BattleMode::Singles,
        vec![
            set("Mr. Mime", &["Mimic", "Splash"], "Soundproof", None),
            set("Smeargle", &["Sketch"], "Own Tempo", None),
        ],
        vec![set("Jolteon", &["Tackle"], "Volt Absorb", None)],
        |_| {},
    );
    let (mime, smeargle) = (PokemonId(1), PokemonId(2));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.moves(SideId::P2, mime).0, vec![(MoveId::TACKLE, 0)]);
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.moves(SideId::P2, mime).0, vec![(MoveId::TACKLE, 1)]);
    arena.turn(&[Act::Switch(smeargle)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.moves(SideId::P2, mime).0, vec![(MoveId::MIMIC, 1)]);
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        arena.moves(SideId::P2, smeargle).0,
        vec![(MoveId::TACKLE, 0)]
    );
    arena.turn(&[Act::Switch(mime)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        arena.moves(SideId::P2, smeargle).0,
        vec![(MoveId::TACKLE, 0)]
    );
}

#[test]
fn an_illusions_moves_are_its_holders_once_unmasked_and_the_disguises_if_it_left_in_it() {
    let team = || {
        vec![
            set("Zoroark", &["Tackle", "Splash"], "Illusion", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ]
    };
    let foe = || vec![set("Machamp", &["Splash", "Tackle"], "Guts", None)];
    let (zoroark, snorlax) = (PokemonId(1), PokemonId(2));
    // Unmasked by a hit: the Tackle and the Splash it used in the disguise are its own.
    let mut arena = battle(Generation::Gen5, BattleMode::Singles, team(), foe(), |_| {});
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        hit(100),
    );
    assert_eq!(
        arena.moves(SideId::P2, snorlax).0,
        vec![(MoveId::TACKLE, 1)]
    );
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], hit(100));
    assert_eq!(
        arena.moves(SideId::P2, zoroark).0,
        vec![(MoveId::TACKLE, 1), (MoveId::SPLASH, 1)]
    );
    assert_eq!(arena.moves(SideId::P2, snorlax).0, Vec::new());
    // Gone still disguised: believed the disguise's — not to its own side, which knows better.
    let mut arena = battle(Generation::Gen5, BattleMode::Singles, team(), foe(), |_| {});
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        hit(100),
    );
    arena.turn(&[Act::Switch(snorlax)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        arena.moves(SideId::P2, snorlax).0,
        vec![(MoveId::TACKLE, 1)]
    );
    let own = arena.moves_by(Observer::Side(SideId::P1), SideId::P1, snorlax);
    assert_eq!(own, (vec![(MoveId::HARDEN, 0)], true));
    // Back in the disguise and unmasked: what it shows now is its own, what it showed before
    // is still believed the disguise's.
    arena.turn(&[Act::Switch(zoroark)], &[Act::Move(0, None)], Vec::new());
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], hit(100));
    assert_eq!(
        arena.moves(SideId::P2, zoroark).0,
        vec![(MoveId::SPLASH, 1)]
    );
    assert_eq!(
        arena.moves(SideId::P2, snorlax).0,
        vec![(MoveId::TACKLE, 1)]
    );
}

#[test]
fn an_illusions_moves_shown_over_its_stays_add_up() {
    // A move shown in two stays in the disguise, each left in it: the disguise's, used twice (its
    // two records become one as it leaves the second time — checked in debug builds).
    let team = || {
        vec![
            set("Zoroark", &["Tackle", "Splash"], "Illusion", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ]
    };
    let foe = || vec![set("Machamp", &["Splash", "Tackle"], "Guts", None)];
    let (zoroark, snorlax) = (PokemonId(1), PokemonId(2));
    let mut arena = battle(Generation::Gen5, BattleMode::Singles, team(), foe(), |_| {});
    for _ in 0..2 {
        arena.turn(&[Act::Move(1, None)], &[Act::Move(0, None)], Vec::new());
        arena.turn(&[Act::Switch(snorlax)], &[Act::Move(0, None)], Vec::new());
        arena.turn(&[Act::Switch(zoroark)], &[Act::Move(0, None)], Vec::new());
    }
    assert_eq!(
        arena.moves(SideId::P2, snorlax).0,
        vec![(MoveId::SPLASH, 2)]
    );
    // Shown in the disguise and unmasked, then so again in a later stay: its own, used twice
    // (the two records become one as it is unmasked the second time).
    let mut arena = battle(Generation::Gen5, BattleMode::Singles, team(), foe(), |_| {});
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], hit(100));
    arena.turn(&[Act::Switch(snorlax)], &[Act::Move(0, None)], Vec::new());
    arena.turn(&[Act::Switch(zoroark)], &[Act::Move(0, None)], Vec::new());
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], hit(100));
    assert_eq!(
        arena.moves(SideId::P2, zoroark).0,
        vec![(MoveId::SPLASH, 2)]
    );
    assert_eq!(arena.moves(SideId::P2, snorlax).0, Vec::new());
}

#[test]
fn a_disguise_left_terastallized_is_believed_so_until_its_holder_is_unmasked() {
    // Zoroark Terastallizes in Snorlax's guise and leaves in it: Snorlax is believed
    // Terastallized. Back in the guise and unmasked, the Tera is Zoroark's (a side Terastallizes
    // once): Snorlax, never seen itself, is believed so no more — Showdown's client (`replace`).
    let mut zoroark_set = set("Zoroark", &["Tackle", "Splash"], "Illusion", None);
    zoroark_set.tera_type = Some(PokemonType::Dark);
    let team = vec![
        zoroark_set,
        set("Pikachu", &["Splash"], "Static", None),
        set("Snorlax", &["Harden"], "Thick Fat", None),
    ];
    let foe = vec![set("Garchomp", &["Splash", "Tackle"], "Rough Skin", None)];
    let (zoroark, pikachu, snorlax) = (PokemonId(1), PokemonId(2), PokemonId(3));
    let mut arena = battle(Generation::Gen9, BattleMode::Singles, team, foe, |_| {});
    let tera = |arena: &Arena, pokemon| arena.seen(SideId::P2, pokemon, |p| p.terastallized());
    arena.turn(
        &[Act::With(1, MechanicChoice::Terastallize)],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(tera(&arena, snorlax), Some(PokemonType::Dark));
    arena.turn(&[Act::Switch(pikachu)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(tera(&arena, snorlax), Some(PokemonType::Dark));
    arena.turn(&[Act::Switch(zoroark)], &[Act::Move(0, None)], Vec::new());
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], hit(100));
    assert_eq!(tera(&arena, zoroark), Some(PokemonType::Dark));
    assert_eq!(tera(&arena, snorlax), None);
}

#[test]
fn gen_1_transforming_into_a_transformed_pokemon_takes_its_copy() {
    // Gen 1 transforms into a transformed Pokémon: Ditto took Snorlax's moves, so Snorlax,
    // transforming into Ditto, takes its own back — a move it uses is its own, not Ditto's. Its
    // copy's uses are those since it transformed (its Transform came before).
    let mut arena = battle(
        Generation::Gen1,
        BattleMode::Singles,
        vec![
            set("Ditto", &["Transform"], "", None),
            set("Chansey", &["Splash"], "", None),
        ],
        vec![set("Snorlax", &["Transform", "Splash"], "", None)],
        |_| {},
    );
    let (ditto, snorlax) = (PokemonId(1), PokemonId(3));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    arena.turn(
        &[Act::Switch(PokemonId(2))],
        &[Act::Move(1, None)],
        Vec::new(),
    );
    assert_eq!(
        sorted(arena.moves_by(Observer::Spectator, SideId::P2, snorlax).0),
        sorted(vec![(MoveId::TRANSFORM, 0), (MoveId::SPLASH, 1)])
    );
    assert_eq!(
        arena.moves_by(Observer::Spectator, SideId::P1, ditto).0,
        vec![(MoveId::TRANSFORM, 1)]
    );
}

#[test]
fn gen_3_intimidate_is_not_announced_when_every_foe_is_behind_a_substitute() {
    // Gen 3 hints that every target has a Substitute and says nothing of the ability
    // (pokeemerald skips a target behind one, its message per target affected); Gen 5 announces
    // it and the Substitute answers `-immune` (DESIGN.md R-M10-24).
    for (generation, shown) in [(Generation::Gen3, false), (Generation::Gen5, true)] {
        let mut arena = battle(
            generation,
            BattleMode::Singles,
            vec![
                set("Snorlax", &["Splash"], "Thick Fat", None),
                set("Gyarados", &["Splash"], "Intimidate", None),
            ],
            vec![set(
                "Blissey",
                &["Substitute", "Splash"],
                "Natural Cure",
                None,
            )],
            |_| {},
        );
        arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
        arena.turn(&[Act::Switch(B)], &[Act::Move(1, None)], Vec::new());
        let expected = if shown {
            Seen::Known(AbilityId::INTIMIDATE)
        } else {
            Seen::Unknown
        };
        assert_eq!(arena.ability(SideId::P2, B), expected, "{generation:?}");
    }
}

#[test]
fn a_volatile_that_shows_itself_is_seen_until_it_ends_and_a_mark_is_not() {
    // Substitute and Focus Energy show themselves (`-start`); Defense Curl's mark does not. The
    // Pokémon's own side sees everything; gone from the field, nothing is seen on it.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![
            set("Snorlax", &["Defense Curl", "Splash"], "Thick Fat", None),
            set("Blissey", &["Splash"], "Natural Cure", None),
        ],
        vec![set(
            "Conkeldurr",
            &["Substitute", "Focus Energy"],
            "Guts",
            None,
        )],
        |_| {},
    );
    let conkeldurr = PokemonId(3);
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], Vec::new());
    assert_eq!(
        arena.volatiles(SideId::P1, conkeldurr),
        vec![
            VolatileConditionKind::Substitute,
            VolatileConditionKind::FocusEnergy
        ]
    );
    assert_eq!(arena.volatiles(SideId::P2, A), Vec::new());
    let own = arena.engine.view(Observer::Side(SideId::P1));
    let snorlax = own.side(SideId::P1).team().next().unwrap();
    assert!(
        snorlax
            .volatiles()
            .contains(VolatileConditionKind::DefenseCurl)
    );
    arena.turn(&[Act::Switch(B)], &[Act::Move(1, None)], Vec::new());
    assert_eq!(arena.volatiles(SideId::P2, A), Vec::new());
}

#[test]
fn a_trap_its_trapper_left_is_still_seen_and_a_single_move_one_goes_as_it_acts() {
    // Mean Look's `trapped` has no `onEnd`: its trapper leaving frees the Pokémon with no word,
    // and it is seen trapped until it leaves. Destiny Bond (`-singlemove`) ends with no word too,
    // seen gone as its user next acts.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        // Not a Ghost type: from Gen 6 one escapes trapping.
        vec![set(
            "Snorlax",
            &["Destiny Bond", "Splash"],
            "Thick Fat",
            None,
        )],
        vec![
            set("Umbreon", &["Mean Look", "Splash"], "Synchronize", None),
            set("Blissey", &["Splash"], "Natural Cure", None),
        ],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        sorted_kinds(arena.volatiles(SideId::P2, A)),
        sorted_kinds(vec![
            VolatileConditionKind::DestinyBond,
            VolatileConditionKind::Trapped
        ])
    );
    arena.turn(
        &[Act::Move(1, None)],
        &[Act::Switch(PokemonId(3))],
        Vec::new(),
    );
    assert!(
        !arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Trapped),
        "free"
    );
    assert_eq!(
        arena.volatiles(SideId::P2, A),
        vec![VolatileConditionKind::Trapped]
    );
}

fn sorted_kinds(mut kinds: Vec<VolatileConditionKind>) -> Vec<VolatileConditionKind> {
    kinds.sort_by_key(|kind| format!("{kind:?}"));
    kinds
}

#[test]
fn a_one_turn_volatile_removed_with_no_word_is_seen_gone_with_the_turn() {
    // Snatch (`-singleturn`) goes as it steals with no word: gone, as seen, as the turn ends.
    let mut arena = battle(
        Generation::Gen7,
        BattleMode::Singles,
        vec![set("Sableye", &["Snatch"], "Prankster", None)],
        vec![set("Snorlax", &["Swords Dance"], "Thick Fat", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.volatiles(SideId::P2, A), Vec::new());
}

#[test]
fn baton_pass_hands_on_what_was_seen() {
    let mut arena = battle(
        Generation::Gen7,
        BattleMode::Singles,
        vec![
            set(
                "Ninjask",
                &["Focus Energy", "Baton Pass"],
                "Speed Boost",
                None,
            ),
            set("Snorlax", &["Splash"], "Thick Fat", None),
        ],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    arena.turn(&[Act::Move(1, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        arena.volatiles(SideId::P2, B),
        vec![VolatileConditionKind::FocusEnergy]
    );
}

#[test]
fn dynamax_and_an_abilitys_volatile_show_and_go() {
    // Dynamax has its own `-start`; Flash Fire's boost goes as Skill Swap takes the ability
    // (`-end|p|ability: Flash Fire|[silent]`).
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Snorlax", &["Tackle"], "Thick Fat", None)],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |_| {},
    );
    arena.turn(
        &[Act::With(0, MechanicChoice::Dynamax)],
        &[Act::Move(0, None)],
        hit(100),
    );
    assert_eq!(
        arena.volatiles(SideId::P2, A),
        vec![VolatileConditionKind::Dynamax]
    );
    let mut fire = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Heatran", &["Splash"], "Flash Fire", None)],
        vec![set(
            "Alakazam",
            &["Ember", "Skill Swap"],
            "Synchronize",
            None,
        )],
        |_| {},
    );
    fire.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        fire.volatiles(SideId::P2, A),
        vec![VolatileConditionKind::FlashFire]
    );
    fire.turn(&[Act::Move(0, None)], &[Act::Move(1, None)], Vec::new());
    assert_eq!(fire.volatiles(SideId::P2, A), Vec::new());
}

#[test]
fn gen_1_rage_shows_nothing_and_bide_lost_to_a_full_paralysis_is_still_seen() {
    // Gen 1 starts Rage with no line of its own; Bide's `-start` shows, and a full paralysis
    // takes it with no word (the gen1 `par`'s `removeVolatile('bide')`, no `onEnd`).
    let mut rage = battle(
        Generation::Gen1,
        BattleMode::Singles,
        vec![set("Snorlax", &["Rage"], "", None)],
        vec![set("Chansey", &["Splash"], "", None)],
        |_| {},
    );
    rage.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(rage.volatiles(SideId::P2, A), Vec::new());
    let mut bide = battle(
        Generation::Gen1,
        BattleMode::Singles,
        vec![set("Snorlax", &["Bide"], "", None)],
        vec![set("Jolteon", &["Splash"], "", None)],
        |pokemon| {
            pokemon[0].status = Some(pokemon_battle_engine::state::StatusState::new(
                pokemon_battle_engine::state::NonVolatileStatus::Paralysis,
            ))
        },
    );
    bide.turn(
        &[Act::Move(0, None)],
        &[Act::Move(0, None)],
        vec![RandomOverride::bool(
            RandomRequestKind::FullParalysis,
            false,
        )],
    );
    assert_eq!(
        bide.volatiles(SideId::P2, A),
        vec![VolatileConditionKind::Bide]
    );
    bide.turn(
        &[Act::Move(0, None)],
        &[Act::Move(0, None)],
        vec![RandomOverride::bool(RandomRequestKind::FullParalysis, true)],
    );
    assert!(
        !bide
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Bide),
        "lost"
    );
    assert_eq!(
        bide.volatiles(SideId::P2, A),
        vec![VolatileConditionKind::Bide]
    );
}

#[test]
fn magic_guard_names_the_ability_whose_damage_it_keeps_off() {
    // Rough Skin, Rocky Helmet and Aftermath show on their damage's line (`[from] ability: Rough
    // Skin`, R-M10-37). Kept off by the attacker's Magic Guard, an ability's is named from Gen 7
    // (`-activate|holder|ability: Rough Skin`), never an item's; Gen 4-6 say nothing (R-M10-42).
    let garchomp = || set("Garchomp", &["Splash"], "Rough Skin", Some("Rocky Helmet"));
    let clefable = || set("Clefable", &["Tackle"], "Magic Guard", None);
    for (generation, attacker, ability_shown, item_shown) in [
        (Generation::Gen9, clefable(), true, false),
        (Generation::Gen6, clefable(), false, false),
        (
            Generation::Gen9,
            set("Snorlax", &["Tackle"], "Thick Fat", None),
            true,
            true,
        ),
    ] {
        let mut arena = battle(
            generation,
            BattleMode::Singles,
            vec![attacker],
            vec![garchomp()],
            |_| {},
        );
        let holder = PokemonId(2);
        arena.turn(
            &[Act::Move(0, Some((SideId::P2, 0)))],
            &[Act::Move(0, None)],
            hit(100),
        );
        let seen = (
            arena.ability(SideId::P1, holder),
            arena.item(SideId::P1, holder),
        );
        let expected = (
            if ability_shown {
                Seen::Known(AbilityId::ROUGH_SKIN)
            } else {
                Seen::Unknown
            },
            if item_shown {
                Seen::Known(ItemId::ROCKY_HELMET)
            } else {
                Seen::Unknown
            },
        );
        assert_eq!(seen, expected, "{generation:?}");
    }
    for (generation, shown) in [(Generation::Gen9, true), (Generation::Gen6, false)] {
        let mut arena = battle(
            generation,
            BattleMode::Singles,
            vec![clefable()],
            vec![
                set("Skuntank", &["Splash"], "Aftermath", None),
                set("Snorlax", &["Splash"], "Thick Fat", None),
            ],
            |pokemon| pokemon[1].hp = 1,
        );
        arena.turn(
            &[Act::Move(0, Some((SideId::P2, 0)))],
            &[Act::Move(0, None)],
            hit(100),
        );
        // Knocked out by the Tackle, Skuntank's Aftermath is kept off.
        assert!(
            arena
                .engine
                .state()
                .pokemon(PokemonId(2))
                .unwrap()
                .is_fainted()
        );
        assert_eq!(
            arena.ability(SideId::P1, PokemonId(2)),
            if shown {
                Seen::Known(AbilityId::AFTERMATH)
            } else {
                Seen::Unknown
            },
            "{generation:?}"
        );
    }
}

#[test]
fn an_air_balloon_names_itself_as_its_holder_comes_in() {
    // Showdown's Air Balloon `onStart`: `-item|holder|Air Balloon` as its holder comes in — but
    // under Gravity, or for a holder that ignores its item (Klutz) (R-M10-38).
    let garchomp = PokemonId(3);
    for (ability, gravity, shown) in [
        ("Rough Skin", false, true),
        ("Klutz", false, false),
        ("Rough Skin", true, false),
    ] {
        let mut arena = battle(
            Generation::Gen9,
            BattleMode::Singles,
            vec![set("Clefable", &["Gravity", "Harden"], "Magic Guard", None)],
            vec![
                set("Snorlax", &["Splash"], "Thick Fat", None),
                set("Garchomp", &["Splash"], ability, Some("Air Balloon")),
            ],
            |_| {},
        );
        if gravity {
            arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
        }
        arena.turn(&[Act::Move(1, None)], &[Act::Switch(garchomp)], Vec::new());
        let expected = if shown {
            Seen::Known(ItemId::AIR_BALLOON)
        } else {
            Seen::Unknown
        };
        assert_eq!(
            arena.item(SideId::P1, garchomp),
            expected,
            "{ability}, gravity {gravity}"
        );
    }
}

/// What `observer` sees of the ability of the other side's `pokemon`, as a known id or none.
fn known_ability(arena: &Arena, observer: SideId, pokemon: PokemonId) -> Option<AbilityId> {
    match arena.ability(observer, pokemon) {
        Seen::Known(ability) => Some(ability),
        _ => None,
    }
}

#[test]
fn an_ability_that_announces_itself_is_known() {
    // Showdown `this.add('-ability', pokemon, …)` as the ability starts (Dark Aura, Tablets of
    // Ruin, Comatose) or as its holder comes in (Cloud Nine's `onSwitchIn`) — R-M10-39.
    for (ability, id) in [
        ("Dark Aura", AbilityId::DARK_AURA),
        ("Tablets of Ruin", AbilityId::TABLETS_OF_RUIN),
        ("Comatose", AbilityId::COMATOSE),
        ("Cloud Nine", AbilityId::CLOUD_NINE),
    ] {
        let arena = battle(
            Generation::Gen9,
            BattleMode::Singles,
            vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
            vec![set("Garchomp", &["Splash"], ability, None)],
            |_| {},
        );
        assert_eq!(
            known_ability(&arena, SideId::P1, PokemonId(2)),
            Some(id),
            "{ability}"
        );
    }
}

#[test]
fn an_abilitys_refusal_names_it_where_the_line_does() {
    // Levitate's `-immune|p|[from] ability: Levitate` from Gen 4; Gen 3's hit step writes
    // `-immune|p` alone. An Air Balloon's holder is immune with nothing named of its ability.
    for (generation, named) in [(Generation::Gen4, true), (Generation::Gen3, false)] {
        let mut arena = battle(
            generation,
            BattleMode::Singles,
            vec![set("Snorlax", &["Mud-Slap"], "Thick Fat", None)],
            vec![set("Gengar", &["Splash"], "Levitate", None)],
            |_| {},
        );
        arena.turn(
            &[Act::Move(0, Some((SideId::P2, 0)))],
            &[Act::Move(0, None)],
            hit(100),
        );
        assert_eq!(
            known_ability(&arena, SideId::P1, PokemonId(2)),
            named.then_some(AbilityId::LEVITATE),
            "{generation:?}"
        );
    }
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Garchomp", &["Mud-Slap"], "Rough Skin", None)],
        vec![set(
            "Snorlax",
            &["Splash"],
            "Thick Fat",
            Some("Air Balloon"),
        )],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        hit(100),
    );
    assert_eq!(known_ability(&arena, SideId::P1, PokemonId(2)), None);
    // Truant names itself as its holder loafs (`cant|p|ability: Truant`); Damp as it stops a
    // move (`cant|user|ability: Damp|move|[of] holder`).
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        vec![set("Slaking", &["Splash"], "Truant", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(known_ability(&arena, SideId::P1, PokemonId(2)), None);
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::TRUANT)
    );
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Explosion"], "Thick Fat", None)],
        vec![set("Quagsire", &["Splash"], "Damp", None)],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::DAMP)
    );
}

#[test]
fn frisk_illusion_and_the_terrain_and_weather_setters_name_their_ability() {
    // Frisk's line names the frisker (`[of]`); a Surge's `-fieldstart … [from] ability:`; a
    // weather ability a strong weather keeps off (`-ability|p|Drought|[from] Desolate
    // Land|[fail]`) — R-M10-39.
    for (ability, id) in [
        ("Frisk", AbilityId::FRISK),
        ("Electric Surge", AbilityId::ELECTRIC_SURGE),
    ] {
        let arena = battle(
            Generation::Gen9,
            BattleMode::Singles,
            vec![set("Snorlax", &["Splash"], "Thick Fat", Some("Leftovers"))],
            vec![set("Garchomp", &["Splash"], ability, None)],
            |_| {},
        );
        assert_eq!(
            known_ability(&arena, SideId::P1, PokemonId(2)),
            Some(id),
            "{ability}"
        );
    }
    let arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Garchomp", &["Splash"], "Desolate Land", None)],
        vec![set("Snorlax", &["Splash"], "Drought", None)],
        |_| {},
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::DROUGHT)
    );
    // Unmasked, an Illusion's holder is seen as itself, its ability named (`-end|p|Illusion`).
    let mut arena = battle(
        Generation::Gen5,
        BattleMode::Singles,
        vec![
            set("Zoroark", &["Splash"], "Illusion", None),
            set("Snorlax", &["Harden"], "Thick Fat", None),
        ],
        vec![set("Machamp", &["Tackle"], "Guts", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(
        known_ability(&arena, SideId::P2, PokemonId(1)),
        Some(AbilityId::ILLUSION)
    );
}

#[test]
fn an_ability_change_names_the_old_and_the_new_where_the_line_does() {
    // Showdown's `setAbility` writes `-ability|p|new|old|[from] effect|[of] source`: Role Play's
    // `[of]` is its target, whose ability is the new one; the old is the user's own while it had
    // not changed since it came in, back as it leaves (R-M10-39).
    let (snorlax, garchomp) = (PokemonId(1), PokemonId(3));
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![
            set("Snorlax", &["Role Play"], "Thick Fat", None),
            set("Clefable", &["Splash"], "Magic Guard", None),
        ],
        vec![
            set("Garchomp", &["Splash"], "Rough Skin", None),
            set("Quagsire", &["Splash"], "Damp", None),
        ],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P2, snorlax),
        Some(AbilityId::ROUGH_SKIN)
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, garchomp),
        Some(AbilityId::ROUGH_SKIN)
    );
    arena.turn(
        &[Act::Switch(PokemonId(2))],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P2, snorlax),
        Some(AbilityId::THICK_FAT)
    );
    // Worry Seed's line has no `[of]`: its target's old and new.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Worry Seed"], "Thick Fat", None)],
        vec![
            set("Garchomp", &["Splash"], "Rough Skin", None),
            set("Quagsire", &["Splash"], "Damp", None),
        ],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(known_ability(&arena, SideId::P2, snorlax), None);
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::INSOMNIA)
    );
    arena.turn(
        &[Act::Move(0, None)],
        &[Act::Switch(PokemonId(3))],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::ROUGH_SKIN)
    );
}

#[test]
fn a_skill_swap_between_foes_names_both_abilities_from_gen_5() {
    // `battle.skillSwap`: `-activate|user|Skill Swap|target's|user's|[of] target` between foes
    // from Gen 5; Gen 4's names neither, and nothing is known of either after.
    for (generation, named) in [(Generation::Gen5, true), (Generation::Gen4, false)] {
        let mut arena = battle(
            generation,
            BattleMode::Singles,
            vec![set("Snorlax", &["Skill Swap"], "Thick Fat", None)],
            vec![set("Gengar", &["Splash"], "Levitate", None)],
            |_| {},
        );
        arena.turn(
            &[Act::Move(0, Some((SideId::P2, 0)))],
            &[Act::Move(0, None)],
            Vec::new(),
        );
        assert_eq!(
            known_ability(&arena, SideId::P1, PokemonId(2)),
            named.then_some(AbilityId::THICK_FAT),
            "{generation:?}"
        );
        assert_eq!(
            known_ability(&arena, SideId::P2, PokemonId(1)),
            named.then_some(AbilityId::LEVITATE),
            "{generation:?}"
        );
    }
}

#[test]
fn a_grass_types_powder_immunity_names_no_overcoat() {
    // Overcoat's `onTryHit` asks `getImmunity('powder', target)`, false for a Grass type: the
    // TryImmunity step's `-immune|p` turns the powder away, naming nothing; a holder of another
    // type is turned away by the ability (`-immune|p|[from] ability: Overcoat`).
    for (species, named) in [("Leavanny", false), ("Pineco", true)] {
        let mut arena = battle(
            Generation::Gen7,
            BattleMode::Singles,
            vec![set("Snorlax", &["Sleep Powder"], "Thick Fat", None)],
            vec![set(species, &["Splash"], "Overcoat", None)],
            |_| {},
        );
        arena.turn(
            &[Act::Move(0, Some((SideId::P2, 0)))],
            &[Act::Move(0, None)],
            Vec::new(),
        );
        assert_eq!(
            known_ability(&arena, SideId::P1, PokemonId(2)),
            named.then_some(AbilityId::OVERCOAT),
            "{species}"
        );
    }
}

#[test]
fn a_stat_drop_refusal_names_what_refused_it_where_the_line_does() {
    // Clear Body's `-fail|p|unboost|[from] ability: Clear Body|[of] p`, but for a damaging
    // move's secondary effect (`!effect.secondaries`) or Octolock's drops (R-M10-40).
    let metagross = PokemonId(2);
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Snarl", "Growl"], "Thick Fat", None)],
        vec![set("Metagross", &["Splash"], "Clear Body", None)],
        |_| {},
    );
    // Snarl's 95.
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], tape);
    assert_eq!(known_ability(&arena, SideId::P1, metagross), None);
    arena.turn(&[Act::Move(1, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        known_ability(&arena, SideId::P1, metagross),
        Some(AbilityId::CLEAR_BODY)
    );
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Grapploct", &["Octolock"], "Limber", None)],
        vec![set("Metagross", &["Splash"], "Clear Body", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(known_ability(&arena, SideId::P1, metagross), None);
    // Hyper Cutter refusing Intimidate as the leads come in.
    let arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Gyarados", &["Splash"], "Intimidate", None)],
        vec![set("Crabominable", &["Splash"], "Hyper Cutter", None)],
        |_| {},
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::HYPER_CUTTER)
    );
    // Clear Amulet's line names the item, which its holder keeps.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Growl"], "Thick Fat", None)],
        vec![set(
            "Garchomp",
            &["Splash"],
            "Rough Skin",
            Some("Clear Amulet"),
        )],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        arena.item(SideId::P1, PokemonId(2)),
        Seen::Known(ItemId::CLEAR_AMULET)
    );
    // Mirror Armor's `-ability|p|Mirror Armor` while the culprit has HP, the drop sent back
    // landing or not (the Growl user's Attack already at -6).
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Growl"], "Thick Fat", None)],
        vec![set("Corviknight", &["Splash"], "Mirror Armor", None)],
        |team| team[0].stages.attack = -6,
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::MIRROR_ARMOR)
    );
}

#[test]
fn flower_veil_names_its_holder_as_it_keeps_an_allys_drop_off() {
    // `-block|ally|ability: Flower Veil|[of] holder`: a Grass-type ally's drop refused.
    let (florges, venusaur) = (PokemonId(3), PokemonId(4));
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Doubles,
        vec![
            set("Snorlax", &["Growl"], "Thick Fat", None),
            set("Clefable", &["Splash"], "Magic Guard", None),
        ],
        vec![
            set("Florges", &["Splash"], "Flower Veil", None),
            set("Venusaur", &["Splash"], "Overgrow", None),
        ],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, None), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, florges),
        Some(AbilityId::FLOWER_VEIL)
    );
    assert_eq!(known_ability(&arena, SideId::P1, venusaur), None);
}

#[test]
fn an_abilitys_status_refusal_names_it_for_a_status_its_effect_carries() {
    // Limber's `onSetStatus`: `-immune|p|[from] ability: Limber` when the effect carries the
    // status (`effect.status`: Thunder Wave), nothing for a secondary effect's (Body Slam's 30%)
    // (R-M10-40).
    let persian = PokemonId(2);
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set(
            "Snorlax",
            &["Body Slam", "Thunder Wave"],
            "Thick Fat",
            None,
        )],
        vec![set("Persian", &["Splash"], "Limber", None)],
        |_| {},
    );
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, true));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], tape);
    assert_eq!(known_ability(&arena, SideId::P1, persian), None);
    arena.turn(
        &[Act::Move(1, None)],
        &[Act::Move(0, None)],
        vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)],
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, persian),
        Some(AbilityId::LIMBER)
    );
    // Sweet Veil's `-block|p|ability: Sweet Veil|[of] holder` for any sleep: Effect Spore's on
    // the holder's own contact move.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Breloom", &["Splash"], "Effect Spore", None)],
        vec![set("Alcremie", &["Tackle"], "Sweet Veil", None)],
        |_| {},
    );
    let mut tape = hit(100);
    tape.push(RandomOverride::range(RandomRequestKind::EffectSpore, 5));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], tape);
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::SWEET_VEIL)
    );
}

#[test]
fn ice_face_broken_is_still_known_after_its_holder_leaves() {
    // Ice Face's `-activate` names it; its forme change for good (`detailschange`) leaves the
    // ability (`!['disguise', 'iceface'].includes(source.id)`), so the holder leaving takes back
    // Ice Face, not an unknown one (R-M10-40).
    let eiscue = PokemonId(2);
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Snorlax", &["Tackle"], "Thick Fat", None)],
        vec![
            set("Eiscue", &["Splash"], "Ice Face", None),
            set("Clefable", &["Splash"], "Magic Guard", None),
        ],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(
        known_ability(&arena, SideId::P1, eiscue),
        Some(AbilityId::ICE_FACE)
    );
    arena.turn(
        &[Act::Move(0, None)],
        &[Act::Switch(PokemonId(3))],
        hit(100),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, eiscue),
        Some(AbilityId::ICE_FACE)
    );
}

#[test]
fn an_item_an_ability_takes_and_curious_medicine_name_the_ability() {
    // Pickpocket's and Magician's `-item|holder|item|[from] ability: …|[of] other`; Curious
    // Medicine's `-clearboost|ally|[from] ability: Curious Medicine|[of] holder`, stages or not
    // (R-M10-40).
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Tackle"], "Thick Fat", Some("Leftovers"))],
        vec![set("Weavile", &["Splash"], "Pickpocket", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::PICKPOCKET)
    );
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Splash"], "Thick Fat", Some("Leftovers"))],
        vec![set("Delphox", &["Ember"], "Magician", None)],
        |_| {},
    );
    let mut tape = hit(100);
    tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], tape);
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::MAGICIAN)
    );
    let arena = battle(
        Generation::Gen9,
        BattleMode::Doubles,
        vec![
            set("Snorlax", &["Splash"], "Thick Fat", None),
            set("Clefable", &["Splash"], "Magic Guard", None),
        ],
        vec![
            set("Slowking-Galar", &["Splash"], "Curious Medicine", None),
            set("Garchomp", &["Splash"], "Rough Skin", None),
        ],
        |_| {},
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(3)),
        Some(AbilityId::CURIOUS_MEDICINE)
    );
}

#[test]
fn a_transforms_ability_is_known_to_whoever_knows_its_targets() {
    // `-transform|user|target` names no ability, but the user's is the target's
    // (`setAbility(pokemon.ability, …, isTransform)`): the target's side knows it, everyone once
    // the target's was named (Intimidate's) (R-M10-40).
    let ditto = PokemonId(2);
    let spectator = |arena: &Arena| {
        arena.seen_by(Observer::Spectator, SideId::P2, ditto, |p| {
            match p.ability() {
                Seen::Known(ability) => Some(ability),
                _ => None,
            }
        })
    };
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        vec![set("Ditto", &["Transform"], "Limber", None)],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, None)],
        &[Act::Move(0, Some((SideId::P1, 0)))],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, ditto),
        Some(AbilityId::THICK_FAT)
    );
    assert_eq!(spectator(&arena), None);
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Gyarados", &["Splash"], "Intimidate", None)],
        vec![set("Ditto", &["Transform"], "Limber", None)],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, None)],
        &[Act::Move(0, Some((SideId::P1, 0)))],
        Vec::new(),
    );
    assert_eq!(spectator(&arena), Some(AbilityId::INTIMIDATE));
    // Imposter is named, then the ability taken; back on its own as it leaves.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        vec![
            set("Ditto", &["Transform"], "Imposter", None),
            set("Clefable", &["Splash"], "Magic Guard", None),
        ],
        |_| {},
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, ditto),
        Some(AbilityId::THICK_FAT)
    );
    arena.turn(
        &[Act::Move(0, None)],
        &[Act::Switch(PokemonId(3))],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, ditto),
        Some(AbilityId::IMPOSTER)
    );
}

#[test]
fn rest_refused_synchronize_passed_on_and_a_dance_name_their_ability() {
    // Rest's `-fail|user|[from] ability: Insomnia|[of] user` (but at full HP, `-fail|user|heal`);
    // Gen 4 Synchronize's status line (`-status|culprit|par|[from] ability: Synchronize|[of]
    // holder`); Dancer's `-activate|dancer|ability: Dancer` (R-M10-40).
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        vec![set("Hypno", &["Rest"], "Insomnia", None)],
        |team| team[1].hp = 50,
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::INSOMNIA)
    );
    let mut arena = battle(
        Generation::Gen4,
        BattleMode::Singles,
        vec![set("Snorlax", &["Thunder Wave"], "Thick Fat", None)],
        vec![set("Alakazam", &["Splash"], "Synchronize", None)],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::SYNCHRONIZE)
    );
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Doubles,
        vec![
            set("Snorlax", &["Swords Dance"], "Thick Fat", None),
            set("Clefable", &["Splash"], "Magic Guard", None),
        ],
        vec![
            set("Oricorio", &["Splash"], "Dancer", None),
            set("Garchomp", &["Splash"], "Rough Skin", None),
        ],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, None), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(3)),
        Some(AbilityId::DANCER)
    );
}

#[test]
fn a_status_moves_volatile_an_ability_refuses_names_it() {
    // Aroma Veil's `-block|target|ability: Aroma Veil|[of] holder` for a move's Taunt;
    // Insomnia's `-immune|target|[from] ability: Insomnia` for Yawn (R-M10-40).
    for (species, ability, id, move_) in [
        ("Aromatisse", "Aroma Veil", AbilityId::AROMA_VEIL, "Taunt"),
        ("Noctowl", "Insomnia", AbilityId::INSOMNIA, "Yawn"),
    ] {
        let mut arena = battle(
            Generation::Gen8,
            BattleMode::Singles,
            vec![set("Snorlax", &[move_], "Thick Fat", None)],
            vec![set(species, &["Splash"], ability, None)],
            |_| {},
        );
        arena.turn(
            &[Act::Move(0, Some((SideId::P2, 0)))],
            &[Act::Move(0, None)],
            Vec::new(),
        );
        assert_eq!(
            known_ability(&arena, SideId::P1, PokemonId(2)),
            Some(id),
            "{ability}"
        );
    }
}

#[test]
fn a_forme_for_good_shows_its_ability_as_its_own() {
    // `detailschange` to a forme for good: its first ability, its own from then on
    // (`setAbility(species.abilities[0])`, `baseAbility`) — Palafin's Hero forme as it leaves,
    // known before it comes back to announce it (R-M10-40).
    let palafin = PokemonId(2);
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        vec![
            set("Palafin", &["Splash"], "Zero to Hero", None),
            set("Clefable", &["Splash"], "Magic Guard", None),
        ],
        |_| {},
    );
    assert_eq!(known_ability(&arena, SideId::P1, palafin), None);
    arena.turn(
        &[Act::Move(0, None)],
        &[Act::Switch(PokemonId(3))],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, palafin),
        Some(AbilityId::ZERO_TO_HERO)
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Switch(palafin)], Vec::new());
    assert_eq!(
        known_ability(&arena, SideId::P1, palafin),
        Some(AbilityId::ZERO_TO_HERO)
    );
}

#[test]
fn a_trick_refused_shows_nothing_and_one_given_nothing_is_seen_empty_handed() {
    // A Trick that leaves one side with nothing says so (`-enditem|p|item|[silent]|[from] move:
    // Trick`); one an item refuses gives each its own back unseen, what was seen standing
    // (R-M10-40).
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set(
            "Alakazam",
            &["Trick"],
            "Synchronize",
            Some("Choice Scarf"),
        )],
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.item(SideId::P2, A), Seen::KnownNone);
    assert_eq!(arena.item(SideId::P1, B), Seen::Known(ItemId::CHOICE_SCARF));
    // Gen 7: a Z-Crystal is not taken; Frisk named the scarf as the leads came in.
    let mut arena = battle(
        Generation::Gen7,
        BattleMode::Singles,
        vec![set(
            "Alakazam",
            &["Trick"],
            "Synchronize",
            Some("Choice Scarf"),
        )],
        vec![set("Gothitelle", &["Splash"], "Frisk", Some("Normalium Z"))],
        |_| {},
    );
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::CHOICE_SCARF));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::CHOICE_SCARF));
}

#[test]
fn poltergeist_harvest_and_a_z_move_name_what_they_show() {
    // Poltergeist's `-activate|target|move: Poltergeist|<item>`; Harvest's `-item|holder|berry|
    // [from] ability: Harvest`; a Z-Move's user holds the Z-Crystal it shows (R-M10-40).
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Dragapult", &["Poltergeist"], "Infiltrator", None)],
        vec![set("Garchomp", &["Splash"], "Rough Skin", Some("Life Orb"))],
        |_| {},
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        tape,
    );
    assert_eq!(
        arena.item(SideId::P1, PokemonId(2)),
        Seen::Known(ItemId::LIFE_ORB)
    );
    // In the sun Harvest takes back the berry eaten (`lastItem`) at the turn's end.
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Torkoal", &["Splash"], "Drought", None)],
        vec![set("Exeggutor", &["Splash"], "Harvest", None)],
        |team| team[1].history.last_item = Some(ItemId::SITRUS_BERRY),
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::HARVEST)
    );
    assert_eq!(
        arena.item(SideId::P1, PokemonId(2)),
        Seen::Known(ItemId::SITRUS_BERRY)
    );
    let mut arena = battle(
        Generation::Gen7,
        BattleMode::Singles,
        vec![set("Garchomp", &["Splash"], "Rough Skin", None)],
        vec![set(
            "Snorlax",
            &["Splash"],
            "Thick Fat",
            Some("Normalium Z"),
        )],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, None)],
        &[Act::With(0, MechanicChoice::ZMove)],
        Vec::new(),
    );
    assert_eq!(
        arena.item(SideId::P1, PokemonId(2)),
        Seen::Known(ItemId::NORMALIUM_Z)
    );
}

#[test]
fn a_sticky_barb_back_with_its_holder_is_named_by_its_residual() {
    // Switcheroo takes the barb (`-enditem|holder|Sticky Barb|[silent]`: seen gone); a contact
    // move brings it back unannounced, and its residual names it on its holder again, whatever
    // the holder was seen losing (R-M10-40).
    let ferrothorn = PokemonId(1);
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set(
            "Ferrothorn",
            &["Splash", "Power Whip"],
            "Iron Barbs",
            Some("Sticky Barb"),
        )],
        vec![set("Malamar", &["Switcheroo", "Splash"], "Contrary", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.item(SideId::P2, ferrothorn), Seen::KnownNone);
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], tape);
    assert_eq!(
        arena.item(SideId::P2, ferrothorn),
        Seen::Known(ItemId::STICKY_BARB)
    );
}

#[test]
fn an_attract_an_ability_is_not_asked_about_names_nothing() {
    // Attract's `onTryImmunity` wants one male and one female before an ability can refuse
    // (`TryImmunity::OppositeGender`): two females, it fails with no Aroma Veil named. Gen 5
    // Oblivious keeps infatuation off through `onImmunity` (`runStatusImmunity`: `-immune|p`,
    // the ability unnamed) (R-M10-40).
    use pokemon_battle_engine::state::Gender;
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Snorlax", &["Attract"], "Thick Fat", None)],
        vec![set("Aromatisse", &["Splash"], "Aroma Veil", None)],
        |team| {
            team[0].gender = Gender::Female;
            team[1].gender = Gender::Female;
        },
    );
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(known_ability(&arena, SideId::P1, PokemonId(2)), None);
    let mut arena = battle(
        Generation::Gen5,
        BattleMode::Singles,
        vec![set("Snorlax", &["Attract"], "Thick Fat", None)],
        vec![set("Jynx", &["Splash"], "Oblivious", None)],
        |team| {
            team[0].gender = Gender::Male;
            team[1].gender = Gender::Female;
        },
    );
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0)))],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(known_ability(&arena, SideId::P1, PokemonId(2)), None);
}

#[test]
fn poison_puppeteers_confusion_names_it() {
    // `-start|target|confusion|[from] ability: Poison Puppeteer|[of] holder` (R-M10-40).
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        vec![set("Pecharunt", &["Toxic"], "Poison Puppeteer", None)],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, None)],
        &[Act::Move(0, Some((SideId::P1, 0)))],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, PokemonId(2)),
        Some(AbilityId::POISON_PUPPETEER)
    );
}

#[test]
fn max_guard_keeps_flower_shield_off_its_holder() {
    // Flower Shield's own `runEvent('TryHit')` on a Max Guard holder (`-activate|p|move: Max
    // Guard`): the Grass type behind it gains nothing (R-M10-40).
    let gourgeist = PokemonId(1);
    let cherubi = PokemonId(3);
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Doubles,
        vec![
            set("Gourgeist", &["Protect"], "Frisk", None),
            set("Snorlax", &["Splash"], "Thick Fat", None),
        ],
        vec![
            set("Cherubi", &["Flower Shield"], "Chlorophyll", None),
            set("Clefable", &["Splash"], "Magic Guard", None),
        ],
        |_| {},
    );
    arena.turn(
        &[Act::With(0, MechanicChoice::Dynamax), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        Vec::new(),
    );
    let defense = |id: PokemonId| arena.engine.state().pokemon(id).map(|p| p.stages.defense);
    assert_eq!(defense(gourgeist), Some(0));
    assert_eq!(defense(cherubi), Some(1));
}

#[test]
fn a_mega_stone_and_a_primal_orb_name_themselves() {
    // `-mega|p|species|Audinite` names the stone its holder Mega Evolves with; `-primal|p|Red Orb`
    // the orb its holder reverts with as it comes in (R-M10-41).
    let mut arena = battle(
        Generation::Gen7,
        BattleMode::Singles,
        vec![set("Audino", &["Splash"], "Healer", Some("Audinite"))],
        vec![set("Groudon", &["Splash"], "Drought", Some("Red Orb"))],
        |_| {},
    );
    assert_eq!(arena.item(SideId::P1, B), Seen::Known(ItemId::RED_ORB));
    assert_eq!(arena.item(SideId::P2, A), Seen::Unknown);
    arena.turn(
        &[Act::With(0, MechanicChoice::MegaEvolve)],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::AUDINITE));
}

#[test]
fn a_mega_rayquaza_names_no_item() {
    // The `-mega` line names the forme's `requiredItem`: Mega Rayquaza asks Dragon Ascent
    // instead (`-mega|p|Rayquaza|`), and the plate it holds stays unseen (R-M10-62).
    let mut arena = battle(
        Generation::Gen7,
        BattleMode::Singles,
        vec![set(
            "Rayquaza",
            &["Dragon Ascent", "Splash"],
            "Air Lock",
            Some("Meadow Plate"),
        )],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |_| {},
    );
    arena.turn(
        &[Act::With(1, MechanicChoice::MegaEvolve)],
        &[Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(
        pokemon_battle_engine::engine::species_of(arena.engine.state().pokemon(A).unwrap()),
        arena
            .dex
            .species_by_name("Rayquaza-Mega")
            .map(|species| species.id)
    );
    assert_eq!(arena.item(SideId::P2, A), Seen::Unknown);
}

#[test]
fn pickup_names_itself_with_what_it_picks_up() {
    // `-item|holder|Sitrus Berry|[from] ability: Pickup`: the berry a foe ate this turn, taken at
    // the turn's end (R-M10-41).
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Munchlax", &["Tackle"], "Pickup", None)],
        vec![set(
            "Blissey",
            &["Splash"],
            "Natural Cure",
            Some("Sitrus Berry"),
        )],
        |pokemon| pokemon[1].hp = pokemon[1].max_hp / 2 + 1,
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(
        arena.engine.state().pokemon(A).unwrap().held_item,
        Some(ItemId::SITRUS_BERRY)
    );
    assert_eq!(
        known_ability(&arena, SideId::P2, A),
        Some(AbilityId::PICKUP)
    );
    assert_eq!(arena.item(SideId::P2, A), Seen::Known(ItemId::SITRUS_BERRY));
}

#[test]
fn baton_pass_hands_on_a_trap_still_seen_after_its_trapper_left() {
    // Mean Look's `trapped` ends with no line as its trapper leaves: the Pokémon is still seen
    // trapped (R-M10-26), and a Gen 2-4 Baton Pass hands that on as Showdown's client does, by
    // kind (R-M10-41). From Gen 5 a trap is not handed on (`noCopy`).
    for (generation, handed_on) in [(Generation::Gen3, true), (Generation::Gen5, false)] {
        let (snorlax, jolteon, lapras) = (PokemonId(2), PokemonId(3), PokemonId(4));
        let mut arena = battle(
            generation,
            BattleMode::Singles,
            vec![
                set("Umbreon", &["Mean Look"], "Synchronize", None),
                set("Snorlax", &["Splash"], "Thick Fat", None),
            ],
            vec![
                set("Jolteon", &["Baton Pass", "Splash"], "Volt Absorb", None),
                set("Lapras", &["Splash"], "Water Absorb", None),
            ],
            |_| {},
        );
        arena.turn(
            &[Act::Move(0, Some((SideId::P2, 0)))],
            &[Act::Move(1, None)],
            Vec::new(),
        );
        assert!(
            arena
                .volatiles(SideId::P1, jolteon)
                .contains(&VolatileConditionKind::Trapped)
        );
        arena.turn(&[Act::Switch(snorlax)], &[Act::Move(1, None)], Vec::new());
        let trapped = |arena: &Arena, id: PokemonId| {
            arena
                .engine
                .state()
                .pokemon(id)
                .unwrap()
                .volatiles
                .contains(VolatileConditionKind::Trapped)
        };
        assert!(
            !trapped(&arena, jolteon),
            "{generation:?}: the trap is gone"
        );
        assert!(
            arena
                .volatiles(SideId::P1, jolteon)
                .contains(&VolatileConditionKind::Trapped),
            "{generation:?}: but not seen gone"
        );
        arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
        assert_eq!(
            arena.engine.state().side(SideId::P2).active[0],
            Some(lapras)
        );
        assert!(!trapped(&arena, lapras));
        assert_eq!(
            arena
                .volatiles(SideId::P1, lapras)
                .contains(&VolatileConditionKind::Trapped),
            handed_on,
            "{generation:?}"
        );
    }
}

#[test]
fn a_focus_punch_that_lost_its_focus_is_an_action() {
    // `cant|p|Focus Punch|Focus Punch` (its `beforeMoveCallback`, past BeforeMove): the user
    // acted, so the Grudge it showed is seen gone, and the move is shown (R-M10-41).
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Garchomp", &["Splash", "Tackle"], "Rough Skin", None)],
        vec![set(
            "Corvisquire",
            &["Grudge", "Focus Punch"],
            "Pressure",
            None,
        )],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert!(
        arena
            .volatiles(SideId::P1, B)
            .contains(&VolatileConditionKind::Grudge)
    );
    arena.turn(&[Act::Move(1, None)], &[Act::Move(1, None)], hit(100));
    assert!(
        !arena
            .volatiles(SideId::P1, B)
            .contains(&VolatileConditionKind::Grudge)
    );
    assert!(
        arena
            .moves(SideId::P1, B)
            .0
            .iter()
            .any(|(move_id, _)| *move_id == MoveId::FOCUS_PUNCH)
    );
}

#[test]
fn psych_up_replaces_its_users_crit_volatiles_seen() {
    // Gen 6+ Psych Up drops its user's Dragon Cheer, Focus Energy and Laser Focus before it
    // copies the target's: no line of their own, but the `-copyboost` shows the user's are now
    // the target's, as the clients read it (R-M10-45; R-M10-41 had them unseen).
    let elekid = PokemonId(2);
    for (dragonite_move, elekid_move, kind) in [
        (0, 0, VolatileConditionKind::DragonCheer),
        (1, 2, VolatileConditionKind::FocusEnergy),
    ] {
        let mut arena = battle(
            Generation::Gen9,
            BattleMode::Doubles,
            vec![
                set(
                    "Dragonite",
                    &["Dragon Cheer", "Splash"],
                    "Inner Focus",
                    None,
                ),
                set(
                    "Elekid",
                    &["Splash", "Psych Up", "Focus Energy"],
                    "Static",
                    None,
                ),
            ],
            vec![
                set("Snorlax", &["Splash"], "Thick Fat", None),
                set("Blissey", &["Splash"], "Natural Cure", None),
            ],
            |_| {},
        );
        arena.turn(
            &[
                Act::Move(dragonite_move, Some((SideId::P1, 1))),
                Act::Move(elekid_move, None),
            ],
            &[Act::Move(0, None), Act::Move(0, None)],
            Vec::new(),
        );
        assert!(
            arena.volatiles(SideId::P2, elekid).contains(&kind),
            "{kind:?}"
        );
        arena.turn(
            &[Act::Move(1, None), Act::Move(1, Some((SideId::P2, 0)))],
            &[Act::Move(0, None), Act::Move(0, None)],
            Vec::new(),
        );
        assert!(
            !arena
                .engine
                .state()
                .pokemon(elekid)
                .unwrap()
                .volatiles
                .contains(kind)
        );
        assert!(
            !arena.volatiles(SideId::P2, elekid).contains(&kind),
            "{kind:?}"
        );
    }
}

#[test]
fn a_status_refusal_is_the_fastest_holders_before_a_terrains() {
    // Showdown's `SetStatus` handlers run by their holder's speed, none with a priority: those a
    // Pokémon holds (an ability's, an uproar's) before Safeguard and the terrain, which have none
    // (R-M10-42). Alcremie's Sweet Veil keeps its own Rest off before Electric Terrain does
    // (`-block|p|ability: Sweet Veil|[of] p`).
    let alcremie = PokemonId(1);
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Doubles,
        vec![
            set("Alcremie", &["Rest"], "Sweet Veil", None),
            set("Tapu Koko", &["Splash"], "Electric Surge", None),
        ],
        vec![
            set("Snorlax", &["Splash"], "Thick Fat", None),
            set("Blissey", &["Splash"], "Natural Cure", None),
        ],
        |pokemon| pokemon[0].hp = pokemon[0].max_hp / 2,
    );
    arena.turn(
        &[Act::Move(0, None), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(arena.engine.state().pokemon(alcremie).unwrap().status, None);
    assert_eq!(
        known_ability(&arena, SideId::P2, alcremie),
        Some(AbilityId::SWEET_VEIL)
    );
    // A fast Insomnia holder's refusal comes before a slow foe's uproar
    // (`-immune|p|[from] ability: Insomnia`).
    let murkrow = PokemonId(1);
    let mut arena = battle(
        Generation::Gen7,
        BattleMode::Doubles,
        vec![
            set("Murkrow", &["Splash"], "Insomnia", None),
            set("Blissey", &["Splash"], "Natural Cure", None),
        ],
        vec![
            set("Snorlax", &["Uproar"], "Thick Fat", None),
            set("Breloom", &["Spore", "Splash"], "Effect Spore", None),
        ],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, None), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(1, None)],
        Vec::new(),
    );
    assert!(
        arena
            .engine
            .state()
            .pokemon(PokemonId(3))
            .unwrap()
            .volatiles
            .uproar()
            .is_some()
    );
    assert_eq!(known_ability(&arena, SideId::P2, murkrow), None);
    arena.turn(
        &[Act::Move(0, None), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, Some((SideId::P1, 0)))],
        Vec::new(),
    );
    assert_eq!(arena.engine.state().pokemon(murkrow).unwrap().status, None);
    assert_eq!(
        known_ability(&arena, SideId::P2, murkrow),
        Some(AbilityId::INSOMNIA)
    );
}

/// One turn as `Arena::turn` plays it, but read as `actor`'s action has gone, before the rest of
/// the turn: whether `observer` (of the other side) sees `kind` on it then.
fn seen_after_action(
    arena: &mut Arena,
    p1: &[Act],
    p2: &[Act],
    actor: PokemonId,
    observer: SideId,
    kind: VolatileConditionKind,
) -> Option<bool> {
    let choices = [arena.choice(SideId::P1, p1), arena.choice(SideId::P2, p2)];
    arena.engine.submit_turn_choices(choices).unwrap();
    let start = arena.engine.trace().events().len();
    let mut seen = None;
    while let EngineStep::Progressed { .. } = arena.engine.step().unwrap() {
        let acted = arena.engine.trace().events()[start..].iter().any(
            |event| matches!(event, TraceEvent::BeforeMoveResolved { actor: a, .. } if *a == actor),
        );
        if acted && seen.is_none() {
            seen = Some(arena.volatiles(observer, actor).contains(&kind));
        }
    }
    seen
}

#[test]
fn focus_punchs_focus_is_seen_until_the_move_is_used_or_the_focus_lost() {
    // `-singleturn|p|move: Focus Punch` lasts the turn: a sleeping user's `cant` leaves it seen;
    // the move used, or the focus lost (`cant|p|Focus Punch|Focus Punch`), shows it gone at once
    // (the client's `useMove`, `cantUseMove`, R-M10-43).
    let combusken = PokemonId(2);
    for (asleep, p1_move, seen) in [(true, 0, true), (false, 0, false), (false, 1, false)] {
        let mut arena = battle(
            Generation::Gen9,
            BattleMode::Singles,
            vec![set("Garchomp", &["Splash", "Tackle"], "Rough Skin", None)],
            vec![set("Combusken", &["Focus Punch"], "Blaze", None)],
            |pokemon| {
                if asleep {
                    let mut sleep =
                        pokemon_battle_engine::state::StatusState::new(NonVolatileStatus::Sleep);
                    sleep.sleep_turns_remaining = Some(3);
                    sleep.sleep_turns_rolled = Some(3);
                    pokemon[1].status = Some(sleep);
                }
            },
        );
        assert_eq!(
            seen_after_action(
                &mut arena,
                &[Act::Move(p1_move, None)],
                &[Act::Move(0, None)],
                combusken,
                SideId::P1,
                VolatileConditionKind::FocusPunch,
            ),
            Some(seen),
            "asleep {asleep}, P1's move {p1_move}"
        );
        // Gone with the turn.
        assert!(
            !arena
                .volatiles(SideId::P1, combusken)
                .contains(&VolatileConditionKind::FocusPunch)
        );
    }
}

#[test]
fn gen_1_mimic_shows_the_move_it_drew_on_its_target() {
    // Gen 1 Mimic draws among its target's moves (`this.sample(target.moves)`): the move its
    // `-start` names is one the target has (R-M10-43).
    let mut arena = battle(
        Generation::Gen1,
        BattleMode::Singles,
        vec![set("Alakazam", &["Mimic"], "", None)],
        vec![set("Snorlax", &["Body Slam"], "", None)],
        |pokemon| {
            let mut sleep =
                pokemon_battle_engine::state::StatusState::new(NonVolatileStatus::Sleep);
            sleep.sleep_turns_remaining = Some(3);
            sleep.sleep_turns_rolled = Some(3);
            pokemon[1].status = Some(sleep);
        },
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        arena.engine.state().pokemon(A).unwrap().move_slots()[0].move_id,
        MoveId::BODY_SLAM
    );
    assert_eq!(arena.moves(SideId::P1, B).0, vec![(MoveId::BODY_SLAM, 0)]);
}

#[test]
fn protective_pads_name_the_contact_ability_they_keep_off_and_themselves() {
    // `checkMoveMakesContact(move, source, target, true)` (Rough Skin, Iron Barbs, Aftermath,
    // Gooey, Tangling Hair; Mummy and Lingering Aroma against a foe, R-M10-51): a contact move
    // the attacker's Protective Pads keep from making contact names the holder's ability and the
    // pads (`-activate|holder|ability: …`, `-activate|attacker|item: Protective Pads`, R-M10-44).
    for (holder, ability) in [
        (
            set("Garchomp", &["Splash"], "Rough Skin", None),
            AbilityId::ROUGH_SKIN,
        ),
        (set("Goodra", &["Splash"], "Gooey", None), AbilityId::GOOEY),
        (set("Blissey", &["Splash"], "Mummy", None), AbilityId::MUMMY),
        (
            set("Oinkologne", &["Splash"], "Lingering Aroma", None),
            AbilityId::LINGERING_AROMA,
        ),
    ] {
        let mut arena = battle(
            Generation::Gen9,
            BattleMode::Singles,
            vec![set(
                "Snorlax",
                &["Tackle"],
                "Thick Fat",
                Some("Protective Pads"),
            )],
            vec![holder],
            |_| {},
        );
        arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
        let snorlax = arena.engine.state().pokemon(A).unwrap();
        assert_eq!((snorlax.hp, snorlax.stages.speed), (snorlax.max_hp, 0));
        assert_eq!(snorlax.ability, AbilityId::THICK_FAT);
        assert_eq!(known_ability(&arena, SideId::P1, B), Some(ability));
        assert_eq!(
            arena.item(SideId::P2, A),
            Seen::Known(ItemId::PROTECTIVE_PADS)
        );
    }
    // Mummy's and Lingering Aroma's `!source.isAlly(target)`: a partner's pads go unnamed.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Doubles,
        vec![
            set("Snorlax", &["Tackle"], "Thick Fat", Some("Protective Pads")),
            set("Clefable", &["Splash"], "Mummy", None),
        ],
        vec![
            set("Blissey", &["Splash"], "Natural Cure", None),
            set("Chansey", &["Splash"], "Natural Cure", None),
        ],
        |_| {},
    );
    arena.turn(
        &[Act::Move(0, Some((SideId::P1, 1))), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        hit(100),
    );
    assert_eq!(known_ability(&arena, SideId::P2, B), None);
    assert_eq!(arena.item(SideId::P2, A), Seen::Unknown);
}

#[test]
fn a_turns_protect_stays_seen_where_the_battle_ends_in_the_turn() {
    // `-singleturn|p|Protect` ends with no line: the observers see it gone as the turn ends
    // (`upkeep`) — not where the battle ends in the turn's residual, a Perish Song count's faint
    // coming before the Protect's end (R-M10-44; the battle Nightly 20261028-044008 found, where
    // the Protect's end does run after the faint, is a fixture).
    let mut arena = battle(
        Generation::Gen3,
        BattleMode::Singles,
        vec![set(
            "Snorlax",
            &["Perish Song", "Splash"],
            "Thick Fat",
            None,
        )],
        vec![set("Blissey", &["Protect", "Splash"], "Soundproof", None)],
        |_| {},
    );
    for p1_move in [0, 1, 1] {
        arena.turn(
            &[Act::Move(p1_move, None)],
            &[Act::Move(1, None)],
            Vec::new(),
        );
    }
    assert!(arena.engine.state().outcome.is_none());
    arena.turn(&[Act::Move(1, None)], &[Act::Move(0, None)], Vec::new());
    assert!(arena.engine.state().outcome.is_some());
    assert!(!arena.engine.state().pokemon(B).unwrap().is_fainted());
    assert!(
        arena
            .volatiles(SideId::P1, B)
            .contains(&VolatileConditionKind::Protect)
    );
}

#[test]
fn an_item_that_refuses_a_move_or_an_ability_change_names_itself() {
    // Safety Goggles' `-activate|p|item: Safety Goggles|Spore`, Ability Shield's `-block|p|item:
    // Ability Shield` against Gastro Acid and against Entrainment's `SetAbility` (R-M10-46).
    for (holder, user_move, item) in [
        (
            set("Snorlax", &["Splash"], "Thick Fat", Some("Safety Goggles")),
            "Spore",
            ItemId::SAFETY_GOGGLES,
        ),
        (
            set("Snorlax", &["Splash"], "Thick Fat", Some("Ability Shield")),
            "Gastro Acid",
            ItemId::ABILITY_SHIELD,
        ),
        (
            set("Snorlax", &["Splash"], "Thick Fat", Some("Ability Shield")),
            "Entrainment",
            ItemId::ABILITY_SHIELD,
        ),
    ] {
        let mut arena = battle(
            Generation::Gen9,
            BattleMode::Singles,
            vec![set("Breloom", &[user_move], "Effect Spore", None)],
            vec![holder],
            |_| {},
        );
        arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
        assert_eq!(arena.item(SideId::P1, B), Seen::Known(item), "{user_move}");
        assert_eq!(
            known_ability(&arena, SideId::P1, B),
            None,
            "{user_move}: the ability stays its own"
        );
    }
    // Mummy's change refused: the line names Mummy on its holder first (`-ability|holder|
    // Mummy`), then the attacker's shield.
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set(
            "Snorlax",
            &["Tackle"],
            "Thick Fat",
            Some("Ability Shield"),
        )],
        vec![set("Garchomp", &["Splash"], "Mummy", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(known_ability(&arena, SideId::P1, B), Some(AbilityId::MUMMY));
    assert_eq!(
        arena.item(SideId::P2, A),
        Seen::Known(ItemId::ABILITY_SHIELD)
    );
}

#[test]
fn heal_bell_names_a_soundproof_partner_and_wind_power_names_itself() {
    // Heal Bell's `-immune|ally|[from] ability: Soundproof` for an active partner it passes over;
    // Charge's `-start|p|Charge|move|[from] ability: Wind Power` (R-M10-46).
    // (Gen 3: from Gen 6 the partner's Soundproof says so on its own too, `onAllyTryHitSide`.)
    let mut arena = battle(
        Generation::Gen3,
        BattleMode::Doubles,
        vec![
            set("Blissey", &["Heal Bell"], "Natural Cure", None),
            set("Exploud", &["Splash"], "Soundproof", None),
        ],
        vec![
            set("Snorlax", &["Splash"], "Thick Fat", None),
            set("Lapras", &["Splash"], "Water Absorb", None),
        ],
        |pokemon| {
            pokemon[1].status = Some(pokemon_battle_engine::state::StatusState::new(
                NonVolatileStatus::Poison,
            ));
        },
    );
    arena.turn(
        &[Act::Move(0, None), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        Vec::new(),
    );
    assert_eq!(
        known_ability(&arena, SideId::P2, PokemonId(2)),
        Some(AbilityId::SOUNDPROOF)
    );
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Corviknight", &["Gust"], "Pressure", None)],
        vec![set("Wattrel", &["Splash"], "Wind Power", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], hit(100));
    assert_eq!(
        known_ability(&arena, SideId::P1, B),
        Some(AbilityId::WIND_POWER)
    );
}

#[test]
fn a_pokemon_seen_fainting_seen_standing_again_did_not_faint() {
    // Zoroark faints in Garchomp's guise (to its poison: a move's hit breaks the Illusion first):
    // Garchomp is seen fainting. The real Garchomp coming in shows the fallen one was an
    // Illusion's holder: Garchomp did not faint, off the field too (R-M10-47).
    let (zoroark, snorlax, garchomp) = (PokemonId(1), PokemonId(2), PokemonId(3));
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![
            set("Zoroark", &["Splash"], "Illusion", None),
            set("Snorlax", &["Splash"], "Thick Fat", None),
            set("Garchomp", &["Splash"], "Rough Skin", None),
        ],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |pokemon| {
            pokemon[0].hp = 1;
            pokemon[0].status = Some(pokemon_battle_engine::state::StatusState::new(
                NonVolatileStatus::Poison,
            ));
        },
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert!(arena.engine.state().pokemon(zoroark).unwrap().is_fainted());
    assert!(arena.seen(SideId::P2, garchomp, |p| p.fainted()));
    arena.turn(&[Act::Switch(garchomp)], &[Act::Move(0, None)], Vec::new());
    arena.turn(&[Act::Switch(snorlax)], &[Act::Move(0, None)], Vec::new());
    assert!(!arena.seen(SideId::P2, garchomp, |p| p.fainted()));
}

#[test]
fn a_soundproof_user_of_heal_bell_names_its_own_soundproof() {
    // Gen 6+ `onAllyTryHitSide` runs for `alliesAndSelf`: Soundproof names its holder's own Heal
    // Bell (`-immune|user|[from] ability: Soundproof`) — Gen 8, where Heal Bell's `onHit` leaves
    // its user out (Gen 6-7's names it there too); Sap Sipper leaves its holder's own
    // Aromatherapy out (`source === this.effectState.target`, R-M10-53).
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Snorlax", &["Heal Bell"], "Soundproof", None)],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        known_ability(&arena, SideId::P2, A),
        Some(AbilityId::SOUNDPROOF)
    );
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Snorlax", &["Aromatherapy"], "Sap Sipper", None)],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(arena.engine.state().pokemon(A).unwrap().stages.attack, 0);
}

#[test]
fn of_two_guards_on_a_side_the_faster_holders_refuses_and_is_named() {
    // `runEvent`'s `speedSort`: Aroma Veil (`onAllyTryAddVolatile`) and Flower Veil
    // (`onAllyTryBoost`) on both foes — the faster holder's refuses, `[of]` it (R-M10-53; an
    // Aroma Veil Entrainment gave a Druddigon beside its partner Milcery's own).
    let foes = |ability: &str| {
        vec![
            set("Abomasnow", &["Splash"], ability, None),
            set("Weavile", &["Splash"], ability, None),
        ]
    };
    let ours = |user_move: &str| {
        vec![
            set("Snorlax", &[user_move], "Thick Fat", None),
            set("Blissey", &["Splash"], "Natural Cure", None),
        ]
    };
    let abomasnow = PokemonId(3);
    let weavile = PokemonId(4);
    for (user_move, ability) in [
        ("Taunt", AbilityId::AROMA_VEIL),
        ("Growl", AbilityId::FLOWER_VEIL),
    ] {
        let name = if ability == AbilityId::AROMA_VEIL {
            "Aroma Veil"
        } else {
            "Flower Veil"
        };
        let mut arena = battle(
            Generation::Gen9,
            BattleMode::Doubles,
            ours(user_move),
            foes(name),
            |_| {},
        );
        arena.turn(
            &[Act::Move(0, Some((SideId::P2, 0))), Act::Move(0, None)],
            &[Act::Move(0, None), Act::Move(0, None)],
            Vec::new(),
        );
        assert_eq!(
            known_ability(&arena, SideId::P1, weavile),
            Some(ability),
            "{name}"
        );
        assert_eq!(known_ability(&arena, SideId::P1, abomasnow), None, "{name}");
    }
}

#[test]
fn a_choice_locks_refusal_is_a_use_and_a_gen_1_waking_turn_no_action() {
    // Gen 7+ `choicelock.onBeforeMove` is no `cant`: `move|p|<move>|[still]`, then `-fail` — a
    // use as anyone sees it, though no PP goes. Gen 1's waking turn (`slp.onBeforeMove`, `cant`
    // only while `time > 0`) shows nothing: no action, a recharge it owes still seen (R-M10-53).
    use pokemon_battle_engine::engine::{BeforeMoveBlockReason, CanMoveResult};
    use pokemon_battle_engine::state::Disclosure;
    let locked = TraceEvent::BeforeMoveResolved {
        actor: A,
        own_move: Some(MoveId::PSYCHIC_TERRAIN),
        result: CanMoveResult::Blocked(BeforeMoveBlockReason::ChoiceLock),
    };
    assert_eq!(
        locked.disclosures(),
        [
            Some(Disclosure::MoveUsed(A, MoveId::PSYCHIC_TERRAIN)),
            Some(Disclosure::Acted(A)),
        ]
    );
    let waking = TraceEvent::BeforeMoveResolved {
        actor: A,
        own_move: Some(MoveId::TACKLE),
        result: CanMoveResult::Blocked(BeforeMoveBlockReason::WakingTurn),
    };
    assert_eq!(waking.disclosures(), [None, None]);
}

#[test]
fn an_ability_transform_gave_goes_as_its_holder_faints() {
    // A faint clears the volatiles as leaving does (`clearVolatile`, the client's `faint`): the
    // ability Transform gave is gone, the holder's own seen again — unknown here, for a Ditto a
    // Revival Blessing may bring back (R-M10-54).
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Ditto", &["Transform"], "Limber", None)],
        vec![set("Snorlax", &["Splash", "Tackle"], "Thick Fat", None)],
        |pokemon| pokemon[0].hp = 1,
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    assert_eq!(
        known_ability(&arena, SideId::P2, A),
        Some(AbilityId::THICK_FAT)
    );
    // The copy's Speed is Snorlax's: the ties are drawn first (as the order is made, and again
    // as it is sorted anew; marked optional).
    let mut tape = vec![
        RandomOverride::raw(RandomRequestKind::SpeedTie, 1).optional(),
        RandomOverride::raw(RandomRequestKind::SpeedTie, 1).optional(),
    ];
    tape.extend(hit(100));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(1, None)], tape);
    assert!(arena.engine.state().pokemon(A).unwrap().is_fainted());
    assert_eq!(known_ability(&arena, SideId::P2, A), None);
}

#[test]
fn ability_shield_names_itself_where_trace_or_neutralizing_gas_passes_it_over() {
    // Trace's `onStart` and Neutralizing Gas's `onSwitchIn` write `-block|p|item: Ability
    // Shield` for a holder they pass over (R-M10-55; a Porygon2 tracing nothing as it came in).
    let traced = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set(
            "Porygon2",
            &["Splash"],
            "Trace",
            Some("Ability Shield"),
        )],
        vec![set("Snorlax", &["Splash"], "Thick Fat", None)],
        |_| {},
    );
    assert_eq!(
        traced.item(SideId::P2, A),
        Seen::Known(ItemId::ABILITY_SHIELD)
    );
    let gassed = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Weezing", &["Splash"], "Neutralizing Gas", None)],
        vec![set(
            "Snorlax",
            &["Splash"],
            "Thick Fat",
            Some("Ability Shield"),
        )],
        |_| {},
    );
    assert_eq!(
        gassed.item(SideId::P1, B),
        Seen::Known(ItemId::ABILITY_SHIELD)
    );
}

#[test]
fn a_gen_9_charge_outlasts_the_recharge_turn() {
    // Gen 9 Charge ends on an Electric move's `onAfterMove` / `onMoveAborted`; the recharge
    // turn aborts Showdown's `recharge` (`mustrecharge.onLockMove`, a Normal move), not the move
    // chosen — Charge stays whatever was picked (R-M10-57; a Kilowattrel's Hyper Beam).
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set(
            "Pikachu",
            &["Charge", "Hyper Beam", "Thunderbolt"],
            "Static",
            None,
        )],
        vec![set("Blissey", &["Splash"], "Natural Cure", None)],
        |_| {},
    );
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], Vec::new());
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true)];
    tape.extend(hit(100));
    arena.turn(&[Act::Move(1, None)], &[Act::Move(0, None)], tape);
    let charged = |arena: &Arena| {
        arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Charge)
    };
    assert!(charged(&arena));
    arena.turn(&[Act::Move(2, None)], &[Act::Move(0, None)], Vec::new());
    assert!(
        !arena
            .engine
            .state()
            .pokemon(A)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::MustRecharge)
    );
    assert!(charged(&arena));
}

#[test]
fn aroma_veil_names_itself_refusing_a_secondarys_heal_block() {
    // Aroma Veil's `onAllyTryAddVolatile` names itself for a move's effect, a secondary's too
    // (`-block|target|ability: Aroma Veil|[of] holder`: Psychic Noise's Heal Block, R-M10-59).
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Singles,
        vec![set("Snorlax", &["Psychic Noise"], "Thick Fat", None)],
        vec![set("Milcery", &["Splash"], "Aroma Veil", None)],
        |_| {},
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true).optional()];
    tape.extend(hit(100));
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], tape);
    assert!(
        !arena
            .engine
            .state()
            .pokemon(B)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::HealBlock)
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, B),
        Some(AbilityId::AROMA_VEIL)
    );
}

#[test]
fn aroma_veil_says_nothing_of_a_torment_the_target_has_already() {
    // `addVolatile` refuses a volatile the target has before `TryAddVolatile` runs: a second
    // Torment fails on its own, and Aroma Veil does not name itself (R-M10-60).
    let mut arena = battle(
        Generation::Gen8,
        BattleMode::Singles,
        vec![set("Snorlax", &["Torment"], "Thick Fat", None)],
        vec![set("Alcremie", &["Splash"], "Aroma Veil", None)],
        |pokemon| {
            assert!(
                pokemon[1]
                    .volatiles
                    .set_marker(VolatileConditionKind::Torment)
            );
        },
    );
    let tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true).optional()];
    arena.turn(&[Act::Move(0, None)], &[Act::Move(0, None)], tape);
    assert!(
        arena
            .engine
            .state()
            .pokemon(B)
            .unwrap()
            .volatiles
            .contains(VolatileConditionKind::Torment)
    );
    assert_eq!(known_ability(&arena, SideId::P1, B), None);
}

#[test]
fn a_supreme_overlord_the_gas_lets_start_again_counts_the_holder_knocked_out() {
    // Showdown `faintMessages` counts a faint (`side.totalFainted++`) before its ability's `End`:
    // a Neutralizing Gas holder a move knocks out is counted by the time its gas ends at the hit
    // loop's end, and the Supreme Overlord it lets start again counts it — and says so
    // (`-activate|p|ability: Supreme Overlord`, R-M10-61).
    const KINGAMBIT: PokemonId = PokemonId(4);
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Doubles,
        vec![
            set("Espeon", &["Psychic"], "Synchronize", None),
            set("Blissey", &["Splash"], "Natural Cure", None),
        ],
        vec![
            set("Weezing", &["Splash"], "Neutralizing Gas", None),
            set("Kingambit", &["Splash"], "Supreme Overlord", None),
        ],
        |pokemon| pokemon[2].hp = 1,
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true).optional()];
    tape.extend(hit(100));
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0))), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        tape,
    );
    assert!(
        arena
            .engine
            .state()
            .pokemon(PokemonId(3))
            .unwrap()
            .is_fainted()
    );
    assert_eq!(
        arena
            .engine
            .state()
            .pokemon(KINGAMBIT)
            .unwrap()
            .ability_state
            .fallen,
        1
    );
    assert_eq!(arena.engine.state().side(SideId::P2).total_fainted, 1);
    assert_eq!(
        known_ability(&arena, SideId::P1, KINGAMBIT),
        Some(AbilityId::SUPREME_OVERLORD)
    );
}

#[test]
fn a_receiver_taking_a_knocked_out_partners_supreme_overlord_counts_it() {
    // The `Faint` event follows `totalFainted++`: a Receiver taking the Supreme Overlord of the
    // partner a move knocked out (`onAllyFaint`, at the hit loop's end) counts that partner as
    // the ability starts (R-M10-61).
    const PASSIMIAN: PokemonId = PokemonId(4);
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Doubles,
        vec![
            set("Espeon", &["Tackle"], "Synchronize", None),
            set("Blissey", &["Splash"], "Natural Cure", None),
        ],
        vec![
            set("Kingambit", &["Splash"], "Supreme Overlord", None),
            set("Passimian", &["Splash"], "Receiver", None),
        ],
        |pokemon| pokemon[2].hp = 1,
    );
    let mut tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true).optional()];
    tape.extend(hit(100));
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0))), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        tape,
    );
    assert!(
        arena
            .engine
            .state()
            .pokemon(PokemonId(3))
            .unwrap()
            .is_fainted()
    );
    assert_eq!(
        arena
            .engine
            .state()
            .pokemon(PASSIMIAN)
            .unwrap()
            .ability_state
            .fallen,
        1
    );
    assert_eq!(arena.engine.state().side(SideId::P2).total_fainted, 1);
}

#[test]
fn a_gas_holder_fainting_at_the_turns_end_is_counted_before_its_gas_ends() {
    // At the action's end too `faintMessages` counts the faint before its ability's `End`: a
    // Neutralizing Gas holder Leech Seed drains out at the turn's end lets a Supreme Overlord
    // start again that counts it, and says so (R-M10-61).
    const KINGAMBIT: PokemonId = PokemonId(4);
    let mut arena = battle(
        Generation::Gen9,
        BattleMode::Doubles,
        vec![
            set("Espeon", &["Leech Seed"], "Synchronize", None),
            set("Blissey", &["Splash"], "Natural Cure", None),
        ],
        vec![
            set("Weezing", &["Splash"], "Neutralizing Gas", None),
            set("Kingambit", &["Splash"], "Supreme Overlord", None),
        ],
        |pokemon| pokemon[2].hp = 1,
    );
    let tape = vec![RandomOverride::bool(RandomRequestKind::Accuracy, true).optional()];
    arena.turn(
        &[Act::Move(0, Some((SideId::P2, 0))), Act::Move(0, None)],
        &[Act::Move(0, None), Act::Move(0, None)],
        tape,
    );
    assert!(
        arena
            .engine
            .state()
            .pokemon(PokemonId(3))
            .unwrap()
            .is_fainted()
    );
    assert_eq!(
        arena
            .engine
            .state()
            .pokemon(KINGAMBIT)
            .unwrap()
            .ability_state
            .fallen,
        1
    );
    assert_eq!(
        known_ability(&arena, SideId::P1, KINGAMBIT),
        Some(AbilityId::SUPREME_OVERLORD)
    );
}

#[test]
fn orichalcum_pulse_entering_the_sun_names_itself_not_where_cloud_nine_hides_it() {
    // In the sun already Orichalcum Pulse sets nothing and is only announced (`-activate|…|
    // ability: Orichalcum Pulse`, R-M10-77); under Cloud Nine `isWeather` reads no sun: nothing.
    for (foe, announced) in [("Thick Fat", true), ("Cloud Nine", false)] {
        let mut arena = battle(
            Generation::Gen9,
            BattleMode::Singles,
            vec![
                set("Torkoal", &["Splash"], "Drought", None),
                set("Koraidon", &["Splash"], "Orichalcum Pulse", None),
            ],
            vec![set("Snorlax", &["Splash"], foe, None)],
            |_| {},
        );
        arena.turn(&[Act::Switch(B)], &[Act::Move(0, None)], Vec::new());
        let expected = if announced {
            Seen::Known(AbilityId::ORICHALCUM_PULSE)
        } else {
            Seen::Unknown
        };
        assert_eq!(arena.ability(SideId::P2, B), expected, "{foe}");
    }
}
