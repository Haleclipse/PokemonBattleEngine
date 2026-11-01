use crate::dex::MoveCategory;
use crate::engine::OffensiveStatContext;

/// Huge Power / Pure Power: `chainModify(2)` on physical moves.
pub fn modify_offense(ctx: &mut OffensiveStatContext<'_>) -> Option<u32> {
    (ctx.move_category() == MoveCategory::Physical).then_some(8192)
}
