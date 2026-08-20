//! Live checks against standardebooks.org, every one `#[ignore]`. Run them
//! with `cargo test -p steb-native --test live -- --ignored
//! --test-threads=1`.

use steb_native::se::url::{Endpoint, Listing};
use steb_native::se::{book, feed, http, listing};

fn client() -> http::Client {
    http::Client::new()
}

#[test]
#[ignore = "hits standardebooks.org"]
fn tls_handshake_succeeds_with_the_rustcrypto_provider() {
    let page = client()
        .text(&Endpoint::Listing(Listing::default()))
        .expect("TLS handshake + GET /ebooks");
    assert!(
        page.contains("schema:Book"),
        "connected but got something that is not a listing page"
    );
}

#[test]
#[ignore = "hits standardebooks.org"]
fn the_opening_screen_still_parses() {
    let html = client()
        .text(&Endpoint::Listing(Listing::default()))
        .expect("GET /ebooks");
    let page = listing::parse(&html).expect("parse listing");

    assert!(!page.hits.is_empty(), "bare /ebooks should list books");
    // The bounds below are set against `per-page=48`.
    assert!(
        page.total_pages > 10,
        "expected a multi-page catalogue, got {} pages",
        page.total_pages
    );
    assert!(
        page.total_pages < 100,
        "{} pages at 48/page implies a catalogue far larger than SE's — \
         total_pages is probably matching `per-page=` again",
        page.total_pages
    );
    assert!(page.has_next, "page 1 of the catalogue has a next page");
    assert!(
        !page.tags.is_empty(),
        "subject vocabulary drives the filter menu"
    );
    for hit in &page.hits {
        assert!(!hit.title.is_empty());
        assert!(!hit.author.is_empty());
    }
}

#[test]
#[ignore = "hits standardebooks.org"]
fn a_book_page_still_yields_an_azw3() {
    let html = client()
        .text(&Endpoint::Listing(Listing {
            query: Some("dracula".into()),
            ..Default::default()
        }))
        .expect("search");
    let hit = listing::parse(&html)
        .expect("parse")
        .hits
        .into_iter()
        .next()
        .expect("at least one hit for `dracula`");

    let page_html = client()
        .text(&Endpoint::Book(hit.path.clone()))
        .expect("GET book page");
    let parsed = book::parse(&page_html).expect("book page should carry an azw3");
    assert!(parsed.azw3.file_name().ends_with(".azw3"));
}

#[test]
#[ignore = "hits standardebooks.org"]
fn the_feed_answers_a_conditional_request_with_304() {
    // A conditional request carrying a validator answers 304 with no body.
    let c = client();
    let first = c
        .text_if_modified(&Endpoint::Feed, &http::Validators::default())
        .expect("first feed fetch");
    let validators = match first {
        http::Fresh::Changed { body, validators } => {
            assert!(!feed::parse(&body).is_empty(), "feed should carry entries");
            validators
        }
        http::Fresh::Unchanged => panic!("no validators were sent, so 304 is impossible"),
    };
    assert!(
        !validators.is_empty(),
        "SE must send ETag/Last-Modified for the 304 path to work"
    );

    match c
        .text_if_modified(&Endpoint::Feed, &validators)
        .expect("conditional feed fetch")
    {
        http::Fresh::Unchanged => {}
        http::Fresh::Changed { .. } => {
            panic!("replaying validators should have produced 304")
        }
    }
}
