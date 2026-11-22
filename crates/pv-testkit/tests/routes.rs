//! Each route behaves as documented. These tests use a real HTTP client (reqwest,
//! HTTP/1.1, no automatic decompression) against a server on an ephemeral port.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::io::Read as _;
use std::time::{Duration, Instant};

use pv_testkit::{Fixture, TestServer, sha256_hex};
use reqwest::{Client, StatusCode, header};

const NAME: &str = "data.bin";
const SIZE: usize = 100_000;

/// (Range header, expected status, expected inclusive slice for 206)
type Case = (&'static str, StatusCode, Option<(usize, usize)>);

fn fixture() -> Fixture {
    Fixture::random(NAME, SIZE, 42)
}

async fn server() -> (TestServer, Fixture) {
    let f = fixture();
    (TestServer::start([f.clone()]).await.unwrap(), f)
}

fn client() -> Client {
    Client::builder().http1_only().no_proxy().build().unwrap()
}

async fn get(url: &str, headers: &[(&str, &str)]) -> reqwest::Response {
    let mut req = client().get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

fn hdr<'a>(resp: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    resp.headers().get(name).and_then(|v| v.to_str().ok())
}

/// Reads a body until EOF or error; returns (bytes received, whether it errored).
async fn read_until_error(mut resp: reqwest::Response) -> (Vec<u8>, bool) {
    let mut got = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(c)) => got.extend_from_slice(&c),
            Ok(None) => return (got, false),
            Err(_) => return (got, true),
        }
    }
}

#[tokio::test]
async fn file_serves_full_body_with_validators() {
    let (s, f) = server().await;
    let resp = get(&s.url("/file/data.bin"), &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(hdr(&resp, "accept-ranges"), Some("bytes"));
    assert_eq!(hdr(&resp, "content-length"), Some("100000"));
    let etag = hdr(&resp, "etag").unwrap().to_owned();
    assert!(
        etag.starts_with('"') && !etag.starts_with("W/"),
        "strong etag: {etag}"
    );
    assert!(hdr(&resp, "last-modified").unwrap().ends_with("GMT"));
    assert_eq!(sha256_hex(&resp.bytes().await.unwrap()), f.sha256);
}

#[tokio::test]
async fn file_range_edge_cases() {
    let (s, f) = server().await;
    let url = s.url("/file/data.bin");
    let cases: &[Case] = &[
        ("bytes=0-0", StatusCode::PARTIAL_CONTENT, Some((0, 0))),
        (
            "bytes=1000-1999",
            StatusCode::PARTIAL_CONTENT,
            Some((1000, 1999)),
        ),
        (
            "bytes=99990-",
            StatusCode::PARTIAL_CONTENT,
            Some((99_990, 99_999)),
        ),
        (
            "bytes=-10",
            StatusCode::PARTIAL_CONTENT,
            Some((99_990, 99_999)),
        ),
        (
            "bytes=99999-200000",
            StatusCode::PARTIAL_CONTENT,
            Some((99_999, 99_999)),
        ),
        ("bytes=100000-", StatusCode::RANGE_NOT_SATISFIABLE, None),
        ("bytes=5-2", StatusCode::OK, None),
        ("bytes=0-1,5-6", StatusCode::OK, None),
    ];
    for &(range, status, slice) in cases {
        let resp = get(&url, &[("range", range)]).await;
        assert_eq!(resp.status(), status, "{range}");
        match (status, slice) {
            (StatusCode::PARTIAL_CONTENT, Some((lo, hi))) => {
                let expected_cr = format!("bytes {lo}-{hi}/{SIZE}");
                assert_eq!(hdr(&resp, "content-range"), Some(expected_cr.as_str()));
                assert_eq!(
                    resp.bytes().await.unwrap(),
                    f.bytes.slice(lo..=hi),
                    "{range}"
                );
            }
            (StatusCode::RANGE_NOT_SATISFIABLE, _) => {
                assert_eq!(hdr(&resp, "content-range"), Some("bytes */100000"));
            }
            _ => assert_eq!(resp.bytes().await.unwrap().len(), SIZE, "{range} ignored"),
        }
    }
}

#[tokio::test]
async fn if_range_honors_only_matching_strong_validators() {
    let (s, _f) = server().await;
    let url = s.url("/file/data.bin");
    let first = get(&url, &[]).await;
    let etag = hdr(&first, "etag").unwrap().to_owned();
    let lm = hdr(&first, "last-modified").unwrap().to_owned();
    let weak = format!("W/{etag}");

    for (if_range, expected) in [
        (etag.as_str(), StatusCode::PARTIAL_CONTENT),
        (lm.as_str(), StatusCode::PARTIAL_CONTENT),
        ("\"something-else\"", StatusCode::OK),
        (weak.as_str(), StatusCode::OK),
    ] {
        let resp = get(&url, &[("range", "bytes=10-19"), ("if-range", if_range)]).await;
        assert_eq!(resp.status(), expected, "If-Range: {if_range}");
    }
}

#[tokio::test]
async fn head_returns_headers_only() {
    let (s, _f) = server().await;
    let resp = client().head(s.url("/file/data.bin")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(hdr(&resp, "content-length"), Some("100000"));
    assert!(resp.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn unknown_fixture_is_404() {
    let (s, _f) = server().await;
    for route in [
        "/file/nope",
        "/norange/nope",
        "/throttle/nope?bps=1",
        "/flaky/nope",
        "/hang/nope",
        "/disposition/nope",
        "/mutable/nope",
        "/gzip/nope",
    ] {
        assert_eq!(
            get(&s.url(route), &[]).await.status(),
            StatusCode::NOT_FOUND,
            "{route}"
        );
    }
    let resp = client()
        .post(s.url("/control/mutate/nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(s.mutate("nope").is_none());
}

#[tokio::test]
async fn norange_ignores_range() {
    let (s, f) = server().await;
    let resp = get(&s.url("/norange/data.bin"), &[("range", "bytes=0-9")]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(hdr(&resp, "accept-ranges"), Some("none"));
    assert!(hdr(&resp, "content-range").is_none());
    assert_eq!(sha256_hex(&resp.bytes().await.unwrap()), f.sha256);
}

#[tokio::test]
async fn throttle_paces_each_response_and_supports_ranges() {
    let (s, f) = server().await;
    // 50 KB at 200 KB/s should take ~250 ms.
    let started = Instant::now();
    let resp = get(
        &s.url("/throttle/data.bin?bps=200000"),
        &[("range", "bytes=0-49999")],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(resp.bytes().await.unwrap(), f.bytes.slice(0..50_000));
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(200),
        "too fast: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(3), "too slow: {elapsed:?}");

    // Two parallel connections each get the full rate (per-connection cap).
    let throttled = s.url("/throttle/data.bin?bps=200000");
    let started = Instant::now();
    let (a, b) = tokio::join!(
        get(&throttled, &[("range", "bytes=0-49999")]),
        get(&throttled, &[("range", "bytes=50000-99999")]),
    );
    let (a, b) = (a.bytes().await.unwrap(), b.bytes().await.unwrap());
    assert_eq!([a, b].concat(), f.bytes.to_vec());
    assert!(
        started.elapsed() < Duration::from_millis(450),
        "{:?}",
        started.elapsed()
    );

    assert_eq!(
        get(&s.url("/throttle/data.bin?bps=0"), &[]).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&s.url("/throttle/data.bin"), &[]).await.status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn flaky_fail_rate_extremes() {
    let (s, f) = server().await;
    let always = get(&s.url("/flaky/data.bin?fail_rate=1"), &[]).await;
    assert_eq!(always.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(hdr(&always, "retry-after"), Some("0"));
    let never = get(&s.url("/flaky/data.bin?fail_rate=0"), &[]).await;
    assert_eq!(never.status(), StatusCode::OK);
    assert_eq!(sha256_hex(&never.bytes().await.unwrap()), f.sha256);
}

#[tokio::test]
async fn flaky_drop_after_truncates_the_connection() {
    let (s, f) = server().await;
    let resp = get(
        &s.url("/flaky/data.bin?drop_after=30000"),
        &[("range", "bytes=1000-")],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        hdr(&resp, "content-length"),
        Some("99000"),
        "promises the full range"
    );
    let (got, errored) = read_until_error(resp).await;
    assert!(errored, "client must observe an aborted body");
    assert_eq!(
        got,
        f.bytes.slice(1000..31_000),
        "exactly drop_after bytes, correct offset"
    );

    // drop_rate=0 disables drops.
    let resp = get(&s.url("/flaky/data.bin?drop_after=10&drop_rate=0"), &[]).await;
    assert_eq!(read_until_error(resp).await, (f.bytes.to_vec(), false));
}

#[tokio::test]
async fn flaky_is_deterministic_for_a_seed() {
    async fn statuses(seed: u64) -> Vec<u16> {
        let (s, _f) = server().await;
        let mut out = Vec::new();
        for _ in 0..30 {
            let url = s.url(&format!("/flaky/data.bin?fail_rate=0.5&seed={seed}"));
            out.push(client().head(url).send().await.unwrap().status().as_u16());
        }
        out
    }
    let a = statuses(7).await;
    assert_eq!(a, statuses(7).await, "same seed, same sequence");
    assert_ne!(a, statuses(8).await, "different seed, different sequence");
    assert!(
        a.contains(&200) && a.contains(&503),
        "mix of outcomes: {a:?}"
    );
}

#[tokio::test]
async fn hang_stalls_without_closing_and_shutdown_releases_it() {
    let (s, f) = server().await;
    let c = Client::builder()
        .http1_only()
        .no_proxy()
        .read_timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    let mut resp = c
        .get(s.url("/hang/data.bin?after=20000"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut got = Vec::new();
    let err = loop {
        match resp.chunk().await {
            Ok(Some(c)) => got.extend_from_slice(&c),
            Ok(None) => panic!("body must not complete"),
            Err(e) => break e,
        }
    };
    assert!(err.is_timeout(), "idle timeout, not a close: {err}");
    assert_eq!(got, f.bytes.slice(0..20_000));

    // A second, still-hanging request must not block shutdown.
    let pending = c.get(s.url("/hang/data.bin?after=0")).send().await.unwrap();
    let started = Instant::now();
    s.shutdown().await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    drop(pending);
}

#[tokio::test]
async fn redirect_chain_ends_at_file() {
    let (s, f) = server().await;
    let resp = get(&s.url("/redirect/3/data.bin"), &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.url().path(), "/file/data.bin");
    assert_eq!(sha256_hex(&resp.bytes().await.unwrap()), f.sha256);
    let paths: Vec<String> = s.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        [
            "/redirect/3/data.bin",
            "/redirect/2/data.bin",
            "/redirect/1/data.bin",
            "/redirect/0/data.bin",
            "/file/data.bin"
        ]
    );

    let no_follow = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let hop = no_follow
        .get(s.url("/redirect/1/data.bin"))
        .send()
        .await
        .unwrap();
    assert_eq!(hop.status(), StatusCode::FOUND);
    assert_eq!(hdr(&hop, "location"), Some("/redirect/0/data.bin"));
    assert_eq!(
        get(&s.url("/redirect/51/data.bin"), &[]).await.status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn disposition_echoes_header() {
    let (s, f) = server().await;
    let cd = "attachment; filename*=UTF-8''na%C3%AFve%20file.txt";
    let url =
        reqwest::Url::parse_with_params(&s.url("/disposition/data.bin"), &[("cd", cd)]).unwrap();
    let resp = get(url.as_str(), &[]).await;
    assert_eq!(hdr(&resp, header::CONTENT_DISPOSITION.as_str()), Some(cd));
    assert_eq!(sha256_hex(&resp.bytes().await.unwrap()), f.sha256);

    let default = get(&s.url("/disposition/data.bin"), &[("range", "bytes=0-0")]).await;
    assert_eq!(
        default.status(),
        StatusCode::PARTIAL_CONTENT,
        "ranges still work"
    );
    assert_eq!(
        hdr(&default, "content-disposition"),
        Some("attachment; filename=\"data.bin\"")
    );

    let bad = reqwest::Url::parse_with_params(&s.url("/disposition/data.bin"), &[("cd", "a\nb")])
        .unwrap();
    assert_eq!(
        get(bad.as_str(), &[]).await.status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn mutable_changes_content_and_etag() {
    let (s, f) = server().await;
    let url = s.url("/mutable/data.bin");
    let v0 = get(&url, &[]).await;
    let etag0 = hdr(&v0, "etag").unwrap().to_owned();
    let lm0 = hdr(&v0, "last-modified").unwrap().to_owned();
    assert_eq!(sha256_hex(&v0.bytes().await.unwrap()), f.sha256);

    let resp = client()
        .post(s.url("/control/mutate/data.bin"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let new_sha = resp.text().await.unwrap();
    assert_ne!(new_sha, f.sha256);

    let v1 = get(&url, &[]).await;
    assert_ne!(hdr(&v1, "etag").unwrap(), etag0);
    assert_ne!(hdr(&v1, "last-modified").unwrap(), lm0);
    assert_eq!(
        hdr(&v1, "content-length"),
        Some("100000"),
        "same size: only validators reveal the change"
    );
    assert_eq!(sha256_hex(&v1.bytes().await.unwrap()), new_sha);

    // A resume with the old validator gets the whole new file, not a mixed one.
    let resumed = get(&url, &[("range", "bytes=500-"), ("if-range", &etag0)]).await;
    assert_eq!(resumed.status(), StatusCode::OK);
    assert_eq!(sha256_hex(&resumed.bytes().await.unwrap()), new_sha);

    // The method form mutates again; /file is never affected.
    let v2 = s.mutate(NAME).unwrap();
    assert_ne!(v2.sha256, new_sha);
    assert_eq!(
        sha256_hex(&get(&url, &[]).await.bytes().await.unwrap()),
        v2.sha256
    );
    assert_eq!(
        sha256_hex(
            &get(&s.url("/file/data.bin"), &[])
                .await
                .bytes()
                .await
                .unwrap()
        ),
        f.sha256
    );
}

#[tokio::test]
async fn gzip_compresses_unless_identity_requested() {
    let (s, f) = server().await;
    let url = s.url("/gzip/data.bin");

    let compressed = get(&url, &[("range", "bytes=0-9")]).await;
    assert_eq!(
        compressed.status(),
        StatusCode::OK,
        "range ignored when compressing"
    );
    assert_eq!(hdr(&compressed, "content-encoding"), Some("gzip"));
    let gz = compressed.bytes().await.unwrap();
    let mut plain = Vec::new();
    flate2::read::GzDecoder::new(&gz[..])
        .read_to_end(&mut plain)
        .unwrap();
    assert_eq!(sha256_hex(&plain), f.sha256);

    let identity = get(
        &url,
        &[("accept-encoding", "identity"), ("range", "bytes=0-9")],
    )
    .await;
    assert_eq!(identity.status(), StatusCode::PARTIAL_CONTENT);
    assert!(hdr(&identity, "content-encoding").is_none());
    assert_eq!(identity.bytes().await.unwrap(), f.bytes.slice(0..10));

    let both = get(&url, &[("accept-encoding", "gzip, identity")]).await;
    assert_eq!(hdr(&both, "content-encoding"), Some("gzip"));
}

#[tokio::test]
async fn request_log_records_headers() {
    let (s, _f) = server().await;
    get(
        &s.url("/file/data.bin"),
        &[("range", "bytes=0-0"), ("accept-encoding", "identity")],
    )
    .await;
    let reqs = s.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].header("Range"), Some("bytes=0-0"));
    assert_eq!(reqs[0].header("ACCEPT-ENCODING"), Some("identity"));
    assert_eq!(reqs[0].header("x-missing"), None);
    s.clear_requests();
    assert_eq!(s.requests(), Vec::new());
}

#[tokio::test]
async fn servers_run_in_parallel_and_stop_cleanly() {
    let servers = futures::future::join_all(
        (0..8).map(|i| TestServer::start([Fixture::random(format!("f{i}"), 1000 + i, i as u64)])),
    )
    .await;
    let servers: Vec<TestServer> = servers.into_iter().map(Result::unwrap).collect();
    let mut ports: Vec<u16> = servers.iter().map(|s| s.addr().port()).collect();
    ports.sort_unstable();
    ports.dedup();
    assert_eq!(ports.len(), 8, "distinct ephemeral ports");

    for (i, s) in servers.iter().enumerate() {
        let body = get(&s.url(&format!("/file/f{i}")), &[])
            .await
            .bytes()
            .await
            .unwrap();
        assert_eq!(body.len(), 1000 + i);
    }
    let urls: Vec<String> = servers.iter().map(|s| s.url("/file/f0")).collect();
    for s in servers {
        s.shutdown().await;
    }
    // Short connect timeout: on Windows a refused localhost connect takes ~2 s to fail.
    let quick = Client::builder()
        .connect_timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    let results = futures::future::join_all(urls.iter().map(|u| quick.get(u).send())).await;
    assert!(
        results.iter().all(Result::is_err),
        "a server is still reachable after shutdown"
    );

    // Dropping (instead of shutdown) also stops the server.
    let s = TestServer::start([fixture()]).await.unwrap();
    let url = s.url("/file/data.bin");
    drop(s);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(quick.get(&url).send().await.is_err());
}

#[tokio::test]
async fn fixtures_can_be_added_after_start() {
    let s = TestServer::start([]).await.unwrap();
    assert_eq!(
        get(&s.url("/file/late.bin"), &[]).await.status(),
        StatusCode::NOT_FOUND
    );
    let late = Fixture::from_bytes("late.bin", b"hello".to_vec());
    s.add_fixture(late.clone());
    assert_eq!(
        get(&s.url("/file/late.bin"), &[])
            .await
            .text()
            .await
            .unwrap(),
        "hello"
    );
}
