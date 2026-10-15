//! Go to (`:`): type part of a tenancy's code, a repo's name, or a merge
//! request's title or number, and the tree's selection jumps to the best
//! match as you type, opening what it is inside. `⏎` keeps it there, `esc`
//! goes back to where you were. Unlike `/` it hides nothing.

use radar_core::design::help_bar::KeyHint;
use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Effect, Outcome};
use radar_core::ratatui::crossterm::event::KeyEvent;
use radar_core::log;

use crate::{GitlabPlugin, screen};

#[derive(Debug)]
pub(crate) struct Goto {
    pub input: TextInput,
    /// Where the tree's selection was, for `esc`.
    pub before: Vec<String>,
    /// What the selection jumped to, in words; `None` before typing or with
    /// no match.
    pub found: Option<String>,
}

/// One row of the tree it can go to: its path in the tree, what it is
/// called (for the line under the input), and the words it answers to.
pub(crate) struct Target {
    pub path: Vec<String>,
    pub name: String,
    pub words: Vec<String>,
}

/// How well `query` names something called `words`: an exact word beats
/// the start of one, which beats the start of a part of one (after `-`,
/// `/`, a space…), which beats anywhere inside. 0 is no match.
pub(crate) fn score(query: &str, words: &[String]) -> u32 {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 0;
    }
    words
        .iter()
        .map(|w| {
            let w = w.to_lowercase();
            if w == q {
                100
            } else if w.starts_with(&q) {
                80
            } else if w
                .split(|c: char| !c.is_alphanumeric())
                .any(|part| !part.is_empty() && part.starts_with(&q))
            {
                60
            } else if w.contains(&q) {
                40
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0)
}

/// The best match for `query` among `targets`; the first in the tree wins a
/// tie, so a tenancy comes before its repos and a repo before its merge
/// requests.
pub(crate) fn best(query: &str, targets: &[Target]) -> Option<usize> {
    let mut found: Option<(usize, u32)> = None;
    for (i, t) in targets.iter().enumerate() {
        let s = score(query, &t.words);
        if s > 0 && found.is_none_or(|(_, b)| s > b) {
            found = Some((i, s));
        }
    }
    found.map(|(i, _)| i)
}

pub(crate) fn start(p: &mut GitlabPlugin) {
    p.goto = Some(Goto {
        input: TextInput::new(""),
        before: p.tree.borrow().selected().to_vec(),
        found: None,
    });
}

/// Move the tree's selection to the best match for what is typed, opening
/// the tenancy and repo it is in; back to where it was when nothing matches.
fn jump(p: &GitlabPlugin, g: &mut Goto) {
    let targets = screen::targets(p);
    let query = g.input.value();
    let mut state = p.tree.borrow_mut();
    match best(&query, &targets) {
        Some(i) => {
            let path = targets[i].path.clone();
            for n in 1..path.len() {
                state.open(path[..n].to_vec());
            }
            state.select(path);
            g.found = Some(targets[i].name.clone());
        }
        None => {
            state.select(g.before.clone());
            g.found = None;
        }
    }
}

pub(crate) fn key(p: &mut GitlabPlugin, key: KeyEvent) -> Outcome {
    let Some(mut g) = p.goto.take() else {
        return Outcome::Ignored;
    };
    match g.input.key(key) {
        Edit::Cancel => {
            p.tree.borrow_mut().select(g.before);
            Outcome::done()
        }
        Edit::Submit(text) => {
            log::debug(|| format!("gitlab go to {text:?}: {:?}", g.found));
            let fx: Vec<Effect> = screen::moved(p, &g.before);
            Outcome::Handled(fx)
        }
        Edit::Typing => {
            jump(p, &mut g);
            p.goto = Some(g);
            Outcome::done()
        }
    }
}

pub(crate) fn paste(p: &mut GitlabPlugin, text: &str) {
    if let Some(mut g) = p.goto.take() {
        g.input.paste(text);
        jump(p, &mut g);
        p.goto = Some(g);
    }
}

pub(crate) fn hints() -> Vec<KeyHint<'static>> {
    vec![KeyHint::new("⏎", "stay here"), KeyHint::new("esc", "back")]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::sample;
    use radar_core::ratatui::crossterm::event::{KeyCode, KeyModifiers};

    fn t(path: &str, words: &[&str]) -> Target {
        Target {
            path: vec![path.into()],
            name: path.into(),
            words: words.iter().map(|w| w.to_string()).collect(),
        }
    }

    #[test]
    fn exact_beats_start_beats_part_beats_inside() {
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(score("api", &w(&["api"])), 100);
        assert_eq!(score("ap", &w(&["api"])), 80);
        assert_eq!(score("stat", &w(&["add a stats endpoint"])), 60);
        assert_eq!(score("tats", &w(&["add a stats endpoint"])), 40);
        assert_eq!(score("zzz", &w(&["api"])), 0);
        assert_eq!(score("  ", &w(&["api"])), 0);
        assert_eq!(score("API", &w(&["api"])), 100, "case does not matter");
    }

    #[test]
    fn the_best_wins_and_the_first_in_the_tree_wins_a_tie() {
        let targets = [
            t("t:team-a", &["a", "team-a"]),
            t("r:api", &["api", "acme/team-a/api"]),
            t("m:263", &["!263", "263", "ABC-12: Add a stats endpoint"]),
            t("m:5", &["!5", "5", "api docs"]),
        ];
        assert_eq!(best("api", &targets), Some(1), "the repo named api");
        assert_eq!(best("263", &targets), Some(2));
        assert_eq!(best("!263", &targets), Some(2));
        assert_eq!(best("a", &targets), Some(0), "the tenancy code");
        assert_eq!(best("endpoint", &targets), Some(2));
        assert_eq!(best("nothing", &targets), None);
    }

    fn press(p: &mut GitlabPlugin, code: KeyCode) {
        key(p, KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn draw(p: &GitlabPlugin) {
        let theme = radar_core::Theme::new(radar_core::design::theme::Tier::Mono);
        let (cursor, hits) = (std::cell::Cell::new(None), std::cell::RefCell::new(vec![]));
        let cx = radar_core::view_tui::Cx {
            theme: &theme,
            now: p.now,
            focused: true,
            focus: radar_core::Pane::Middle,
            bar: false,
            tabs: false,
            cursor: &cursor,
            hits: &hits,
            parts: radar_core::Parts::none(),
        };
        let area = radar_core::ratatui::layout::Rect::new(0, 0, 160, 40);
        p.render("repos", area, &mut radar_core::ratatui::buffer::Buffer::empty(area), &cx);
    }

    #[test]
    fn typing_jumps_live_enter_keeps_and_esc_goes_back() {
        let mut p = sample();
        draw(&p);
        let start = p.tree.borrow().selected().to_vec();
        start_and_type(&mut p, "263");
        let at = p.tree.borrow().selected().to_vec();
        assert_eq!(at.last().map(String::as_str), Some("m:acme/team-a/api!263"));
        assert_eq!(at.len(), 3, "inside its tenancy and repo, which are opened");
        assert!(
            p.goto
                .as_ref()
                .unwrap()
                .found
                .as_deref()
                .unwrap()
                .contains("!263")
        );
        press(&mut p, KeyCode::Esc);
        assert!(p.goto.is_none());
        assert_eq!(p.tree.borrow().selected(), start, "back where it was");

        start_and_type(&mut p, "web");
        press(&mut p, KeyCode::Enter);
        assert!(p.goto.is_none());
        assert_eq!(
            p.tree.borrow().selected().last().map(String::as_str),
            Some("r:acme/team-a/web")
        );
        // It hides nothing.
        assert!(p.filter.is_empty());
        assert_eq!(p.visible().len(), 4);
    }

    #[test]
    fn no_match_puts_the_selection_back_until_something_matches() {
        let mut p = sample();
        draw(&p);
        let start = p.tree.borrow().selected().to_vec();
        start_and_type(&mut p, "zzz");
        assert_eq!(p.tree.borrow().selected(), start);
        assert_eq!(p.goto.as_ref().unwrap().found, None);
    }

    fn start_and_type(p: &mut GitlabPlugin, text: &str) {
        start(p);
        for c in text.chars() {
            press(p, KeyCode::Char(c));
        }
    }
}
