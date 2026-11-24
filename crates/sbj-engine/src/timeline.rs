use std::collections::HashSet;
use std::f32::consts::FRAC_PI_2;
use std::fmt;
use std::sync::Arc;

use crate::{AudioBuffer, Samples};

/// Shape of a fade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FadeCurve {
    #[default]
    Linear,
    /// Sine-shaped; sounds smoother than linear for fades to silence.
    EqualPower,
}

impl FadeCurve {
    /// Gain for normalised progress `x` in `0.0..=1.0` (0 = silent, 1 = full).
    #[inline]
    pub(crate) fn gain(self, x: f32) -> f32 {
        match self {
            Self::Linear => x,
            Self::EqualPower => (x * FRAC_PI_2).sin(),
        }
    }
}

/// A fade-in or fade-out on a clip. A length of zero means no fade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fade {
    pub length: Samples,
    pub curve: FadeCurve,
}

impl Fade {
    pub const NONE: Self = Self {
        length: 0,
        curve: FadeCurve::Linear,
    };

    pub fn new(length: Samples, curve: FadeCurve) -> Self {
        Self { length, curve }
    }
}

/// A region of an [`AudioBuffer`] placed on the timeline.
#[derive(Clone, Debug)]
pub struct Clip {
    pub buffer: Arc<AudioBuffer>,
    /// Where the clip starts on the timeline.
    pub start: Samples,
    /// Where in the buffer playback begins.
    pub source_offset: Samples,
    /// How many frames of the buffer are played.
    pub length: Samples,
    /// Linear gain (1.0 = unchanged).
    pub gain: f32,
    pub fade_in: Fade,
    pub fade_out: Fade,
}

impl Clip {
    /// A clip playing the whole buffer from timeline position 0.
    pub fn new(buffer: Arc<AudioBuffer>) -> Self {
        let length = buffer.frames();
        Self {
            buffer,
            start: 0,
            source_offset: 0,
            length,
            gain: 1.0,
            fade_in: Fade::NONE,
            fade_out: Fade::NONE,
        }
    }

    pub fn at(mut self, start: Samples) -> Self {
        self.start = start;
        self
    }

    /// Plays `length` frames of the buffer starting at `source_offset`.
    pub fn trimmed(mut self, source_offset: Samples, length: Samples) -> Self {
        self.source_offset = source_offset;
        self.length = length;
        self
    }

    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    pub fn with_fade_in(mut self, fade: Fade) -> Self {
        self.fade_in = fade;
        self
    }

    pub fn with_fade_out(mut self, fade: Fade) -> Self {
        self.fade_out = fade;
        self
    }

    /// Timeline position just after the clip's last frame.
    pub fn end(&self) -> Samples {
        self.start + self.length
    }

    /// Envelope gain (fades only) at `offset` frames into the clip.
    #[inline]
    pub(crate) fn envelope(&self, offset: Samples) -> f32 {
        let mut g = 1.0;
        if offset < self.fade_in.length {
            g *= self
                .fade_in
                .curve
                .gain(offset as f32 / self.fade_in.length as f32);
        }
        let remaining = self.length - offset;
        if remaining <= self.fade_out.length {
            g *= self
                .fade_out
                .curve
                .gain(remaining as f32 / self.fade_out.length as f32);
        }
        g
    }
}

/// Stable identifier the UI uses to refer to a marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MarkerId(pub u64);

/// How many times a [`MarkerAction::Loop`] jumps back before letting the
/// playhead continue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopCount {
    /// Loop until something else (the operator, another command) intervenes.
    Infinite,
    /// Jump back this many times, so the looped section plays `n + 1` times
    /// in total, then continue past the marker.
    Times(u32),
}

/// What happens when the playhead reaches a marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerAction {
    /// Move the playhead to `to` and keep playing.
    Jump { to: Samples },
    /// Jump back to `to` a number of times, then continue. The counter resets
    /// whenever the playhead passes through, so nested loops behave like
    /// musical repeats.
    Loop { to: Samples, count: LoopCount },
    /// Halt at the marker. Resuming continues past it.
    Pause,
    /// Halt and return the playhead to the start of the timeline.
    Stop,
    /// Fade everything out over `length` frames, then stop.
    FadeOut { length: Samples },
}

/// A point on the timeline that triggers an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub id: MarkerId,
    pub position: Samples,
    pub action: MarkerAction,
}

impl Marker {
    pub fn new(id: u64, position: Samples, action: MarkerAction) -> Self {
        Self {
            id: MarkerId(id),
            position,
            action,
        }
    }
}

/// Why a timeline failed validation.
#[derive(Debug, Clone, PartialEq)]
pub enum TimelineError {
    InvalidSampleRate,
    /// A clip's buffer is at a different sample rate than the timeline.
    /// Buffers must be resampled when they are loaded.
    SampleRateMismatch {
        clip: usize,
        buffer_rate: u32,
        timeline_rate: u32,
    },
    /// A clip reads past the end of its buffer.
    ClipOutOfRange {
        clip: usize,
    },
    /// A clip's gain is negative, NaN or infinite.
    InvalidGain {
        clip: usize,
    },
    /// Two markers share an id.
    DuplicateMarkerId(MarkerId),
    /// A jump or loop target lies beyond the end of the timeline.
    TargetOutOfRange {
        marker: MarkerId,
        target: Samples,
    },
}

impl fmt::Display for TimelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => write!(f, "timeline sample rate must be non-zero"),
            Self::SampleRateMismatch {
                clip,
                buffer_rate,
                timeline_rate,
            } => write!(
                f,
                "clip {clip} is {buffer_rate} Hz but the timeline is {timeline_rate} Hz"
            ),
            Self::ClipOutOfRange { clip } => {
                write!(f, "clip {clip} reads past the end of its audio")
            }
            Self::InvalidGain { clip } => write!(f, "clip {clip} has an invalid gain"),
            Self::DuplicateMarkerId(id) => write!(f, "marker id {} is used twice", id.0),
            Self::TargetOutOfRange { marker, target } => write!(
                f,
                "marker {} targets position {target}, past the end of the timeline",
                marker.0
            ),
        }
    }
}

impl std::error::Error for TimelineError {}

/// Collects clips and markers, then validates them into a [`Timeline`].
#[derive(Debug, Clone)]
pub struct TimelineBuilder {
    sample_rate: u32,
    clips: Vec<Clip>,
    markers: Vec<Marker>,
    length: Option<Samples>,
}

impl TimelineBuilder {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            clips: Vec::new(),
            markers: Vec::new(),
            length: None,
        }
    }

    pub fn clip(mut self, clip: Clip) -> Self {
        self.clips.push(clip);
        self
    }

    pub fn marker(mut self, marker: Marker) -> Self {
        self.markers.push(marker);
        self
    }

    /// Removes every marker with `id` (e.g. one that failed validation).
    pub fn without_marker(mut self, id: MarkerId) -> Self {
        self.markers.retain(|m| m.id != id);
        self
    }

    /// Sets an explicit length. The timeline is never shorter than its
    /// furthest clip end or marker.
    pub fn length(mut self, length: Samples) -> Self {
        self.length = Some(length);
        self
    }

    pub fn build(mut self) -> Result<Timeline, TimelineError> {
        if self.sample_rate == 0 {
            return Err(TimelineError::InvalidSampleRate);
        }
        for (i, clip) in self.clips.iter().enumerate() {
            if clip.buffer.sample_rate() != self.sample_rate {
                return Err(TimelineError::SampleRateMismatch {
                    clip: i,
                    buffer_rate: clip.buffer.sample_rate(),
                    timeline_rate: self.sample_rate,
                });
            }
            let source_end = clip.source_offset.checked_add(clip.length);
            if source_end.is_none_or(|end| end > clip.buffer.frames()) {
                return Err(TimelineError::ClipOutOfRange { clip: i });
            }
            if !clip.gain.is_finite() || clip.gain < 0.0 {
                return Err(TimelineError::InvalidGain { clip: i });
            }
        }
        let mut ids = HashSet::new();
        for m in &self.markers {
            if !ids.insert(m.id) {
                return Err(TimelineError::DuplicateMarkerId(m.id));
            }
        }

        let content_end = self
            .clips
            .iter()
            .map(Clip::end)
            .chain(self.markers.iter().map(|m| m.position))
            .max()
            .unwrap_or(0);
        let length = self.length.unwrap_or(0).max(content_end);

        for m in &self.markers {
            if let MarkerAction::Jump { to } | MarkerAction::Loop { to, .. } = m.action
                && to > length
            {
                return Err(TimelineError::TargetOutOfRange {
                    marker: m.id,
                    target: to,
                });
            }
        }

        // Stable sorts: markers sharing a position fire in insertion order.
        self.clips.sort_by_key(|c| c.start);
        self.markers.sort_by_key(|m| m.position);

        Ok(Timeline {
            sample_rate: self.sample_rate,
            clips: self.clips,
            markers: self.markers,
            length,
        })
    }
}

/// A validated, immutable arrangement of clips and markers.
#[derive(Debug, Clone)]
pub struct Timeline {
    sample_rate: u32,
    clips: Vec<Clip>,
    markers: Vec<Marker>,
    length: Samples,
}

impl Timeline {
    pub fn builder(sample_rate: u32) -> TimelineBuilder {
        TimelineBuilder::new(sample_rate)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Clips sorted by start position.
    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }

    /// Markers sorted by position.
    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    pub fn length(&self) -> Samples {
        self.length
    }

    /// Index range of markers sitting exactly at `position`.
    #[inline]
    pub(crate) fn markers_at(&self, position: Samples) -> std::ops::Range<usize> {
        let start = self.markers.partition_point(|m| m.position < position);
        let end = self.markers.partition_point(|m| m.position <= position);
        start..end
    }

    /// Position of the first marker strictly after `position`, if any.
    #[inline]
    pub(crate) fn next_marker_after(&self, position: Samples) -> Option<Samples> {
        let i = self.markers.partition_point(|m| m.position <= position);
        self.markers.get(i).map(|m| m.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(frames: usize) -> Arc<AudioBuffer> {
        Arc::new(AudioBuffer::from_interleaved(48_000, 1, vec![0.0; frames]).unwrap())
    }

    #[test]
    fn length_covers_clips_and_markers() {
        let t = Timeline::builder(48_000)
            .clip(Clip::new(buf(100)).at(50))
            .marker(Marker::new(1, 300, MarkerAction::Pause))
            .build()
            .unwrap();
        assert_eq!(t.length(), 300);
    }

    #[test]
    fn rejects_clip_past_buffer_end() {
        let err = Timeline::builder(48_000)
            .clip(Clip::new(buf(100)).trimmed(50, 51))
            .build()
            .unwrap_err();
        assert_eq!(err, TimelineError::ClipOutOfRange { clip: 0 });
    }

    #[test]
    fn rejects_sample_rate_mismatch() {
        let b = Arc::new(AudioBuffer::from_interleaved(44_100, 1, vec![0.0; 10]).unwrap());
        let err = Timeline::builder(48_000)
            .clip(Clip::new(b))
            .build()
            .unwrap_err();
        assert!(matches!(err, TimelineError::SampleRateMismatch { .. }));
    }

    #[test]
    fn rejects_out_of_range_targets_and_duplicate_ids() {
        let err = Timeline::builder(48_000)
            .clip(Clip::new(buf(100)))
            .marker(Marker::new(1, 10, MarkerAction::Jump { to: 101 }))
            .build()
            .unwrap_err();
        assert!(matches!(err, TimelineError::TargetOutOfRange { .. }));

        let err = Timeline::builder(48_000)
            .marker(Marker::new(1, 10, MarkerAction::Pause))
            .marker(Marker::new(1, 20, MarkerAction::Pause))
            .build()
            .unwrap_err();
        assert_eq!(err, TimelineError::DuplicateMarkerId(MarkerId(1)));
    }

    #[test]
    fn marker_lookup() {
        let t = Timeline::builder(48_000)
            .marker(Marker::new(1, 20, MarkerAction::Pause))
            .marker(Marker::new(2, 10, MarkerAction::Pause))
            .marker(Marker::new(3, 10, MarkerAction::Stop))
            .build()
            .unwrap();
        assert_eq!(t.markers_at(10), 0..2);
        assert_eq!(
            t.markers()[0].id,
            MarkerId(2),
            "stable order at same position"
        );
        assert_eq!(t.markers_at(15), 2..2);
        assert_eq!(t.next_marker_after(10), Some(20));
        assert_eq!(t.next_marker_after(20), None);
    }

    #[test]
    fn envelope_fades() {
        let c = Clip::new(buf(100))
            .with_fade_in(Fade::new(10, FadeCurve::Linear))
            .with_fade_out(Fade::new(10, FadeCurve::Linear));
        assert_eq!(c.envelope(0), 0.0);
        assert_eq!(c.envelope(5), 0.5);
        assert_eq!(c.envelope(50), 1.0);
        assert_eq!(c.envelope(95), 0.5);
        assert!((c.envelope(99) - 0.1).abs() < 1e-6);
    }
}
