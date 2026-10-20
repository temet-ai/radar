//! Local code in setup (`radar setup local`): git's step, on this machine.
//! Its card is the `checkouts` section, the rows Settings edits: the
//! folders radar looks in for checkouts, the folders in the home folder
//! found holding checkouts once you let setup look (⏎ looks in one), how
//! far down, the last look. Its sheet is the selected row's detail, then
//! what local code is and where it shows. The terminal is a section of its
//! own (`terminal`), not part of this step.

use radar_core::plugin::{SetupGroup, SetupStep};
use radar_core::ui::icon::Icon;
use radar_core::view::{Block, Ink, Text};

use crate::GitPlugin;

pub(crate) fn step(p: &GitPlugin) -> SetupStep {
    let step = SetupStep::new("local", "local code", "checkouts")
        .heading("Where your code lives on this machine.", "The folders radar looks in for Git checkouts.")
        .icon(Icon::Local)
        .about("checkouts, branches")
        .group(SetupGroup::Machine);
    match summary(p) {
        Some(s) => step.summary(s),
        None => step,
    }
}

/// `40 checkouts`, once a look found some; `2 folders` before.
pub(crate) fn summary(p: &GitPlugin) -> Option<String> {
    let plural = |n: usize, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    match (&p.local.index, p.cfg.roots.len()) {
        (_, 0) => None,
        (Some(index), _) if !index.checkouts.is_empty() => Some(plural(index.checkouts.len(), "checkout")),
        (_, n) => Some(plural(n, "folder")),
    }
}

fn line(words: &str) -> Block {
    Block::line(Text::of(words, Ink::Secondary))
}

/// What local code is, what radar does with it, and where it shows.
pub(crate) fn help() -> Vec<Block> {
    vec![
        Block::section(Some(Icon::Folder), "what it reads", None, vec![
            line("The git checkouts under these folders, as far down as look down."),
            line("New clones inside them show up at the next look."),
            line("With git on this machine: branches, changes, ahead, behind."),
        ]),
        Block::section(Some(Icon::Branch), "where it shows", None, vec![
            line("Branches (B): every branch in these checkouts."),
            line("A repo or merge request: where it is checked out, e and t."),
            line("Agents start in the checkout you pick."),
        ]),
        Block::section(Some(Icon::Settings), "change it later", None, vec![line("Settings / git / checkouts")]),
        Block::section(Some(Icon::Info), "skip it?", None, vec![
            line("Branches stay empty until a folder is set."),
            line("The services work without it."),
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_code_is_the_checkouts_section_on_this_machine() {
        let mut p = GitPlugin::new();
        let step = step(&p);
        assert_eq!((step.id, step.section, step.group), ("local", "checkouts", SetupGroup::Machine));
        assert_eq!(step.summary, None, "no folder yet");
        p.cfg.roots = vec!["~/ankh-morpork".into(), "~/watch".into()];
        assert_eq!(summary(&p), Some("2 folders".into()));
        p.local.index = Some(crate::local::Index::default());
        assert_eq!(summary(&p), Some("2 folders".into()), "a look that found none");
    }

    fn cx() -> radar_core::view::ViewCx<'static> {
        radar_core::view::ViewCx {
            now: 0,
            focus: radar_core::view::PaneId::Middle,
            width: 80,
            height: 24,
            parts: radar_core::compose::Parts::none(),
            icon_lead: 0,
            setup: false,
        }
    }

    /// Without a yes to setup's look nothing in the home folder is read:
    /// the card offers to add a folder, how far down, the last look; no
    /// terminal, no found folders.
    #[test]
    fn without_a_yes_the_card_offers_only_to_add_a_folder() {
        use radar_core::Plugin;
        let mut p = GitPlugin::new();
        p.tick(0);
        let rows = |p: &GitPlugin| match crate::settings::section_view(p, "checkouts", &cx()).unwrap().body {
            radar_core::view::Node::List { rows, .. } => rows.iter().map(|r| r.text.words()).collect::<Vec<_>>(),
            _ => panic!("a list"),
        };
        let words = rows(&p).join("\n");
        assert!(words.contains("add a folder") && !words.contains("found") && !words.contains("terminal"), "{words}");
        // The yes, under the sandbox: nothing is walked; a test gives the answer.
        p.setup_looked();
        assert!(p.tick(0).is_empty());
        assert!(!rows(&p).join("\n").contains("found"));
    }

    /// A folder found holding checkouts is offered, with how many, until it
    /// is looked in; ⏎ adds it to the roots and looks, and the sheet says
    /// what local code is.
    #[test]
    fn a_found_place_is_looked_in_with_enter() {
        use radar_core::view::{Chord, Key as KeyCode};
        let mut p = GitPlugin::new();
        crate::local::walked(&mut p, vec![("~/ankh-morpork".into(), 3), ("~/watch".into(), 1)]);
        p.local.rows[0] = 1;
        let view = crate::settings::section_view(&p, "checkouts", &cx()).unwrap();
        let body = format!("{:?}", view.body);
        assert!(body.contains("~/ankh-morpork") && body.contains("3 checkouts") && body.contains("1 checkout\""), "{body}");
        assert!(body.find("~/ankh-morpork") < body.find("~/watch"), "most first");
        assert!(format!("{:?}", view.detail_blocks).contains("WHAT IT READS"));
        crate::settings::key(&mut p, "checkouts", Chord::new(KeyCode::Enter));
        assert_eq!(p.cfg.roots, vec!["~/ankh-morpork".to_string()]);
        let view = crate::settings::section_view(&p, "checkouts", &cx()).unwrap();
        assert_eq!(format!("{:?}", view.body).matches("~/ankh-morpork").count(), 1, "offered no more");
    }
}
