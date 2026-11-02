use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RandomRequestKind {
    SpeedTie,
    Accuracy,
    Critical,
    DamageRoll,
    Secondary,
    /// Sleep / confusion `time` rolled on application.
    Duration,
    RandomTarget,
    /// BeforeMove: 25 % full paralysis.
    FullParalysis,
    /// BeforeMove: chance to thaw.
    Thaw,
    /// BeforeMove: chance to hit oneself in confusion.
    ConfusionSelfHit,
    /// BeforeMove: 1 in 2 for an infatuated Pokémon (Attract) to be immobilised by love.
    Infatuation,
    /// Number of hits of a multi-hit move (the value is the hit count itself).
    MultiHitCount,
    /// A contact-hit ability's chance to status the attacker (Static, Flame Body, Poison Point).
    ContactReaction,
    /// Shed Skin's end-of-turn cure.
    ShedSkin,
    /// Protect / Detect used again right after a use: 1 / stall counter.
    ProtectSuccess,
    /// A chance to act first in the priority bracket: Quick Claw (Gen 2-3 once per turn, Gen
    /// 4+ per move action), Quick Draw.
    PriorityChance,
    /// Gen 1 Disable: which of the target's moves with PP it seals (index among them).
    DisableSlot,
    /// Showdown `sample`: which of a move's listed options it takes (Tri Attack's status; the
    /// value is the index; Magnitude's: the magnitude).
    Sample,
    /// A move's own chance to change its power (Fickle Beam doubling it).
    PowerChance,
    /// Shell Side Arm's coin flip when its physical and special estimates tie (true: physical).
    CategoryTie,
    /// A chance a reaction to a damaging hit takes after the ordered ones (Cursed Body, Poison
    /// Touch, Toxic Chain; Gen 3-4 Effect Spore).
    HitReaction,
    /// Gen 5+ Effect Spore's `random(100)`: which status, if any (the value is the draw).
    EffectSpore,
    /// Healer's 3 in 10 chance to cure an adjacent ally at the end of the turn.
    Healer,
    /// Focus Band's chance to leave its holder at 1 HP against a move's damage that would knock
    /// it out (`ConditionRules::focus_band_chance`).
    FocusBand,
    /// Harvest's 1 in 2 chance out of the sun, rolled at every end of the turn.
    Harvest,
    /// Gen 2-3 Spite: how much PP it takes (2-5).
    PpDrop,
    /// Gen 7 (DESIGN.md §44.3): where a Mega Evolution puts the Pokémon's move action back in
    /// the queue (`insertChoice`'s `battle.random(firstIndex, lastIndex + 1)`).
    QueueInsert,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RandomOverrideValue {
    Bool(bool),
    Range(u16),
    Raw(#[serde(with = "crate::snapshot::u64_string")] u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomOverride {
    pub kind: RandomRequestKind,
    pub value: RandomOverrideValue,
    /// A decision the engine may not draw at all: skipped when the next request is of another
    /// kind (Gen 1-2 roll no damage variance for a damage of 1, which a plan cannot know
    /// before the damage is computed).
    pub optional: bool,
}

impl RandomOverride {
    pub const fn bool(kind: RandomRequestKind, value: bool) -> Self {
        Self {
            kind,
            value: RandomOverrideValue::Bool(value),
            optional: false,
        }
    }

    pub const fn range(kind: RandomRequestKind, value: u16) -> Self {
        Self {
            kind,
            value: RandomOverrideValue::Range(value),
            optional: false,
        }
    }

    pub const fn raw(kind: RandomRequestKind, value: u64) -> Self {
        Self {
            kind,
            value: RandomOverrideValue::Raw(value),
            optional: false,
        }
    }

    /// The same decision, skipped if the engine does not ask for it.
    pub const fn optional(self) -> Self {
        Self {
            optional: true,
            ..self
        }
    }
}

/// Deterministic random source stored inside battle state.
///
/// Two test-control modes coexist intentionally:
///
/// - `raw_tape`: exact PRNG-frame compatibility testing.
/// - `semantic_tape`: architecture-independent rule testing. Every entry names why randomness is
///   being requested (Accuracy, Critical, DamageRoll, ...). Requesting the wrong kind panics in a
///   test instead of silently consuming a value intended for another rule stage.
///
/// Both cursors and the fallback SplitMix64 state are part of `BattleState`, so snapshots are
/// reproducible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleRng {
    // `u64`s are decimal strings in a snapshot's JSON (DESIGN.md §48.2): beyond what a
    // JavaScript number holds exactly.
    #[serde(with = "crate::snapshot::u64_string")]
    state: u64,
    #[serde(with = "crate::snapshot::u64_strings")]
    raw_tape: Vec<u64>,
    raw_cursor: usize,
    semantic_tape: Vec<RandomOverride>,
    semantic_cursor: usize,
}

impl BattleRng {
    pub const fn new(seed: u64) -> Self {
        Self {
            state: seed,
            raw_tape: Vec::new(),
            raw_cursor: 0,
            semantic_tape: Vec::new(),
            semantic_cursor: 0,
        }
    }

    /// Exact PRNG-frame override. Prefer `with_semantic_tape` for ordinary rule tests.
    pub fn with_tape(seed: u64, tape: Vec<u64>) -> Self {
        Self {
            state: seed,
            raw_tape: tape,
            raw_cursor: 0,
            semantic_tape: Vec::new(),
            semantic_cursor: 0,
        }
    }

    pub fn with_semantic_tape(seed: u64, tape: Vec<RandomOverride>) -> Self {
        Self {
            state: seed,
            raw_tape: Vec::new(),
            raw_cursor: 0,
            semantic_tape: tape,
            semantic_cursor: 0,
        }
    }

    pub fn tape_cursor(&self) -> usize {
        self.raw_cursor
    }

    pub fn semantic_tape_cursor(&self) -> usize {
        self.semantic_cursor
    }

    /// Every planned decision was drawn (optional ones the engine never asked for aside).
    pub fn semantic_tape_exhausted(&self) -> bool {
        self.semantic_tape[self.semantic_cursor.min(self.semantic_tape.len())..]
            .iter()
            .all(|entry| entry.optional)
    }

    pub fn next_u64(&mut self) -> u64 {
        if let Some(value) = self.raw_tape.get(self.raw_cursor).copied() {
            self.raw_cursor += 1;
            return value;
        }

        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_u64_for(&mut self, kind: RandomRequestKind) -> u64 {
        match self.take_semantic(kind) {
            Some(RandomOverrideValue::Raw(value)) => value,
            Some(other) => {
                panic!("semantic random override for {kind:?} must be Raw, got {other:?}")
            }
            None => self.next_u64(),
        }
    }

    pub fn range_inclusive(&mut self, min: u16, max: u16) -> u16 {
        assert!(min <= max, "invalid inclusive random range");
        let width = u64::from(max - min) + 1;
        min + (self.next_u64() % width) as u16
    }

    pub fn range_inclusive_for(&mut self, kind: RandomRequestKind, min: u16, max: u16) -> u16 {
        assert!(min <= max, "invalid inclusive random range");
        if min == max {
            return min;
        }
        match self.take_semantic(kind) {
            Some(RandomOverrideValue::Range(value)) => {
                assert!(
                    (min..=max).contains(&value),
                    "semantic random override {value} for {kind:?} outside [{min}, {max}]"
                );
                value
            }
            Some(other) => {
                panic!("semantic random override for {kind:?} must be Range, got {other:?}")
            }
            None => self.range_inclusive(min, max),
        }
    }

    pub fn roll_basis_points(&mut self, chance: u16) -> bool {
        match chance {
            0 => false,
            10_000.. => true,
            _ => (self.next_u64() % 10_000) < u64::from(chance),
        }
    }

    /// Weighted choice among `values`; a semantic `Range` override names the value directly
    /// (it must be one of `values`), so a tape can fix e.g. a multi-hit count without knowing
    /// the generation's distribution.
    pub fn sample_weighted_for(
        &mut self,
        kind: RandomRequestKind,
        values: &[u16],
        weights: &[u16],
    ) -> u16 {
        assert!(
            !values.is_empty() && values.len() == weights.len(),
            "weighted sample needs one weight per value"
        );
        match self.take_semantic(kind) {
            Some(RandomOverrideValue::Range(value)) => {
                assert!(
                    values.contains(&value),
                    "semantic random override {value} for {kind:?} is not one of {values:?}"
                );
                value
            }
            Some(other) => {
                panic!("semantic random override for {kind:?} must be Range, got {other:?}")
            }
            None => {
                let total: u64 = weights.iter().map(|w| u64::from(*w)).sum();
                assert!(total > 0, "weighted sample needs a positive total weight");
                let mut pick = self.next_u64() % total;
                for (value, weight) in values.iter().zip(weights) {
                    let weight = u64::from(*weight);
                    if pick < weight {
                        return *value;
                    }
                    pick -= weight;
                }
                values[values.len() - 1]
            }
        }
    }

    pub fn roll_basis_points_for(&mut self, kind: RandomRequestKind, chance: u16) -> bool {
        match chance {
            0 => return false,
            10_000.. => return true,
            _ => {}
        }
        match self.take_semantic(kind) {
            Some(RandomOverrideValue::Bool(value)) => value,
            Some(other) => {
                panic!("semantic random override for {kind:?} must be Bool, got {other:?}")
            }
            None => self.roll_basis_points(chance),
        }
    }

    /// Showdown's `randomChance(numerator, denominator)`: true with probability n / d.
    pub fn roll_ratio(&mut self, numerator: u16, denominator: u16) -> bool {
        if numerator == 0 {
            return false;
        }
        if numerator >= denominator {
            return true;
        }
        (self.next_u64() % u64::from(denominator)) < u64::from(numerator)
    }

    pub fn roll_ratio_for(
        &mut self,
        kind: RandomRequestKind,
        numerator: u16,
        denominator: u16,
    ) -> bool {
        if numerator == 0 {
            return false;
        }
        if numerator >= denominator {
            return true;
        }
        match self.take_semantic(kind) {
            Some(RandomOverrideValue::Bool(value)) => value,
            Some(other) => {
                panic!("semantic random override for {kind:?} must be Bool, got {other:?}")
            }
            None => self.roll_ratio(numerator, denominator),
        }
    }

    pub fn roll_one_in(&mut self, denominator: u16) -> bool {
        denominator <= 1 || self.next_u64().is_multiple_of(u64::from(denominator))
    }

    pub fn roll_one_in_for(&mut self, kind: RandomRequestKind, denominator: u16) -> bool {
        if denominator <= 1 {
            return true;
        }
        match self.take_semantic(kind) {
            Some(RandomOverrideValue::Bool(value)) => value,
            Some(other) => {
                panic!("semantic random override for {kind:?} must be Bool, got {other:?}")
            }
            None => self.roll_one_in(denominator),
        }
    }

    pub fn roll_percent(&mut self, chance: u8) -> bool {
        match chance {
            0 => false,
            100.. => true,
            _ => (self.next_u64() % 100) < u64::from(chance),
        }
    }

    pub fn roll_percent_for(&mut self, kind: RandomRequestKind, chance: u8) -> bool {
        match chance {
            0 => return false,
            100.. => return true,
            _ => {}
        }
        match self.take_semantic(kind) {
            Some(RandomOverrideValue::Bool(value)) => value,
            Some(other) => {
                panic!("semantic random override for {kind:?} must be Bool, got {other:?}")
            }
            None => self.roll_percent(chance),
        }
    }

    fn take_semantic(&mut self, kind: RandomRequestKind) -> Option<RandomOverrideValue> {
        // An optional decision the engine did not ask for is passed over.
        while let Some(entry) = self.semantic_tape.get(self.semantic_cursor)
            && entry.optional
            && entry.kind != kind
        {
            self.semantic_cursor += 1;
        }
        let entry = self.semantic_tape.get(self.semantic_cursor).copied()?;
        assert_eq!(
            entry.kind, kind,
            "semantic random plan desynchronized at index {}",
            self.semantic_cursor
        );
        self.semantic_cursor += 1;
        Some(entry.value)
    }
}

impl Default for BattleRng {
    fn default() -> Self {
        Self::new(0)
    }
}
