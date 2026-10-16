//! Images in the terminal: a picture where a document has one (a chat's
//! screenshot, a description's diagram), drawn with the terminal's graphics
//! protocol through `ratatui-image` (kitty's, then sixel or iTerm2's).
//!
//! The runner turns them on ([`start`]) when `[theme] images` and the
//! terminal allow; otherwise, and in tests, nothing here draws and rich text
//! shows an image as the line naming it and its link. A part supplies only
//! how to fetch one ([`Fetch`], from [`fetch`]): the url and its own
//! credentials, kept in the part. The kit keeps the rest: which images are
//! wanted (only those scrolled into view are fetched, a few at a time), the
//! pictures held in memory, a cache on disk under radar's cache folder with a
//! size cap, and drawing them over the rows rich text keeps for them
//! ([`card`], [`scrolled`]), cut where a panel's edge cuts them.

use std::cell::Cell as Flag;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use image::DynamicImage;
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ratatui_image::Resize;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};

use super::rich::{Media, Placed};
use super::scroll::Scroll;
use crate::log;
use crate::theme::Theme;

/// An image's bytes, on their way.
pub type Bytes = Pin<Box<dyn Future<Output = anyhow::Result<Vec<u8>>> + Send>>;

/// How a part fetches an image: given its `src`, the bytes. The part keeps
/// its credentials inside; the kit never sees them.
pub type Fetch = Arc<dyn Fn(String) -> Bytes + Send + Sync>;

/// A [`Fetch`] from an async function or closure.
pub fn fetch<F, Fut>(f: F) -> Fetch
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<Vec<u8>>> + Send + 'static,
{
    Arc::new(move |src| Box::pin(f(src)))
}

/// The most rows an image takes.
pub const MAX_ROWS: u16 = 20;
/// Pictures held in memory.
const HELD: usize = 32;
/// Fetches running at once.
const AT_ONCE: usize = 3;
/// The disk cache's size cap.
pub const DISK_CAP: u64 = 200 * 1024 * 1024;
/// An image bigger than this is not drawn.
const MAX_BYTES: usize = 25 * 1024 * 1024;
/// A picture is held no bigger than this on either side.
const MAX_SIDE: u32 = 2048;

thread_local! {
    /// Whether images are drawn, on the thread that draws.
    static ON: Flag<bool> = const { Flag::new(false) };
}

enum State {
    Loading,
    Ready(Held),
    Failed,
}

struct Held {
    image: DynamicImage,
    /// The picture encoded for the terminal, by size in cells.
    drawn: HashMap<(u16, u16), SlicedProtocol>,
}

#[derive(Default)]
struct Store {
    picker: Option<Picker>,
    dir: Option<PathBuf>,
    images: HashMap<String, State>,
    /// Pictures held, the least lately drawn first.
    ready: VecDeque<String>,
}

fn store() -> MutexGuard<'static, Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    let m = S.get_or_init(|| Mutex::new(Store::default()));
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn permits() -> &'static tokio::sync::Semaphore {
    static P: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    P.get_or_init(|| tokio::sync::Semaphore::new(AT_ONCE))
}

/// Draw images with `picker`'s protocol, caching under `dir`.
pub fn start(picker: Picker, dir: Option<PathBuf>) {
    let mut s = store();
    s.picker = Some(picker);
    s.dir = dir;
    ON.with(|f| f.set(true));
}

/// Draw no more images (the setting went off).
pub fn stop() {
    ON.with(|f| f.set(false));
}

/// Whether images are drawn on this thread.
pub fn on() -> bool {
    ON.with(Flag::get) && store().picker.is_some()
}

/// The terminal's graphics, as `[theme] images` says: `off`, `on` (ask the
/// terminal, inside tmux too), or `auto` (ask, except inside tmux, which
/// may not pass the pictures on). Only kitty's, sixel and iTerm2's count:
/// a terminal that answers none of them gets the text line. Call it with
/// the terminal in raw mode, before anything else reads its input.
pub fn detect(setting: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<Picker> {
    if setting == "off" {
        return None;
    }
    let tmux = var("TMUX").is_some_and(|v| !v.is_empty())
        || var("TERM").is_some_and(|t| t.starts_with("tmux") || t.starts_with("screen"));
    if setting != "on" && tmux {
        return None;
    }
    let options = ratatui_image::picker::cap_parser::QueryStdioOptions {
        timeout: std::time::Duration::from_millis(500),
        ..Default::default()
    };
    let picker = Picker::from_query_stdio_with_options(options).ok()?;
    graphic(&picker).then_some(picker)
}

/// A cell's size in pixels while pictures go by kitty's graphics: what the
/// splash's picture is drawn at ([`crate::splash::picture`]). Sixel and
/// iTerm2's cannot replace a picture in place, so a moving one is kitty's
/// alone.
pub fn kitty_cell() -> Option<(u16, u16)> {
    if !on() {
        return None;
    }
    let s = store();
    let picker = s.picker.as_ref()?;
    (picker.protocol_type() == ProtocolType::Kitty).then(|| {
        let f = picker.font_size();
        (f.width, f.height)
    })
}

/// Whether `picker` draws pictures (not letters standing in for them).
fn graphic(picker: &Picker) -> bool {
    matches!(
        picker.protocol_type(),
        ProtocolType::Kitty | ProtocolType::Sixel | ProtocolType::Iterm2
    )
}

/// The cells an image takes in a panel `width` wide: its own size in the
/// terminal's cells, made smaller to fit the width and [`MAX_ROWS`]; one
/// row while its size is not known yet. Nothing when images are not drawn
/// here, it is not an image, has nowhere to come from, or could not be had.
pub fn fit(m: &Media, width: usize) -> Option<(u16, u16)> {
    if !m.image || m.src.is_empty() || width < 4 || !on() {
        return None;
    }
    let s = store();
    let font = s.picker.as_ref()?.font_size();
    let px = match s.images.get(&m.src) {
        Some(State::Failed) => return None,
        Some(State::Ready(h)) => Some((h.image.width(), h.image.height())),
        _ => m.size,
    };
    let width = width.min(u16::MAX as usize) as f64;
    let Some((w, h)) = px.filter(|(w, h)| *w > 0 && *h > 0) else {
        return Some((width as u16, 1));
    };
    let cols = (f64::from(w) / f64::from(font.width.max(1))).ceil();
    let rows = (f64::from(h) / f64::from(font.height.max(1))).ceil();
    let scale = 1f64.min(width / cols).min(f64::from(MAX_ROWS) / rows);
    Some((
        ((cols * scale).floor() as u16).max(1),
        ((rows * scale).ceil() as u16).clamp(1, MAX_ROWS),
    ))
}

/// Ask for `src` to be had, once: from the disk cache, or through `fetch`.
/// The answer is drawn on a later frame.
pub fn want(src: &str, fetch: &Fetch) {
    {
        let mut s = store();
        if s.images.contains_key(src) {
            return;
        }
        s.images.insert(src.to_string(), State::Loading);
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        store().images.insert(src.to_string(), State::Failed);
        return;
    };
    let src = src.to_string();
    let fetch = fetch.clone();
    rt.spawn(async move {
        let _permit = permits().acquire().await;
        let dir = store().dir.clone();
        let cached = dir.as_deref().and_then(|d| std::fs::read(file_for(d, &src)).ok());
        let bytes = match cached {
            Some(b) => Ok((b, false)),
            None => fetch(src.clone()).await.map(|b| (b, true)),
        };
        let at = src.clone();
        let held = tokio::task::spawn_blocking(move || -> anyhow::Result<DynamicImage> {
            let (bytes, fresh) = bytes?;
            anyhow::ensure!(bytes.len() <= MAX_BYTES, "{} bytes, more than an image is drawn at", bytes.len());
            let image = decode(&bytes)?;
            if fresh && let Some(d) = dir.as_deref() {
                keep(d, &at, &bytes);
            }
            Ok(image)
        })
        .await;
        let state = match held {
            Ok(Ok(image)) => State::Ready(Held {
                image,
                drawn: HashMap::new(),
            }),
            Ok(Err(e)) => {
                log::warn(format!("image {src} not drawn: {e:#}"));
                State::Failed
            }
            Err(e) => {
                log::warn(format!("image {src} not drawn: {e}"));
                State::Failed
            }
        };
        hold(&src, state);
    });
}

fn decode(bytes: &[u8]) -> anyhow::Result<DynamicImage> {
    let image = image::load_from_memory(bytes)?;
    Ok(if image.width() > MAX_SIDE || image.height() > MAX_SIDE {
        image.thumbnail(MAX_SIDE, MAX_SIDE)
    } else {
        image
    })
}

/// Keep what became of `src`, letting the least lately drawn picture go
/// once more than [`HELD`] are held.
fn hold(src: &str, state: State) {
    let mut s = store();
    if matches!(state, State::Ready(_)) {
        s.ready.retain(|k| k != src);
        s.ready.push_back(src.to_string());
        while s.ready.len() > HELD {
            if let Some(old) = s.ready.pop_front() {
                s.images.remove(&old);
            }
        }
    }
    s.images.insert(src.to_string(), state);
}

/// Where `src` is kept on disk: a name from its address, never the address.
fn file_for(dir: &Path, src: &str) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in src.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    dir.join(format!("{h:016x}"))
}

/// Write `bytes` for `src` into the cache (readable by you alone), then
/// let the oldest files go until the cache is under its cap.
fn keep(dir: &Path, src: &str, bytes: &[u8]) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let path = file_for(dir, src);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, bytes));
    if let Err(e) = written {
        log::warn(format!("image cache {} not written: {e}", path.display()));
        return;
    }
    prune(dir, DISK_CAP);
}

/// The oldest files in `dir` removed until what is left is at most `cap`
/// bytes.
fn prune(dir: &Path, cap: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file()
                .then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Draw `src`, when it is had, `cols` by `rows` cells, at `x` cells in and
/// `top` rows down `area` (above its top when negative: the rows above are
/// cut off, as are those past its foot).
pub fn draw(src: &str, cols: u16, rows: u16, area: Rect, buf: &mut Buffer, x: u16, top: i32) {
    let mut s = store();
    let Some(picker) = s.picker.clone() else {
        return;
    };
    let Some(State::Ready(held)) = s.images.get_mut(src) else {
        return;
    };
    let size = (cols, rows);
    if !held.drawn.contains_key(&size) {
        match SlicedProtocol::new_with_resize(
            &picker,
            held.image.clone(),
            Size::new(cols, rows),
            Resize::Fit(None),
        ) {
            Ok(p) => {
                held.drawn.insert(size, p);
            }
            Err(e) => {
                log::warn(format!("image {src} not drawn: {e}"));
                s.images.insert(src.to_string(), State::Failed);
                return;
            }
        }
    }
    let Some(State::Ready(held)) = s.images.get(src) else {
        return;
    };
    if let Some(p) = held.drawn.get(&size) {
        let at = SignedPosition {
            x: x.min(i16::MAX as u16) as i16,
            y: top.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        };
        SlicedImage::new(p, at).render(area, buf);
    }
    s.ready.retain(|k| k != src);
    s.ready.push_back(src.to_string());
}

/// Lines wrapped to `body`, scrolled, with the scrollbar on `frame`'s edge
/// (as [`super::scroll::scrolled`]), and the images placed in them drawn
/// over the rows kept for them: asked for once they are in view.
#[allow(clippy::too_many_arguments)]
pub fn scrolled(
    theme: &Theme,
    scroll: &Scroll,
    frame: Rect,
    body: Rect,
    buf: &mut Buffer,
    lines: Vec<Line<'static>>,
    media: &[Placed],
    fetch: &Fetch,
) {
    // The row each image starts on, once the lines before it are wrapped.
    let starts: Vec<i32> = media
        .iter()
        .map(|m| {
            let before = lines[..m.line.min(lines.len())].to_vec();
            Paragraph::new(before)
                .wrap(Wrap { trim: false })
                .line_count(body.width) as i32
        })
        .collect();
    super::scroll::scrolled(theme, scroll, frame, body, buf, lines);
    let at = i32::from(scroll.at());
    for (m, start) in media.iter().zip(starts) {
        let top = start - at;
        if top + i32::from(m.rows) <= 0 || top >= i32::from(body.height) {
            continue;
        }
        want(&m.src, fetch);
        draw(&m.src, m.cols, m.rows, body, buf, m.x, top);
    }
}

/// A card ([`super::card::card`]) whose text has images placed in it.
#[allow(clippy::too_many_arguments)]
pub fn card(
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    focused: bool,
    scroll: &Scroll,
    lines: Vec<Line<'static>>,
    media: &[Placed],
    fetch: &Fetch,
) -> Rect {
    let inner = crate::widgets::padded(super::card::panel(theme, area, buf, title, focused));
    scrolled(theme, scroll, area, inner, buf, lines, media, fetch);
    inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::theme::Tier;
    use crate::ui::rich::{self, Block, Doc, Inline};
    use ratatui_image::FontSize;

    /// Images on, on this thread, as a kitty terminal with 10x20 cells (a
    /// picker made without asking a terminal: what tests need).
    #[allow(deprecated)]
    fn kitty() {
        let mut p = Picker::from_fontsize(FontSize {
            width: 10,
            height: 20,
        });
        p.set_protocol_type(ProtocolType::Kitty);
        // As start does, leaving the disk cache as another test set it.
        store().picker = Some(p);
        ON.with(|f| f.set(true));
    }

    fn picture(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 90, 255])))
    }

    fn media(src: &str, size: Option<(u32, u32)>) -> Media {
        Media {
            image: true,
            name: "shot.png".into(),
            url: format!("https://x/{src}"),
            src: src.into(),
            size,
        }
    }

    #[test]
    fn without_graphics_nothing_is_kept_and_the_text_line_stays() {
        stop();
        assert_eq!(fit(&media("a", Some((800, 600))), 40), None);
        let doc = Doc::new(vec![Block::Media(media("a", Some((800, 600))))]);
        let laid = rich::layout(&doc, &rich::Look::new(&Theme::new(Tier::Mono)), 40);
        assert!(laid.media.is_empty());
        assert_eq!(rich::text_of(&laid.lines), "▣ image shot.png https://x/a");
    }

    #[test]
    fn an_image_fits_the_width_and_the_row_cap_keeping_its_shape() {
        kitty();
        // 800x600 px in 10x20 cells is 80x30 cells: half of that fits 40.
        assert_eq!(fit(&media("b1", Some((800, 600))), 40), Some((40, 15)));
        // Tall (10x100 cells): the rows give way, and the width with them.
        assert_eq!(fit(&media("b2", Some((100, 2000))), 40), Some((2, MAX_ROWS)));
        // Small stays small.
        assert_eq!(fit(&media("b3", Some((50, 20))), 40), Some((5, 1)));
        // Unknown until had: a row.
        assert_eq!(fit(&media("b4", None), 40), Some((40, 1)));
        hold("b4", State::Ready(Held { image: picture(200, 100), drawn: HashMap::new() }));
        assert_eq!(fit(&media("b4", None), 40), Some((20, 5)));
        hold("b5", State::Failed);
        assert_eq!(fit(&media("b5", Some((800, 600))), 40), None, "the text line again");
        let file = Media { image: false, ..media("b6", Some((800, 600))) };
        assert_eq!(fit(&file, 40), None);
        stop();
    }

    #[test]
    fn rich_text_keeps_rows_for_an_image_and_says_where() {
        kitty();
        let doc = Doc::new(vec![
            Block::para(vec![Inline::text("before")]),
            Block::Quote {
                blocks: vec![Block::Media(media("c1", Some((200, 100))))],
            },
            Block::para(vec![Inline::text("after")]),
        ]);
        let laid = rich::layout(&doc, &rich::Look::new(&Theme::new(Tier::Mono)), 40);
        assert_eq!(
            laid.media,
            [Placed {
                line: 2,
                x: 2,
                cols: 20,
                rows: 5,
                src: "c1".into()
            }]
        );
        let text: Vec<String> = rich::text_of(&laid.lines).lines().map(String::from).collect();
        assert_eq!(
            text,
            ["before", "", "│ ▣ shot.png", "│", "│", "│", "│", "", "after"]
        );
        // Last in a document, its rows stay.
        let doc = Doc::new(vec![Block::Media(media("c1", Some((200, 100))))]);
        let laid = rich::layout(&doc, &rich::Look::new(&Theme::new(Tier::Mono)), 40);
        assert_eq!((laid.lines.len(), laid.media[0].line), (5, 0));
        stop();
    }

    #[test]
    fn kitty_sends_the_picture_once_then_only_its_placeholders() {
        kitty();
        hold("d1", State::Ready(Held { image: picture(40, 40), drawn: HashMap::new() }));
        let area = Rect::new(0, 0, 10, 4);
        let mut buf = Buffer::empty(area);
        draw("d1", 4, 2, area, &mut buf, 1, 0);
        let first = buf[(1, 0)].symbol().to_string();
        assert!(first.starts_with("\x1b_G"), "the picture is sent: {:?}", &first[..20.min(first.len())]);
        assert!(first.contains('\u{10EEEE}'), "and placed");
        assert!(buf[(2, 1)].symbol().contains('\u{10EEEE}'));
        assert_eq!(buf[(0, 0)].symbol(), " ", "left of it untouched");
        let mut again = Buffer::empty(area);
        draw("d1", 4, 2, area, &mut again, 1, 0);
        assert!(!again[(1, 0)].symbol().contains("\x1b_G"), "sent once");
        assert!(again[(1, 0)].symbol().contains('\u{10EEEE}'));
        stop();
    }

    #[test]
    fn an_image_scrolled_half_out_of_view_draws_its_lower_rows() {
        kitty();
        hold("e1", State::Ready(Held { image: picture(40, 80), drawn: HashMap::new() }));
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        // Four rows, its top two above the panel.
        draw("e1", 4, 4, area, &mut buf, 0, -2);
        assert!(buf[(0, 0)].symbol().contains('\u{10EEEE}'));
        assert!(buf[(0, 1)].symbol().contains('\u{10EEEE}'));
        assert_eq!(buf[(0, 2)].symbol(), " ", "nothing under it");
        stop();
    }

    #[test]
    fn a_card_draws_the_images_in_view_and_asks_only_for_those() {
        kitty();
        hold("f1", State::Ready(Held { image: picture(40, 40), drawn: HashMap::new() }));
        let mut lines: Vec<Line<'static>> = (0..30).map(|i| Line::raw(format!("line {i}"))).collect();
        lines[2] = Line::raw("▣ seen");
        let media = [
            Placed { line: 2, x: 0, cols: 4, rows: 2, src: "f1".into() },
            Placed { line: 25, x: 0, cols: 4, rows: 2, src: "f2-not-in-view".into() },
        ];
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = asked.clone();
        let f = fetch(move |src| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(src);
                anyhow::bail!("not here")
            }
        });
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let scroll = Scroll::new();
        card(&Theme::new(Tier::Mono), area, &mut buf, "t", true, &scroll, lines, &media, &f);
        // Inside the border and a cell of room: the image's first row is
        // the card's third.
        assert!(buf[(2, 3)].symbol().contains('\u{10EEEE}'));
        assert!(!store().images.contains_key("f2-not-in-view"), "not asked for out of view");
        assert!(asked.lock().unwrap().is_empty());
        stop();
    }

    /// On tokio's paused clock: the fetches' sleeps and the waits below are
    /// virtual, and the clock moves only while no task can run (a decode on
    /// the blocking pool holds it), so the waits end on what the tasks did,
    /// not on how fast the machine is. On real threads and a real clock the
    /// waits were 4 s and 2 s, which a loaded machine could outlast.
    #[tokio::test(start_paused = true)]
    async fn images_are_fetched_a_few_at_a_time_kept_on_disk_and_had_again_from_there() {
        kitty();
        let dir = std::env::temp_dir().join(format!("radar-images-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        store().dir = Some(dir.clone());
        let mut png = Vec::new();
        picture(8, 8)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let now = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let most = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (n, m, bytes) = (now.clone(), most.clone(), png.clone());
        let f = fetch(move |_src| {
            let (n, m, bytes) = (n.clone(), m.clone(), bytes.clone());
            async move {
                use std::sync::atomic::Ordering::SeqCst;
                let k = n.fetch_add(1, SeqCst) + 1;
                m.fetch_max(k, SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                n.fetch_sub(1, SeqCst);
                Ok(bytes)
            }
        });
        let srcs: Vec<String> = (0..8).map(|i| format!("g{i}-{}", std::process::id())).collect();
        for s in &srcs {
            want(s, &f);
        }
        for _ in 0..200 {
            if srcs.iter().all(|s| matches!(store().images.get(s), Some(State::Ready(_)))) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(srcs.iter().all(|s| matches!(store().images.get(s), Some(State::Ready(_)))));
        assert!(most.load(std::sync::atomic::Ordering::SeqCst) <= AT_ONCE);
        // On disk, by a name that is not the address.
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).collect();
        assert_eq!(files.len(), 8);
        assert!(files.iter().all(|e| !e.file_name().to_string_lossy().contains('g')));
        // Forgotten in memory, it comes back from the disk, not the fetch.
        let failing = fetch(|_| async { anyhow::bail!("offline") });
        store().images.remove(&srcs[0]);
        want(&srcs[0], &failing);
        for _ in 0..100 {
            if matches!(store().images.get(&srcs[0]), Some(State::Ready(_))) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(matches!(store().images.get(&srcs[0]), Some(State::Ready(_))));
        let _ = std::fs::remove_dir_all(&dir);
        store().dir = None;
        stop();
    }

    #[test]
    fn the_disk_cache_lets_its_oldest_files_go_past_its_cap() {
        let dir = std::env::temp_dir().join(format!("radar-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, name) in ["old", "mid", "new"].iter().enumerate() {
            let p = dir.join(name);
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let t = std::time::SystemTime::now() - std::time::Duration::from_secs(300 - 100 * i as u64);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        prune(&dir, 250);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["mid", "new"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_does_not_ask_inside_tmux_and_off_never_asks() {
        let tmux = |k: &str| (k == "TMUX").then(|| "/tmp/tmux-1/default,1,0".to_string());
        assert!(detect("auto", &tmux).is_none());
        assert!(detect("off", &|_| None).is_none());
    }
}
