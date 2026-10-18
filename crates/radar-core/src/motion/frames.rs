//! Renderer-local geometry and bounded clean-state retention for explicit interaction motion.
use super::interaction::{self, Border, FocusBand, FoldDirection, FoldRows};
use super::transition::{MotionContext, MotionFold};
use crate::{theme::Theme, view::PaneId};
use ratatui::{
    buffer::{Buffer, Cell},
    layout::Rect,
    style::Color,
};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameKey {
    pub pane: PaneId,
    pub slot: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameMark {
    pub key: FrameKey,
    pub area: Rect,
    pub owns_keys: bool,
    pub top_title: Option<Rect>,
    pub keep: Vec<Rect>,
}
impl FrameMark {
    fn border(&self) -> Border<'_> {
        Border {
            area: self.area,
            top_title: self.top_title,
            keep: &self.keep,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeMark {
    pub key: FrameKey,
    pub area: Rect,
    pub rows: Vec<(Vec<String>, u16)>,
    pub open: Vec<Vec<String>>,
}
thread_local! { static FRAME: RefCell<Option<FrameKey>> = const { RefCell::new(None) }; }

pub fn enabled() -> bool {
    super::MARKS.with(|marks| marks.borrow().context.is_some())
}

pub fn page_identified() -> bool {
    super::MARKS.with(|marks| {
        !matches!(
            &marks.borrow().context,
            Some(MotionContext::Screen { page: None, .. })
        )
    })
}

/// Scope the kit's frame registration without making parts know about motion.
pub fn with_frame<T>(pane: PaneId, slot: &str, draw: impl FnOnce() -> T) -> T {
    if !enabled() {
        return draw();
    }
    struct Restore(Option<FrameKey>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FRAME.with(|key| *key.borrow_mut() = self.0.take());
        }
    }
    let old = FRAME.with(|key| {
        key.replace(Some(FrameKey {
            pane,
            slot: slot.into(),
        }))
    });
    let _restore = Restore(old);
    draw()
}
pub fn context(context: Option<MotionContext>, fold: Option<MotionFold>) {
    let context = context.filter(context_bounded);
    super::MARKS.with(|marks| {
        let mut m = marks.borrow_mut();
        m.fold = context.as_ref().and(fold);
        m.context = context;
    });
}
fn context_bounded(context: &MotionContext) -> bool {
    let length = match context {
        MotionContext::Screen {
            plugin,
            screen,
            page,
            depth,
            ..
        } => {
            if *depth > 64 {
                return false;
            }
            plugin.len() + screen.len() + page.as_ref().map_or(0, String::len)
        }
        MotionContext::Settings { plugin, section } => {
            plugin.as_ref().map_or(0, String::len) + section.len()
        }
        MotionContext::History => 0,
        MotionContext::Setup {
            step,
            plugin,
            section,
        } => {
            step.len()
                + plugin.as_ref().map_or(0, String::len)
                + section.as_ref().map_or(0, String::len)
        }
    };
    length <= 16_384
}
fn path_bounded(path: &[String]) -> bool {
    path.len() <= 64 && path.iter().map(String::len).sum::<usize>() <= 4096
}

pub fn frame(area: Rect, owns_keys: bool, keep: &[Rect]) {
    super::MARKS.with(|marks| {
        let mut m = marks.borrow_mut();
        if m.context.is_none() || area.is_empty() || m.frames.len() >= 32 {
            return;
        }
        let Some(key) = FRAME.with(|key| key.borrow().clone()) else {
            return;
        };
        let retained = m
            .frames
            .iter()
            .map(|frame| crate::ui::frame::perimeter(frame.area).len())
            .sum::<usize>();
        if key.slot.len() > 1024
            || keep.len() > 64
            || usize::from(area.width) * usize::from(area.height) > interaction::MAX_VIEWPORT_CELLS
            || retained.saturating_add(crate::ui::frame::perimeter(area).len())
                > interaction::MAX_VIEWPORT_CELLS
            || m.frames.iter().any(|frame| frame.key == key)
        {
            m.context = None;
            return;
        }
        m.frames.push(FrameMark {
            key,
            area,
            owns_keys,
            top_title: keep.iter().find(|r| r.y == area.y).copied(),
            keep: keep.to_vec(),
        });
    });
}
pub fn protect(area: Rect) {
    super::MARKS.with(|marks| {
        let mut m = marks.borrow_mut();
        for frame in &mut m.frames {
            if frame.area.intersects(area) {
                if frame.keep.len() >= 64 {
                    m.context = None;
                    return;
                }
                frame.keep.push(area);
            }
        }
    });
}
pub fn tree(
    pane: PaneId,
    slot: &str,
    area: Rect,
    rows: Vec<(Vec<String>, u16)>,
    open: Vec<Vec<String>>,
) {
    super::MARKS.with(|marks| {
        let mut m = marks.borrow_mut();
        let key = FrameKey {
            pane,
            slot: slot.into(),
        };
        let metadata: usize = m
            .trees
            .iter()
            .flat_map(|tree| {
                tree.rows
                    .iter()
                    .map(|(path, _)| path)
                    .chain(tree.open.iter())
            })
            .map(|path| path.iter().map(String::len).sum::<usize>())
            .sum();
        let added: usize = rows
            .iter()
            .map(|(path, _)| path)
            .chain(open.iter())
            .map(|path| path.iter().map(String::len).sum::<usize>())
            .sum();
        if slot.len() > 1024
            || rows.len() > usize::from(area.height)
            || open.len() > rows.len()
            || rows.iter().any(|(path, _)| !path_bounded(path))
            || open.iter().any(|path| !path_bounded(path))
            || metadata.saturating_add(added) > 65_536
            || m.trees.iter().any(|tree| tree.key == key)
        {
            m.context = None;
            return;
        }
        if m.context.is_some()
            && m.trees.len() < 16
            && usize::from(area.width) * usize::from(area.height) <= interaction::MAX_VIEWPORT_CELLS
        {
            m.trees.push(TreeMark {
                key,
                area,
                rows,
                open,
            });
        }
    });
}
#[derive(Clone)]
struct HeldTree {
    mark: TreeMark,
    cells: Rc<Vec<Cell>>,
}
#[derive(Clone)]
struct HeldFrame {
    mark: FrameMark,
    foregrounds: Vec<Color>,
}
struct Baseline {
    context: MotionContext,
    area: Rect,
    trees: Vec<HeldTree>,
    frames: Vec<HeldFrame>,
}
struct Folding {
    context: MotionContext,
    final_tree: HeldTree,
    area: Rect,
    expanded: Rc<Vec<Cell>>,
    compact: Rc<Vec<Cell>>,
    offset: usize,
    descendants: usize,
    direction: FoldDirection,
    started: u128,
}
struct Focusing {
    context: MotionContext,
    incoming: HeldFrame,
    outgoing: HeldFrame,
    started: u128,
}
#[derive(Default)]
pub(crate) struct FrameMotion {
    previous: Option<Baseline>,
    fold: Option<Folding>,
    focus: Option<Focusing>,
    armed: bool,
    settle: bool,
}
fn cells(buf: &Buffer, area: Rect) -> Option<Vec<Cell>> {
    if area.intersection(buf.area) != area {
        return None;
    }
    Some(
        (area.y..area.bottom())
            .flat_map(|y| (area.x..area.right()).map(move |x| buf[(x, y)].clone()))
            .collect(),
    )
}
impl FrameMotion {
    pub fn running(&self) -> bool {
        self.fold.is_some() || self.focus.is_some() || self.settle
    }
    pub fn background(&mut self) {
        self.armed = false;
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn input(&mut self) {
        self.settle = self.fold.is_some() || self.focus.is_some();
        self.fold = None;
        self.focus = None;
        self.armed = true;
    }
    pub fn draw(&mut self, buf: &mut Buffer, theme: &Theme, marks: &super::Marks, now: u128) {
        self.settle = false;
        if usize::from(buf.area.width) * usize::from(buf.area.height)
            > interaction::MAX_VIEWPORT_CELLS
        {
            self.clear();
            return;
        }
        let Some(context) = marks.context.clone() else {
            self.clear();
            return;
        };
        let mut budget = interaction::MAX_VIEWPORT_CELLS;
        let trees: Vec<HeldTree> = marks
            .trees
            .iter()
            .filter_map(|mark| {
                let count = usize::from(mark.area.width) * usize::from(mark.area.height);
                if count > budget {
                    return None;
                }
                budget -= count;
                let drawn = cells(buf, mark.area)?;
                let shared = self
                    .previous
                    .as_ref()
                    .and_then(|previous| {
                        previous
                            .trees
                            .iter()
                            .find(|tree| tree.mark == *mark && *tree.cells == drawn)
                    })
                    .map(|tree| Rc::clone(&tree.cells))
                    .unwrap_or_else(|| Rc::new(drawn));
                Some(HeldTree {
                    mark: mark.clone(),
                    cells: shared,
                })
            })
            .collect();
        let frames: Vec<HeldFrame> = marks
            .frames
            .iter()
            .filter_map(|mark| {
                Some(HeldFrame {
                    mark: mark.clone(),
                    foregrounds: interaction::border_foregrounds(buf, mark.border())?,
                })
            })
            .collect();
        let current = Baseline {
            context: context.clone(),
            area: buf.area,
            trees,
            frames,
        };
        if self
            .previous
            .as_ref()
            .is_some_and(|old| old.context != current.context || old.area != current.area)
        {
            self.fold = None;
            self.focus = None;
            self.armed = false;
            self.previous = Some(current);
            return;
        }
        if self.armed {
            if let Some(previous) = &self.previous
                && previous.context == current.context
                && previous.area == current.area
            {
                self.focus = focus_transition(previous, &current, now);
                self.fold = marks
                    .fold
                    .as_ref()
                    .and_then(|fold| fold_transition(previous, &current, fold, now));
            }
            self.armed = false;
        }
        if let Some(fold) = &self.fold {
            let matches = fold.context == context
                && current
                    .trees
                    .iter()
                    .any(|t| t.mark == fold.final_tree.mark && t.cells == fold.final_tree.cells);
            let elapsed = now.saturating_sub(fold.started).min(u128::from(u32::MAX)) as u32;
            if !matches || elapsed >= interaction::FOLD_MS {
                self.fold = None;
                self.settle = true;
            } else {
                interaction::compose_fold(
                    buf,
                    fold.final_tree.mark.area,
                    FoldRows {
                        area: fold.area,
                        expanded: &fold.expanded[fold.offset..],
                        compact: &fold.compact[fold.offset..],
                        descendants: fold.descendants,
                        direction: fold.direction,
                    },
                    elapsed,
                );
            }
        }
        if let Some(focus) = &self.focus {
            let incoming = current.frames.iter().find(|f| {
                f.mark.key == focus.incoming.mark.key
                    && f.mark.owns_keys
                    && f.mark.area == focus.incoming.mark.area
                    && f.mark.keep == focus.incoming.mark.keep
                    && f.mark.top_title == focus.incoming.mark.top_title
            });
            let outgoing = current.frames.iter().find(|f| {
                f.mark.key == focus.outgoing.mark.key
                    && !f.mark.owns_keys
                    && f.mark.area == focus.outgoing.mark.area
                    && f.mark.keep == focus.outgoing.mark.keep
                    && f.mark.top_title == focus.outgoing.mark.top_title
            });
            let elapsed = now.saturating_sub(focus.started).min(u128::from(u32::MAX)) as u32;
            if context != focus.context
                || elapsed >= interaction::FOCUS_MS
                || incoming.is_none()
                || outgoing.is_none()
            {
                self.focus = None;
                self.settle = true;
            } else if let (Some(incoming), Some(outgoing), Some(band)) =
                (incoming, outgoing, FocusBand::from_theme(theme))
            {
                interaction::focus_lap(
                    buf,
                    focus.incoming.mark.border(),
                    incoming.mark.border(),
                    &focus.incoming.foregrounds,
                    band,
                    elapsed,
                );
                interaction::focus_drain(
                    buf,
                    focus.outgoing.mark.border(),
                    outgoing.mark.border(),
                    &focus.outgoing.foregrounds,
                    elapsed,
                );
            } else {
                self.focus = None;
                self.settle = true;
            }
        }
        self.previous = Some(current);
    }
}
fn focus_transition(before: &Baseline, now: &Baseline, started: u128) -> Option<Focusing> {
    if before.frames.iter().filter(|f| f.mark.owns_keys).count() != 1
        || now.frames.iter().filter(|f| f.mark.owns_keys).count() != 1
    {
        return None;
    }
    let old = before.frames.iter().find(|f| f.mark.owns_keys)?;
    let new = now.frames.iter().find(|f| f.mark.owns_keys)?;
    if old.mark.key == new.mark.key {
        return None;
    }
    let incoming = before
        .frames
        .iter()
        .find(|f| f.mark.key == new.mark.key && !f.mark.owns_keys)?;
    let outgoing = now
        .frames
        .iter()
        .find(|f| f.mark.key == old.mark.key && !f.mark.owns_keys)?;
    if incoming.mark.area != new.mark.area
        || incoming.mark.keep != new.mark.keep
        || outgoing.mark.area != old.mark.area
        || outgoing.mark.keep != old.mark.keep
    {
        return None;
    }
    Some(Focusing {
        context: now.context.clone(),
        incoming: incoming.clone(),
        outgoing: old.clone(),
        started,
    })
}
fn fold_transition(
    before: &Baseline,
    now: &Baseline,
    cue: &MotionFold,
    started: u128,
) -> Option<Folding> {
    if cue.context != now.context || cue.before_open == cue.after_open {
        return None;
    }
    let key = FrameKey {
        pane: cue.pane,
        slot: cue.slot.clone(),
    };
    let old = before.trees.iter().find(|t| t.mark.key == key)?;
    let new = now.trees.iter().find(|t| t.mark.key == key)?;
    if old.mark.area != new.mark.area
        || old.mark.open.contains(&cue.path) != cue.before_open
        || new.mark.open.contains(&cue.path) != cue.after_open
    {
        return None;
    }
    let old_y = old.mark.rows.iter().find(|(path, _)| path == &cue.path)?.1;
    let new_y = new.mark.rows.iter().find(|(path, _)| path == &cue.path)?.1;
    if old_y != new_y {
        return None;
    }
    let (expanded, compact, direction) = if cue.after_open {
        (new, old, FoldDirection::Open)
    } else {
        (old, new, FoldDirection::Close)
    };
    let descendant =
        |path: &Vec<String>| path.len() > cue.path.len() && path.starts_with(&cue.path);
    let descendants = expanded
        .mark
        .rows
        .iter()
        .filter(|(path, _)| descendant(path))
        .count();
    if descendants == 0 {
        return None;
    }
    let filtered: Vec<_> = expanded
        .mark
        .rows
        .iter()
        .filter(|(path, _)| !descendant(path))
        .map(|(path, _)| path)
        .collect();
    let compact_paths: Vec<_> = compact.mark.rows.iter().map(|(path, _)| path).collect();
    if filtered.len() > compact_paths.len()
        || filtered.iter().zip(&compact_paths).any(|(a, b)| a != b)
    {
        return None;
    }
    let parent = expanded
        .mark
        .rows
        .iter()
        .position(|(path, _)| path == &cue.path)?;
    if expanded.mark.rows.iter().enumerate().any(|(i, (path, y))| {
        descendant(path) != (i > parent && i <= parent + descendants)
            || *y != expanded.mark.area.y.saturating_add(i as u16)
    }) || compact
        .mark
        .rows
        .iter()
        .enumerate()
        .any(|(i, (_, y))| *y != compact.mark.area.y.saturating_add(i as u16))
    {
        return None;
    }
    let area = Rect::new(
        new.mark.area.x,
        new_y.saturating_add(1),
        new.mark.area.width,
        new.mark
            .area
            .bottom()
            .saturating_sub(new_y.saturating_add(1)),
    );
    if area.is_empty() {
        return None;
    }
    let offset = usize::from(area.y - new.mark.area.y) * usize::from(area.width);
    Some(Folding {
        context: now.context.clone(),
        final_tree: new.clone(),
        area,
        expanded: Rc::clone(&expanded.cells),
        compact: Rc::clone(&compact.cells),
        offset,
        descendants,
        direction,
        started,
    })
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
