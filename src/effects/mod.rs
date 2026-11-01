pub mod abilities;
pub mod items;
pub mod moves;
mod registry;

pub use registry::BehaviorRegistry;

use crate::engine::BattleContext;
use crate::ids::PokemonId;
use crate::rules::ResidualKind;

/// An ability or item's part in the end-of-turn residual schedule (Leftovers, Shed Skin).
/// Showdown: `onResidual`.
pub type ResidualHandler = for<'a> fn(&mut BattleContext<'a>, PokemonId);

/// An ability or item reacting to its holder entering or leaving the field (Intimidate,
/// Natural Cure, a Choice item clearing its lock).
/// Showdown: `onStart` (entering), `onSwitchOut` (leaving).
pub type SwitchHandler = for<'a> fn(&mut BattleContext<'a>, PokemonId);

/// One component of an effect's behavior and the Showdown callbacks it stands for (DESIGN.md
/// §25.1). Moves, abilities and items each keep a table of these next to their behavior struct,
/// and the same table drives both sides of the registry: the dex's support classification
/// (every callback a generation declares must be covered) and the per-generation behavior the
/// engine runs (a component is active only where the generation declares one of its
/// callbacks: Gen 2's Growth has no `onModifyMove`, Gen 2's Dragon Fang declares nothing).
pub(crate) struct BehaviorComponent<B> {
    /// The Showdown callbacks this component stands for (upstream terms, data only).
    pub source_callbacks: fn(&B) -> &'static [&'static str],
    /// Switches the component off in `behavior`.
    pub disable: fn(&mut B),
}

/// Every Showdown callback some component of `behavior` stands for.
pub(crate) fn covered_callbacks<B>(
    components: &[BehaviorComponent<B>],
    behavior: &B,
) -> Vec<&'static str> {
    let mut covered = Vec::new();
    for component in components {
        for callback in (component.source_callbacks)(behavior) {
            if !covered.contains(callback) {
                covered.push(*callback);
            }
        }
    }
    covered
}

/// `behavior` without the components none of whose callbacks `declared` lists. A component that
/// stands for no callback (Showdown hard-codes it: Facade's exemption from the burn drop) is
/// kept.
pub(crate) fn restricted_to<B>(
    components: &[BehaviorComponent<B>],
    mut behavior: B,
    declared: &[String],
) -> B {
    for component in components {
        let callbacks = (component.source_callbacks)(&behavior);
        if !callbacks.is_empty()
            && !callbacks
                .iter()
                .any(|callback| declared.iter().any(|d| d == callback))
        {
            (component.disable)(&mut behavior);
        }
    }
    behavior
}

/// Where an ability or item moves its holder's move inside its priority bracket (Showdown
/// `onFractionalPriority`, DESIGN.md §12.4): the fixed shifts apply first, then the chances in
/// handler order (Quick Draw at −1, Quick Claw at −2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FractionalPriority {
    /// Stall (−0.1, and in Gen 4 only while nothing has lowered it yet), Lagging Tail and Full
    /// Incense (`ActionOrderRules::delay_item_tenths`): last in the bracket.
    Delay,
    /// Quick Claw (Gen 4+): first in the bracket on a 1-in-5 chance, rolled only while nothing
    /// has raised it yet.
    QuickClaw,
    /// Quick Draw: first in the bracket on a 3-in-10 chance, damaging moves only.
    QuickDraw,
    /// Mycelium Might (`onFractionalPriorityPriority` −1, after Lagging Tail and Stall, before
    /// Quick Claw): a status move is last in its bracket (−0.1), and a Quick Claw stays
    /// quiet for it.
    DelayStatus,
}

/// An ability raising its holder's move priority (Showdown `onModifyPriority`, DESIGN.md §12.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityBoost {
    /// Prankster (Gen 5+): +1 for a status move, which is then `pranksterBoosted` (from Gen 7
    /// a Dark-type foe is immune to it).
    Prankster,
    /// Gale Wings (Gen 6+): +1 for a Flying move — from Gen 7 only at full HP
    /// (`ActionOrderRules::gale_wings_full_hp`).
    GaleWings,
    /// Triage (Gen 7+): +3 for a healing move (`heal` flag).
    Triage,
}

/// An ability or item that takes part in the end-of-turn residual schedule; `kind` looks up
/// the generation's order key (ROADMAP §7.3).
#[derive(Clone, Copy)]
pub struct ResidualParticipant {
    pub kind: ResidualKind,
    pub handler: ResidualHandler,
}
