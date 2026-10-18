//! One displayed preview lease. Raw views never allocate or retire targets.

use super::{Host, HostEffect, Input, Pane};
use crate::plugin::DetailIdentity;
use crate::view::detail::{FootActions, TARGET_PREFIX};
use crate::view::{ActionSpec, Intent, PaneId, ScreenView, Target};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Subject {
    owner: usize,
    scope: String,
    identity: DetailIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Offered {
    subject: Subject,
    foot: FootActions,
    /// Full, unshortened Detail action specs. Invalid or ambiguous slots are inert.
    canonical: Vec<Option<ActionSpec>>,
}

#[derive(Clone, Debug)]
struct Lease {
    serial: u64,
    offered: Offered,
}

#[derive(Default)]
pub(super) struct State {
    serial: u64,
    active: Option<Lease>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Action(usize),
    Menu,
}

fn parse(target: &str) -> Option<(u64, Slot)> {
    let (serial, slot) = target.strip_prefix(TARGET_PREFIX)?.split_once(':')?;
    let serial: u64 = serial.parse().ok()?;
    let slot = if slot == "menu" {
        Slot::Menu
    } else {
        Slot::Action(slot.parse().ok()?)
    };
    // Accept only the exact emitted spelling, including a nonzero serial.
    (serial != 0 && target == target_of(serial, slot).as_str()).then_some((serial, slot))
}

fn target_of(serial: u64, slot: Slot) -> Target {
    Target::new(match slot {
        Slot::Action(index) => format!("{TARGET_PREFIX}{serial}:{index}"),
        Slot::Menu => format!("{TARGET_PREFIX}{serial}:menu"),
    })
}

fn clear_foot(foot: &mut FootActions) {
    for action in &mut foot.actions {
        action.target = None;
    }
    foot.menu.target = None;
}

fn clear_targets(view: &mut ScreenView) {
    for pane in view
        .panes
        .iter_mut()
        .chain(view.page.iter_mut().flat_map(|page| page.panes.iter_mut()))
        .chain(view.overlay.iter_mut().map(|pane| &mut **pane))
    {
        if let Some(foot) = pane.foot_actions.as_mut() {
            clear_foot(foot);
        }
    }
}

fn offered(subject: Subject, view: &ScreenView) -> Option<Offered> {
    if view.scope != subject.scope || view.page.is_some() || view.overlay.is_some() {
        return None;
    }
    let mut panes = view.panes.iter().filter(|pane| pane.pane == PaneId::Detail);
    let mut foot = panes.next()?.foot_actions.clone()?;
    if panes.next().is_some() {
        return None;
    }
    clear_foot(&mut foot);
    // This is a kit control, never an owner-supplied command in disguise.
    if foot.menu != FootActions::new(vec![]).menu {
        return None;
    }
    let canonical = foot
        .actions
        .iter()
        .map(|control| {
            let action = &control.action;
            if action.heading || action.id.is_empty() {
                return None;
            }
            let mut matches = view.actions.iter().filter(|candidate| {
                !candidate.heading && !candidate.id.is_empty() && candidate.id == action.id
            });
            let candidate = matches.next()?;
            (matches.next().is_none()
                && candidate.key == action.key
                && candidate.change == action.change)
                .then(|| candidate.clone())
        })
        .collect();
    Some(Offered {
        subject,
        foot,
        canonical,
    })
}

impl State {
    fn reconcile(&mut self, subject: Option<&Subject>) {
        if self
            .active
            .as_ref()
            .is_some_and(|lease| Some(&lease.offered.subject) != subject)
        {
            self.active = None;
        }
    }

    fn project(&mut self, offered: Offered) -> Option<&Lease> {
        if self
            .active
            .as_ref()
            .is_none_or(|lease| lease.offered != offered)
        {
            self.active = None;
            self.serial = self.serial.checked_add(1)?;
            self.active = Some(Lease {
                serial: self.serial,
                offered,
            });
        }
        self.active.as_ref()
    }
}

impl Host {
    /// Cheap event-epilogue check, including changes while no renderer reads a view.
    pub(super) fn reconcile_detail_lease(&self) {
        let subject = self.detail_subject();
        self.detail_lease.borrow_mut().reconcile(subject.as_ref());
    }

    fn detail_subject(&self) -> Option<Subject> {
        if self.menu.is_some()
            || self.palette.is_some()
            || self.history.is_some()
            || self.notice_history.is_some()
            || self.setup.is_some()
            || self.capturing()
        {
            return None;
        }
        let (owner, spec) = self.screen()?;
        if !self.ready(owner) || self.plugins[owner].depth(spec.id) != 0 {
            return None;
        }
        Some(Subject {
            owner,
            scope: spec.id.to_owned(),
            identity: self.plugins[owner].detail_identity(spec.id, Pane::Detail)?,
        })
    }

    fn detail_view(&self, subject: &Subject, width: u16, height: u16) -> Option<ScreenView> {
        let mut cx = self.view_cx(subject.owner, width, height);
        cx.focus = PaneId::Detail;
        self.plugins[subject.owner].view(&subject.scope, &cx)
    }

    /// An explicit displayed projection. Introspection must keep using raw views.
    pub fn displayed_screen_view_in(
        &self,
        owner: usize,
        scope: &str,
        width: u16,
        height: u16,
    ) -> Option<ScreenView> {
        let Some(mut view) = self.screen_view_in(owner, scope, width, height) else {
            if self
                .screen()
                .is_some_and(|(p, spec)| p == owner && spec.id == scope)
            {
                self.detail_lease.borrow_mut().active = None;
            }
            return None;
        };
        clear_targets(&mut view);
        // Reading another registered scope cannot disturb the displayed lease.
        if self
            .screen()
            .is_none_or(|(p, spec)| p != owner || spec.id != scope)
        {
            return Some(view);
        }
        self.frontend.set_room(width, height);
        self.project_displayed_detail(&mut view, width, height);
        view.dock = self.dock_view();
        Some(view)
    }

    /// The actual screen, sized after host chrome, with live controls attached.
    /// General fixtures intentionally use `view::screen_showing` instead.
    pub fn displayed_screen(&self, width: u16, height: u16) -> Option<ScreenView> {
        let body = crate::shell::body_of(self, width, height);
        self.frontend.set_room(body.width, body.height);
        let Some((owner, spec)) = self.screen() else {
            self.reconcile_detail_lease();
            return None;
        };
        if self.setting_up().is_some() {
            self.reconcile_detail_lease();
            return crate::view::screen_showing(self, body.width, body.height);
        }
        self.displayed_screen_view_in(owner, spec.id, body.width, body.height)
    }

    fn project_displayed_detail(&self, view: &mut ScreenView, width: u16, height: u16) {
        self.reconcile_detail_lease();
        let Some(subject) = self.detail_subject() else {
            return;
        };
        let Some(canonical) = self.detail_view(&subject, width, height) else {
            self.detail_lease.borrow_mut().active = None;
            return;
        };
        let Some(candidate) = offered(subject.clone(), &canonical) else {
            self.detail_lease.borrow_mut().active = None;
            return;
        };
        // The displayed footer must be the same description as the Detail-focused one.
        let Some(pane) = view
            .panes
            .iter_mut()
            .find(|pane| pane.pane == PaneId::Detail)
        else {
            return;
        };
        if view.page.is_some()
            || view.overlay.is_some()
            || pane.foot_actions.as_ref() != Some(&candidate.foot)
            || self.detail_subject().as_ref() != Some(&subject)
        {
            self.detail_lease.borrow_mut().active = None;
            return;
        }
        let mut state = self.detail_lease.borrow_mut();
        let Some(lease) = state.project(candidate) else {
            return;
        };
        let foot = pane.foot_actions.as_mut().expect("validated footer");
        for (index, control) in foot.actions.iter_mut().enumerate() {
            if control.disabled.is_none() && lease.offered.canonical[index].is_some() {
                control.target = Some(target_of(lease.serial, Slot::Action(index)));
            }
        }
        foot.menu.target = Some(target_of(lease.serial, Slot::Menu));
    }

    /// Only an explicit activation of an exactly current displayed control can run.
    /// Focus changes happen after all admission checks, under the normal input route.
    pub(super) fn activate_detail_control(&mut self, target: &Target) -> Vec<HostEffect> {
        // Physical-Down capture arrives as an intent, bypassing on_mouse.
        self.last_press = None;
        let Some((serial, slot)) = parse(target.as_str()) else {
            return vec![];
        };
        self.reconcile_detail_lease();
        let Some(lease) = self
            .detail_lease
            .borrow()
            .active
            .clone()
            .filter(|lease| lease.serial == serial)
        else {
            return vec![];
        };
        let Some(subject) = self.detail_subject() else {
            return vec![];
        };
        let (width, height) = self.frontend.room();
        let fresh = self
            .detail_view(&subject, width, height)
            .and_then(|view| offered(subject.clone(), &view));
        if fresh.as_ref() != Some(&lease.offered)
            || self.detail_subject().as_ref() != Some(&subject)
        {
            self.detail_lease.borrow_mut().active = None;
            return vec![];
        }
        let action = match slot {
            Slot::Action(index) => {
                let Some(control) = lease.offered.foot.actions.get(index) else {
                    return vec![];
                };
                let Some(canonical) = lease.offered.canonical.get(index).and_then(Option::as_ref)
                else {
                    return vec![];
                };
                if control.disabled.is_some() {
                    return vec![];
                }
                Some(canonical.id.clone())
            }
            Slot::Menu => None,
        };
        self.focus = Pane::Detail;
        self.bar = false;
        self.sources = false;
        self.tabs = false;
        self.clear_message_compat();
        match action {
            Some(action) => self
                .deliver_input(Input::Intent(Intent::Invoke { action }))
                .unwrap_or_default(),
            None => {
                self.open_keys_menu();
                vec![]
            }
        }
    }
}

#[cfg(test)]
#[path = "detail_actions_tests.rs"]
mod tests;
