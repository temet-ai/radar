//! Contract fixtures are captures of the production native painter's real cells.
use super::*;
use serde_json::{Value, json};

fn rect(r: Rect) -> [u16; 4] {
    [r.x, r.y, r.width, r.height]
}
fn rows(buf: &Buffer) -> Value {
    let lines: Vec<Vec<Value>> = (buf.area.y..buf.area.bottom())
        .map(|y| {
            let mut runs = Vec::new();
            let mut symbols = String::new();
            let mut count = 0usize;
            let mut previous: Option<(String, String, String)> = None;
            for x in buf.area.x..buf.area.right() {
                let cell = &buf[(x, y)];
                let fg = crate::ipc::look::colour_word(Some(cell.fg));
                let bg = crate::ipc::look::colour_word(Some(cell.bg));
                let mods = [
                    (Modifier::BOLD, "bold"),
                    (Modifier::DIM, "dim"),
                    (Modifier::ITALIC, "italic"),
                    (Modifier::UNDERLINED, "underline"),
                    (Modifier::REVERSED, "reversed"),
                    (Modifier::CROSSED_OUT, "strike"),
                ]
                .iter()
                .filter_map(|(flag, word)| cell.modifier.contains(*flag).then_some(*word))
                .collect::<Vec<_>>()
                .join("+");
                let style = (fg, bg, mods);
                if previous.as_ref().is_some_and(|p| *p != style) {
                    let (fg, bg, mods) = previous.take().unwrap();
                    runs.push(json!([symbols, count, fg, bg, mods]));
                    symbols = String::new();
                    count = 0;
                }
                previous = Some(style);
                symbols.push_str(cell.symbol());
                count += 1;
            }
            if let Some((fg, bg, mods)) = previous {
                runs.push(json!([symbols, count, fg, bg, mods]));
            }
            runs
        })
        .collect();
    json!(lines)
}

#[test]
fn native_notice_cells() {
    struct Case {
        name: String,
        width: u16,
        height: u16,
        tier: Tier,
        icons: &'static str,
        now_ms: u64,
        motion: bool,
        hidden: u64,
        history: bool,
        unicode: bool,
        override_slots: bool,
        detail_scroll: usize,
        omitted: u64,
    }
    let mut cases = Vec::new();
    for (width, height) in [(80, 24), (160, 40)] {
        for (name, now_ms, motion, history) in [
            ("start", 1000, true, false),
            ("entering", 1060, true, false),
            ("partial", 2500, true, false),
            ("motion-off", 2500, false, false),
            ("expired", 11_000, true, false),
            ("history", 11_000, true, true),
        ] {
            cases.push(Case {
                name: format!("{name}-{width}x{height}"),
                width,
                height,
                tier: Tier::TrueColor,
                icons: "unicode",
                now_ms,
                motion,
                hidden: 0,
                history,
                unicode: false,
                override_slots: false,
                detail_scroll: 0,
                omitted: 0,
            });
        }
    }
    for (name, tier) in [
        ("mono", Tier::Mono),
        ("ansi16", Tier::Ansi16),
        ("ansi256", Tier::Ansi256),
    ] {
        cases.push(Case {
            name: format!("{name}-80x24"),
            width: 80,
            height: 24,
            tier,
            icons: "off",
            now_ms: 2500,
            motion: true,
            hidden: 0,
            history: false,
            unicode: false,
            override_slots: false,
            detail_scroll: 0,
            omitted: 0,
        });
    }
    for (
        name,
        width,
        height,
        icons,
        hidden,
        unicode,
        override_slots,
        history,
        detail_scroll,
        omitted,
    ) in [
        ("unicode-18x8", 18, 8, "emoji", 0, true, false, false, 0, 0),
        (
            "narrow-overflow-8x5",
            8,
            5,
            "off",
            9,
            true,
            false,
            false,
            0,
            0,
        ),
        (
            "overflow-80x24",
            80,
            24,
            "off",
            9,
            false,
            false,
            false,
            0,
            0,
        ),
        (
            "override-80x24",
            80,
            24,
            "unicode",
            0,
            false,
            true,
            false,
            0,
            0,
        ),
        (
            "history-paged-80x24",
            80,
            24,
            "unicode",
            0,
            false,
            false,
            true,
            5,
            0,
        ),
        (
            "history-omitted-80x24",
            80,
            24,
            "off",
            0,
            false,
            false,
            true,
            0,
            17,
        ),
    ] {
        cases.push(Case {
            name: name.into(),
            width,
            height,
            tier: Tier::TrueColor,
            icons,
            now_ms: 2500,
            motion: true,
            hidden,
            history,
            unicode,
            override_slots,
            detail_scroll,
            omitted,
        });
    }
    assert_eq!(cases.len(), 21);
    let destination = std::env::var_os("RADAR_NOTICE_CELLS_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &destination {
        std::fs::create_dir_all(dir).unwrap();
    }
    for case in cases {
        let mut config = ThemeConfig {
            icons: case.icons.into(),
            ..Default::default()
        };
        if case.override_slots {
            config.slots.insert("bg.overlay".into(), "#102030".into());
            config.slots.insert("status.info".into(), "#a0d0f0".into());
            config.slots.insert("status.error".into(), "#ff7040".into());
        }
        let theme = Theme::build(&config, case.tier, &[]);
        let area = Rect::new(0, 0, case.width, case.height);
        let body = Rect::new(0, 1, case.width, case.height.saturating_sub(2));
        let content = body.inner(ratatui::layout::Margin::new(1, 1));
        let mut buf = Buffer::empty(area);
        for p in area.positions() {
            buf[p]
                .set_symbol(".")
                .set_style(theme.s(Slot::TextPrimary).patch(theme.bg(Slot::BgBase)));
        }
        let top = Rect::new(0, 0, case.width, 1);
        let status = Rect::new(0, case.height - 1, case.width, 1);
        clear(
            &mut buf,
            top,
            theme
                .bold(Slot::TextEmphasis)
                .patch(theme.bg(Slot::BgOverlay)),
        );
        draw_line(
            &mut buf,
            top,
            &Line::styled(
                "Watch / Notices",
                theme
                    .bold(Slot::TextEmphasis)
                    .patch(theme.bg(Slot::BgOverlay)),
            ),
        );
        clear(
            &mut buf,
            status,
            theme.s(Slot::TextMuted).patch(theme.bg(Slot::BgOverlay)),
        );
        draw_line(
            &mut buf,
            status,
            &Line::styled(
                "~ history  esc dismiss",
                theme.s(Slot::TextMuted).patch(theme.bg(Slot::BgOverlay)),
            ),
        );
        let before = rows(&buf);
        let entries = if case.unicode {
            vec![
                notice(61, NoticeTone::Info, "界界 e\u{301} 👩‍💻 Watch notes", 0),
                notice(62, NoticeTone::Error, "界 Watch request failed", 1000),
            ]
        } else {
            let mut error = notice(
                52,
                NoticeTone::Error,
                "The Watch request failed. Check the connection before trying again.",
                1000,
            );
            error.icon = Some(Icon::Error);
            vec![
                notice(51, NoticeTone::Info, "Copied the Watch link", 0),
                error,
            ]
        };
        let mut detail = entries.last().unwrap().clone();
        if case.detail_scroll > 0 {
            detail.text = (1..=16)
                .map(|n| {
                    format!(
                        "Watch detail paragraph {n}: Ankh-Morpork connection needs another attempt."
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        let history_view = case.history.then(|| NoticeHistoryView {
            entries: entries.clone(),
            detail: Some(detail),
            omitted: case.omitted,
            selected: 0,
            scroll: 0,
            detail_scroll: case.detail_scroll,
        });
        let stack = if let Some(view) = &history_view {
            history(&theme, view, body, &mut buf);
            Stack::default()
        } else {
            let stack = layout(&theme, &entries, case.hidden, content, case.now_ms, case.motion);
            paint(&stack, &theme, &mut buf);
            stack
        };
        let value = json!({ "format": 1, "name": case.name, "width": case.width, "height": case.height,
            "look": crate::ipc::Look::of(&theme), "body": rect(body), "content": rect(content), "now_ms": case.now_ms,
            "motion": case.motion, "notices": entries, "hidden": case.hidden, "history": history_view,
            "before": before, "rows": rows(&buf),
            "toasts": stack.toasts.iter().map(|n| json!({"id":n.id,"area":rect(n.area),"fill":n.fill,"older":n.older,"hidden":n.hidden})).collect::<Vec<_>>(),
            "summary": stack.summary.map(|s| json!({"area":rect(s.area),"hidden":s.hidden})) });
        let name = format!("{}.json", case.name);
        if let Some(dir) = &destination {
            std::fs::write(
                dir.join(name),
                format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
            )
            .unwrap();
        } else {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../ink/test/fixtures/notices")
                .join(name);
            let recorded: Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| {
                    panic!("{}: {e}; regenerate native notice captures", path.display())
                }))
                .unwrap();
            assert_eq!(recorded, value, "{}", path.display());
        }
    }
}
