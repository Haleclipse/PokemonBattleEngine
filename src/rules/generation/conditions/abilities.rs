//! The abilities' generation tables: what breaks, copies, suppresses or replaces them.

use super::*;

/// Showdown's `breakable` flag by the generations it holds in (Dark Aura and Fairy Aura lose it
/// in Gen 8), as exported into `tests/fixtures/oracle/ability-flags.json`.
const BREAKABLE: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::BATTLE_ARMOR, Generation::Gen3, Generation::Gen9),
    (AbilityId::CLEAR_BODY, Generation::Gen3, Generation::Gen9),
    (AbilityId::DAMP, Generation::Gen3, Generation::Gen9),
    (AbilityId::FLASH_FIRE, Generation::Gen3, Generation::Gen9),
    (AbilityId::HYPER_CUTTER, Generation::Gen3, Generation::Gen9),
    (AbilityId::IMMUNITY, Generation::Gen3, Generation::Gen9),
    (AbilityId::INNER_FOCUS, Generation::Gen3, Generation::Gen9),
    (AbilityId::INSOMNIA, Generation::Gen3, Generation::Gen9),
    (AbilityId::KEEN_EYE, Generation::Gen3, Generation::Gen9),
    (AbilityId::LEVITATE, Generation::Gen3, Generation::Gen9),
    (AbilityId::LIGHTNING_ROD, Generation::Gen3, Generation::Gen9),
    (AbilityId::LIMBER, Generation::Gen3, Generation::Gen9),
    (AbilityId::MAGMA_ARMOR, Generation::Gen3, Generation::Gen9),
    (AbilityId::MARVEL_SCALE, Generation::Gen3, Generation::Gen9),
    (AbilityId::OBLIVIOUS, Generation::Gen3, Generation::Gen9),
    (AbilityId::OWN_TEMPO, Generation::Gen3, Generation::Gen9),
    (AbilityId::SAND_VEIL, Generation::Gen3, Generation::Gen9),
    (AbilityId::SHELL_ARMOR, Generation::Gen3, Generation::Gen9),
    (AbilityId::SHIELD_DUST, Generation::Gen3, Generation::Gen9),
    (AbilityId::SOUNDPROOF, Generation::Gen3, Generation::Gen9),
    (AbilityId::STICKY_HOLD, Generation::Gen3, Generation::Gen9),
    (AbilityId::STURDY, Generation::Gen3, Generation::Gen9),
    (AbilityId::SUCTION_CUPS, Generation::Gen3, Generation::Gen9),
    (AbilityId::THICK_FAT, Generation::Gen3, Generation::Gen9),
    (AbilityId::VITAL_SPIRIT, Generation::Gen3, Generation::Gen9),
    (AbilityId::VOLT_ABSORB, Generation::Gen3, Generation::Gen9),
    (AbilityId::WATER_ABSORB, Generation::Gen3, Generation::Gen9),
    (AbilityId::WATER_VEIL, Generation::Gen3, Generation::Gen9),
    (AbilityId::WHITE_SMOKE, Generation::Gen3, Generation::Gen9),
    (AbilityId::WONDER_GUARD, Generation::Gen3, Generation::Gen9),
    (AbilityId::DRY_SKIN, Generation::Gen4, Generation::Gen9),
    (AbilityId::FILTER, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen4, Generation::Gen9),
    (AbilityId::HEATPROOF, Generation::Gen4, Generation::Gen9),
    (AbilityId::LEAF_GUARD, Generation::Gen4, Generation::Gen9),
    (AbilityId::MOTOR_DRIVE, Generation::Gen4, Generation::Gen9),
    (AbilityId::SIMPLE, Generation::Gen4, Generation::Gen9),
    (AbilityId::SNOW_CLOAK, Generation::Gen4, Generation::Gen9),
    (AbilityId::SOLID_ROCK, Generation::Gen4, Generation::Gen9),
    (AbilityId::STORM_DRAIN, Generation::Gen4, Generation::Gen9),
    (AbilityId::TANGLED_FEET, Generation::Gen4, Generation::Gen9),
    (AbilityId::UNAWARE, Generation::Gen4, Generation::Gen9),
    (AbilityId::BIG_PECKS, Generation::Gen5, Generation::Gen9),
    (AbilityId::CONTRARY, Generation::Gen5, Generation::Gen9),
    (AbilityId::FRIEND_GUARD, Generation::Gen5, Generation::Gen9),
    (AbilityId::HEAVY_METAL, Generation::Gen5, Generation::Gen9),
    (AbilityId::LIGHT_METAL, Generation::Gen5, Generation::Gen9),
    (AbilityId::MAGIC_BOUNCE, Generation::Gen5, Generation::Gen9),
    (AbilityId::MULTISCALE, Generation::Gen5, Generation::Gen9),
    (AbilityId::SAP_SIPPER, Generation::Gen5, Generation::Gen9),
    (AbilityId::TELEPATHY, Generation::Gen5, Generation::Gen9),
    (AbilityId::WONDER_SKIN, Generation::Gen5, Generation::Gen9),
    (AbilityId::AROMA_VEIL, Generation::Gen6, Generation::Gen9),
    (AbilityId::AURA_BREAK, Generation::Gen6, Generation::Gen9),
    (AbilityId::BULLETPROOF, Generation::Gen6, Generation::Gen9),
    (AbilityId::DARK_AURA, Generation::Gen6, Generation::Gen7),
    (AbilityId::FAIRY_AURA, Generation::Gen6, Generation::Gen7),
    (AbilityId::FLOWER_VEIL, Generation::Gen6, Generation::Gen9),
    (AbilityId::FUR_COAT, Generation::Gen6, Generation::Gen9),
    (AbilityId::GRASS_PELT, Generation::Gen6, Generation::Gen9),
    (AbilityId::OVERCOAT, Generation::Gen6, Generation::Gen9),
    (AbilityId::SWEET_VEIL, Generation::Gen6, Generation::Gen9),
    (AbilityId::DAZZLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (AbilityId::FLUFFY, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::QUEENLY_MAJESTY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::WATER_BUBBLE, Generation::Gen7, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_SCALES, Generation::Gen8, Generation::Gen9),
    (AbilityId::MIRROR_ARMOR, Generation::Gen8, Generation::Gen9),
    (AbilityId::PASTEL_VEIL, Generation::Gen8, Generation::Gen9),
    (AbilityId::PUNK_ROCK, Generation::Gen8, Generation::Gen9),
    (AbilityId::ARMOR_TAIL, Generation::Gen9, Generation::Gen9),
    (AbilityId::EARTH_EATER, Generation::Gen9, Generation::Gen9),
    (AbilityId::GOOD_AS_GOLD, Generation::Gen9, Generation::Gen9),
    (AbilityId::GUARD_DOG, Generation::Gen9, Generation::Gen9),
    (AbilityId::ILLUMINATE, Generation::Gen9, Generation::Gen9),
    (AbilityId::MINDS_EYE, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::PURIFYING_SALT,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::THERMAL_EXCHANGE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::WELL_BAKED_BODY,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::WIND_RIDER, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `failroleplay` flag by the generations it holds in: Role Play (and Doodle) fails on
/// these abilities.
const FAIL_ROLE_PLAY: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::WONDER_GUARD, Generation::Gen3, Generation::Gen9),
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen5, Generation::Gen9),
    (AbilityId::FORECAST, Generation::Gen5, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::IMPOSTER, Generation::Gen5, Generation::Gen9),
    (AbilityId::TRACE, Generation::Gen5, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (
        AbilityId::POWER_OF_ALCHEMY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RECEIVER, Generation::Gen7, Generation::Gen9),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `failskillswap` flag by the generations it holds in: Skill Swap fails when either
/// Pokémon has one of these abilities.
const FAIL_SKILL_SWAP: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::WONDER_GUARD, Generation::Gen3, Generation::Gen9),
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `noentrain` flag by the generations it holds in: Entrainment fails when its user has
/// one of these abilities.
const NO_ENTRAIN: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen5, Generation::Gen9),
    (AbilityId::FORECAST, Generation::Gen5, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::IMPOSTER, Generation::Gen5, Generation::Gen9),
    (AbilityId::TRACE, Generation::Gen5, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (
        AbilityId::POWER_OF_ALCHEMY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RECEIVER, Generation::Gen7, Generation::Gen9),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::WONDER_GUARD, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `noreceiver` flag by the generations it holds in: Receiver and Power of Alchemy do
/// not take these abilities from a fainted ally.
const NO_RECEIVER: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::WONDER_GUARD, Generation::Gen3, Generation::Gen9),
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen5, Generation::Gen9),
    (AbilityId::FORECAST, Generation::Gen5, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::IMPOSTER, Generation::Gen5, Generation::Gen9),
    (AbilityId::TRACE, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (
        AbilityId::POWER_OF_ALCHEMY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RECEIVER, Generation::Gen7, Generation::Gen9),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// Showdown's `notrace` flag by the generations it holds in: Trace copies none of these.
const NO_TRACE: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::FORECAST, Generation::Gen4, Generation::Gen9),
    (AbilityId::MULTITYPE, Generation::Gen4, Generation::Gen9),
    (AbilityId::TRACE, Generation::Gen4, Generation::Gen9),
    (AbilityId::FLOWER_GIFT, Generation::Gen5, Generation::Gen9),
    (AbilityId::ILLUSION, Generation::Gen5, Generation::Gen9),
    (AbilityId::IMPOSTER, Generation::Gen5, Generation::Gen9),
    (AbilityId::ZEN_MODE, Generation::Gen5, Generation::Gen9),
    (AbilityId::STANCE_CHANGE, Generation::Gen6, Generation::Gen9),
    (AbilityId::BATTLE_BOND, Generation::Gen7, Generation::Gen9),
    (AbilityId::COMATOSE, Generation::Gen7, Generation::Gen9),
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::POWER_CONSTRUCT,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (
        AbilityId::POWER_OF_ALCHEMY,
        Generation::Gen7,
        Generation::Gen9,
    ),
    (AbilityId::RECEIVER, Generation::Gen7, Generation::Gen9),
    (AbilityId::RKS_SYSTEM, Generation::Gen7, Generation::Gen9),
    (AbilityId::SCHOOLING, Generation::Gen7, Generation::Gen9),
    (AbilityId::SHIELDS_DOWN, Generation::Gen7, Generation::Gen9),
    (
        AbilityId::AS_ONE_GLASTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::AS_ONE_SPECTRIER,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen8),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (AbilityId::COMMANDER, Generation::Gen9, Generation::Gen9),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::POISON_PUPPETEER,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERAFORM_ZERO, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHELL, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// How the forme-changing abilities differ by generation (DESIGN.md §43.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormeRules {
    /// Gen 7-8: Battle Bond's Greninja-Bond becomes Ash-Greninja. Gen 9 raises its Attack,
    /// Special Attack and Speed instead (the Ash-Greninja forme went with the generation).
    pub battle_bond_becomes_ash_greninja: bool,
    /// Gen 8+: breaking Disguise costs an eighth of the holder's max HP (Gen 7 costs nothing).
    pub disguise_costs_hp: bool,
    /// Gen 7 (the gen7 mod's `onDamage`; Sword and Shield fixed it in 1.3.1): a Rollout / Ice
    /// Ball hit Disguise takes counts as a hit and not as a doubling, and a chain that ends its
    /// five hits short of doublings stores the rest (the Rollout storage glitch, issues.md I212).
    pub disguise_delays_rolling: bool,
    /// The abilities whose permanent forme goes back to the set's as the holder faints (Showdown
    /// `formeRegression`, which these set themselves): Power Construct (Gen 7+), Battle Bond
    /// (the gen8 mod, Gen 7-8), Disguise (the gen7 and gen8 mods; the base's, Gen 9, does not) —
    /// R-M8-111, R-M8-116.
    pub regressing_abilities: &'static [AbilityId],
    /// Gen 6+: Autotomize lightens `weighthg` itself, which every forme change sets anew to the
    /// forme's (`setSpecies`): the weight it took off is gone (Bulbapedia: "A Pokémon's weight
    /// is reset if it changes form (from Generation VI onward)", R-M9-43). Gen 5's lightening is
    /// the user's volatile (`onModifyWeight`), which stays.
    pub resets_autotomize: bool,
}

/// Showdown's `notransform` flag by the generations it holds in (DESIGN.md §43.3).
const NO_TRANSFORM: &[(AbilityId, Generation, Generation)] = &[
    (AbilityId::DISGUISE, Generation::Gen7, Generation::Gen9),
    (AbilityId::GULP_MISSILE, Generation::Gen8, Generation::Gen9),
    (AbilityId::HUNGER_SWITCH, Generation::Gen8, Generation::Gen9),
    (AbilityId::ICE_FACE, Generation::Gen8, Generation::Gen9),
    (
        AbilityId::NEUTRALIZING_GAS,
        Generation::Gen8,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_CORNERSTONE,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_HEARTHFLAME,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_TEAL,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::EMBODY_ASPECT_WELLSPRING,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (
        AbilityId::PROTOSYNTHESIS,
        Generation::Gen9,
        Generation::Gen9,
    ),
    (AbilityId::QUARK_DRIVE, Generation::Gen9, Generation::Gen9),
    (AbilityId::TERA_SHIFT, Generation::Gen9, Generation::Gen9),
    (AbilityId::ZERO_TO_HERO, Generation::Gen9, Generation::Gen9),
];

/// A set of abilities: one of Showdown's ability flags as a generation has it (DESIGN.md §41).
/// Its members in id order (`Deref` to the slice), and a bitset over the ids, so `contains` —
/// asked whenever a handler or a suppression is looked up — is one bit test (R-M10-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilitySet {
    members: &'static [AbilityId],
    bits: &'static [u64],
}

impl AbilitySet {
    pub fn contains(&self, ability: &AbilityId) -> bool {
        let id = usize::from(ability.0);
        self.bits
            .get(id / 64)
            .is_some_and(|word| (word >> (id % 64)) & 1 == 1)
    }
}

impl std::ops::Deref for AbilitySet {
    type Target = [AbilityId];

    fn deref(&self) -> &[AbilityId] {
        self.members
    }
}

impl IntoIterator for AbilitySet {
    type Item = &'static AbilityId;
    type IntoIter = std::slice::Iter<'static, AbilityId>;

    fn into_iter(self) -> Self::IntoIter {
        self.members.iter()
    }
}

/// Where one generation's set lives for the program's life: its members and its bitset.
type SetCell = std::sync::OnceLock<(Vec<AbilityId>, Vec<u64>)>;

/// The set `members` builds, built once into `cell`.
fn ability_set(cell: &'static SetCell, members: impl FnOnce() -> Vec<AbilityId>) -> AbilitySet {
    let (members, bits) = cell.get_or_init(|| {
        let members = members();
        let words = members
            .iter()
            .map(|ability| usize::from(ability.0) / 64 + 1)
            .max()
            .unwrap_or(0);
        let mut bits = vec![0u64; words];
        for ability in &members {
            bits[usize::from(ability.0) / 64] |= 1 << (ability.0 % 64);
        }
        (members, bits)
    });
    AbilitySet { members, bits }
}

/// The abilities a flag table holds in `generation`, resolved once per generation into `sets`.
fn flagged(
    sets: &'static [SetCell; 9],
    table: &[(AbilityId, Generation, Generation)],
    generation: Generation,
) -> AbilitySet {
    ability_set(&sets[usize::from(generation.number() - 1)], || {
        table
            .iter()
            .filter(|(_, from, until)| (*from..=*until).contains(&generation))
            .map(|(ability, ..)| *ability)
            .collect()
    })
}

/// `breakable` for `generation`, from `BREAKABLE`.
pub(super) fn breakable(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, BREAKABLE, generation)
}

/// `failroleplay` for `generation`, from `FAIL_ROLE_PLAY`.
pub(super) fn fail_role_play(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, FAIL_ROLE_PLAY, generation)
}

/// `noentrain` for `generation`, from `NO_ENTRAIN`.
pub(super) fn no_entrain(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, NO_ENTRAIN, generation)
}

/// `noreceiver` for `generation`, from `NO_RECEIVER`.
pub(super) fn no_receiver(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, NO_RECEIVER, generation)
}

/// `notrace` for `generation`, from `NO_TRACE`.
pub(super) fn no_trace(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, NO_TRACE, generation)
}

/// `notransform` for `generation`, from `NO_TRANSFORM`.
pub(super) fn no_transform(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, NO_TRANSFORM, generation)
}

/// `failskillswap` for `generation`, from `FAIL_SKILL_SWAP`.
pub(super) fn fail_skill_swap(generation: Generation) -> AbilitySet {
    static LISTS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    flagged(&LISTS, FAIL_SKILL_SWAP, generation)
}

/// `cantsuppress` for `generation`.
pub(super) fn cant_suppress(generation: Generation) -> AbilitySet {
    static SETS: [SetCell; 9] = [const { std::sync::OnceLock::new() }; 9];
    ability_set(&SETS[usize::from(generation.number() - 1)], || {
        let members: &[AbilityId] = match generation {
            Generation::Gen1 | Generation::Gen2 | Generation::Gen3 => &[],
            Generation::Gen4 | Generation::Gen5 => &[AbilityId::MULTITYPE],
            Generation::Gen6 => &[AbilityId::MULTITYPE, AbilityId::STANCE_CHANGE],
            Generation::Gen7 => GEN7_CANT_SUPPRESS,
            Generation::Gen8 => GEN8_CANT_SUPPRESS,
            Generation::Gen9 => GEN9_CANT_SUPPRESS,
        };
        members.to_vec()
    })
}

/// Gen 7's `cantsuppress` abilities: the battle-form ones join Multitype and Stance Change.
const GEN7_CANT_SUPPRESS: &[AbilityId] = &[
    AbilityId::BATTLE_BOND,
    AbilityId::COMATOSE,
    AbilityId::DISGUISE,
    AbilityId::MULTITYPE,
    AbilityId::POWER_CONSTRUCT,
    AbilityId::RKS_SYSTEM,
    AbilityId::SCHOOLING,
    AbilityId::SHIELDS_DOWN,
    AbilityId::STANCE_CHANGE,
    AbilityId::ZEN_MODE,
];

/// Gen 8's: Gen 7's, As One, Gulp Missile and Ice Face.
const GEN8_CANT_SUPPRESS: &[AbilityId] = &[
    AbilityId::AS_ONE_GLASTRIER,
    AbilityId::AS_ONE_SPECTRIER,
    AbilityId::BATTLE_BOND,
    AbilityId::COMATOSE,
    AbilityId::DISGUISE,
    AbilityId::GULP_MISSILE,
    AbilityId::ICE_FACE,
    AbilityId::MULTITYPE,
    AbilityId::POWER_CONSTRUCT,
    AbilityId::RKS_SYSTEM,
    AbilityId::SCHOOLING,
    AbilityId::SHIELDS_DOWN,
    AbilityId::STANCE_CHANGE,
    AbilityId::ZEN_MODE,
];

/// Gen 9's: Gen 8's, Tera Shift and Zero to Hero.
const GEN9_CANT_SUPPRESS: &[AbilityId] = &[
    AbilityId::AS_ONE_GLASTRIER,
    AbilityId::AS_ONE_SPECTRIER,
    AbilityId::BATTLE_BOND,
    AbilityId::COMATOSE,
    AbilityId::DISGUISE,
    AbilityId::GULP_MISSILE,
    AbilityId::ICE_FACE,
    AbilityId::MULTITYPE,
    AbilityId::POWER_CONSTRUCT,
    AbilityId::RKS_SYSTEM,
    AbilityId::SCHOOLING,
    AbilityId::SHIELDS_DOWN,
    AbilityId::STANCE_CHANGE,
    AbilityId::TERA_SHIFT,
    AbilityId::ZEN_MODE,
    AbilityId::ZERO_TO_HERO,
];

/// How an ability changes in battle (DESIGN.md §41.3): Skill Swap, Role Play, Trace …
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AbilityChangeRules {
    /// Whether and when the new ability starts (its switch-in effect).
    pub start_new: StartNewAbility,
    /// Gen 5 (Showdown `skillSwap`: `gen <= 5` and the same ability fails; Gen 3-4 follow the
    /// cartridge, which has no such check: issues.md A6 of §41.10).
    pub skill_swap_same_fails: bool,
    /// Gen 5+ (Showdown Role Play's `onTryHit`; Gen 3-4 follow the cartridge, A6).
    pub role_play_same_fails: bool,
}

/// When a Pokémon's new ability starts (`AbilityChangeRules::start_new`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartNewAbility {
    /// Gen 3 (pokeemerald, Showdown's `gen > 3`): it does not.
    Never,
    /// Gen 4 (pokeplatinum `BattleSystem_TriggerEffectOnSwitch` at the move's end): the
    /// switch-in effect it has, unless this stint on the field already announced that kind of
    /// effect (`PokemonHistory::announced`).
    Unannounced,
    /// Gen 5+ (Showdown `setAbility`'s `Start`): at once.
    AtOnce,
}
