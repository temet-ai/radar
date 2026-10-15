//! Pictures in a merge request's description: their addresses made whole
//! (a description writes an upload as `/uploads/<secret>/<file>`, the
//! project's), and their bytes fetched with the token, which goes only to
//! the configured server.

use anyhow::{Result, bail};
use radar_core::ui::image::Fetch;
use radar_core::ui::rich::{Block, Doc, Kind};

/// `src` as a whole address: an upload is the project's, another path the
/// server's, a whole address stays.
pub fn whole(src: &str, server: &str, project: &str) -> String {
    let server = server.trim_end_matches('/');
    if src.starts_with("/uploads/") {
        format!("{server}/{}{src}", project.trim_matches('/'))
    } else if src.starts_with('/') && !src.starts_with("//") {
        format!("{server}{src}")
    } else {
        src.to_string()
    }
}

/// Every image and link in `doc` with a whole address.
pub fn make_whole(doc: &mut Doc, server: &str, project: &str) {
    fn walk(blocks: &mut [Block], f: &dyn Fn(&str) -> String) {
        for b in blocks {
            match b {
                Block::Media(m) => {
                    m.url = f(&m.url);
                    m.src = f(&m.src);
                }
                Block::Para { text } | Block::Heading { text, .. } => runs(text, f),
                Block::Details { summary, blocks } => {
                    runs(summary, f);
                    walk(blocks, f);
                }
                Block::Quote { blocks } | Block::Callout { blocks, .. } => walk(blocks, f),
                Block::List { items } => items.iter_mut().for_each(|i| walk(&mut i.blocks, f)),
                Block::Table { head, rows, .. } => {
                    head.iter_mut().for_each(|c| runs(c, f));
                    rows.iter_mut().flatten().for_each(|c| runs(c, f));
                }
                Block::Code { .. } | Block::Rule => {}
            }
        }
    }
    fn runs(text: &mut [radar_core::ui::rich::Inline], f: &dyn Fn(&str) -> String) {
        for i in text {
            match &mut i.kind {
                Kind::Link { url } => *url = f(url),
                Kind::Image { src } => *src = f(src),
                _ => {}
            }
        }
    }
    walk(&mut doc.blocks, &|s| if s.is_empty() { String::new() } else { whole(s, server, project) });
}

/// Whether `url` is on `server` (same scheme, host and port).
pub fn on_server(url: &str, server: &str) -> bool {
    match (reqwest::Url::parse(url), reqwest::Url::parse(server)) {
        (Ok(u), Ok(s)) => {
            u.scheme() == s.scheme() && u.host_str() == s.host_str() && u.port() == s.port()
        }
        _ => false,
    }
}

/// Where to ask for `url` on `server`, in order: for an upload, the API's
/// address for it (a token is taken there), then the address itself.
pub fn tries(url: &str, server: &str) -> Vec<String> {
    let server = server.trim_end_matches('/');
    let mut out = Vec::new();
    if let Some(path) = url.strip_prefix(server)
        && let Some((project, file)) = path.split_once("/uploads/")
    {
        let project = project.trim_start_matches('/').trim_end_matches("/-");
        let id = match project.strip_prefix("-/project/") {
            Some(id) => id.to_string(),
            None => project.replace('/', "%2F"),
        };
        if !id.is_empty() {
            out.push(format!("{server}/api/v4/projects/{id}/uploads/{file}"));
        }
    }
    out.push(url.to_string());
    out
}

/// How the kit fetches a description's picture: on `server`, with the token
/// `spec` resolves to (asked off the event loop: `glab` runs a program);
/// anywhere else, without it.
pub fn fetch(server: String, spec: String) -> Fetch {
    radar_core::ui::image::fetch(move |src: String| {
        let (server, spec) = (server.clone(), spec.clone());
        async move { picture(&src, &server, &spec).await }
    })
}

async fn picture(src: &str, server: &str, spec: &str) -> Result<Vec<u8>> {
    let http = crate::client::shared_http(server)?;
    if !on_server(src, server) {
        return get(&http, src, None).await;
    }
    let (spec2, server2) = (spec.to_string(), server.to_string());
    let token = tokio::task::spawn_blocking(move || crate::config::resolve_token(&spec2, &server2)).await??;
    radar_core::log::secret(&token);
    let mut last = None;
    for url in tries(src, server) {
        match get(&http, &url, Some(&token)).await {
            Ok(b) => return Ok(b),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("nowhere to ask")))
}

async fn get(http: &reqwest::Client, url: &str, token: Option<&str>) -> Result<Vec<u8>> {
    let mut req = http.get(url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("GitLab answered {status}");
    }
    let kind = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if kind.starts_with("text/html") {
        bail!("not a picture ({kind}): a sign-in page");
    }
    Ok(resp.bytes().await?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: &str = "https://git.example";

    #[test]
    fn an_upload_is_the_projects_and_a_path_the_servers() {
        assert_eq!(
            whole("/uploads/abc/shot.png", SERVER, "acme/team-a/api"),
            "https://git.example/acme/team-a/api/uploads/abc/shot.png"
        );
        assert_eq!(whole("/-/project/7/uploads/abc/s.png", "https://git.example/", "x"), "https://git.example/-/project/7/uploads/abc/s.png");
        assert_eq!(whole("https://img.example/a.png", SERVER, "x"), "https://img.example/a.png");
        assert_eq!(whole("//cdn.example/a.png", SERVER, "x"), "//cdn.example/a.png");
        let mut doc = radar_core::ui::markdown::parse("![shot](/uploads/abc/shot.png)\n\nsee [log](/uploads/def/log.txt)");
        make_whole(&mut doc, SERVER, "acme/api");
        let Block::Media(m) = &doc.blocks[0] else { panic!("{doc:?}") };
        assert_eq!(m.src, "https://git.example/acme/api/uploads/abc/shot.png");
        assert_eq!(m.url, m.src);
        let Block::Para { text } = &doc.blocks[1] else { panic!() };
        assert_eq!(text[1].kind, Kind::Link { url: "https://git.example/acme/api/uploads/def/log.txt".into() });
    }

    #[test]
    fn an_upload_is_asked_of_the_api_first_and_the_token_stays_on_the_server() {
        assert_eq!(
            tries("https://git.example/acme/team-a/api/uploads/abc/shot.png", SERVER),
            [
                "https://git.example/api/v4/projects/acme%2Fteam-a%2Fapi/uploads/abc/shot.png",
                "https://git.example/acme/team-a/api/uploads/abc/shot.png"
            ]
        );
        assert_eq!(
            tries("https://git.example/-/project/7/uploads/abc/s.png", SERVER),
            ["https://git.example/api/v4/projects/7/uploads/abc/s.png", "https://git.example/-/project/7/uploads/abc/s.png"]
        );
        assert_eq!(tries("https://git.example/acme/a.png", SERVER), ["https://git.example/acme/a.png"]);
        assert!(on_server("https://git.example/x", SERVER));
        assert!(!on_server("https://git.example.evil/x", SERVER));
        assert!(!on_server("http://git.example/x", SERVER));
        assert!(!on_server("https://img.example/a.png", SERVER));
    }
}
