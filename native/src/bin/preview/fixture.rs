//! The data every shot is drawn from. No network, no cache, no device: a
//! fixed set of books, subjects and covers, so two runs of the same shot are
//! byte-identical and a geometry change is the only thing a diff can show.

use image::{DynamicImage, Rgb, RgbImage};

use steb_native::se::listing::Hit;
use steb_native::se::url::{BookPath, CoverHref};

/// Title, author, and the slug both URLs are built from.
const BOOKS: &[(&str, &str, &str)] = &[
    (
        "The Time Machine",
        "H. G. Wells",
        "h-g-wells_the-time-machine",
    ),
    ("Middlemarch", "George Eliot", "george-eliot_middlemarch"),
    ("Moby Dick", "Herman Melville", "herman-melville_moby-dick"),
    ("The Iliad", "Homer", "homer_the-iliad_alexander-pope"),
    (
        "Sonnets",
        "William Shakespeare",
        "william-shakespeare_sonnets",
    ),
    ("Dracula", "Bram Stoker", "bram-stoker_dracula"),
    (
        "Walden",
        "Henry David Thoreau",
        "henry-david-thoreau_walden",
    ),
    ("Ulysses", "James Joyce", "james-joyce_ulysses"),
    ("Persuasion", "Jane Austen", "jane-austen_persuasion"),
    (
        "Kokoro",
        "Natsume Sōseki",
        "natsume-soseki_kokoro_edwin-mcclellan",
    ),
    (
        "The Republic",
        "Plato",
        "plato_the-republic_benjamin-jowett",
    ),
    (
        "Leaves of Grass",
        "Walt Whitman",
        "walt-whitman_leaves-of-grass",
    ),
    (
        "A Room With a View",
        "E. M. Forster",
        "e-m-forster_a-room-with-a-view",
    ),
    (
        "The Trial",
        "Franz Kafka",
        "franz-kafka_the-trial_david-wyllie",
    ),
    ("Nostromo", "Joseph Conrad", "joseph-conrad_nostromo"),
    ("Erewhon", "Samuel Butler", "samuel-butler_erewhon"),
    (
        "The Age of Innocence",
        "Edith Wharton",
        "edith-wharton_the-age-of-innocence",
    ),
    ("Anabasis", "Xenophon", "xenophon_anabasis_h-g-dakyns"),
];

/// The `<select name="tags[]">` vocabulary, enough of it to page the menu.
pub const TAGS: &[&str] = &[
    "Adventure",
    "Autobiography",
    "Biography",
    "Childrens",
    "Comedy",
    "Drama",
    "Fantasy",
    "Fiction",
    "Gothic",
    "Historical fiction",
    "Horror",
    "Memoir",
    "Mystery",
    "Nonfiction",
    "Philosophy",
    "Poetry",
    "Romance",
    "Satire",
    "Science fiction",
    "Shorts",
    "Spirituality",
    "Travel",
];

/// `n` books, repeating the list where `n` runs past it so a wide panel still
/// fills its grid.
pub fn hits(n: usize) -> Vec<Hit> {
    (0..n)
        .map(|i| {
            let (title, author, slug) = BOOKS[i % BOOKS.len()];
            // Past one pass, the title carries its round: a repeated cell
            // stays distinguishable in a shot.
            let title = match i / BOOKS.len() {
                0 => title.to_string(),
                round => format!("{title} ({})", round + 1),
            };
            // Stands in for a content hash; it only has to differ per book.
            let sha = format!("{:08x}", (i as u32).wrapping_mul(0x9E37_79B1));
            Hit {
                path: BookPath::parse(&format!("/ebooks/{}", slug.replace('_', "/")))
                    .expect("fixture path"),
                title,
                author: author.to_string(),
                cover: CoverHref::parse(&format!("/images/covers/{slug}/{sha}/cover@2x.jpg"))
                    .expect("fixture cover"),
            }
        })
        .collect()
}

/// A stand-in cover: a plate in one of four greys with a lighter band across
/// the lower third, at Standard Ebooks' 2:3. The point is the *shape* a cell
/// letterboxes, not the art.
pub fn cover(i: usize, w: u32, h: u32) -> DynamicImage {
    let shade = [0x3A, 0x55, 0x72, 0x8E][i % 4];
    let mut img = RgbImage::from_pixel(w.max(1), h.max(1), Rgb([shade, shade, shade]));
    let band = h * 68 / 100;
    for y in band..h {
        for x in 0..w {
            img.put_pixel(x, y, Rgb([0xE8, 0xE8, 0xE8]));
        }
    }
    DynamicImage::ImageRgb8(img)
}
