//! `[github] api`: where requests go, and what a response may point at.

use super::*;

/// Answers every request with `body`, and keeps what was asked and whether a
/// token went with it.
struct Recorder {
    body: Value,
    asked: Mutex<Vec<(String, bool)>>,
}
impl Recorder {
    fn new(body: Value) -> Arc<Self> {
        Arc::new(Self {
            body,
            asked: Mutex::default(),
        })
    }
    fn urls(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|a| a.0.clone())
            .collect()
    }
}
impl Transport for Recorder {
    fn send(&self, request: Request) -> ResponseFuture<'_> {
        Box::pin(async move {
            self.asked
                .lock()
                .unwrap()
                .push((request.url, request.authorization.is_some()));
            Ok(response(200, self.body.clone()))
        })
    }
}

const TOKEN: &str = "github_pat_synthetic_token_123";

fn client_at(api: &str, body: Value) -> (Client, Arc<Recorder>) {
    let recorder = Recorder::new(body);
    let api = Endpoint::parse(api).unwrap();
    (
        Client::with_endpoint(&[id("org/repo")], TOKEN, api, recorder.clone()).unwrap(),
        recorder,
    )
}

#[test]
fn the_default_api_is_github_com_and_builds_the_urls_it_always_did() {
    let api = Endpoint::default();
    assert_eq!(crate::config::DEFAULT_API, "https://api.github.com");
    assert_eq!(api.rest(), "https://api.github.com");
    assert_eq!(api.graphql(), "https://api.github.com/graphql");
    assert!(api.is_https());
    assert_eq!(api.base_path(), "");
}

#[test]
fn a_plain_http_origin_is_accepted_and_builds_rest_and_graphql_urls() {
    let api = Endpoint::parse("http://127.0.0.1:8123/").unwrap();
    assert_eq!(api.rest(), "http://127.0.0.1:8123");
    assert_eq!(api.graphql(), "http://127.0.0.1:8123/graphql");
    assert!(!api.is_https());
}

#[test]
fn enterprise_server_serves_graphql_beside_its_rest_prefix() {
    let api = Endpoint::parse("https://ghe.example/api/v3").unwrap();
    assert_eq!(api.rest(), "https://ghe.example/api/v3");
    assert_eq!(api.graphql(), "https://ghe.example/api/graphql");
    assert_eq!(api.base_path(), "/api/v3");
    let api = Endpoint::parse(" https://ghe.example:8443/api/v3/ ").unwrap();
    assert_eq!(api.graphql(), "https://ghe.example:8443/api/graphql");
}

#[test]
fn an_address_that_could_carry_a_login_or_leave_http_is_refused() {
    for bad in [
        "",
        "api.github.com",
        "ftp://api.github.com",
        "https://user:pw@api.github.com",
        "https://api.github.com?x=1",
        "https://api.github.com#frag",
        "https://api.github.com/%61pi",
        "file:///etc",
    ] {
        assert!(Endpoint::parse(bad).is_err(), "{bad:?}");
    }
}

#[tokio::test]
async fn rest_requests_go_to_the_configured_origin() {
    let (client, seen) = client_at("http://127.0.0.1:8123", json!({"id": 1, "login": "alice"}));
    client.check_connection().await.unwrap();
    assert_eq!(seen.urls(), ["http://127.0.0.1:8123/user"]);
    let seen = Recorder::new(json!({"resources": {"core": {}}}));
    let ghe = Endpoint::parse("https://ghe.example/api/v3").unwrap();
    let public = Client::with_endpoint(&[], "", ghe, seen.clone()).unwrap();
    public.check_connection().await.unwrap();
    assert_eq!(seen.urls(), ["https://ghe.example/api/v3/rate_limit"]);
}

#[tokio::test]
async fn graphql_goes_to_the_configured_graphql_address() {
    let body = json!({"data": {"r0": null}});
    let (client, seen) = client_at("http://127.0.0.1:8123", body.clone());
    client.markers(&[id("org/repo")]).await.unwrap();
    assert_eq!(seen.urls(), ["http://127.0.0.1:8123/graphql"]);
    let (client, seen) = client_at("https://ghe.example/api/v3", body);
    client.markers(&[id("org/repo")]).await.unwrap();
    assert_eq!(seen.urls(), ["https://ghe.example/api/graphql"]);
    assert!(seen.asked.lock().unwrap()[0].1, "the token goes to the API");
}

#[tokio::test]
async fn a_body_goes_only_to_the_configured_graphql_address() {
    let transport = NativeTransport::new(&Endpoint::parse("http://127.0.0.1:9").unwrap()).unwrap();
    for url in [
        "https://api.github.com/graphql",
        "http://127.0.0.1:9/repos/org/repo",
        "http://127.0.0.1:10/graphql",
    ] {
        let result = transport
            .send(Request {
                url: url.into(),
                headers: HeaderMap::new(),
                authorization: None,
                graphql: Some(b"{}".to_vec()),
            })
            .await;
        assert_eq!(
            result.err().unwrap().kind,
            ApiErrorKind::InvalidTarget,
            "{url}"
        );
    }
}

fn link(url: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        LINK,
        HeaderValue::from_str(&format!("<{url}>; rel=\"next\"")).unwrap(),
    );
    headers
}

#[test]
fn a_next_page_must_be_on_the_configured_host_scheme_and_port() {
    let ghe = Endpoint::parse("https://ghe.example/api/v3").unwrap();
    let next = |api: &Endpoint, url: &str| next_page(api, &link(url), "/repos/org/repo/pulls", 1);
    assert_eq!(
        next(
            &ghe,
            "https://ghe.example/api/v3/repos/org/repo/pulls?page=2"
        ),
        Ok(true)
    );
    assert_eq!(
        next(
            &ghe,
            "https://ghe.example/api/v3/repositories/9/pulls?page=2"
        ),
        Ok(true)
    );
    assert_eq!(
        next(
            &ghe,
            "https://ghe.example/api/v3/repositories/9/issues?page=2"
        ),
        Err(()),
        "an alias is for the same suffix only"
    );
    for foreign in [
        // The default host is not the configured one.
        "https://api.github.com/repos/org/repo/pulls?page=2",
        "https://api.github.com/api/v3/repos/org/repo/pulls?page=2",
        "https://ghe.example.evil.test/api/v3/repos/org/repo/pulls?page=2",
        "https://ghe.example@evil.test/api/v3/repos/org/repo/pulls?page=2",
        "http://ghe.example/api/v3/repos/org/repo/pulls?page=2",
        "https://ghe.example:8443/api/v3/repos/org/repo/pulls?page=2",
        // Same host, outside the API's own path.
        "https://ghe.example/repos/org/repo/pulls?page=2",
        "https://ghe.example/api/v3/repos/org/other/pulls?page=2",
    ] {
        assert_eq!(next(&ghe, foreign), Err(()), "{foreign}");
    }
    let local = Endpoint::parse("http://127.0.0.1:8123").unwrap();
    assert_eq!(
        next(&local, "http://127.0.0.1:8123/repos/org/repo/pulls?page=2"),
        Ok(true)
    );
    assert_eq!(
        next(&local, "http://127.0.0.1:8124/repos/org/repo/pulls?page=2"),
        Err(())
    );
    assert_eq!(
        next(&local, "http://localhost:8123/repos/org/repo/pulls?page=2"),
        Err(())
    );
    // And the default still refuses what it always refused.
    let default = Endpoint::default();
    assert_eq!(
        next(
            &default,
            "https://api.github.com/repos/org/repo/pulls?page=2"
        ),
        Ok(true)
    );
    assert_eq!(
        next(
            &default,
            "https://api.github.com:8443/repos/org/repo/pulls?page=2"
        ),
        Err(())
    );
    assert_eq!(
        next(&default, "https://ghe.example/repos/org/repo/pulls?page=2"),
        Err(())
    );
}

#[test]
fn response_addresses_are_recognised_under_the_configured_api_only() {
    let ghe = Endpoint::parse("https://ghe.example/api/v3").unwrap();
    let repo = id("org/repo");
    let status = |api: &Endpoint, host: &str| {
        valid_status_url(
            api,
            &format!("{host}/repos/org/repo/statuses/{SHA}"),
            &repo,
            SHA,
        )
    };
    assert!(status(&ghe, "https://ghe.example/api/v3"));
    assert!(!status(&ghe, "https://api.github.com"));
    assert!(status(&Endpoint::default(), "https://api.github.com"));
    assert!(!status(&Endpoint::default(), "https://ghe.example/api/v3"));
}
