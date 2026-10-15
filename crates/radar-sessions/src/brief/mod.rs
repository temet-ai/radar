//! The opening prompt radar sends an agent session when an item is sent to it.
//!
//! It reads like one a person would type: the task first, in plain words (the
//! operator's request and the recipe's instructions), then what is known about
//! the item (title, link, fields, text) as headed sections. Nothing in it
//! comments on radar or on who wrote what. Two mechanical things are done to the
//! information: a block of text is fenced longer than any run of backticks
//! inside it so it cannot break the layout, and invisible or control
//! characters and token-shaped secrets are removed.
//!
//! The rules are written for people in `agents/briefing.md`; keep the two in
//! step.

pub mod recipe;
mod redact;

use std::fmt::Write as _;

use radar_core::agent_context::{
    AgentContext, ContextField, MAX_CONTEXT_BYTES, MAX_FIELD_BYTES, MAX_FIELDS, cap_text,
};

pub use recipe::{Recipe, RecipeError};
pub use redact::redact_secrets;

/// Version of the prompt layout; a saved review records it.
pub const TEMPLATE_VERSION: u32 = 2;

/// Most contexts one brief may carry.
pub const MAX_CONTEXTS: usize = 8;
/// Most bytes of the operator's task text.
pub const MAX_TASK_BYTES: usize = 16 * 1024;
/// Most bytes a built brief may have; contexts that no longer fit are left
/// out and named in [`Brief::omitted`].
pub const MAX_BRIEF_BYTES: usize = 96 * 1024;

/// What to build a brief from.
#[derive(Debug, Clone)]
pub struct BriefRequest<'a> {
    pub recipe: &'a Recipe,
    /// The operator's own words for this send.
    pub task: String,
    /// What the owning parts contributed, in the order to give them.
    pub contexts: Vec<AgentContext>,
}

/// A built brief. `text` is everything that will be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    pub recipe: String,
    pub recipe_version: u32,
    pub text: String,
    /// Contexts left out because the brief reached its cap (their titles).
    pub omitted: Vec<String>,
    /// Secrets found in the information and replaced by `[redacted]`.
    pub redactions: usize,
}

impl Brief {
    pub fn build(request: &BriefRequest<'_>) -> Brief {
        let recipe = request.recipe;
        let mut redactions = 0;
        let mut task = request.task.clone();
        cap_text(&mut task, MAX_TASK_BYTES);
        let (task, n) = clean(&task);
        redactions += n;

        let mut head = String::new();
        let task = task.trim();
        if !task.is_empty() {
            let _ = writeln!(head, "{task}\n");
        }
        let instructions = recipe.instructions.trim();
        if !instructions.is_empty() {
            let _ = writeln!(head, "{instructions}\n");
        }
        if !recipe.skills.is_empty() {
            let _ = writeln!(head, "Use these skills: {}\n", recipe.skills.join(", "));
        }

        let mut body = String::new();
        let mut omitted = vec![];
        for (i, context) in request.contexts.iter().enumerate() {
            if i >= MAX_CONTEXTS {
                omitted.push(context.title.clone());
                continue;
            }
            let (block, n) = render_context(context);
            if head.len() + body.len() + block.len() > MAX_BRIEF_BYTES {
                omitted.push(context.title.clone());
                continue;
            }
            redactions += n;
            body.push_str(&block);
        }
        if !omitted.is_empty() {
            let _ = writeln!(
                body,
                "Also attached, but too long to include: {}.\n",
                omitted.iter().map(|t| format!("\"{}\"", one_line(t))).collect::<Vec<_>>().join(", ")
            );
        }
        let mut text = head;
        text.push_str(&body);
        let text = text.trim_end().to_owned() + "\n";
        Brief { recipe: recipe.name.clone(), recipe_version: recipe.version, text, omitted, redactions }
    }
}

/// One item as a headed section: its kind and title, its link, then each field.
fn render_context(context: &AgentContext) -> (String, usize) {
    let mut redactions = 0;
    let mut out = String::new();
    let (title, n) = clean(&one_line(&context.title));
    redactions += n;
    let _ = writeln!(out, "## {}: {title}\n", sentence(&one_line(&context.kind)));
    if let Some(link) = &context.link {
        let _ = writeln!(out, "Link: {}", one_line(link));
    }
    let mut used = 0;
    for field in context.fields.iter().take(MAX_FIELDS) {
        let mut field: ContextField = field.clone();
        // Parts cap their own text; the builder does not take their word for it.
        if cap_text(&mut field.value, MAX_FIELD_BYTES) {
            field.truncated = true;
        }
        if used + field.name.len() + field.value.len() > MAX_CONTEXT_BYTES {
            continue;
        }
        used += field.name.len() + field.value.len();
        let (value, n) = clean(&field.value);
        redactions += n;
        if value.trim().is_empty() {
            continue;
        }
        let name = sentence(&one_line(&field.name).replace('_', " "));
        let cut = if field.truncated { " (cut short)" } else { "" };
        if value.contains('\n') {
            let _ = writeln!(out, "{name}{cut}:\n{}", fenced(&value));
        } else {
            let _ = writeln!(out, "{name}: {value}{cut}");
        }
    }
    out.push('\n');
    (out, redactions)
}

/// The first letter in capitals.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// `text` in a code fence longer than any run of backticks inside it, so
/// nothing in the text can close the fence early.
fn fenced(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{}\n{fence}", text.trim_end_matches('\n'))
}

/// Control characters other than newline and tab removed, secrets replaced;
/// the count of secrets.
fn clean(text: &str) -> (String, usize) {
    let kept: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .filter(|c| !is_invisible(*c))
        .collect();
    redact_secrets(&kept)
}

/// Zero-width and bidirectional controls: text nobody can see in the prompt.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
