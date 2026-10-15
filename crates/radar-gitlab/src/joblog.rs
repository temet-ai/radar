//! A job's log as GitLab sends it, made readable, and why a failed job
//! failed: plain functions.
//!
//! - [`parse`]: the runner's timestamps (`2026-10-06T12:11:38.498843Z 00O `)
//!   go, a `+` after them continues the line before; `section_start:` and
//!   `section_end:` markers become named sections (folded when GitLab says
//!   `collapsed=true`, and the runner's own housekeeping); a carriage return
//!   keeps what was written last; colours become the kit's inks.
//! - [`excerpt`]: the lines around the first error ([`is_error`]), or the
//!   last lines when none is recognised.
//! - [`reason`]: GitLab's `failure_reason` in words.

use std::ops::Range;

use radar_core::ui::log::{LogLine, LogText, Section};

use crate::model::TracePart;

/// What is held of a job's log: the text from byte `start` to the end of a
/// log of `total` bytes, and what it reads as.
#[derive(Debug, Clone, PartialEq)]
pub struct JobLog {
    pub raw: String,
    pub start: u64,
    pub total: u64,
    pub text: LogText,
    pub excerpt: Option<Excerpt>,
}

impl JobLog {
    pub fn new(part: TracePart) -> Self {
        let text = parse(&part.text, part.start == 0);
        Self {
            excerpt: excerpt(&text),
            raw: part.text,
            start: part.start,
            total: part.total.max(part.start),
            text,
        }
    }

    /// An earlier part arrived: put it before what is held. How many lines
    /// it added.
    pub fn prepend(&mut self, part: TracePart) -> usize {
        let before = self.text.lines.len();
        let mut raw = part.text;
        raw.push_str(&self.raw);
        *self = Self::new(TracePart {
            text: raw,
            start: part.start,
            total: self.total,
        });
        self.text.lines.len().saturating_sub(before)
    }

    /// There is more before what is held.
    pub fn earlier(&self) -> bool {
        self.start > 0
    }
}

/// Sections the runner writes around every job: folded until asked for.
const HOUSEKEEPING: [&str; 8] = [
    "resolve_secrets",
    "prepare_executor",
    "prepare_script",
    "get_sources",
    "restore_cache",
    "download_artifacts",
    "archive_cache",
    "cleanup_file_variables",
];

/// A log's text (all of it from the start when `whole`; else a part that
/// starts inside a line, whose first line is dropped) as lines and sections.
pub fn parse(raw: &str, whole: bool) -> LogText {
    let mut physical: Vec<String> = Vec::new();
    let mut parts = raw.split('\n');
    if !whole {
        parts.next();
    }
    for l in parts {
        match timestamped(l) {
            Some((true, rest)) if !physical.is_empty() => {
                physical.last_mut().expect("a line").push_str(rest)
            }
            Some((_, rest)) => physical.push(rest.to_string()),
            None => physical.push(l.strip_suffix('\r').unwrap_or(l).to_string()),
        }
    }
    if physical.last().is_some_and(String::is_empty) {
        physical.pop();
    }

    struct Open {
        name: String,
        at: Option<u64>,
        index: usize,
    }
    let mut text = LogText::default();
    let mut open: Vec<Open> = Vec::new();
    for line in &physical {
        let mut rest = line.as_str();
        let mut started = None;
        let mut marked = false;
        loop {
            let r = erased(rest);
            let (start, marker) = if let Some(m) = r.strip_prefix("section_start:") {
                (true, m)
            } else if let Some(m) = r.strip_prefix("section_end:") {
                (false, m)
            } else {
                rest = r;
                break;
            };
            marked = true;
            let (marker, after) = marker.split_once('\r').unwrap_or((marker, ""));
            rest = after;
            let (at, name) = marker.split_once(':').unwrap_or(("", marker));
            let at = at.parse::<u64>().ok();
            let (name, options) = name.split_once('[').unwrap_or((name, ""));
            let at_line = text.lines.len();
            if start {
                text.sections.push(Section {
                    title: name.replace('_', " "),
                    detail: String::new(),
                    start: at_line,
                    end: usize::MAX,
                    folded: options.contains("collapsed=true") || HOUSEKEEPING.contains(&name),
                });
                started = Some(text.sections.len() - 1);
                open.push(Open {
                    name: name.to_string(),
                    at,
                    index: text.sections.len() - 1,
                });
            } else if let Some(k) = open.iter().rposition(|o| o.name == name) {
                // It closes, and anything still open inside it.
                for o in open.drain(k..) {
                    let s = &mut text.sections[o.index];
                    s.end = at_line;
                    if let (Some(a), Some(b)) = (o.at, at)
                        && o.name == name
                    {
                        s.detail = radar_core::ui::duration(b.saturating_sub(a));
                    }
                }
            }
        }
        let shown = written_last(rest);
        let line = LogLine::ansi(shown);
        let blank = line.text().trim().is_empty();
        match started {
            // A section's title follows its marker.
            Some(s) if !blank => text.sections[s].title = line.text().trim().to_string(),
            _ if marked && blank => {}
            Some(_) => {}
            None => text.lines.push(line),
        }
    }
    let n = text.lines.len();
    for s in &mut text.sections {
        s.end = s.end.min(n);
    }
    text
}

/// `2026-10-06T12:11:38.498843Z 00O+rest`: whether it continues the line
/// before (`+`), and the rest. `None` when the line has no timestamp.
fn timestamped(l: &str) -> Option<(bool, &str)> {
    let b = l.as_bytes();
    let z = l.find("Z ")?;
    if !(19..=36).contains(&z) {
        return None;
    }
    let digit = |i: usize| b[i].is_ascii_digit();
    let shape = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if !shape.iter().all(|&(i, c)| b[i] == c) || ![0, 1, 2, 3, 5, 6, 8, 9].iter().all(|&i| digit(i)) {
        return None;
    }
    let rest = &b[z + 2..];
    if rest.len() < 3 || !rest[0].is_ascii_hexdigit() || !rest[1].is_ascii_hexdigit() || !matches!(rest[2], b'O' | b'E') {
        return None;
    }
    let append = rest.get(3) == Some(&b'+');
    Some((append, l.get(z + 6..).unwrap_or("")))
}

/// Without the erase-line codes a runner puts at the start (`ESC[0K`).
fn erased(mut s: &str) -> &str {
    loop {
        if let Some(r) = s.strip_prefix("\u{1b}[0K").or_else(|| s.strip_prefix("\u{1b}[K")) {
            s = r;
        } else {
            return s;
        }
    }
}

/// After carriage returns (a progress bar), what was written last.
fn written_last(s: &str) -> &str {
    if !s.contains('\r') {
        return s;
    }
    s.rsplit('\r')
        .find(|seg| !LogLine::ansi(seg).text().trim().is_empty())
        .unwrap_or("")
}

/// Lines from a failure to show: around the first error, or the last ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    pub lines: Range<usize>,
    /// The first error's line; `None` when none was recognised.
    pub error: Option<usize>,
}

/// Lines shown before the first error, and at most this many in all.
const BEFORE: usize = 2;
const SHOWN: usize = 10;

/// The lines around the first error, or the last lines that say anything
/// (without the runner's sign-off) when no error is recognised.
pub fn excerpt(text: &LogText) -> Option<Excerpt> {
    let plain: Vec<String> = text.lines.iter().map(LogLine::text).collect();
    if let Some(i) = plain.iter().position(|l| is_error(l)) {
        let from = i.saturating_sub(BEFORE);
        let mut to = (from + SHOWN).min(plain.len());
        while to > i + 1 && (plain[to - 1].trim().is_empty() || sign_off(&plain[to - 1])) {
            to -= 1;
        }
        return Some(Excerpt {
            lines: from..to,
            error: Some(i),
        });
    }
    let mut end = plain.len();
    while end > 0 && (plain[end - 1].trim().is_empty() || sign_off(&plain[end - 1])) {
        end -= 1;
    }
    (end > 0).then(|| Excerpt {
        lines: end.saturating_sub(8)..end,
        error: None,
    })
}

/// The runner's last word: never the first error, nor worth the room.
fn sign_off(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with("ERROR: Job failed") || t.starts_with("Job succeeded")
}

/// Lines that start an error, as build tools, test runners, shells and
/// runtimes write them.
const STARTS: [&str; 20] = [
    "error:",
    "Error:",
    "ERROR:",
    "ERROR ",
    "[ERROR]",
    "error[",
    "fatal:",
    "FATAL",
    "FAILED",
    "FAIL ",
    "npm ERR!",
    "npm error",
    "Traceback (most recent call last)",
    "Exception in thread",
    "Error from server",
    "panic:",
    "E   ",
    "✗ ",
    "✕ ",
    "Caused by:",
];

/// Words anywhere in a line that say it went wrong.
const CONTAINS: [&str; 12] = [
    " panicked at ",
    "BUILD FAILED",
    "BUILD FAILURE",
    "] Error ",
    ": error:",
    ": command not found",
    "No such file or directory",
    "Permission denied",
    "Segmentation fault",
    "OOMKilled",
    "Tests failed",
    "AssertionError",
];

/// Whether `line` is (the start of) an error. The runner's own `ERROR: Job
/// failed` and commands it echoes (`$ …`) are not.
pub fn is_error(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty() || t.starts_with("$ ") || sign_off(t) {
        return false;
    }
    if STARTS.iter().any(|s| t.starts_with(s))
        || CONTAINS.iter().any(|c| t.contains(c))
        // A test runner's verdict on one test: `… > aTest() FAILED`.
        || t.trim_end().ends_with(" FAILED")
    {
        return true;
    }
    // `TypeError: …`, `java.lang.IllegalStateException: …`
    t.split_whitespace()
        .next()
        .is_some_and(|w| w.ends_with("Error:") || w.ends_with("Exception:") || w.ends_with("Exception"))
}

/// GitLab's `failure_reason` in words; one it does not know with its
/// underscores as spaces.
pub fn reason(failure_reason: &str) -> String {
    let words = match failure_reason {
        "" => return String::new(),
        "script_failure" => "the script failed",
        "job_execution_timeout" => "it ran out of time",
        "stuck_or_timeout_failure" => "it was stuck or timed out",
        "runner_system_failure" => "the runner failed",
        "runner_unsupported" => "no runner supports it",
        "no_matching_runner" => "no runner matches its tags",
        "api_failure" => "GitLab's API failed",
        "missing_dependency_failure" => "a job it needs left no artifacts",
        "unmet_prerequisites" => "its prerequisites were not met",
        "scheduler_failure" => "it could not be scheduled",
        "data_integrity_failure" => "GitLab lost track of it",
        "archived_failure" => "its project is archived",
        "stale_schedule" => "its schedule was stale",
        "forward_deployment_failure" => "a newer deployment ran first",
        "protected_environment_failure" => "its environment is protected",
        "downstream_pipeline_creation_failed" => "its downstream pipeline was not created",
        "secrets_provider_not_found" => "a secrets provider is missing",
        "ci_quota_exceeded" => "out of CI minutes",
        "trace_size_exceeded" => "its log grew too large",
        "unknown_failure" => "GitLab does not know why",
        other => return other.replace('_', " "),
    };
    words.to_string()
}

/// Sample jobs and logs for the sample plugin.
#[cfg(any(test, feature = "demo"))]
pub mod fixtures {
    use crate::model::{JobInfo, TracePart};

    pub fn job(id: u64, project: &str, name: &str, stage: &str, reason: &str, pipeline: u64) -> JobInfo {
        JobInfo {
            id,
            project: project.into(),
            name: name.into(),
            stage: stage.into(),
            status: "failed".into(),
            duration_secs: Some(74),
            failure_reason: reason.into(),
            url: format!("https://gitlab.com/{project}/-/jobs/{id}"),
            allow_failure: false,
            pipeline,
        }
    }

    /// A test job that failed on an assertion, the whole log.
    pub fn failed_test() -> TracePart {
        let text = "Running with gitlab-runner 17.0\n\
section_start:1791270000:prepare_executor\r\u{1b}[0K\u{1b}[0K\u{1b}[36;1mPreparing the \"docker\" executor\u{1b}[0;m\n\
Using docker image gradle:8\n\
section_end:1791270004:prepare_executor\r\u{1b}[0K\n\
section_start:1791270004:step_script\r\u{1b}[0K\u{1b}[0K\u{1b}[36;1mExecuting \"step_script\" stage of the job script\u{1b}[0;m\n\
\u{1b}[32;1m$ ./gradlew test\u{1b}[0;m\n\
> Task :compileJava\n\
> Task :test\n\
StatsControllerTest > returnsTheCachedView() FAILED\n\
\u{20}   org.opentest4j.AssertionFailedError: expected: <200> but was: <500>\n\
\u{20}       at StatsControllerTest.java:41\n\
\n\
3 tests completed, 1 failed\n\
section_end:1791270078:step_script\r\u{1b}[0K\n\
\u{1b}[31;1mERROR: Job failed: exit code 1\u{1b}[0;m\n";
        TracePart {
            text: text.into(),
            start: 0,
            total: text.len() as u64,
        }
    }

    /// The last part of a long job that ran out of time.
    pub fn timed_out() -> TracePart {
        let text = "cut line\nwaiting for the stack to settle\nstill waiting (19m)\nstill waiting (20m)\n\
\u{1b}[31;1mERROR: Job failed: execution took longer than 20m0s seconds\u{1b}[0;m\n";
        TracePart {
            text: text.into(),
            start: 900_000,
            total: 900_000 + text.len() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed deploy as a runner with timestamps writes it.
    const STAMPED: &str = "2026-10-06T12:11:38.498843Z 00O \u{1b}[0KRunning with gitlab-runner 19.3.1\u{1b}[0;m
2026-10-06T12:11:38.499151Z 00O section_start:1791288698:prepare_executor\r\u{1b}[0K
2026-10-06T12:11:38.499157Z 00O+\u{1b}[0K\u{1b}[36;1mPreparing the \"docker\" executor\u{1b}[0;m\u{1b}[0;m
2026-10-06T12:11:38.499211Z 00O \u{1b}[0KUsing docker image alpine\u{1b}[0;m
2026-10-06T12:11:38.526235Z 00O section_end:1791288699:prepare_executor\r\u{1b}[0K
2026-10-06T12:11:38.526446Z 00O+section_start:1791288699:step_script\r\u{1b}[0K
2026-10-06T12:11:38.526454Z 00O+\u{1b}[0K\u{1b}[36;1mExecuting \"step_script\" stage of the job script\u{1b}[0;m\u{1b}[0;m
2026-10-06T12:11:59.234395Z 01O \u{1b}[32;1m$ kubectl apply -k .\u{1b}[0;m
2026-10-06T12:12:07.244830Z 01O configmap/api-config created
2026-10-06T12:12:08.747283Z 01O Error from server (Forbidden): error when retrieving current configuration of:
2026-10-06T12:12:08.747289Z 01O Resource: \"ingresses\", Name: \"api\"
2026-10-06T12:12:08.747309Z 01O
2026-10-06T12:12:08.750035Z 00O section_end:1791288728:step_script\r\u{1b}[0K
2026-10-06T12:12:08.751421Z 00O+section_start:1791288728:cleanup_file_variables\r\u{1b}[0K
2026-10-06T12:12:08.751431Z 00O+\u{1b}[0K\u{1b}[36;1mCleaning up project directory and file based variables\u{1b}[0;m\u{1b}[0;m
2026-10-06T12:12:09.247851Z 00O section_end:1791288729:cleanup_file_variables\r\u{1b}[0K
2026-10-06T12:12:09.277534Z 00O \u{1b}[31;1mERROR: Job failed: exit code 1\u{1b}[0;m
";

    /// The same kind of log from a runner without timestamps.
    const PLAIN: &str = "Running with gitlab-runner 16.0\n\
section_start:100:build[collapsed=true]\r\u{1b}[0K\u{1b}[0K\u{1b}[36;1mBuilding\u{1b}[0;m\n\
compiling 1\n\
compiling 2\n\
section_start:101:inner\r\u{1b}[0Kdeep\n\
section_end:130:build\r\u{1b}[0K\n\
progress 10%\rprogress 50%\rprogress 100%\r\n\
\ttabbed\n";

    fn lines(t: &LogText) -> Vec<String> {
        t.lines.iter().map(LogLine::text).collect()
    }

    #[test]
    fn timestamps_go_continued_lines_join_and_sections_get_their_titles() {
        let t = parse(STAMPED, true);
        assert_eq!(
            lines(&t),
            [
                "Running with gitlab-runner 19.3.1",
                "Using docker image alpine",
                "$ kubectl apply -k .",
                "configmap/api-config created",
                "Error from server (Forbidden): error when retrieving current configuration of:",
                "Resource: \"ingresses\", Name: \"api\"",
                "",
                "ERROR: Job failed: exit code 1",
            ]
        );
        let s: Vec<(&str, &str, usize, usize, bool)> = t
            .sections
            .iter()
            .map(|s| (s.title.as_str(), s.detail.as_str(), s.start, s.end, s.folded))
            .collect();
        assert_eq!(
            s,
            [
                ("Preparing the \"docker\" executor", "1s", 1, 2, true),
                ("Executing \"step_script\" stage of the job script", "29s", 2, 7, false),
                ("Cleaning up project directory and file based variables", "1s", 7, 7, true),
            ]
        );
        // Colours stay as inks: the command is green and bold.
        let cmd = &t.lines[2].pieces[0];
        assert_eq!((cmd.ink, cmd.bold), (radar_core::ui::log::Ink::Good, true));
    }

    #[test]
    fn markers_without_timestamps_nest_fold_and_close_what_is_inside() {
        let t = parse(PLAIN, true);
        assert_eq!(
            lines(&t),
            ["Running with gitlab-runner 16.0", "compiling 1", "compiling 2", "progress 100%", "    tabbed"]
        );
        assert_eq!(t.sections[0].title, "Building");
        assert!(t.sections[0].folded, "collapsed=true");
        assert_eq!((t.sections[0].start, t.sections[0].end), (1, 3));
        assert_eq!(t.sections[0].detail, "30s");
        // Closed with its outer one; a section never closed ends with the log.
        assert_eq!(t.sections[1].title, "deep");
        assert_eq!((t.sections[1].start, t.sections[1].end), (3, 3));
    }

    #[test]
    fn a_part_from_the_middle_drops_its_cut_first_line_and_an_end_without_a_start() {
        let t = parse("t of a line\nsection_end:1:build\r\u{1b}[0K\nlast\n", false);
        assert_eq!(lines(&t), ["last"]);
        assert!(t.sections.is_empty());
    }

    #[test]
    fn earlier_parts_go_before_what_is_held() {
        let mut log = JobLog::new(TracePart {
            text: "e two\nthree\n".into(),
            start: 10,
            total: 22,
        });
        assert_eq!(lines(&log.text), ["three"]);
        assert!(log.earlier());
        let added = log.prepend(TracePart {
            text: "one\nlin".into(),
            start: 0,
            total: 22,
        });
        assert_eq!(lines(&log.text), ["one", "line two", "three"]);
        assert_eq!(added, 2);
        assert!(!log.earlier());
    }

    #[test]
    fn the_excerpt_is_around_the_first_error_not_the_runners_sign_off() {
        let t = parse(STAMPED, true);
        let e = excerpt(&t).unwrap();
        assert_eq!(e.error, Some(4));
        assert_eq!(e.lines, 2..6, "two before, up to ten in all, no blank or sign-off at the end");
        // No error recognised: the last lines that say anything.
        let t = parse("a\nb\nstill waiting\n\nERROR: Job failed: execution took longer than 1h\n", true);
        let e = excerpt(&t).unwrap();
        assert_eq!((e.lines.clone(), e.error), (0..3, None));
        assert_eq!(excerpt(&parse("", true)), None);
    }

    #[test]
    fn errors_are_recognised_as_tools_write_them() {
        for yes in [
            "error: could not compile `api`",
            "npm ERR! code ELIFECYCLE",
            "[ERROR] Failed to execute goal on project api",
            "Traceback (most recent call last):",
            "E   assert 1 == 2",
            "thread 'main' panicked at src/main.rs:3:5:",
            "TypeError: cannot read properties of undefined",
            "java.lang.IllegalStateException: no context",
            "make: *** [build] Error 2",
            "src/a.c:3:1: error: expected ';'",
            "/bin/sh: eval: line 1: mvn: command not found",
            "FAIL src/app.test.ts",
            "ERROR: Preparation failed: pull image",
            "StatsControllerTest > returnsTheCachedView() FAILED",
        ] {
            assert!(is_error(yes), "{yes}");
        }
        for no in [
            "ERROR: Job failed: exit code 1",
            "$ echo \"error: never\"",
            "Tests run: 10, Failures: 0, Errors: 0",
            "compiling error_codes.rs",
            "warning: unused variable",
            "",
        ] {
            assert!(!is_error(no), "{no}");
        }
    }

    #[test]
    fn failure_reasons_read_as_words() {
        assert_eq!(reason("script_failure"), "the script failed");
        assert_eq!(reason("job_execution_timeout"), "it ran out of time");
        assert_eq!(reason("some_new_reason"), "some new reason");
        assert_eq!(reason(""), "");
    }
}
