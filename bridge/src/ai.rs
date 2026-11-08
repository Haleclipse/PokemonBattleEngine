//! A legal-choice policy. It reads `choice_options` and does not calculate damage.

use pokemon_battle_engine::{
    ActionChoice, BattleEngine, BattleRequest, ReviveChoice, SideChoice, SideId, SlotChoice,
};

#[derive(Clone)]
pub struct Policy(pub u64);

impl Policy {
    pub(crate) fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n.max(1) as u64) as usize
    }

    pub fn turn(&mut self, engine: &BattleEngine, side: SideId) -> SideChoice {
        let options = engine.choice_options(side).expect("turn options");
        let mut taken = Vec::new();
        let actions = options
            .actors
            .iter()
            .map(|actor| {
                let moves: Vec<ActionChoice> = actor
                    .moves
                    .iter()
                    .flat_map(|option| {
                        option.targets.iter().map(|target| ActionChoice::Move {
                            slot: option.slot,
                            target: *target,
                            mechanic: None,
                        })
                    })
                    .collect();
                let switches: Vec<_> = actor
                    .switches
                    .iter()
                    .filter(|member| !taken.contains(*member))
                    .copied()
                    .collect();
                let action = if let Some(locked) = &actor.locked {
                    locked.clone()
                } else if !moves.is_empty() {
                    moves[self.below(moves.len())].clone()
                } else if !switches.is_empty() {
                    let pokemon = switches[self.below(switches.len())];
                    taken.push(pokemon);
                    ActionChoice::Switch { pokemon }
                } else {
                    ActionChoice::Pass
                };
                SlotChoice {
                    actor: actor.actor,
                    action,
                }
            })
            .collect();
        SideChoice { side, actions }
    }

    pub fn revive(&mut self, request: &BattleRequest) -> Option<ReviveChoice> {
        let BattleRequest::Revive { side, candidates } = request else {
            return None;
        };
        if candidates.is_empty() {
            return None;
        }
        Some(ReviveChoice {
            side: *side,
            pokemon: candidates[self.below(candidates.len())],
        })
    }
}
