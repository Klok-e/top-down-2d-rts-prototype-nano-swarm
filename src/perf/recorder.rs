use std::collections::VecDeque;

use serde::{Serialize, Serializer, ser::SerializeMap};

pub const HITCH_FRAME_THRESHOLD_NS: u64 = 33_300_000;
pub const HITCH_FIXED_TICK_THRESHOLD: u64 = 1;
pub const PRE_HITCH_FRAMES: usize = 120;
pub const POST_HITCH_FRAMES: usize = 30;
pub const EPISODE_FRAME_LIMIT: usize = 600;
pub const EPISODE_RETENTION_LIMIT: usize = 32;
const EPISODE_HEAD_LIMIT: usize = EPISODE_FRAME_LIMIT / 2;
const EPISODE_TAIL_LIMIT: usize = EPISODE_FRAME_LIMIT - EPISODE_HEAD_LIMIT;
const SUMMARY_PHASE_COUNT: usize = ScheduleTimes::COUNT + FixedPhaseTimes::COUNT + 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum FixedPhase {
    Movement,
    Threat,
    Combat,
    Maintenance,
    AllocationProject,
    AllocationInvalidate,
    AllocationAcquire,
    StrategicController,
    #[default]
    Other,
}

impl FixedPhase {
    pub(super) const ALL: [Self; 9] = [
        Self::Movement,
        Self::Threat,
        Self::Combat,
        Self::Maintenance,
        Self::AllocationProject,
        Self::AllocationInvalidate,
        Self::AllocationAcquire,
        Self::StrategicController,
        Self::Other,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Movement => "movement",
            Self::Threat => "threat",
            Self::Combat => "combat",
            Self::Maintenance => "maintenance",
            Self::AllocationProject => "allocation_project",
            Self::AllocationInvalidate => "allocation_invalidate",
            Self::AllocationAcquire => "allocation_acquire",
            Self::StrategicController => "strategic_controller",
            Self::Other => "fixed_other",
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FixedPhaseTimes([u64; Self::COUNT]);

impl FixedPhaseTimes {
    const COUNT: usize = FixedPhase::ALL.len();

    pub(super) fn add(&mut self, phase: FixedPhase, duration_ns: u64) {
        self.0[phase as usize] += duration_ns;
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&'static str, u64)> + '_ {
        FixedPhase::ALL
            .into_iter()
            .map(|phase| (phase.name(), self.0[phase as usize]))
    }

    pub fn total(self) -> u64 {
        self.0.into_iter().sum()
    }
}

impl Serialize for FixedPhaseTimes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(Self::COUNT))?;
        for (phase, time) in self.iter() {
            map.serialize_entry(phase, &time)?;
        }
        map.end()
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ScheduleTimes {
    pub first: u64,
    pub pre_update: u64,
    pub state_transition: u64,
    pub run_fixed_main_loop: u64,
    pub update: u64,
    pub spawn_scene: u64,
    pub post_update: u64,
    pub last: u64,
}

impl ScheduleTimes {
    const COUNT: usize = 8;

    fn entries(self) -> [(&'static str, u64); Self::COUNT] {
        [
            ("First", self.first),
            ("PreUpdate", self.pre_update),
            ("StateTransition", self.state_transition),
            ("RunFixedMainLoop", self.run_fixed_main_loop),
            ("Update", self.update),
            ("SpawnScene", self.spawn_scene),
            ("PostUpdate", self.post_update),
            ("Last", self.last),
        ]
    }

    pub fn total(self) -> u64 {
        self.first
            + self.pre_update
            + self.state_transition
            + self.run_fixed_main_loop
            + self.update
            + self.spawn_scene
            + self.post_update
            + self.last
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct FrameRecord {
    pub frame: u64,
    pub fixed_tick: u64,
    pub frame_ns: u64,
    pub fixed_ticks_run: u64,
    pub schedule_ns: ScheduleTimes,
    pub fixed_phase_ns: FixedPhaseTimes,
    pub unmeasured_ns: u64,
    pub nanobots: usize,
    pub structures: usize,
}

#[derive(Clone, Copy)]
pub enum HitchDetection {
    Excluded,
    Paced,
    Unbounded { fixed_tick_ns: u64 },
}

impl HitchDetection {
    fn is_hitch(self, frame: FrameRecord) -> bool {
        match self {
            Self::Excluded => false,
            Self::Paced => {
                frame.frame_ns > HITCH_FRAME_THRESHOLD_NS
                    || frame.fixed_ticks_run > HITCH_FIXED_TICK_THRESHOLD
            }
            Self::Unbounded { fixed_tick_ns } => fixed_tick_ns > HITCH_FRAME_THRESHOLD_NS,
        }
    }
}

impl FrameRecord {
    fn phases(&self) -> impl Iterator<Item = (&'static str, u64)> {
        let mut schedules = self.schedule_ns;
        schedules.run_fixed_main_loop = schedules
            .run_fixed_main_loop
            .saturating_sub(self.fixed_phase_ns.total());
        schedules
            .entries()
            .into_iter()
            .chain(self.fixed_phase_ns.iter())
            .chain([("unmeasured", self.unmeasured_ns)])
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct HitchEpisodeSummary {
    start_frame: u64,
    end_frame: u64,
    start_fixed_tick: u64,
    hitch_count: u64,
    worst_frame_ns: u64,
    catch_up_ticks: u64,
    top_phases: [PhaseTotal; 3],
    nanobots: usize,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
struct PhaseTotal {
    phase: &'static str,
    duration_ns: u64,
}

#[derive(Debug, Serialize)]
pub struct HitchEpisode {
    pub id: u64,
    pub first_hitch_frame: u64,
    pub last_hitch_frame: u64,
    pub frames: Vec<FrameRecord>,
    #[serde(skip)]
    summary: HitchEpisodeSummary,
}

impl HitchEpisode {
    /// Returns the compact summary of all hitch frames, including those omitted from detail.
    pub fn summary(&self) -> &HitchEpisodeSummary {
        &self.summary
    }
}

struct OpenEpisode {
    episode: HitchEpisode,
    phase_totals: [PhaseTotal; SUMMARY_PHASE_COUNT],
    tail: VecDeque<FrameRecord>,
    frames_since_hitch: usize,
}

impl OpenEpisode {
    fn record(&mut self, frame: FrameRecord, is_hitch: bool) {
        let summary = &mut self.episode.summary;
        summary.end_frame = frame.frame;
        if is_hitch {
            self.episode.last_hitch_frame = frame.frame;
            self.frames_since_hitch = 0;
            summary.hitch_count += 1;
            summary.catch_up_ticks += frame.fixed_ticks_run.saturating_sub(1);
            if summary.hitch_count == 1 || frame.frame_ns > summary.worst_frame_ns {
                summary.worst_frame_ns = frame.frame_ns;
                summary.nanobots = frame.nanobots;
            }
            for (total, (phase, duration_ns)) in self.phase_totals.iter_mut().zip(frame.phases()) {
                total.phase = phase;
                total.duration_ns = total.duration_ns.saturating_add(duration_ns);
            }
        } else {
            self.frames_since_hitch += 1;
        }

        if self.episode.frames.len() < EPISODE_HEAD_LIMIT {
            self.episode.frames.push(frame);
        } else {
            if self.tail.len() == EPISODE_TAIL_LIMIT {
                self.tail.pop_front();
            }
            self.tail.push_back(frame);
        }
    }

    fn close(mut self) -> HitchEpisode {
        self.phase_totals.sort_unstable_by(|a, b| {
            b.duration_ns
                .cmp(&a.duration_ns)
                .then_with(|| a.phase.cmp(b.phase))
        });
        self.episode
            .summary
            .top_phases
            .copy_from_slice(&self.phase_totals[..3]);
        self.episode.frames.extend(self.tail);
        self.episode
    }
}

#[derive(Default)]
pub struct FrameRecorder {
    history: VecDeque<FrameRecord>,
    open: Option<OpenEpisode>,
    closed: VecDeque<HitchEpisode>,
    dropped: u64,
    last_id: u64,
}

impl FrameRecorder {
    pub(crate) fn reset_session(&mut self) {
        self.history.clear();
        self.open = None;
    }

    /// Records the next consecutive frame and returns an episode if this frame closed one.
    pub fn record(
        &mut self,
        frame: FrameRecord,
        detection: HitchDetection,
    ) -> Option<&HitchEpisode> {
        let is_hitch = detection.is_hitch(frame);
        if is_hitch && self.open.is_none() {
            self.last_id += 1;
            let start = self.history.front().copied().unwrap_or(frame);
            self.open = Some(OpenEpisode {
                episode: HitchEpisode {
                    id: self.last_id,
                    first_hitch_frame: frame.frame,
                    last_hitch_frame: frame.frame,
                    frames: self.history.iter().copied().collect(),
                    summary: HitchEpisodeSummary {
                        start_frame: start.frame,
                        start_fixed_tick: start.fixed_tick.saturating_sub(start.fixed_ticks_run),
                        ..Default::default()
                    },
                },
                phase_totals: [PhaseTotal::default(); SUMMARY_PHASE_COUNT],
                tail: VecDeque::new(),
                frames_since_hitch: 0,
            });
        }
        if self.history.len() == PRE_HITCH_FRAMES {
            self.history.pop_front();
        }
        self.history.push_back(frame);
        let open = self.open.as_mut()?;
        open.record(frame, is_hitch);
        if open.frames_since_hitch == POST_HITCH_FRAMES {
            let closed = self.open.take().unwrap().close();
            if self.closed.len() == EPISODE_RETENTION_LIMIT {
                self.closed.pop_front();
                self.dropped += 1;
            }
            self.closed.push_back(closed);
            return self.closed.back();
        }
        None
    }

    pub fn episodes_after(&self, after_id: Option<u64>) -> impl Iterator<Item = &HitchEpisode> {
        self.closed
            .iter()
            .filter(move |episode| episode.id > after_id.unwrap_or(0))
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(recorder: &mut FrameRecorder, first: u64, last: u64, hitch: bool) {
        for frame in first..=last {
            recorder.record(
                FrameRecord {
                    frame,
                    frame_ns: if hitch { 34_000_000 } else { 16_000_000 },
                    ..Default::default()
                },
                HitchDetection::Paced,
            );
        }
    }

    #[test]
    fn a_long_episode_keeps_the_first_and_last_300_frames_and_summarizes_every_hitch() {
        let mut recorder = FrameRecorder::default();
        record(&mut recorder, 1, 120, false);
        record(&mut recorder, 121, 730, true);
        record(&mut recorder, 731, 760, false);

        let episode = recorder
            .episodes_after(None)
            .next()
            .expect("closed episode");
        let retained = episode.frames.iter().map(|frame| frame.frame);
        assert!(retained.eq((1..=300).chain(461..=760)));
        let summary = serde_json::to_value(episode.summary()).unwrap();
        assert_eq!(summary["hitch_count"], 610);
    }

    #[test]
    fn retention_discards_oldest_episodes_and_counts_each_discard() {
        let mut recorder = FrameRecorder::default();
        for episode in 0..35 {
            let first = episode * 31 + 1;
            record(&mut recorder, first, first, true);
            record(&mut recorder, first + 1, first + 30, false);
        }

        assert_eq!(recorder.dropped(), 3);
        assert!(
            recorder
                .episodes_after(None)
                .map(|episode| episode.id)
                .eq(4..=35)
        );
    }
}
