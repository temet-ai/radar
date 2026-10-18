//! Renderer-neutral semantic identity and verified interaction evidence.
//! Geometry and retained cells remain local to each renderer.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::view::PaneId;

/// The direct pane tree's stable node slot.
pub const BODY_SLOT: &str = "body";
/// The host's settings navigation tree (not a plugin body).
pub const SETTINGS_NAV_SLOT: &str = "settings_nav";

/// Actual route identity, never a title, selection, or display label.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MotionContext {
    Screen {
        plugin: String,
        screen: String,
        page: Option<String>,
        depth: usize,
        tab: Option<usize>,
    },
    Settings {
        plugin: Option<String>,
        section: String,
    },
    History,
    Setup {
        step: String,
        plugin: Option<String>,
        section: Option<String>,
    },
}

/// One accepted, synchronously verified group-open change. A renderer must
/// still match this evidence against its prior clean context/tree/viewport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MotionFold {
    pub context: MotionContext,
    pub pane: PaneId,
    pub slot: String,
    /// Entire root-to-group path, including every ancestor id.
    pub path: Vec<String>,
    pub before_open: bool,
    pub after_open: bool,
    #[ts(type = "number | null")]
    pub origin_seq: Option<u64>,
    #[ts(type = "number | null")]
    pub origin_cursor: Option<u64>,
}

/// Acknowledgement is present even when the newest input changed nothing.
/// Otherwise an eventual background snapshot could look like its response.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
pub struct MotionInteraction {
    /// Unknown actual item pages deliberately have no context.
    pub context: Option<MotionContext>,
    #[ts(type = "number | null")]
    pub processed_seq: Option<u64>,
    /// Current scene still directly answers this input. Cleared by background
    /// host events and after delivery; processed_seq remains acknowledged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub origin_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold: Option<MotionFold>,
}
