//! Ratatui's wrap as goldens for Ink's wrap (T37): radar-core's rich document
//! paragraph wrap (through `rich::plain` on a Doc of one paragraph at room R)
//! and `ui::wrap_words`.
//!
//! RADAR_WRAP_GOLDENS_WRITE=1 CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar --test wrap_goldens
//!
//! With RADAR_WRAP_GOLDENS_WRITE set the test writes ink/test/fixtures/wrap/golden.json;
//! without it the committed file must equal what Ratatui's wrap gives now.
//! Entries: {fn: "rich", text, room, lines} and {fn: "words", text, width, prefix, lines}.

use radar_core::ui::rich::{plain, Block, Doc, Inline};
use radar_core::ui::wrap_words;
use serde_json::{json, Value};
use std::path::PathBuf;

fn rich(text: &str, room: u16) -> Value {
    let doc = Doc::new(vec![Block::para(vec![Inline::text(text)])]);
    let lines: Vec<String> = plain(&doc, room).split('\n').map(String::from).collect();
    json!({"fn": "rich", "text": text, "room": room, "lines": lines})
}

fn words(text: &str, width: u16, prefix: usize) -> Value {
    let lines = wrap_words(text, width, prefix);
    json!({"fn": "words", "text": text, "width": width, "prefix": prefix, "lines": lines})
}

fn goldens() -> Vec<Value> {
    let mut v = Vec::new();
    for room in [5, 6, 7, 8, 9, 10, 12, 13, 44] {
        v.push(rich("alpha beta gamma delta epsilon zeta", room));
    }
    v.push(rich(
        "Responses stay with the repository and commit they were requested for.",
        44,
    ));
    v.push(rich("ab abcdefghijkl cd", 5));
    v.push(rich("漢字漢字 abc", 6));
    v.push(rich("😀😀 hi", 4));
    v.push(rich("a  b   c", 80));
    v.push(rich("a  b   c", 3));
    v.push(rich("   hi  ", 80));
    v.push(words("one  two   three four", 9, 0));
    v.push(words("aaaa bbbb", 8, 0));
    v.push(words("aaaa bbb", 8, 0));
    v.push(words("aaaa bbbb cccc", 3, 0));
    v.push(words("aaaa bbb ccc", 12, 4));
    v.push(words("abcdefghijkl x", 8, 0));
    v.push(words("a\n\nb\n", 20, 0));
    for w in [14, 16, 17] {
        v.push(words("漢字漢 漢字漢", w, 0));
        v.push(words("漢字漢字 漢字漢字", w, 0));
    }
    v
}

#[test]
fn ink_wrap_goldens_are_ratatui_wrap() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ink/test/fixtures/wrap/golden.json");
    let now = Value::Array(goldens());
    if std::env::var_os("RADAR_WRAP_GOLDENS_WRITE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path).expect("golden.json; write it with RADAR_WRAP_GOLDENS_WRITE=1");
    let committed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(committed, now, "Ratatui's wrap changed; rewrite with RADAR_WRAP_GOLDENS_WRITE=1");
}
