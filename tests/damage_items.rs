//! L1 rule tests for the M7 A2 items (DESIGN.md §22.4): the stats the damage- and
//! speed-modifying items lead to — values a replay's damage-roll search can absorb (a ×1.05
//! stat, the order of Gen 2's clamp and item) or that it only sees as a turn order — and the
//! accuracy the accuracy items lead to, which a replay only sees as hit or miss.

use std::sync::Arc;

use pokemon_battle_engine::BattleEngine;
use pokemon_battle_engine::dex::{DexResolver, PokemonSet, ResolvedDex, StatSpread, build_pokemon};
use pokemon_battle_engine::engine::{ActionChoice, EngineStep, SideChoice, SlotChoice, TraceEvent};
use pokemon_battle_engine::ids::{MoveSlot, PokemonId, SideId};
use pokemon_battle_engine::rules::{DeclaredTarget, ResolvedRules, RulesResolver, TargetPattern};
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::{
    BattleRng, BattleState, PokemonState, RandomOverride, RandomRequestKind, SideState,
    WeatherKind, WeatherState,
};

const A: PokemonId = PokemonId(1);
const B: PokemonId = PokemonId(2);

struct Arena {
    rules: Arc<ResolvedRules>,
    dex: Arc<ResolvedDex>,
    engine: BattleEngine,
}

fn set(species: &str, moves: &[&str], item: Option<&str>) -> PokemonSet {
    let mut set = PokemonSet::new(species, moves.iter().copied());
    set.item = item.map(Into::into);
    set
}

fn arena(
    generation: Generation,
    a: PokemonSet,
    b: PokemonSet,
    prepare: impl FnOnce(&mut Vec<PokemonState>),
) -> Arena {
    let rules = Arc::new(
        RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
            .unwrap(),
    );
    let dex = Arc::new(DexResolver::resolve(generation));
    let mut pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &a).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &b).unwrap(),
    ];
    prepare(&mut pokemon);
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(A), side(B)], 1);
    let mut engine = BattleEngine::try_new(rules.clone(), dex.clone(), state).unwrap();
    engine.run_lead_switch_ins();
    Arena { rules, dex, engine }
}

impl Arena {
    fn choice(&self, side: SideId) -> SideChoice {
        let actor = self.engine.state().side(side).active[0].unwrap();
        let move_id = self.engine.state().pokemon(actor).unwrap().moves[0].move_id;
        let target = match self.dex.move_by_id(move_id).unwrap().target {
            TargetPattern::SelectedAny
            | TargetPattern::SelectedAdjacent
            | TargetPattern::SelectedFoe
            | TargetPattern::SelectedAdjacentFoe => Some(DeclaredTarget(
                self.rules.topology().position(side.other(), 0).unwrap(),
            )),
            _ => None,
        };
        SideChoice {
            side,
            actions: vec![SlotChoice {
                actor,
                action: ActionChoice::Move {
                    slot: MoveSlot::new(0).unwrap(),
                    target,
                    mechanic: None,
                },
            }],
        }
    }

    /// One turn — A uses its first move on B, B its first move (a status move) — with no
    /// critical hit, the top damage roll and, if the move has one, no secondary; the stats A's
    /// hit read.
    fn stats_of_the_hit(&mut self, secondary: bool) -> (u32, u32) {
        let mut snapshot = self.engine.snapshot();
        let mut tape = vec![
            RandomOverride::bool(RandomRequestKind::Critical, false),
            RandomOverride::range(
                RandomRequestKind::DamageRoll,
                self.rules.damage().random_max,
            ),
        ];
        if secondary {
            tape.push(RandomOverride::bool(RandomRequestKind::Secondary, false));
        }
        snapshot.state.rng = BattleRng::with_semantic_tape(1, tape);
        self.engine =
            BattleEngine::restore(self.rules.clone(), self.dex.clone(), snapshot).unwrap();
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1), self.choice(SideId::P2)];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        let events = &self.engine.trace().events()[start..];
        let offense = events.iter().find_map(|event| match event {
            TraceEvent::OffensiveStatResolved {
                actor, after_item, ..
            } if *actor == A => Some(*after_item),
            _ => None,
        });
        let defense = events.iter().find_map(|event| match event {
            TraceEvent::DefensiveStatResolved { actor, value, .. } if *actor == A => Some(*value),
            _ => None,
        });
        (
            offense.expect("an attacking stat"),
            defense.expect("a defending stat"),
        )
    }

    /// One turn as in `stats_of_the_hit`: the damage A's hit dealt and what the turn traced.
    fn damage_of_the_hit(&mut self, secondary: bool) -> (u16, Vec<TraceEvent>) {
        self.stats_of_the_hit(secondary);
        // The turn ran on a battle restored for it: its trace is that turn's (DESIGN.md §47.0
        // item 6).
        let events = self.engine.trace().events().to_vec();
        let damage = events
            .iter()
            .find_map(|event| match event {
                TraceEvent::DamageCalculated {
                    actor,
                    final_damage,
                    ..
                } if *actor == A => Some(*final_damage),
                _ => None,
            })
            .expect("a damage calculation");
        (damage, events)
    }

    /// The effective Speed A's action is ordered by in a turn where both sides use their first
    /// move (status moves: nothing is drawn).
    fn action_speed_of_a(&mut self) -> u32 {
        let start = self.engine.trace().events().len();
        let choices = [self.choice(SideId::P1), self.choice(SideId::P2)];
        self.engine.submit_turn_choices(choices).unwrap();
        while let EngineStep::Progressed { .. } = self.engine.step().unwrap() {}
        self.engine.trace().events()[start..]
            .iter()
            .find_map(|event| match event {
                TraceEvent::ActionOrderResolved {
                    actor,
                    effective_speed,
                    ..
                } if *actor == A => Some(*effective_speed),
                _ => None,
            })
            .expect("A's action")
    }

    fn stat(&self, id: PokemonId, pick: fn(&PokemonState) -> u16) -> u32 {
        u32::from(pick(self.engine.state().pokemon(id).unwrap()))
    }
}

fn showdown_modify(value: u32, modifier: u32) -> u32 {
    (value * modifier + 2047) / 4096
}

#[test]
fn species_items_and_incenses_modify_the_stats_their_generation_names() {
    let wall = || set("Snorlax", &["Harden"], None);
    // Gen 3 Sea Incense: ×1.05 on Special Attack (chainModify(1.05) = 4300; the other type
    // items' ×1.1 would be 4505).
    let mut sea = arena(
        Generation::Gen3,
        set("Mew", &["Surf"], Some("Sea Incense")),
        wall(),
        |_| {},
    );
    let spa = sea.stat(A, |p| p.stats.special_attack);
    assert_eq!(sea.stats_of_the_hit(false).0, showdown_modify(spa, 4300));
    // Gen 3 Light Ball: Pikachu's Special Attack only.
    let mut ball = arena(
        Generation::Gen3,
        set("Pikachu", &["Body Slam"], Some("Light Ball")),
        wall(),
        |_| {},
    );
    let atk = ball.stat(A, |p| p.stats.attack);
    assert_eq!(ball.stats_of_the_hit(true).0, atk);
    // Gen 5+: Attack too, and for a Pikachu in a cap.
    let mut cap = arena(
        Generation::Gen9,
        set("Pikachu-Alola", &["Body Slam"], Some("Light Ball")),
        wall(),
        |_| {},
    );
    let atk = cap.stat(A, |p| p.stats.attack);
    assert_eq!(cap.stats_of_the_hit(true).0, 2 * atk);
    // Gen 6 Soul Dew on Latias' Special Defense: ×1.5 at priority 2.
    let mut dew = arena(
        Generation::Gen6,
        set("Mew", &["Thunderbolt"], None),
        set("Latias", &["Harden"], Some("Soul Dew")),
        |_| {},
    );
    let spd = dew.stat(B, |p| p.stats.special_defense);
    assert_eq!(dew.stats_of_the_hit(true).1, showdown_modify(spd, 6144));
}

#[test]
fn gen_2_species_items_multiply_the_clamped_stat() {
    // Gen 2 `getStat`: the stage, the 999 clamp, then the item. Marowak's Attack at +6 is
    // over 999; Thick Club doubles the clamped 999 to 1998, which rolls over to
    // ⌊1998 / 4⌋ mod 256 = 243 (doubling first and clamping after would leave 999: 249).
    let mut marowak = set("Marowak", &["Body Slam"], Some("Thick Club"));
    marowak.evs = StatSpread::uniform(252);
    let mut club = arena(
        Generation::Gen2,
        marowak,
        set("Snorlax", &["Harden"], None),
        |pokemon| pokemon[0].stages.attack = 6,
    );
    assert!(club.stat(A, |p| p.stats.attack) * 4 > 999);
    assert_eq!(club.stats_of_the_hit(true).0, 243);
    // Metal Powder: Ditto's Defense ×1.5, floored.
    let mut powder = arena(
        Generation::Gen2,
        set("Mew", &["Body Slam"], None),
        set("Ditto", &["Harden"], Some("Metal Powder")),
        |_| {},
    );
    let def = powder.stat(B, |p| p.stats.defense);
    assert_eq!(powder.stats_of_the_hit(true).1, def * 3 / 2);
}

#[test]
fn speed_items_set_the_action_speed() {
    // The Speed a Pokémon acts at: a level-100 Mew with 4 Speed EVs has 237; a Power item or
    // an Iron Ball halves it with `modify(237, 2048)` = 118 (Showdown's rounding: half down);
    // Quick Powder doubles Ditto's 132 to 264 and leaves a Mew alone. The replay only sees the
    // order these give.
    let speed = |generation, holder: PokemonSet| {
        let mut arena = arena(
            generation,
            holder,
            set("Snorlax", &["Harden"], None),
            |_| {},
        );
        arena.action_speed_of_a()
    };
    let mew = |item: Option<&str>| {
        let mut mew = set("Mew", &["Harden"], item);
        mew.evs.spe = 4;
        mew
    };
    assert_eq!(speed(Generation::Gen9, mew(None)), 237);
    assert_eq!(speed(Generation::Gen9, mew(Some("Power Anklet"))), 118);
    assert_eq!(speed(Generation::Gen9, mew(Some("Iron Ball"))), 118);
    assert_eq!(speed(Generation::Gen3, mew(Some("Macho Brace"))), 118);
    assert_eq!(
        speed(
            Generation::Gen8,
            set("Ditto", &["Harden"], Some("Quick Powder"))
        ),
        264
    );
    assert_eq!(speed(Generation::Gen8, mew(Some("Quick Powder"))), 237);
}

/// The accuracy check of A's first move on B (`engine::accuracy_roll`) in a battle state
/// `prepare` may adjust; B has not moved yet unless `prepare` says so.
fn accuracy(
    generation: Generation,
    user: PokemonSet,
    target: PokemonSet,
    prepare: impl FnOnce(&mut BattleState),
) -> Option<(u16, u16)> {
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &user).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &target).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let mut state = BattleState::new(pokemon, [side(A), side(B)], 1);
    state.pending_movers = vec![B];
    prepare(&mut state);
    let move_id = state.pokemon(A).unwrap().moves[0].move_id;
    let move_def = dex.move_by_id(move_id).unwrap().clone();
    pokemon_battle_engine::engine::accuracy_roll(&state, &rules, A, B, &move_def)
}

#[test]
fn accuracy_items_follow_the_generation() {
    let (g2, g3, g4, g8) = (
        Generation::Gen2,
        Generation::Gen3,
        Generation::Gen4,
        Generation::Gen8,
    );
    let mew = |moves: &[&str], item: Option<&str>| set("Mew", moves, item);
    let lax = |item: Option<&str>| set("Snorlax", &["Harden"], item);
    let at_rest = |_: &mut BattleState| {};
    let moved = |state: &mut BattleState| state.pending_movers.clear();
    let hypnosis = || mew(&["Hypnosis"], None);

    // Gen 2 Bright Powder: 20 off the byte, after the stages: Body Slam's 255 (no roll) → 235.
    assert_eq!(
        accuracy(
            g2,
            mew(&["Body Slam"], None),
            lax(Some("Bright Powder")),
            at_rest
        ),
        Some((235, 256))
    );
    // Gen 3-4, the cartridge (issues.md I5): Hypnosis 60 × (100 − parameter) / 100 — Bright
    // Powder 10 (54); Lax Incense 5 in Emerald (57), 10 in Platinum (54).
    let powder = || lax(Some("Bright Powder"));
    let incense = || lax(Some("Lax Incense"));
    assert_eq!(accuracy(g3, hypnosis(), powder(), at_rest), Some((54, 100)));
    assert_eq!(
        accuracy(g3, hypnosis(), incense(), at_rest),
        Some((57, 100))
    );
    assert_eq!(
        accuracy(g4, hypnosis(), incense(), at_rest),
        Some((54, 100))
    );
    // Wide Lens × 110 / 100 = 66; Zoom Lens × 120 / 100 = 72 once the target has moved, 60
    // before.
    let wide = mew(&["Hypnosis"], Some("Wide Lens"));
    assert_eq!(accuracy(g4, wide, lax(None), at_rest), Some((66, 100)));
    let zoom = || mew(&["Hypnosis"], Some("Zoom Lens"));
    assert_eq!(accuracy(g4, zoom(), lax(None), moved), Some((72, 100)));
    assert_eq!(accuracy(g4, zoom(), lax(None), at_rest), Some((60, 100)));
    // Every step floored, in the cartridge's order: Compound Eyes before Sand Veil — Sing 55 →
    // 71 → 56 (Sand Veil first would give 44 → 57); and before Bright Powder: Hypnosis 60 → 78
    // → 70.
    let sandstorm = |state: &mut BattleState| {
        state.field.weather = Some(WeatherState {
            kind: WeatherKind::Sandstorm,
            source: A,
            turns_remaining: 5,
            strong: false,
        });
    };
    let mut eyes = mew(&["Sing"], None);
    eyes.ability = Some("Compound Eyes".into());
    let mut veil = lax(None);
    veil.ability = Some("Sand Veil".into());
    assert_eq!(accuracy(g4, eyes.clone(), veil, sandstorm), Some((56, 100)));
    eyes.moves = vec!["Hypnosis".into()];
    assert_eq!(accuracy(g4, eyes, powder(), at_rest), Some((70, 100)));
    // Gen 5+: Showdown's chain — [3686, 4096] 54, [4505, 4096] 66, [4915, 4096] 72.
    assert_eq!(accuracy(g8, hypnosis(), powder(), at_rest), Some((54, 100)));
    assert_eq!(
        accuracy(g8, hypnosis(), incense(), at_rest),
        Some((54, 100))
    );
    let wide = mew(&["Hypnosis"], Some("Wide Lens"));
    assert_eq!(accuracy(g8, wide, lax(None), at_rest), Some((66, 100)));
    assert_eq!(accuracy(g8, zoom(), lax(None), moved), Some((72, 100)));
    assert_eq!(accuracy(g8, zoom(), lax(None), at_rest), Some((60, 100)));
}

/// A's first move as the dex resolves it and as A uses it (`engine::modified_move`).
fn moves_of(
    generation: Generation,
    user: PokemonSet,
) -> Vec<(
    pokemon_battle_engine::dex::ResolvedMove,
    pokemon_battle_engine::dex::ResolvedMove,
)> {
    let rules = RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
        .unwrap();
    let dex = DexResolver::resolve(generation);
    let pokemon = vec![
        build_pokemon(&dex, A, SideId::P1, &user).unwrap(),
        build_pokemon(&dex, B, SideId::P2, &set("Snorlax", &["Harden"], None)).unwrap(),
    ];
    let side = |id: PokemonId| {
        let mut side = SideState::new(vec![id], 1);
        side.active[0] = Some(id);
        side
    };
    let state = BattleState::new(pokemon, [side(A), side(B)], 1);
    state
        .pokemon(A)
        .unwrap()
        .moves
        .iter()
        .map(|slot| {
            let move_def = dex.move_by_id(slot.move_id).unwrap().clone();
            let (modified, _) =
                pokemon_battle_engine::engine::modified_move(&state, &rules, A, &move_def, false);
            (move_def, modified)
        })
        .collect()
}

#[test]
fn loaded_dice_and_punching_glove_edit_the_move() {
    use pokemon_battle_engine::dex::MultiHit;
    let g9 = Generation::Gen9;
    // Loaded Dice: 2-5 hits become 4-5, Population Bomb's 10 becomes 4-10 (Showdown: 10 −
    // random(7)), and no hit after the first checks accuracy again (Triple Axel too).
    let dice = |ability: Option<&str>| {
        let mut mew = set(
            "Mew",
            &["Bullet Seed", "Population Bomb", "Triple Axel"],
            Some("Loaded Dice"),
        );
        mew.ability = ability.map(Into::into);
        moves_of(g9, mew)
    };
    let moves = dice(None);
    assert_eq!(moves[0].1.multi_hit, MultiHit::Range { min: 4, max: 5 });
    assert_eq!(moves[1].0.multi_hit, MultiHit::Fixed(10));
    assert!(moves[1].0.multi_accuracy);
    assert_eq!(moves[1].1.multi_hit, MultiHit::Range { min: 4, max: 10 });
    assert!(!moves[1].1.multi_accuracy);
    assert!(moves[2].0.multi_accuracy && !moves[2].1.multi_accuracy);
    // After Skill Link: 2-5 moves already strike 5; Population Bomb is still 4-10.
    let moves = dice(Some("Skill Link"));
    assert_eq!(moves[0].1.multi_hit, MultiHit::Fixed(5));
    assert_eq!(moves[1].1.multi_hit, MultiHit::Range { min: 4, max: 10 });
    // Punching Glove: a punch loses contact, another move keeps it.
    let glove = moves_of(
        g9,
        set("Mew", &["Ice Punch", "Body Slam"], Some("Punching Glove")),
    );
    assert!(glove[0].0.flags.contact && !glove[0].1.flags.contact);
    assert!(glove[1].1.flags.contact);
}

#[test]
fn species_crit_items_follow_the_generation() {
    use pokemon_battle_engine::engine::{CriticalRoll, critical_roll};
    let roll = |generation, user: PokemonSet| {
        let rules =
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .unwrap();
        let dex = DexResolver::resolve(generation);
        let pokemon = vec![
            build_pokemon(&dex, A, SideId::P1, &user).unwrap(),
            build_pokemon(&dex, B, SideId::P2, &set("Snorlax", &["Harden"], None)).unwrap(),
        ];
        let side = |id: PokemonId| {
            let mut side = SideState::new(vec![id], 1);
            side.active[0] = Some(id);
            side
        };
        let state = BattleState::new(pokemon, [side(A), side(B)], 1);
        let move_id = state.pokemon(A).unwrap().moves[0].move_id;
        let move_def = dex.move_by_id(move_id).unwrap().clone();
        critical_roll(&state, &rules, &dex, A, B, &move_def)
    };
    let (g2, g4, g8) = (Generation::Gen2, Generation::Gen4, Generation::Gen8);
    // Gen 2: the item sets the ratio to 3 (stage 2) whatever the move — Slash too (64/256, not
    // the 85/256 of 2 + 2).
    assert_eq!(
        roll(g2, set("Farfetch'd", &["Slash"], Some("Stick"))),
        CriticalRoll::Chance(64, 256)
    );
    assert_eq!(
        roll(g2, set("Chansey", &["Body Slam"], Some("Lucky Punch"))),
        CriticalRoll::Chance(64, 256)
    );
    // Gen 3+: +2 — Gen 4 Body Slam 1 + 2 = 3 (1/4), Slash 2 + 2 = 4 (1/3); another species
    // holding it gets nothing.
    assert_eq!(
        roll(g4, set("Farfetch'd", &["Body Slam"], Some("Stick"))),
        CriticalRoll::Chance(1, 4)
    );
    assert_eq!(
        roll(g4, set("Farfetch'd", &["Slash"], Some("Stick"))),
        CriticalRoll::Chance(1, 3)
    );
    assert_eq!(
        roll(g4, set("Snorlax", &["Body Slam"], Some("Lucky Punch"))),
        CriticalRoll::Chance(1, 16)
    );
    // Gen 8 Leek: Sirfetch'd's Leaf Blade is certain, a Galarian Farfetch'd's Brutal Swing 1/2.
    assert_eq!(
        roll(g8, set("Sirfetch'd", &["Leaf Blade"], Some("Leek"))),
        CriticalRoll::Always
    );
    assert_eq!(
        roll(g8, set("Farfetch'd-Galar", &["Brutal Swing"], Some("Leek"))),
        CriticalRoll::Chance(1, 2)
    );
}

#[test]
fn eviolite_and_assault_vest() {
    use pokemon_battle_engine::engine::{ChoiceError, must_struggle, validate_side_choice};
    // `nfe` from each generation's own evolution data: Chansey has Blissey from Gen 2, Bisharp
    // Kingambit from Gen 9.
    let nfe = |generation, species: &str| {
        DexResolver::resolve(generation)
            .species_by_name(species)
            .unwrap()
            .nfe
    };
    assert!(!nfe(Generation::Gen1, "Chansey") && nfe(Generation::Gen2, "Chansey"));
    assert!(!nfe(Generation::Gen8, "Bisharp") && nfe(Generation::Gen9, "Bisharp"));
    assert!(!nfe(Generation::Gen9, "Blissey"));
    // Eviolite ×1.5 (priority 2) on Chansey's Defense.
    let mut chansey = arena(
        Generation::Gen9,
        set("Mew", &["Body Slam"], None),
        set("Chansey", &["Harden"], Some("Eviolite")),
        |_| {},
    );
    let def = chansey.stat(B, |p| p.stats.defense);
    assert_eq!(chansey.stats_of_the_hit(true).1, showdown_modify(def, 6144));
    // Assault Vest ×1.5 on Special Defense (its holder, knowing only Harden, struggles).
    let mut vest = arena(
        Generation::Gen9,
        set("Mew", &["Psychic"], None),
        set("Snorlax", &["Harden"], Some("Assault Vest")),
        |_| {},
    );
    let spd = vest.stat(B, |p| p.stats.special_defense);
    assert_eq!(vest.stats_of_the_hit(true).1, showdown_modify(spd, 6144));
    // The request: a status move is refused, while another move is left there is no Struggle.
    let refusal = arena(
        Generation::Gen9,
        set("Mew", &["Psychic"], None),
        set("Snorlax", &["Body Slam", "Harden"], Some("Assault Vest")),
        |_| {},
    );
    let harden = SideChoice {
        side: SideId::P2,
        actions: vec![SlotChoice {
            actor: B,
            action: ActionChoice::Move {
                slot: MoveSlot::new(1).unwrap(),
                target: None,
                mechanic: None,
            },
        }],
    };
    assert!(matches!(
        validate_side_choice(
            refusal.engine.state(),
            &refusal.rules,
            &refusal.dex,
            SideId::P2,
            &harden
        ),
        Err(ChoiceError::MoveDisabledByItem { .. })
    ));
    assert!(!must_struggle(
        refusal.engine.state(),
        &refusal.rules,
        &refusal.dex,
        B
    ));
    let only_status = arena(
        Generation::Gen9,
        set("Mew", &["Psychic"], None),
        set("Snorlax", &["Harden"], Some("Assault Vest")),
        |_| {},
    );
    assert!(must_struggle(
        only_status.engine.state(),
        &only_status.rules,
        &only_status.dex,
        B
    ));
}

#[test]
fn a_resist_berry_is_eaten_by_the_hit_it_halves() {
    // Mew's Flamethrower on a Skarmory with an Occa Berry, the top roll twice: the first hit
    // eats the berry (`ItemEaten`, the item gone) and deals the second's ×2048 / 4096 (the
    // ModifyDamage chain comes last); the second finds no berry.
    let mut occa = arena(
        Generation::Gen9,
        set("Mew", &["Flamethrower"], None),
        set("Skarmory", &["Harden"], Some("Occa Berry")),
        |_| {},
    );
    let (first, events) = occa.damage_of_the_hit(true);
    assert!(events.iter().any(|event| matches!(
        event,
        TraceEvent::ItemEaten { holder, .. } if *holder == B
    )));
    assert_eq!(occa.engine.state().pokemon(B).unwrap().held_item, None);
    let (second, events) = occa.damage_of_the_hit(true);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, TraceEvent::ItemEaten { .. }))
    );
    assert_eq!(u32::from(first), showdown_modify(u32::from(second), 2048));
}

#[test]
fn gen_3_one_hit_ko_moves_hit_one_point_less() {
    // pokeemerald `Cmd_tryKO`: `Random() % 100 + 1 < chance` (issues.md I11) — Fissure at the
    // same level hits 29 times in 100 in Gen 3, 30 in Gen 4; 70 levels above, Gen 3 still
    // rolls (99), 71 above it cannot miss.
    let fissure = |generation, level_gap: u8| {
        let mut mew = set("Mew", &["Fissure"], None);
        mew.level = 100;
        let mut wall = set("Snorlax", &["Harden"], None);
        wall.level = 100 - level_gap;
        accuracy(generation, mew, wall, |_| {})
    };
    assert_eq!(fissure(Generation::Gen3, 0), Some((29, 100)));
    assert_eq!(fissure(Generation::Gen4, 0), Some((30, 100)));
    assert_eq!(fissure(Generation::Gen3, 70), Some((99, 100)));
    assert_eq!(fissure(Generation::Gen4, 70), None);
    assert_eq!(fissure(Generation::Gen3, 71), None);
}

#[test]
fn kings_rock_adds_a_flinch_of_its_own() {
    use pokemon_battle_engine::dex::SecondaryEffect;
    use pokemon_battle_engine::engine::{SecondaryRoll, secondary_roll};
    use pokemon_battle_engine::state::{NonVolatileStatus, StatusState, VolatileConditionKind};
    let flinch = SecondaryEffect::Volatile(VolatileConditionKind::Flinch);
    let item_flinches =
        |generation, species: &str, moves: &[&str], item: &str, ability: Option<&str>| {
            let mut user = set(species, moves, Some(item));
            user.ability = ability.map(Into::into);
            moves_of(generation, user)
                .into_iter()
                .map(|(_, modified)| {
                    modified
                        .secondaries
                        .iter()
                        .filter(|s| s.from_item && s.effect == flinch)
                        .map(|s| s.chance.basis_points())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
    // Gen 2-4: the cartridge's lists — Tackle yes, Body Slam no; Gen 2 12 %; Gen 4 Dark Pulse
    // keeps its own 20 % and gains the item's.
    assert_eq!(
        item_flinches(
            Generation::Gen2,
            "Mew",
            &["Tackle", "Body Slam"],
            "King's Rock",
            None
        ),
        vec![vec![1200], vec![]]
    );
    assert_eq!(
        item_flinches(
            Generation::Gen4,
            "Mew",
            &["Dark Pulse", "Body Slam"],
            "Razor Fang",
            None
        ),
        vec![vec![1000], vec![]]
    );
    // Gen 5+: every damaging move without a flinch of its own — Body Slam yes, Bite no,
    // Harden no; with Sheer Force Body Slam loses its paralysis and keeps the item's flinch.
    assert_eq!(
        item_flinches(
            Generation::Gen9,
            "Mew",
            &["Body Slam", "Bite", "Harden"],
            "King's Rock",
            None
        ),
        vec![vec![1000], vec![], vec![]]
    );
    let sheer = moves_of(Generation::Gen9, {
        let mut mew = set("Mew", &["Body Slam"], Some("King's Rock"));
        mew.ability = Some("Sheer Force".into());
        mew
    });
    assert_eq!(sheer[0].1.secondaries.len(), 1);
    assert!(sheer[0].1.secondaries[0].from_item);

    // The roll: Gen 2's item flinch still rolls against a sleeping target (the move's own
    // would not), a byte 30 of 256; Serene Grace doubles it from Gen 5 (Showdown, issues.md I13)
    // but not in Gen 3-4 (the cartridge rolls the item apart, issues.md I12).
    let roll = |generation, ability: Option<&str>, move_name: &str, asleep: bool| {
        let rules =
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .unwrap();
        let dex = DexResolver::resolve(generation);
        let mut user = set("Mew", &[move_name], Some("King's Rock"));
        user.ability = ability.map(Into::into);
        let mut pokemon = vec![
            build_pokemon(&dex, A, SideId::P1, &user).unwrap(),
            build_pokemon(&dex, B, SideId::P2, &set("Snorlax", &["Harden"], None)).unwrap(),
        ];
        if asleep {
            pokemon[1].status = Some(StatusState::new(NonVolatileStatus::Sleep));
        }
        let side = |id: PokemonId| {
            let mut side = SideState::new(vec![id], 1);
            side.active[0] = Some(id);
            side
        };
        let state = BattleState::new(pokemon, [side(A), side(B)], 1);
        let move_def = dex
            .move_by_id(state.pokemon(A).unwrap().moves[0].move_id)
            .unwrap()
            .clone();
        let (modified, _) =
            pokemon_battle_engine::engine::modified_move(&state, &rules, A, &move_def, false);
        modified
            .secondaries
            .iter()
            .map(|s| secondary_roll(&state, &rules, A, B, &modified, s))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        roll(Generation::Gen2, None, "Tackle", true),
        vec![SecondaryRoll::Chance(30, 256)]
    );
    assert_eq!(
        roll(Generation::Gen2, None, "Bite", true),
        vec![SecondaryRoll::Skipped]
    );
    for generation in [Generation::Gen3, Generation::Gen4] {
        assert_eq!(
            roll(generation, Some("Serene Grace"), "Tackle", false),
            vec![SecondaryRoll::Chance(1000, 10_000)]
        );
    }
    assert_eq!(
        roll(Generation::Gen9, Some("Serene Grace"), "Tackle", false),
        vec![SecondaryRoll::Chance(2000, 10_000)]
    );
}

/// Hustle rounds the Attack itself (`this.modify(atk, 1.5)`), the Choice Band's `chainModify`
/// applying to what it leaves: 145 → 217 → 325, where one ×2.25 chain gives 326 (a Gen 7
/// Nidorina's Body Slam, R-M10-65). Alone, either gives 217.
#[test]
fn hustle_rounds_the_attack_before_a_choice_bands_chain() {
    for (ability, item, attack) in [
        ("Hustle", Some("Choice Band"), 325),
        ("Hustle", None, 217),
        ("Poison Point", Some("Choice Band"), 217),
    ] {
        let mut nidorina = set("Nidorina", &["Aerial Ace"], item);
        nidorina.ability = Some(ability.into());
        let mut arena = arena(
            Generation::Gen7,
            nidorina,
            set("Blissey", &["Splash"], None),
            |pokemon| pokemon[0].stats.attack = 145,
        );
        let (offense, _) = arena.stats_of_the_hit(false);
        assert_eq!(offense, attack, "{ability} {item:?}");
    }
}
