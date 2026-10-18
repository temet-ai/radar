//! Shared detail identity, facts, content and pinned actions.
//!
//! The typed extension always carries a complete ordinary rich-text fallback.
//! Action targets are absent in owner data and belong to the displayed host lease.

use serde::{Deserialize, Serialize};

use super::{ActionSpec, Block, Ink, Node, PaneId, PaneView, Seg, Target, Text};

pub const CAPABILITY: &str = "detail";
pub const VERSION: u32 = 1;
/// Reserved for host admission, never forwarded as an ordinary owner target.
pub const TARGET_PREFIX: &str = "detail-foot:";
pub const MENU_ID: &str = "detail.extra_actions";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DetailHeader {
    pub path: Text,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<Text>,
}

impl DetailHeader {
    pub fn new(path: impl Into<Text>, title: impl Into<Text>) -> Self {
        Self { path: path.into(), title: title.into(), branch: None, author: None, age: None }
    }

    fn normalize(&mut self) {
        for value in [&mut self.branch, &mut self.author, &mut self.age] {
            if value.as_ref().is_some_and(blank) { *value = None; }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DetailFact {
    pub label: String,
    pub value: Text,
}

impl DetailFact {
    pub fn new(label: impl Into<String>, value: impl Into<Text>) -> Self {
        Self { label: label.into(), value: value.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DetailContent {
    pub header: DetailHeader,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<DetailFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pictures: Option<String>,
}

impl DetailContent {
    pub fn new(header: DetailHeader, facts: Vec<DetailFact>, sections: Vec<Block>) -> Self {
        let mut content = Self { header, facts, sections, pictures: None };
        content.normalize();
        content
    }

    fn normalize(&mut self) {
        self.header.normalize();
        self.facts.retain(|fact| !blank(&fact.value));
    }

    pub fn node(&self, foot: &FootActions) -> Node {
        let mut content = self.clone();
        content.normalize();
        Node::Extension {
            capability: CAPABILITY.into(),
            version: VERSION,
            data: serde_json::to_value(&content).expect("serializable detail data"),
            fallback: Box::new(Node::Rich {
                blocks: fallback_blocks(&content, foot),
                pictures: content.pictures.clone(),
            }),
        }
    }

    pub fn of(node: &Node) -> Option<Self> {
        match node {
            Node::Extension { capability, version: VERSION, data, .. } if capability == CAPABILITY => {
                let mut content: Self = serde_json::from_value(data.clone()).ok()?;
                content.normalize();
                Some(content)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct FootAction {
    pub action: ActionSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
}

impl FootAction {
    pub fn new(action: ActionSpec) -> Self {
        Self { action, disabled: None, target: None }
    }

    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.action.label = caption.into();
        let key = self.action.key.map(|key| key.name()).unwrap_or_default();
        self.action.mnemonic_at = crate::mnemonic::at(&key, &self.action.label);
        self.action.mnemonic_shift = self.action.mnemonic_at.is_some() && crate::mnemonic::shifted(&key);
        self
    }

    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled = Some(reason.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct FootActions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<FootAction>,
    pub menu: FootAction,
}

impl FootActions {
    pub fn new(actions: Vec<FootAction>) -> Self {
        Self {
            actions: actions.into_iter().filter(|item| !item.action.heading && !item.action.id.trim().is_empty()).collect(),
            menu: FootAction::new(ActionSpec::named(MENU_ID, &crate::Binding::new("x", "extra actions"))),
        }
    }
}

pub fn pane(title: impl Into<String>, content: DetailContent, foot: FootActions) -> PaneView {
    let mut pane = PaneView::new(PaneId::Detail, title, content.node(&foot));
    pane.foot_actions = Some(foot);
    pane
}

fn blank(text: &Text) -> bool {
    text.words().trim().is_empty()
}

/// Full identity and unshortened facts survive readers without this extension.
/// Keyed fallback rows are descriptive only: they contain no dispatch target.
fn fallback_blocks(content: &DetailContent, foot: &FootActions) -> Vec<Block> {
    let mut blocks = vec![Block::line(content.header.path.clone()), Block::line(content.header.title.clone())];
    for text in [&content.header.branch, &content.header.author, &content.header.age].into_iter().flatten() {
        blocks.push(Block::line(text.clone()));
    }
    for fact in &content.facts {
        let mut text = Text::of(format!("{}: ", fact.label), Ink::Muted);
        text.0.extend(fact.value.0.clone());
        blocks.push(Block::line(text));
    }
    blocks.extend(content.sections.clone());
    let actions = foot.actions.iter().chain(std::iter::once(&foot.menu)).map(|item| {
        let mut text = Text::plain(item.action.label.clone());
        if let Some(reason) = &item.disabled { text.add(Seg::new(format!(" ({reason})"), Ink::Muted)); }
        Block::Line {
            text,
            mnemonic_at: item.action.mnemonic_at,
            mnemonic_shift: item.action.mnemonic_shift,
            target: None,
            key: item.action.key.map(|key| key.name()),
        }
    }).collect();
    blocks.push(Block::section(None, "actions", None, actions));
    blocks
}

#[cfg(test)]
mod tests;
