//! L1 tests for the self-changing abilities (DESIGN.md §43.4, R-N.2): what the replay cannot
//! see. The frames carry the forme, its types and ability, but not the line each ability draws
//! its forme at, nor a level gate that happens to be satisfied in a scenario.
use pokemon_battle_engine::effects::abilities::{HpDenominator, SelfForme, SelfFormeRule};
use pokemon_battle_engine::ids::SpeciesId;

fn darmanitan() -> SelfForme {
    SelfForme {
        holder: SpeciesId::DARMANITAN,
        rule: SelfFormeRule::ZenMode {
            at: HpDenominator::Half,
            other: SpeciesId::DARMANITAN_ZEN,
            galar: Some((SpeciesId::DARMANITAN_GALAR, SpeciesId::DARMANITAN_GALAR_ZEN)),
        },
    }
}

fn wishiwashi() -> SelfForme {
    SelfForme {
        holder: SpeciesId::WISHIWASHI,
        rule: SelfFormeRule::Threshold {
            at: HpDenominator::Quarter,
            high: SpeciesId::WISHIWASHI_SCHOOL,
            min_level: 20,
        },
    }
}

fn zygarde() -> SelfForme {
    SelfForme {
        holder: SpeciesId::ZYGARDE,
        rule: SelfFormeRule::PowerConstruct {
            at: HpDenominator::Half,
            other: SpeciesId::ZYGARDE_COMPLETE,
        },
    }
}

/// Showdown compares `hp` against the integer division of the max HP, so 90 of 180 is at the
/// line and 91 is above it.
#[test]
fn the_hp_line_is_the_generations_integer_division() {
    assert!(HpDenominator::Half.at_or_below(90, 180));
    assert!(!HpDenominator::Half.at_or_below(91, 180));
    assert!(HpDenominator::Quarter.at_or_below(45, 180));
    assert!(!HpDenominator::Quarter.at_or_below(46, 180));
}

/// Zen Mode: its own forme above the line, the Zen one at or below — the Galar forme its Galar
/// Zen forme.
#[test]
fn zen_mode_picks_by_the_line_and_by_the_base_forme() {
    let rule = darmanitan();
    let own = Some(SpeciesId::DARMANITAN);
    assert_eq!(rule.forme_for(50, 180, 180, own, own), None);
    assert_eq!(
        rule.forme_for(50, 90, 180, own, own),
        Some(SpeciesId::DARMANITAN_ZEN)
    );
    assert_eq!(
        rule.forme_for(50, 100, 180, own, Some(SpeciesId::DARMANITAN_ZEN)),
        Some(SpeciesId::DARMANITAN)
    );
    let galar = Some(SpeciesId::DARMANITAN_GALAR);
    assert_eq!(
        rule.forme_for(50, 40, 180, galar, galar),
        Some(SpeciesId::DARMANITAN_GALAR_ZEN)
    );
    assert_eq!(
        rule.forme_for(50, 170, 180, galar, Some(SpeciesId::DARMANITAN_GALAR_ZEN)),
        Some(SpeciesId::DARMANITAN_GALAR)
    );
}

/// Schooling: its School forme above a quarter of its max HP, its own below, and nothing at all
/// below level 20.
#[test]
fn schooling_asks_the_quarter_and_the_level() {
    let rule = wishiwashi();
    let own = Some(SpeciesId::WISHIWASHI);
    assert_eq!(
        rule.forme_for(20, 180, 180, own, own),
        Some(SpeciesId::WISHIWASHI_SCHOOL)
    );
    assert_eq!(rule.forme_for(19, 180, 180, own, own), None);
    assert_eq!(
        rule.forme_for(50, 45, 180, own, Some(SpeciesId::WISHIWASHI_SCHOOL)),
        Some(SpeciesId::WISHIWASHI)
    );
}

/// Power Construct is one way: at or below half its HP it becomes Complete, and above it — or
/// already Complete — nothing changes.
#[test]
fn power_construct_only_ever_goes_to_complete() {
    let rule = zygarde();
    let own = Some(SpeciesId::ZYGARDE);
    assert_eq!(rule.forme_for(50, 180, 180, own, own), None);
    assert_eq!(
        rule.forme_for(50, 90, 180, own, own),
        Some(SpeciesId::ZYGARDE_COMPLETE)
    );
    let complete = Some(SpeciesId::ZYGARDE_COMPLETE);
    assert_eq!(rule.forme_for(50, 180, 291, complete, complete), None);
    assert_eq!(rule.forme_for(50, 100, 291, complete, complete), None);
}
