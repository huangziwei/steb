//! A book's page, parsed for its `.azw3` and cover thumbnail hrefs.
//! Both are read from the markup: a [`DownloadHref`] carries translator and
//! editor segments (`homer_the-iliad_alexander-pope.azw3`).

use super::url::{BadUrl, DownloadHref, ThumbnailHref};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookPage {
    pub title: String,
    pub azw3: DownloadHref,
    pub thumbnail: Option<ThumbnailHref>,
}

#[derive(Debug)]
pub enum ParseError {
    /// No `class="amazon"` link in the markup.
    NoAzw3,
    Url(BadUrl),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NoAzw3 => write!(f, "no Kindle (azw3) download found on the book page"),
            ParseError::Url(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Extract the `href="…"` of the anchor containing `marker`, searching
/// backwards from it to the enclosing `<a`.
fn href_of_anchor_containing(html: &str, marker: &str) -> Option<String> {
    let at = html.find(marker)?;
    let open = html[..at].rfind("<a ")?;
    let tag_end = html[open..].find('>')? + open;
    let tag = &html[open..tag_end];
    let start = tag.find("href=\"")? + "href=\"".len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

pub fn parse(html: &str) -> Result<BookPage, ParseError> {
    // `class="amazon"` marks the Kindle build alone.
    let azw3 = href_of_anchor_containing(html, "class=\"amazon\"")
        .ok_or(ParseError::NoAzw3)
        .and_then(|h| DownloadHref::parse(&h).map_err(ParseError::Url))?;

    let title = html
        .find("<h1 property=\"schema:name\">")
        .map(|i| i + "<h1 property=\"schema:name\">".len())
        .and_then(|start| html[start..].find("</h1>").map(|e| &html[start..start + e]))
        .unwrap_or_default()
        .trim()
        .to_string();

    // `None` where the page carries no `_EBOK_portrait.jpg`.
    let thumbnail = html
        .find("_EBOK_portrait.jpg")
        .and_then(|at| {
            let open = html[..at].rfind("href=\"")? + "href=\"".len();
            let end = html[open..].find('"')? + open;
            Some(html[open..end].to_string())
        })
        .and_then(|h| ThumbnailHref::parse(&h).ok());

    Ok(BookPage {
        title,
        azw3,
        thumbnail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRACULA: &str = include_str!("../../tests/fixtures/book-dracula.html");

    #[test]
    fn finds_the_azw3_among_four_download_formats() {
        let page = parse(DRACULA).unwrap();
        assert_eq!(page.title, "Dracula");
        assert_eq!(page.azw3.file_name(), "bram-stoker_dracula.azw3");
    }

    #[test]
    fn finds_the_kindle_thumbnail() {
        let thumb = parse(DRACULA).unwrap().thumbnail.expect("thumbnail link");
        assert_eq!(
            thumb.file_name(),
            "thumbnail_164eb70ff819bc597b5498008b4d7b86ae66df93_EBOK_portrait.jpg"
        );
    }

    #[test]
    fn epub_only_page_is_an_error_not_a_silent_miss() {
        let html = r#"<a href="/ebooks/a/b/downloads/a_b.epub" class="epub">Compatible epub</a>"#;
        assert!(matches!(parse(html), Err(ParseError::NoAzw3)));
    }

    #[test]
    fn a_book_with_no_thumbnail_still_downloads() {
        let html = r#"<h1 property="schema:name">X</h1>
            <a href="/ebooks/a/b/downloads/a_b.azw3" class="amazon">azw3</a>"#;
        let page = parse(html).unwrap();
        assert!(page.thumbnail.is_none());
        assert_eq!(page.azw3.file_name(), "a_b.azw3");
    }
}
