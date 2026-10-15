# Ratatui reference for the cell-parity test

`parity.test.tsx` draws `<screen>_<w>x<h>.json` (the view data the host sends: chrome, screen, focus) through the whole Ink App and compares every cell with `<screen>_<w>x<h>.cells.json` (the Ratatui buffer for the same host state at truecolor, icons off: `[symbol, fg, bg, modifier bits]`).

Both are written from the demo host by a scratch Rust test, kept out of the tree on purpose (it is a generator, not a check). To regenerate, put this at `tests/zz_t3_styles.rs`, run it, and delete it:

    RADAR_T3_DIR=$PWD/ink/test/text/look CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar --test zz_t3_styles

```rust
//! Scratch (T3), not committed: ink/test/text/look/<name>_<w>x<h>.{json,cells.json}.
use radar_core::headless::Headless;
use radar_core::host::View;
use radar_core::ratatui::{Terminal, backend::TestBackend, style::Color};
use radar_core::view::{Intent, Target};
use serde_json::{Value, json};

fn targets(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == "target" {
                    if let Some(s) = x.as_str() {
                        out.push(s.to_string());
                    }
                }
                targets(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| targets(x, out)),
        _ => {}
    }
}

fn has_table(v: &Value) -> bool {
    let t = v.to_string();
    t.contains("\"node\":\"table\"") && t.contains("\"rows\":[{")
}

fn col(c: Color) -> Value {
    match c {
        Color::Reset => Value::Null,
        Color::Rgb(r, g, b) => json!(format!("#{r:02x}{g:02x}{b:02x}")),
        o => json!(format!("{o:?}")),
    }
}

fn dump(name: &str, dir: &str, mut head: Headless, w: u16, h: u16) {
    let theme = head.host().build_theme(radar_core::design::theme::Tier::TrueColor);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| radar_core::shell::render(head.host(), &theme, f)).unwrap();
    let rows: Vec<Value> = (0..h)
        .map(|y| {
            Value::Array(
                (0..w)
                    .map(|x| {
                        let c = &term.backend().buffer()[(x, y)];
                        json!([c.symbol(), col(c.fg), col(c.bg), c.modifier.bits()])
                    })
                    .collect(),
            )
        })
        .collect();
    let chrome = serde_json::to_value(head.chrome()).unwrap();
    let screen = serde_json::to_value(head.screen()).unwrap();
    let focus = serde_json::to_value(radar_core::view::PaneId::from(head.host().focus)).unwrap();
    let v = json!({"width": w, "height": h, "chrome": chrome, "screen": screen, "focus": focus});
    std::fs::write(format!("{dir}/{name}_{w}x{h}.json"), serde_json::to_string(&v).unwrap()).unwrap();
    std::fs::write(format!("{dir}/{name}_{w}x{h}.cells.json"), serde_json::to_string(&rows).unwrap()).unwrap();
}

#[test]
fn write() {
    let Ok(dir) = std::env::var("RADAR_T3_DIR") else { return };
    for (w, h) in [(80u16, 24u16), (160, 40)] {
        let mk = || {
            let mut host = radar_core::shot::demo_host(radar::parts()).unwrap();
            host.view = View::Screen(0);
            Headless::new(host, w, h)
        };
        dump("tree", &dir, mk(), w, h);
        let n = mk().screen_count();
        let mut found = None;
        'all: for i in 0..n {
            let mut probe = mk();
            for page in probe.reach_pages(i, 12, 2) {
                if !has_table(&serde_json::to_value(&page.view).unwrap()) {
                    continue;
                }
                let mut head = mk();
                head.show_screen(i);
                for t in &page.via {
                    let before = serde_json::to_string(&head.screen()).unwrap();
                    for pane in [radar_core::view::PaneId::Nav, radar_core::view::PaneId::Middle, radar_core::view::PaneId::Detail] {
                        head.focus(pane);
                        head.send(Intent::Select { target: t.clone() });
                        head.send(Intent::Activate { target: t.clone() });
                        if serde_json::to_string(&head.screen()).unwrap() != before {
                            break;
                        }
                    }
                }
                if head.screen().is_some_and(|s| has_table(&serde_json::to_value(s).unwrap())) {
                    found = Some(head);
                    break 'all;
                }
            }
        }
        let head = found.expect("no table reached");
        dump("table", &dir, head, w, h);
    }
}
```
