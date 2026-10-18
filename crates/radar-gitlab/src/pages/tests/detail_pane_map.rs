//! One held Changes page verifies responsive routing after content disappears.
use super::*;

fn project(p: &GitlabPlugin, width: u16) -> Vec<radar_core::view::PaneId> {
    let cx = radar_core::view::ViewCx {
        now: p.now,
        focus: radar_core::view::PaneId::Middle,
        width,
        height: 40,
        parts: radar_core::compose::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    let view = p.view("repos", &cx).expect("Changes view");
    view.page.expect("Changes page").panes.iter().map(|pane| pane.pane).collect()
}

#[test]
fn loading_and_empty_changes_retire_the_previous_file_pane_jump() {
    use radar_core::view::PaneId;
    let mut p = changes_page();
    assert_eq!(project(&p, 160), [PaneId::Nav, PaneId::Middle]);
    assert_eq!(panes(&p), [Pane::Nav, Pane::Middle]);
    let Kind::Mr { project: path, iid } = &p.pages.last().expect("held MR page").kind else {
        panic!("Changes fixture is an MR page");
    };
    let key = mr_key(path, *iid);
    let held = p.mr_pages.get_mut(&key).unwrap().value.take().unwrap();
    assert_eq!(project(&p, 160), [PaneId::Middle]);
    assert_eq!(panes(&p), [Pane::Middle], "loading has no file-pane jump");
    p.mr_pages.get_mut(&key).unwrap().value = Some(held);
    project(&p, 160);
    assert_eq!(panes(&p), [Pane::Nav, Pane::Middle]);
    let files = std::mem::take(&mut p.mr_pages.get_mut(&key).unwrap().value.as_mut().unwrap().1.files);
    assert_eq!(project(&p, 160), [PaneId::Middle]);
    assert_eq!(panes(&p), [Pane::Middle], "empty has no file-pane jump");
    p.mr_pages.get_mut(&key).unwrap().value.as_mut().unwrap().1.files = files;
    project(&p, 80);
    assert_eq!(panes(&p), [Pane::Middle], "narrow keeps only its actual logical pane");
    project(&p, 160);
    assert_eq!(panes(&p), [Pane::Nav, Pane::Middle]);
}
