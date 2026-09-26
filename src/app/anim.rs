//! Presentation-only transitions. Values are deterministic functions of
//! elapsed time passed in by the event loop; nothing here mutates data.

/// Fixed animation frame budget. The event loop polls quickly only while
/// animation is live and reverts to idle polling when it settles.
pub const TICK_MS: u64 = 16;

/// Wall-clock seconds for one full transition at the default rate.
pub const SETTLE_SECS: f32 = 0.16;

/// Progress below this is treated as settled.
pub const EPSILON: f32 = 0.002;

/// One eased scalar in the 0.0..=1.0 range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ease {
    pos: f32,
    target: f32,
}

impl Ease {
    pub fn new(pos: f32) -> Self {
        let v = clamp01(pos);
        Self { pos: v, target: v }
    }

    /// Current progress, clamped to 0.0..=1.0.
    pub fn pos(&self) -> f32 {
        clamp01(self.pos)
    }

    pub fn target(&self) -> f32 {
        clamp01(self.target)
    }

    /// Retargets without teleporting; the value eases from where it is.
    pub fn set(&mut self, target: f32) {
        let target = clamp01(target);
        if target == self.target {
            return;
        }
        // From exactly here the remaining distance is under the threshold.
        if (target - self.pos).abs() <= EPSILON {
            self.pos = target;
        }
        self.target = target;
    }

    /// Snaps immediately (resize, modal switch, navigation reset).
    pub fn snap(&mut self, target: f32) {
        let target = clamp01(target);
        self.pos = target;
        self.target = target;
    }

    /// True while the value still differs from its target by more than EPSILON.
    pub fn active(&self) -> bool {
        (self.target - self.pos).abs() > EPSILON
    }

    /// Advances by `dt_secs`; returns true when still active.
    pub fn advance(&mut self, dt_secs: f32) -> bool {
        let k = rate(dt_secs);
        self.pos += (self.target - self.pos) * k;
        if (self.target - self.pos).abs() <= EPSILON {
            self.pos = self.target;
        }
        self.active()
    }

    /// Discrete step index in `0..steps` for the current progress, used to
    /// pick one of a few pre-chosen indexed terminal colors.
    pub fn step(&self, steps: usize) -> usize {
        if steps == 0 {
            return 0;
        }
        let p = self.pos();
        if p <= 0.0 {
            return 0;
        }
        if p >= 1.0 {
            return steps - 1;
        }
        ((p * steps as f32) as usize).min(steps - 1)
    }
}

/// Presentation-only transitions. Never consulted by mutation logic.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anim {
    /// Pointer highlight strength on the hovered grid row.
    pub hover: Ease,
    /// Keyboard focus highlight moving between tiles.
    pub focus: Ease,
    /// Drop-target border emphasis while a drag is in flight.
    pub drop_target: Ease,
    /// Context-menu selected-row emphasis.
    pub menu: Ease,
    /// Command-center theme cursor emphasis.
    pub theme_cursor: Ease,
    /// Marquee band emphasis.
    pub marquee: Ease,
    /// Horizontal and vertical drag ghost offsets in cells.
    pub ghost_x: f32,
    pub ghost_y: f32,
}

impl Anim {
    /// True while any eased value is still moving; the event loop uses this
    /// to decide between fast and idle polling.
    pub fn active(&self) -> bool {
        self.hover.active()
            || self.focus.active()
            || self.drop_target.active()
            || self.menu.active()
            || self.theme_cursor.active()
            || self.marquee.active()
    }

    /// Advances every eased value by `dt_secs`.
    pub fn advance(&mut self, dt_secs: f32) {
        self.hover.advance(dt_secs);
        self.focus.advance(dt_secs);
        self.drop_target.advance(dt_secs);
        self.menu.advance(dt_secs);
        self.theme_cursor.advance(dt_secs);
        self.marquee.advance(dt_secs);
    }

    /// Snaps every eased value to its target (resize, navigation, modal change).
    pub fn settle(&mut self) {
        self.hover.snap(self.hover.target());
        self.focus.snap(self.focus.target());
        self.drop_target.snap(self.drop_target.target());
        self.menu.snap(self.menu.target());
        self.theme_cursor.snap(self.theme_cursor.target());
        self.marquee.snap(self.marquee.target());
    }

    /// Eases the drag ghost toward the pointer cell, in cell units.
    pub fn move_ghost(&mut self, x: f32, y: f32, dt_secs: f32) {
        let k = rate(dt_secs);
        self.ghost_x += (x - self.ghost_x) * k;
        self.ghost_y += (y - self.ghost_y) * k;
    }

    /// Teleports the drag ghost, for a fresh drag press.
    pub fn snap_ghost(&mut self, x: f32, y: f32) {
        self.ghost_x = x;
        self.ghost_y = y;
    }

    /// Rounded cell offset for painting the ghost.
    pub fn ghost_cells(&self) -> (i16, i16) {
        (to_cell(self.ghost_x), to_cell(self.ghost_y))
    }
}

/// Per-frame easing fraction. Chosen so the remaining distance reaches
/// `EPSILON` after exactly `SETTLE_SECS` of wall clock, which is what that
/// constant promises; a plain `dt / SETTLE_SECS` would take about six times
/// longer to settle.
fn rate(dt_secs: f32) -> f32 {
    if !dt_secs.is_finite() || dt_secs <= 0.0 {
        return 0.0;
    }
    let steps = dt_secs / SETTLE_SECS;
    (1.0 - EPSILON.powf(steps)).clamp(0.0, 1.0)
}

fn clamp01(v: f32) -> f32 {
    if v.is_nan() {
        return 0.0;
    }
    v.clamp(0.0, 1.0)
}

/// Saturating round; non-finite input collapses to 0 so painting can never panic.
fn to_cell(v: f32) -> i16 {
    if !v.is_finite() {
        return 0;
    }
    let r = v.round();
    if r >= i16::MAX as f32 {
        return i16::MAX;
    }
    if r <= i16::MIN as f32 {
        return i16::MIN;
    }
    r as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = TICK_MS as f32 / 1000.0;

    #[test]
    fn advance_is_deterministic() {
        let mut a = Ease::new(0.0);
        let mut b = Ease::new(0.0);
        for i in 0..64 {
            let dt = DT * (1.0 + (i % 3) as f32 * 0.25);
            a.set(1.0);
            b.set(1.0);
            a.advance(dt);
            b.advance(dt);
            assert_eq!(a.pos().to_bits(), b.pos().to_bits(), "frame {i}");
        }
        assert_eq!(a, b);
    }

    #[test]
    fn settles_within_expected_frames() {
        // SETTLE_SECS is the documented wall-clock time for one transition,
        // and 50% headroom must be enough for the frame quantization.
        let budget = (SETTLE_SECS * 1.5 / DT).ceil() as usize;
        assert_eq!(budget, 15);
        let mut e = Ease::new(0.0);
        e.set(1.0);
        let mut frames = 0usize;
        while e.advance(DT) {
            frames += 1;
            assert!(frames <= budget, "still active after {frames} frames");
        }
        assert!(
            frames <= budget,
            "settled after {frames} frames, budget is {budget}"
        );
        assert_eq!(e.pos(), 1.0);
        assert_eq!(e.target(), 1.0);
        assert!(!e.active());
    }

    #[test]
    fn retarget_does_not_teleport() {
        let mut e = Ease::new(0.0);
        e.set(1.0);
        assert!(e.advance(DT));
        let before = e.pos();
        assert!(before > 0.0 && before < 1.0, "precondition: mid flight");
        assert!((before - 0.0).abs() > EPSILON && (1.0 - before).abs() > EPSILON);
        // Reverse to a target the value is genuinely still away from, so the
        // next frame has to ease instead of snapping.
        e.set(0.5);
        assert!(e.active(), "0.5 is still far from {before}");
        assert_eq!(e.pos(), before, "retarget must not move the value");
        let back = e.advance(DT);
        let after = e.pos();
        assert!(back, "must still be easing");
        assert!(
            (before - after).abs() < 0.05,
            "{after} teleported from {before}"
        );
        assert!(e.target() == 0.5);
        // Retargeting onto the current value is a legal no-op teleport.
        e.set(after);
        assert_eq!(e.pos(), after);
        assert!(!e.active());
    }

    #[test]
    fn snap_is_immediate() {
        let mut e = Ease::new(0.0);
        e.set(0.6);
        e.advance(DT);
        e.snap(1.0);
        assert_eq!(e.pos(), 1.0);
        assert_eq!(e.target(), 1.0);
        assert!(!e.active());
    }

    #[test]
    fn step_index_edges() {
        let at0 = Ease::new(0.0);
        assert_eq!(at0.step(0), 0);
        assert_eq!(at0.step(4), 0);
        let mut at1 = Ease::new(0.0);
        at1.snap(1.0);
        assert_eq!(at1.step(4), 3);
        assert_eq!(at1.step(1), 0);
        let mut e = Ease::new(0.0);
        for n in 0..=1000 {
            let t = n as f32 / 1000.0;
            e.snap(t);
            let s = e.step(4);
            assert!(s <= 3, "t {t} -> {s}");
            assert_eq!(e.step(0), 0);
        }
    }

    #[test]
    fn ghost_tracks_without_snap_drift() {
        let mut a = Anim::default();
        a.move_ghost(10.0, -4.0, DT);
        a.move_ghost(10.0, -4.0, DT);
        // A twin fed the same pointer must track bit-for-bit, and neither may
        // ever differ from a plain f32 re-derivation of the same recurrence:
        // an epsilon snap or a sticky tail would show up as a mismatch.
        let mut b = a;
        let mut ref_x = a.ghost_x;
        let mut ref_y = a.ghost_y;
        let mut prev = (a.ghost_x, a.ghost_y);
        for frame in 0..10_000u32 {
            a.move_ghost(10.0, -4.0, DT);
            b.move_ghost(10.0, -4.0, DT);
            ref_x += (10.0 - ref_x) * rate(DT);
            ref_y += (-4.0 - ref_y) * rate(DT);
            let cur = (a.ghost_x, a.ghost_y);
            assert!(
                cur.0 >= prev.0 && cur.1 <= prev.1,
                "ghost reversed at {frame}"
            );
            assert!(cur.0 <= 10.0 && cur.1 >= -4.0, "ghost overshot: {cur:?}");
            assert_eq!(
                cur.0.to_bits(),
                b.ghost_x.to_bits(),
                "twin drifted at {frame}"
            );
            assert_eq!(
                cur.1.to_bits(),
                b.ghost_y.to_bits(),
                "twin drifted at {frame}"
            );
            assert_eq!(
                cur.0.to_bits(),
                ref_x.to_bits(),
                "not the f32 recurrence at {frame}"
            );
            assert_eq!(
                cur.1.to_bits(),
                ref_y.to_bits(),
                "not the f32 recurrence at {frame}"
            );
            prev = cur;
        }
        let gap = (10.0 - a.ghost_x).abs() + (a.ghost_y + 4.0).abs();
        assert!(gap < 1e-3, "ghost never converged: gap {gap}");
        let cells = a.ghost_cells();
        for _ in 0..100 {
            a.move_ghost(10.0, -4.0, DT);
        }
        assert_eq!(a.ghost_cells(), cells);
        assert_eq!(cells, (10, -4));
        a.snap_ghost(f32::MAX, f32::NEG_INFINITY);
        // IEEE rounds -inf to 0 (ties to even); the positive side saturates.
        assert_eq!(a.ghost_cells(), (i16::MAX, 0));
        a.snap_ghost(f32::NAN, f32::NAN);
        assert_eq!(a.ghost_cells(), (0, 0));
        a.snap_ghost(1e30, -1e30);
        assert_eq!(a.ghost_cells(), (i16::MAX, i16::MIN));
    }

    #[test]
    fn anim_active_only_while_moving() {
        let mut anim = Anim::default();
        assert!(!anim.active());
        anim.hover.set(1.0);
        assert!(anim.active());
        let mut frames = 0usize;
        while anim.active() {
            anim.advance(DT);
            frames += 1;
            assert!(frames <= 59, "anim stuck after {frames} frames");
        }
        assert!(frames <= 15, "anim took {frames} frames, budget is 15");
        assert_eq!(anim.hover.pos(), 1.0);
        anim.settle();
        assert!(!anim.active());
    }
}
