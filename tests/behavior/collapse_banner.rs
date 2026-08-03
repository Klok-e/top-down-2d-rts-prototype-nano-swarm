use top_down_2d_rts_prototype_nano_swarm::{
    nanobot::{MatchOutcome, ProductionCollapseState, latch_match_outcome},
    ui::collapse_banner::{CollapsePresentation, collapse_presentation},
};

#[test]
fn collapse_presentation_prefers_defeat_when_both_swarms_collapse() {
    assert_eq!(
        collapse_presentation(MatchOutcome::InProgress),
        CollapsePresentation::Hidden,
    );
    assert_eq!(
        collapse_presentation(MatchOutcome::Victory),
        CollapsePresentation::Victory,
    );
    assert_eq!(
        collapse_presentation(MatchOutcome::Defeat),
        CollapsePresentation::Defeat,
    );
}

#[test]
fn match_outcome_remains_latched_after_collapse_conditions_clear() {
    let victory = latch_match_outcome(
        MatchOutcome::InProgress,
        ProductionCollapseState {
            opponent_collapsed: true,
            ..Default::default()
        },
    );

    assert_eq!(victory, MatchOutcome::Victory);
    assert_eq!(
        latch_match_outcome(victory, ProductionCollapseState::default()),
        MatchOutcome::Victory,
    );
}
