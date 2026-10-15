//! A job's page: its log, fetched when the page opens (the last part first;
//! earlier parts as you reach the top), its sections folded as GitLab folds
//! them, `/` to search it, `n`/`N` from match to match.

use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Outcome};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::Page;
use crate::model::JobInfo;
use crate::log_ownership::JobKey;
use crate::{GitlabPlugin, why};

pub(super) fn info<'a>(p: &'a GitlabPlugin, project: &str, id: u64) -> Option<&'a JobInfo> {
    p.jobs.get(&JobKey::new(project, id))
}

/// What the page is called: the job's name, or its number until it is known.
pub(super) fn name(p: &GitlabPlugin, project: &str, id: u64) -> String {
    info(p, project, id).map_or_else(|| format!("job #{id}"), |j| j.name.clone())
}

/// `Log`, or `Log · last 256 KB of 1.2 MB` while earlier parts are not here.
pub(super) fn tabs(p: &GitlabPlugin, project: &str, id: u64) -> Vec<String> {
    match p.logs.get(&JobKey::new(project, id)).and_then(|f| f.value.as_ref()) {
        Some(l) if l.earlier() => vec![format!(
            "Log · last {} of {}",
            size(l.total - l.start),
            size(l.total)
        )],
        _ => vec!["Log".into()],
    }
}

pub(super) fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} B"),
    }
}

/// Open the log at the first error, else at its end.
pub(super) fn opened(p: &GitlabPlugin, page: &Page, project: &str, id: u64) {
    if let Some(log) = p.logs.get(&JobKey::new(project, id)).and_then(|f| f.value.as_ref())
        && let Some(line) = log.excerpt.as_ref().and_then(|e| e.error)
    {
        page.log.goto(&log.text, line);
    }
}

/// A key on a job's page; `None` leaves it to the page's own keys.
pub(super) fn key(p: &mut GitlabPlugin, project: &str, id: u64, key: KeyEvent) -> Option<Outcome> {
    if let Some(mut input) = p.log_search.take() {
        let page = p.pages.last()?;
        let text = p.logs.get(&JobKey::new(project, id)).and_then(|f| f.value.as_ref()).map(|l| &l.text);
        return Some(match input.key(key) {
            Edit::Cancel => Outcome::done(),
            Edit::Submit(q) if q.is_empty() => {
                page.log.clear_search();
                Outcome::done()
            }
            Edit::Submit(q) => {
                let n = text.map_or(0, |t| page.log.search(t, &q));
                let say = match n {
                    0 => format!("no line matches {q}"),
                    1 => "1 line matches".to_string(),
                    n => format!("{n} lines match · n and N go between them"),
                };
                Outcome::Handled(vec![Effect::Notify(say)])
            }
            Edit::Typing => {
                p.log_search = Some(input);
                Outcome::done()
            }
        });
    }
    let page = p.pages.last()?;
    let log = p.logs.get(&JobKey::new(project, id)).and_then(|f| f.value.as_ref());
    match key.code {
        KeyCode::Char('/') => {
            p.log_search = Some(TextInput::new(&page.log.query()));
            Some(Outcome::done())
        }
        KeyCode::Esc if !page.log.query().is_empty() => {
            page.log.clear_search();
            Some(Outcome::done())
        }
        KeyCode::Char('c') => Some(Outcome::Handled(match why::excerpt_text(p, project, id) {
            Some(text) => vec![
                Effect::Copy(text),
                Effect::Notify(format!("copied the lines of {}'s log that say why", name(p, project, id))),
            ],
            None => vec![Effect::Notify("no excerpt yet: its log has not arrived".into())],
        })),
        _ => {
            let log = log?;
            if !page.log.key(&log.text, &key) {
                return None;
            }
            // Reaching the top asks for the part before.
            let up = matches!(
                key.code,
                KeyCode::Up | KeyCode::Char('k' | 'g') | KeyCode::PageUp | KeyCode::Home
            );
            let fx = if up && page.log.at_top(&log.text) {
                why::earlier(p, project, id)
            } else {
                vec![]
            };
            Some(Outcome::Handled(fx))
        }
    }
}

/// The job page's own rows in `x`: under `log`.
pub(super) fn bindings(p: &GitlabPlugin, page: &Page, project: &str, id: u64) -> Vec<crate::Row> {
    use crate::{act, head};
    let mut b = Vec::new();
    let log = p.logs.get(&JobKey::new(project, id)).and_then(|f| f.value.as_ref());
    match log.and_then(|l| l.excerpt.as_ref()) {
        Some(e) if e.error.is_some() => b.push(act("gitlab.job.copy_lines", Binding::new("c", "copy the lines around its first error"))),
        Some(_) => b.push(act("gitlab.job.copy_lines", Binding::new("c", "copy its last lines"))),
        None => {}
    }
    let Some(log) = log else {
        return b;
    };
    b.push(head(Binding::heading("log")));
    b.push(act("gitlab.job.search_log", Binding::new("/", "search the log…")));
    if !page.log.query().is_empty() {
        b.push(act("gitlab.job.next_match", Binding::new("n", format!("next line with {}", page.log.query()))));
        b.push(act("gitlab.job.previous_match", Binding::new("N", format!("previous line with {}", page.log.query()))));
    }
    if !log.text.sections.is_empty() {
        b.push(act("gitlab.job.fold_section", Binding::new("space", "fold or open the section the cursor is in")));
    }
    if log.earlier() {
        b.push(act("gitlab.job.load_earlier", Binding::new("g", "go to the start, loading the part before")));
    }
    b
}

/// Where `esc` goes back to, said in the menu.
pub(super) fn esc_words(page: &Page) -> Option<&'static str> {
    (!page.log.query().is_empty()).then_some("clear the search")
}

pub(super) fn click(p: &GitlabPlugin, page: &Page, project: &str, id: u64, row: usize) {
    if !page.log_painted.get() { return; }
    if let Some(log) = p.logs.get(&JobKey::new(project, id)).and_then(|f| f.value.as_ref()) {
        page.log.click(&log.text, row);
    }
}
