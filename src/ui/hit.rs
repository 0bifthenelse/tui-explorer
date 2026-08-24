use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegendAction {
    Quit,
    Help,
    Command,
    Hidden,
    Select,
    QuickTag,
    TagPicker,
    Open,
    OpenWith,
    Parent,
    Cancel,
    Encrypt,
    Sidebar,
    Preview,
    Bookmarks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    Row(usize),
    /// Blank grid space between/around tiles; a left press here arms a
    /// marquee selection instead of touching any entry.
    GridBackground,
    Breadcrumb(usize),
    Sidebar(usize),
    Legend(LegendAction),
    TagBadge,
    ModalConfirm,
    ModalCancel,
    ConflictCancel,
    ConflictSkip,
    ConflictReplace,
    PickerItem(usize),
    PickerNew,
    PickerDelete,
    PickerClose,
    ContextItem(usize),
    MediaTogglePause,
    MediaSeekBack,
    MediaSeekForward,
    MediaVolumeDown,
    MediaVolumeUp,
    MediaStop,
    MediaClose,
    Blocker,
    Details,
}

#[derive(Clone, Debug, Default)]
pub struct HitMap {
    pub regions: Vec<(Rect, HitTarget)>,
}

impl HitMap {
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    pub fn push(&mut self, rect: Rect, target: HitTarget) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        self.regions.push((rect, target));
    }

    pub fn hit(&self, x: u16, y: u16) -> Option<HitTarget> {
        self.regions
            .iter()
            .rev()
            .find(|(rect, _)| {
                x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
            })
            .map(|(_, target)| *target)
    }

    pub fn rect_for(&self, target: HitTarget) -> Option<Rect> {
        self.regions
            .iter()
            .rev()
            .find(|(_, t)| *t == target)
            .map(|(rect, _)| *rect)
    }

    /// Visible row positions whose tile rectangles intersect `area`, sorted
    /// ascending. Centralizes marquee-intersection geometry so the rule is
    /// testable independent of the renderer.
    pub fn rows_intersecting(&self, area: Rect) -> Vec<usize> {
        let mut rows: Vec<usize> = self
            .regions
            .iter()
            .filter_map(|(rect, target)| match target {
                HitTarget::Row(pos) if rects_intersect(*rect, area) => Some(*pos),
                _ => None,
            })
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }
}

/// Half-open cell-rectangle intersection: both `x` and `y` spans overlap.
fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topmost_region_wins() {
        let mut map = HitMap::default();
        map.push(Rect::new(0, 0, 10, 10), HitTarget::Row(0));
        map.push(Rect::new(2, 2, 4, 4), HitTarget::ModalConfirm);
        assert_eq!(map.hit(3, 3), Some(HitTarget::ModalConfirm));
        assert_eq!(map.hit(8, 8), Some(HitTarget::Row(0)));
        assert_eq!(map.hit(20, 20), None);
    }

    #[test]
    fn zero_size_regions_ignored() {
        let mut map = HitMap::default();
        map.push(Rect::new(0, 0, 0, 5), HitTarget::Row(1));
        assert_eq!(map.hit(0, 0), None);
    }
}
