//! Controlled, rotationally balanced geography for AI Battle.

use bevy::prelude::*;

use crate::ai_battle::AiBattleLayout;

use super::{NEUTRAL_DEPOSIT_CELLS, PLAYER_CELL, PLAYER_DEPOSIT_CELL};

#[derive(Clone, Copy)]
pub struct LayoutSetup {
    pub home: IVec2,
    pub home_deposit: IVec2,
    pub neutral_deposits: [IVec2; 4],
}

impl LayoutSetup {
    pub fn for_layout(layout: AiBattleLayout) -> Self {
        let mut setup = Self {
            home: PLAYER_CELL,
            home_deposit: PLAYER_DEPOSIT_CELL,
            neutral_deposits: NEUTRAL_DEPOSIT_CELLS,
        };
        if layout == AiBattleLayout::Flanks {
            setup.home = IVec2::new(0, 2);
            setup.home_deposit = IVec2::new(-1, 1);
            setup.neutral_deposits = [
                IVec2::new(1, 9),
                IVec2::new(23, 15),
                IVec2::new(8, 22),
                IVec2::new(16, 2),
            ];
        }
        setup
    }

    pub fn opposite(cell: IVec2) -> IVec2 {
        IVec2::splat(24) - cell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flanks_relocates_both_economies_and_neutral_deposits_symmetrically() {
        let setup = LayoutSetup::for_layout(AiBattleLayout::Flanks);
        assert_eq!(setup.home, IVec2::new(0, 2));
        assert_eq!(LayoutSetup::opposite(setup.home), IVec2::new(24, 22));
        assert_eq!(setup.home_deposit, IVec2::new(-1, 1));
        assert_eq!(
            LayoutSetup::opposite(setup.home_deposit),
            IVec2::new(25, 23)
        );
        assert_ne!(setup.neutral_deposits, NEUTRAL_DEPOSIT_CELLS);
        for deposit in setup.neutral_deposits {
            assert!(
                setup
                    .neutral_deposits
                    .contains(&LayoutSetup::opposite(deposit))
            );
        }
    }
}
