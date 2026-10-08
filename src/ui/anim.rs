//! Implicit-animation engine.
//!
//! Renderers describe *targets* ("this row is hovered → 1.0") and the
//! animator answers with the current, eased value, starting a tween
//! whenever a target changes. Values are pure functions of the frame clock,
//! so animations cost nothing when idle: the event loop only switches to a
//! fast frame cadence while [`Animator::active`] reports running tweens.
//!
//! Reduced motion (tests, `TUI_EXPLORER_REDUCED_MOTION`, `:set animations
//! off`) snaps every value straight to its target.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::ui::hit::HitTarget;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimKey {
    /// Hover tint of an entry, keyed by a stable path hash.
    RowHover(u64),
    /// Selection fill of an entry, keyed by a stable path hash.
    RowSelect(u64),
    /// Focus glow of an entry, keyed by a stable path hash.
    RowFocus(u64),
    /// Screen row of the gliding cursor highlight in a named pane.
    CursorY(u8),
    /// Hover tint of an interactive control.
    Control(HitTarget),
    /// Generic keyed channel (meters, modal open, status flash, ...).
    Named(&'static str),
    /// Generic indexed channel.
    Indexed(&'static str, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    Linear,
    OutCubic,
    InOutCubic,
    OutBack,
}

impl Ease {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
            Ease::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Ease::OutBack => {
                let c1 = 1.701_58_f32;
                let c3 = c1 + 1.0;
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    delay: Duration,
    dur: Duration,
    ease: Ease,
    seen: u64,
}

impl Tween {
    fn value(&self, now: Instant) -> f32 {
        let begin = self.start + self.delay;
        if now <= begin {
            return self.from;
        }
        let elapsed = now.duration_since(begin);
        if self.dur.is_zero() || elapsed >= self.dur {
            return self.to;
        }
        let t = elapsed.as_secs_f32() / self.dur.as_secs_f32();
        self.from + (self.to - self.from) * self.ease.apply(t)
    }

    fn running(&self, now: Instant) -> bool {
        now < self.start + self.delay + self.dur && (self.to - self.from).abs() > f32::EPSILON
    }
}

pub const HOVER_IN: Duration = Duration::from_millis(110);
pub const HOVER_OUT: Duration = Duration::from_millis(240);
pub const SELECT: Duration = Duration::from_millis(160);
pub const GLIDE: Duration = Duration::from_millis(90);
pub const MODAL: Duration = Duration::from_millis(140);
pub const CASCADE_STEP: Duration = Duration::from_millis(9);
pub const CASCADE_DUR: Duration = Duration::from_millis(200);
pub const CASCADE_MAX_ROWS: usize = 48;

#[derive(Debug)]
pub struct Animator {
    enabled: bool,
    tweens: HashMap<AnimKey, Tween>,
    frame: u64,
    now: Instant,
    /// Start of the staggered row cascade after a directory change.
    cascade: Option<Instant>,
    /// Continuous animations requested this frame (spinners, playback).
    keep_alive: bool,
    /// Reference point for periodic animations (spinners).
    epoch: Instant,
}

impl Default for Animator {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Animator {
    pub fn new(enabled: bool) -> Self {
        Animator {
            enabled,
            tweens: HashMap::new(),
            frame: 0,
            now: Instant::now(),
            cascade: None,
            keep_alive: false,
            epoch: Instant::now(),
        }
    }

    /// Frame index of a periodic animation stepping every `period_ms`
    /// (spinners). Constant when motion is reduced.
    pub fn tick(&self, period_ms: u64) -> usize {
        if !self.enabled {
            return 0;
        }
        (self.now.saturating_duration_since(self.epoch).as_millis() / u128::from(period_ms.max(1)))
            as usize
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.tweens.clear();
            self.cascade = None;
        }
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    /// Starts a frame at `now`; values requested until `end_frame` are
    /// sampled at this instant.
    pub fn begin_frame(&mut self, now: Instant) {
        self.now = now;
        self.frame = self.frame.wrapping_add(1);
        self.keep_alive = false;
    }

    /// Drops tweens that were not requested during this frame, so state
    /// never accumulates for rows that scrolled away.
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        self.tweens.retain(|_, t| t.seen == frame);
    }

    /// Current value for `key` heading to `target`, retargeting with
    /// `dur` when the target changed. First sight snaps to the target.
    pub fn track(&mut self, key: AnimKey, target: f32, dur: Duration) -> f32 {
        self.track_with(key, target, dur, dur, Ease::OutCubic)
    }

    /// Like [`track`](Self::track) with separate rise and fall durations
    /// (hover feels snappy going in and soft going out).
    pub fn track_asym(&mut self, key: AnimKey, target: f32, up: Duration, down: Duration) -> f32 {
        self.track_with(key, target, up, down, Ease::OutCubic)
    }

    pub fn track_with(
        &mut self,
        key: AnimKey,
        target: f32,
        up: Duration,
        down: Duration,
        ease: Ease,
    ) -> f32 {
        if !self.enabled {
            return target;
        }
        let now = self.now;
        let frame = self.frame;
        match self.tweens.get_mut(&key) {
            Some(tween) => {
                tween.seen = frame;
                if (tween.to - target).abs() > f32::EPSILON {
                    let current = tween.value(now);
                    tween.from = current;
                    tween.to = target;
                    tween.start = now;
                    tween.delay = Duration::ZERO;
                    tween.dur = if target > current { up } else { down };
                    tween.ease = ease;
                }
                tween.value(now)
            }
            None => {
                self.tweens.insert(
                    key,
                    Tween {
                        from: target,
                        to: target,
                        start: now,
                        delay: Duration::ZERO,
                        dur: Duration::ZERO,
                        ease,
                        seen: frame,
                    },
                );
                target
            }
        }
    }

    /// Value for `key` that animates from `initial` on first sight (enter
    /// transitions such as a modal opening).
    pub fn enter(&mut self, key: AnimKey, initial: f32, target: f32, dur: Duration) -> f32 {
        if !self.enabled {
            return target;
        }
        let now = self.now;
        let frame = self.frame;
        let tween = self.tweens.entry(key).or_insert(Tween {
            from: initial,
            to: target,
            start: now,
            delay: Duration::ZERO,
            dur,
            ease: Ease::OutCubic,
            seen: frame,
        });
        tween.seen = frame;
        tween.value(now)
    }

    /// Restarts an enter transition for `key` (e.g. a status flash when a
    /// new message arrives).
    pub fn restart(&mut self, key: AnimKey, from: f32, to: f32, dur: Duration) {
        if !self.enabled {
            return;
        }
        self.tweens.insert(
            key,
            Tween {
                from,
                to,
                start: self.now,
                delay: Duration::ZERO,
                dur,
                ease: Ease::OutCubic,
                seen: self.frame,
            },
        );
    }

    /// Kicks off the staggered row cascade (directory changes).
    pub fn start_cascade(&mut self, now: Instant) {
        if self.enabled {
            self.cascade = Some(now);
        }
    }

    /// Appear progress (0..1) of the `index`-th visible row in the cascade.
    pub fn cascade(&self, index: usize) -> f32 {
        let Some(start) = self.cascade else {
            return 1.0;
        };
        let index = index.min(CASCADE_MAX_ROWS);
        let begin = start + CASCADE_STEP * index as u32;
        if self.now <= begin {
            return 0.0;
        }
        let t = self.now.duration_since(begin).as_secs_f32() / CASCADE_DUR.as_secs_f32();
        Ease::OutCubic.apply(t)
    }

    /// Requests continuous frames for this frame (spinners, live meters).
    pub fn keep_alive(&mut self) {
        if self.enabled {
            self.keep_alive = true;
        }
    }

    /// True while any tween, the cascade, or a keep-alive request needs
    /// another frame soon.
    pub fn active(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.keep_alive {
            return true;
        }
        if let Some(start) = self.cascade {
            let end = start + CASCADE_STEP * CASCADE_MAX_ROWS as u32 + CASCADE_DUR;
            if self.now < end {
                return true;
            }
        }
        self.tweens.values().any(|t| t.running(self.now))
    }

    /// Clears the cascade once finished so `active` stays cheap.
    pub fn settle(&mut self) {
        if let Some(start) = self.cascade {
            let end = start + CASCADE_STEP * CASCADE_MAX_ROWS as u32 + CASCADE_DUR;
            if self.now >= end {
                self.cascade = None;
            }
        }
    }
}

/// Stable 64-bit key for a path (animation identity across frames).
pub fn path_key(path: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_animator_snaps_to_target() {
        let mut anim = Animator::new(false);
        anim.begin_frame(Instant::now());
        assert_eq!(anim.track(AnimKey::Named("x"), 1.0, HOVER_IN), 1.0);
        assert!(!anim.active());
    }

    #[test]
    fn tween_eases_toward_new_target() {
        let t0 = Instant::now();
        let mut anim = Animator::new(true);
        anim.begin_frame(t0);
        assert_eq!(anim.track(AnimKey::Named("x"), 0.0, HOVER_IN), 0.0);
        anim.end_frame();
        anim.begin_frame(t0 + Duration::from_millis(1));
        let v = anim.track(AnimKey::Named("x"), 1.0, Duration::from_millis(100));
        assert!(v < 0.1, "starts near the old value: {v}");
        assert!(anim.active());
        anim.end_frame();
        anim.begin_frame(t0 + Duration::from_millis(51));
        let mid = anim.track(AnimKey::Named("x"), 1.0, Duration::from_millis(100));
        assert!(mid > 0.5 && mid < 1.0, "out-cubic midpoint: {mid}");
        anim.end_frame();
        anim.begin_frame(t0 + Duration::from_millis(200));
        assert_eq!(anim.track(AnimKey::Named("x"), 1.0, HOVER_IN), 1.0);
        assert!(!anim.active());
    }

    #[test]
    fn unseen_tweens_are_pruned() {
        let mut anim = Animator::new(true);
        anim.begin_frame(Instant::now());
        anim.track(AnimKey::Named("gone"), 1.0, HOVER_IN);
        anim.end_frame();
        anim.begin_frame(Instant::now());
        anim.end_frame();
        assert!(anim.tweens.is_empty());
    }

    #[test]
    fn cascade_staggers_rows() {
        let t0 = Instant::now();
        let mut anim = Animator::new(true);
        anim.start_cascade(t0);
        anim.begin_frame(t0 + Duration::from_millis(60));
        assert!(anim.cascade(0) > anim.cascade(5));
        assert!(anim.active());
        anim.begin_frame(t0 + Duration::from_secs(2));
        assert_eq!(anim.cascade(30), 1.0);
        anim.settle();
        assert!(!anim.active());
    }

    #[test]
    fn easing_curves_hit_endpoints() {
        for ease in [
            Ease::Linear,
            Ease::OutCubic,
            Ease::InOutCubic,
            Ease::OutBack,
        ] {
            assert!((ease.apply(0.0)).abs() < 1e-5);
            assert!((ease.apply(1.0) - 1.0).abs() < 1e-5);
        }
    }
}
