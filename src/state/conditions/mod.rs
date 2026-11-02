//! Conditions (DESIGN.md §30): a Pokémon's volatile conditions (`volatile*.rs`), a side's
//! (`side.rs`), the field's (`field.rs`).

mod field;
mod side;
mod volatile;
mod volatile_choices;
mod volatile_kinds;
mod volatile_locks;
mod volatile_traps;

pub use field::*;
pub use side::*;
pub use volatile::*;
pub use volatile_kinds::*;
