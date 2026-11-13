//! Baton Pass leaves behind exactly the volatiles Showdown marks `noCopy` (DESIGN.md §30.7,
//! R142): each generation's `SwitchingRules::not_passed` against Showdown's own flags,
//! exported by `tools/oracle-node/src/nocopy.mjs` into `tests/fixtures/oracle/nocopy.json`.
//! The match below names every volatile kind, so a new one does not compile until it says
//! which Showdown condition it is.

use std::collections::BTreeMap;
use std::path::PathBuf;

use pokemon_battle_engine::rules::RulesResolver;
use pokemon_battle_engine::spec::{BattleMode, BattleSpec, Generation};
use pokemon_battle_engine::state::VolatileConditionKind as V;
use serde_json::Value;

/// The Showdown condition a volatile kind is; `None` where there is no condition of its own to
/// read the flag from (Sparkling Aria's is a bare `volatileStatus`).
fn showdown_id(kind: V) -> Option<&'static str> {
    Some(match kind {
        V::Protect => "protect",
        V::Endure => "endure",
        V::Substitute => "substitute",
        V::LeechSeed => "leechseed",
        V::Flinch => "flinch",
        V::Confusion => "confusion",
        V::Taunt => "taunt",
        V::Encore => "encore",
        V::Disable => "disable",
        V::MustRecharge => "mustrecharge",
        V::ChoiceLock => "choicelock",
        V::FollowMe => "followme",
        V::MagicCoat => "magiccoat",
        V::Snatch => "snatch",
        V::MeFirst => "mefirst",
        V::AllySwitch => "allyswitch",
        V::TwoTurnMove => "twoturnmove",
        V::PartiallyTrapped => "partiallytrapped",
        V::Ingrain => "ingrain",
        V::Stall => "stall",
        V::HelpingHand => "helpinghand",
        V::RagePowder => "ragepowder",
        V::SubstituteBroken => "substitutebroken",
        V::ResidualDamage => "residualdmg",
        V::SureHit => "toxic",
        V::Reflect => "reflect",
        V::LightScreen => "lightscreen",
        V::Mist => "mist",
        V::Attract => "attract",
        V::QuarkDrive => "quarkdrive",
        V::Protosynthesis => "protosynthesis",
        V::MudSport => "mudsport",
        V::WaterSport => "watersport",
        V::Yawn => "yawn",
        V::Nightmare => "nightmare",
        V::HealBlock => "healblock",
        V::Embargo => "embargo",
        V::SaltCure => "saltcure",
        V::SyrupBomb => "syrupbomb",
        V::ThroatChop => "throatchop",
        V::TarShot => "tarshot",
        V::Unburden => "unburden",
        V::SparklingAria => return None,
        V::Metronome => "metronome",
        V::PerishSong => "perishsong",
        V::ChillyReception => "chillyreception",
        V::Trapped => "trapped",
        V::Trapper => "trapper",
        V::Octolock => "octolock",
        V::PartialTrappingLock => "partialtrappinglock",
        V::FakePartiallyTrapped => "fakepartiallytrapped",
        V::FocusPunch => "focuspunch",
        V::BeakBlast => "beakblast",
        V::ShellTrap => "shelltrap",
        V::Counter => "counter",
        V::MirrorCoat => "mirrorcoat",
        V::DestinyBond => "destinybond",
        V::Grudge => "grudge",
        V::Spotlight => "spotlight",
        V::Bide => "bide",
        V::Pursuit => "pursuit",
        V::DefenseCurl => "defensecurl",
        V::AquaRing => "aquaring",
        V::Stockpile => "stockpile",
        V::PowerTrick => "powertrick",
        V::GastroAcid => "gastroacid",
        V::FocusEnergy => "focusenergy",
        V::DragonCheer => "dragoncheer",
        V::LaserFocus => "laserfocus",
        V::MicleBerry => "micleberry",
        V::Fling => "fling",
        V::Charge => "charge",
        V::NoRetreat => "noretreat",
        V::Minimize => "minimize",
        V::Roost => "roost",
        V::Electrify => "electrify",
        V::Powder => "powder",
        V::Foresight => "foresight",
        V::MiracleEye => "miracleeye",
        V::SmackDown => "smackdown",
        V::MagnetRise => "magnetrise",
        V::Telekinesis => "telekinesis",
        V::Autotomize => "autotomize",
        V::FlashFire => "flashfire",
        // Ice Ball's `iceball` has the same flags.
        V::Rolling => "rollout",
        V::RolloutStorage => "rolloutstorage",
        // Dynamax's `noCopy` (DESIGN.md §44.5).
        V::Dynamax => "dynamax",
        V::Rampage => "lockedmove",
        V::Uproar => "uproar",
        V::Torment => "torment",
        V::Imprison => "imprison",
        V::LockOn => "lockon",
        V::Curse => "curse",
        V::Rage => "rage",
        V::GlaiveRush => "glaiverush",
        V::Truant => "truant",
        V::Invulnerability => "invulnerability",
        V::SkyDrop => "skydrop",
        // Sky Drop's carried Pokémon: no Showdown condition (`isSkyDropped`).
        V::SkyDropped => return None,
        // Gen 5+ Showdown's `furycutter`; Gen 2-4 the cartridges' count (the oracle patch's
        // `pokemon.m`, issues.md I75), which a switch ends: not Showdown's table there.
        V::FuryCutter => return None,
        // The oracle patch's Gen 2 flag (issues.md I69), no Showdown condition.
        V::LockedOn => return None,
        // Zen Mode's own marker, added and dropped by its ability: Showdown's `zenmode` is
        // `noCopy` and Baton Pass leaves it (DESIGN.md §43.4).
        V::ZenMode => "zenmode",
        // Commander's two (`noCopy`): Baton Pass leaves them, and neither is handed on.
        V::Commanding | V::Commanded => return None,
    })
}

const KINDS: &[V] = &[
    V::Protect,
    V::Endure,
    V::Substitute,
    V::LeechSeed,
    V::Flinch,
    V::Confusion,
    V::Taunt,
    V::Encore,
    V::Disable,
    V::MustRecharge,
    V::ChoiceLock,
    V::FollowMe,
    // Gen 9's Ally Switch count outlasts its turn (`duration: 2`): a Baton Pass can meet it.
    V::AllySwitch,
    V::TwoTurnMove,
    V::PartiallyTrapped,
    V::Ingrain,
    V::Stall,
    V::HelpingHand,
    V::RagePowder,
    V::SubstituteBroken,
    V::ResidualDamage,
    V::SureHit,
    V::Reflect,
    V::LightScreen,
    V::Mist,
    V::Attract,
    V::QuarkDrive,
    V::Protosynthesis,
    V::MudSport,
    V::WaterSport,
    V::Yawn,
    V::Nightmare,
    V::HealBlock,
    V::Embargo,
    V::SaltCure,
    V::SyrupBomb,
    V::ThroatChop,
    V::TarShot,
    V::Unburden,
    V::SparklingAria,
    V::Metronome,
    V::PerishSong,
    V::ChillyReception,
    V::Trapped,
    V::Trapper,
    V::Octolock,
    V::PartialTrappingLock,
    V::FakePartiallyTrapped,
    V::FocusPunch,
    V::BeakBlast,
    V::ShellTrap,
    V::Counter,
    V::MirrorCoat,
    V::DestinyBond,
    V::Grudge,
    V::Spotlight,
    V::Bide,
    V::Pursuit,
    V::DefenseCurl,
    V::AquaRing,
    V::Stockpile,
    V::PowerTrick,
    V::GastroAcid,
    V::FocusEnergy,
    V::DragonCheer,
    V::LaserFocus,
    V::MicleBerry,
    V::Fling,
    V::Charge,
    V::NoRetreat,
    V::Minimize,
    V::Autotomize,
    V::FlashFire,
    V::Rolling,
    V::RolloutStorage,
    V::Rampage,
    V::Uproar,
    V::Torment,
    V::Imprison,
    V::LockOn,
    V::Curse,
    V::Rage,
    V::GlaiveRush,
    V::Truant,
    V::Invulnerability,
    V::SkyDrop,
    V::SkyDropped,
];

#[test]
fn baton_pass_leaves_behind_what_showdown_marks_no_copy() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("tests/fixtures/oracle/nocopy.json")).expect("fixture"),
    )
    .expect("json");
    let no_copy: BTreeMap<String, Vec<Option<bool>>> =
        serde_json::from_value(fixture["noCopy"].clone()).expect("noCopy");
    let mut wrong = Vec::new();
    // Baton Pass exists from Gen 2.
    for generation in [
        Generation::Gen2,
        Generation::Gen3,
        Generation::Gen4,
        Generation::Gen5,
        Generation::Gen6,
        Generation::Gen7,
        Generation::Gen8,
        Generation::Gen9,
    ] {
        let rules =
            RulesResolver::resolve(&BattleSpec::new(generation, BattleMode::Singles, "custom"))
                .expect("rules");
        let not_passed = rules.conditions().switching.not_passed;
        for &kind in KINDS {
            let Some(id) = showdown_id(kind) else {
                continue;
            };
            let flag = no_copy
                .get(id)
                .unwrap_or_else(|| panic!("{id} is not a Showdown condition"))
                [usize::from(generation.number() - 1)];
            // Absent from the generation: nothing to hand on either way.
            let Some(flag) = flag else {
                continue;
            };
            // Where the engine follows the cartridge instead (issues.md I42: Gen 4's Micle Berry
            // boost, the oracle patch marks it `noCopy`; Gen 3, which has no Micle Berry, shares
            // Gen 4's table).
            let flag = flag
                || (matches!(generation, Generation::Gen3 | Generation::Gen4)
                    && kind == V::MicleBerry)
                // The cartridges' rage stays with the switching Pokémon (issues.md I77, I78;
                // the oracle patch marks it `noCopy`).
                || (matches!(
                    generation,
                    Generation::Gen2 | Generation::Gen3 | Generation::Gen4
                ) && kind == V::Rage);
            if flag != not_passed.contains(&kind) {
                wrong.push(format!(
                    "{generation:?} {kind:?} ({id}): Showdown noCopy {flag}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "not_passed disagrees with Showdown:\n{}",
        wrong.join("\n")
    );
}
