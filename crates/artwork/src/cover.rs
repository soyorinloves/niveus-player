//! Getting a cover from a URL to a frame.

use crate::error::ArtworkError;
use crate::frame::Frame;
use crate::geometry::{fit, Size};
use crate::palette::ColourDepth;
use crate::render::{draw, ArtworkMode, Samples};
use std::sync::OnceLock;
use std::time::Duration;

/// How long a cover is given to arrive before the attempt is abandoned.
///
/// Nothing waits on it — the pane stays empty until a picture is ready — but a
/// request that never times out would hold a task for the life of the program.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The largest cover worth reading into memory.
///
/// A cover is displayed in a few dozen cells; anything beyond a couple of
/// megapixels is waste, and an unbounded read is a way to be starved.
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// How a cover is drawn, beyond where it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    pub mode: ArtworkMode,
    /// What the terminal can show. Passed in rather than sensed here: this
    /// crate does not read the environment.
    pub depth: ColourDepth,
}

/// Everything that makes one rendered cover different from another.
///
/// The same combination asked for twice is expected to be answered from what
/// has already been drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverKey {
    pub url: String,
    pub area: Size,
    pub options: Options,
}

impl CoverKey {
    pub fn new(url: impl Into<String>, area: Size, options: Options) -> Self {
        Self {
            url: url.into(),
            area,
            options,
        }
    }
}

/// Keeps the last cover drawn.
///
/// One entry is enough because the things that change what a cover looks like
/// are exactly the things in its key: another track, another size, another
/// colour depth. Anything else would draw the same picture again.
#[derive(Debug, Default)]
pub struct Cache {
    entry: Option<(CoverKey, Frame)>,
}

impl Cache {
    /// The cover for this key, if it is the one already drawn.
    pub fn get(&self, key: &CoverKey) -> Option<Frame> {
        match &self.entry {
            Some((cached, frame)) if cached == key => Some(frame.clone()),
            _ => None,
        }
    }

    pub fn put(&mut self, key: CoverKey, frame: Frame) {
        self.entry = Some((key, frame));
    }
}

/// Fetches a cover and draws it.
pub async fn load(key: &CoverKey) -> Result<Frame, ArtworkError> {
    let bytes = fetch(&key.url).await?;
    render_bytes(&bytes, key.area, key.options)
}

/// Draws an image that has already been fetched.
///
/// Kept apart from fetching so the drawing can be exercised without a network.
pub fn render_bytes(bytes: &[u8], area: Size, options: Options) -> Result<Frame, ArtworkError> {
    let picture = image::load_from_memory(bytes)?;
    let pixels = (picture.width(), picture.height());

    let Some(cells) = fit(pixels, area) else {
        // Not enough room for a picture to say anything. An empty frame is the
        // honest answer: the design asks for blank space, not a block of colour.
        return Ok(Frame::new(0, 0));
    };

    let mode = options.mode.resolve(options.depth);
    let samples = sample(
        &picture,
        cells.samples(mode.samples_per_cell()),
        options.depth,
    );
    Ok(draw(&samples, cells, mode, options.depth))
}

/// Reduces the picture to one colour per sample point.
fn sample(
    picture: &image::DynamicImage,
    (width, height): (u16, u16),
    depth: ColourDepth,
) -> Samples {
    if width == 0 || height == 0 {
        return Samples::new(0, 0, Vec::new());
    }

    let resized = picture.resize_exact(
        u32::from(width),
        u32::from(height),
        image::imageops::FilterType::Triangle,
    );

    if !depth.has_colour() {
        // One channel is all the character ramp needs, and averaging a single
        // channel keeps the shading honest.
        let grey = resized.into_luma8();
        let colours = grey
            .pixels()
            .map(|pixel| (pixel[0], pixel[0], pixel[0]))
            .collect();
        return Samples::new(width, height, colours);
    }

    let rgba = resized.into_rgba8();
    let colours = rgba
        .pixels()
        .map(|pixel| {
            let [r, g, b, a] = pixel.0;
            // A transparent pixel has no colour of its own, and the terminal's
            // real background cannot be asked for, so it is taken as black.
            let scale = u16::from(a) + 1;
            (
                ((u16::from(r) * scale) >> 8) as u8,
                ((u16::from(g) * scale) >> 8) as u8,
                ((u16::from(b) * scale) >> 8) as u8,
            )
        })
        .collect();
    Samples::new(width, height, colours)
}

/// Downloads a cover.
async fn fetch(url: &str) -> Result<Vec<u8>, ArtworkError> {
    if url.trim().is_empty() {
        return Err(ArtworkError::EmptyUrl);
    }

    let response = client().get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(ArtworkError::Status(status.as_u16()));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_BYTES as u64 {
            return Err(ArtworkError::TooLarge(length as usize));
        }
    }

    let bytes = response.bytes().await?;
    if bytes.len() > MAX_BYTES {
        return Err(ArtworkError::TooLarge(bytes.len()));
    }
    Ok(bytes.to_vec())
}

/// How Niveus introduces itself to a cover host.
///
/// Several image hosts refuse a request that does not say who it is, so this is
/// not merely polite: without it some covers simply cannot be fetched.
const USER_AGENT: &str = concat!("niveus/", env!("CARGO_PKG_VERSION"));

/// The client, built once and reused so connections are not re-established for
/// every cover.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_else(|error| {
                // The fallback loses the user agent, which some hosts refuse a
                // request for, so it is worth knowing this happened.
                tracing::warn!(%error, "cannot build the cover client; using a plain one");
                reqwest::Client::new()
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG of one flat colour, written by hand so the test needs no fixture.
    fn flat_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut buffer = image::RgbImage::new(width, height);
        for pixel in buffer.pixels_mut() {
            *pixel = image::Rgb(rgb);
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(buffer)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn options(mode: ArtworkMode, depth: ColourDepth) -> Options {
        Options { mode, depth }
    }

    #[test]
    fn a_cover_is_drawn_at_the_size_its_shape_allows() {
        let png = flat_png(600, 600, [200, 40, 40]);
        let frame = render_bytes(
            &png,
            Size::new(40, 12),
            options(ArtworkMode::HalfBlock, ColourDepth::TrueColor),
        )
        .unwrap();

        assert_eq!((frame.width(), frame.height()), (24, 12));
        // Every cell carries the picture's colour, top and bottom.
        let cell = frame.cell(0, 0).unwrap();
        assert_eq!(cell.symbol, '▀');
        assert!(matches!(cell.fg, crate::frame::Ink::Rgb(_, _, _)));
    }

    #[test]
    fn the_character_ramp_draws_the_same_shape_as_the_half_block() {
        let png = flat_png(600, 600, [128, 128, 128]);
        let area = Size::new(40, 12);

        let blocks = render_bytes(
            &png,
            area,
            options(ArtworkMode::HalfBlock, ColourDepth::TrueColor),
        )
        .unwrap();
        let characters = render_bytes(
            &png,
            area,
            options(ArtworkMode::Characters, ColourDepth::TrueColor),
        )
        .unwrap();

        // The same cells either way. Drawing the ramp on the half-block sample
        // grid would make it twice as tall.
        assert_eq!(
            (blocks.width(), blocks.height()),
            (characters.width(), characters.height())
        );
        assert_eq!((blocks.width(), blocks.height()), (24, 12));
    }

    #[test]
    fn an_area_too_small_for_a_picture_gives_nothing() {
        let png = flat_png(64, 64, [10, 10, 10]);
        let frame = render_bytes(
            &png,
            Size::new(3, 10),
            options(ArtworkMode::HalfBlock, ColourDepth::TrueColor),
        )
        .unwrap();
        assert!(frame.is_empty());
    }

    #[test]
    fn something_that_is_not_an_image_is_an_error_rather_than_a_blank() {
        let error = render_bytes(
            b"this is not a picture",
            Size::new(40, 12),
            options(ArtworkMode::HalfBlock, ColourDepth::TrueColor),
        )
        .unwrap_err();
        assert!(matches!(error, ArtworkError::Decode(_)), "{error:?}");
    }

    #[test]
    fn a_cover_without_colour_is_drawn_with_the_ramp() {
        // Half-blocks with no colour are a solid block; the mode has to give
        // way on its own.
        let png = flat_png(64, 64, [255, 255, 255]);
        let frame = render_bytes(
            &png,
            Size::new(20, 10),
            options(ArtworkMode::HalfBlock, ColourDepth::Monochrome),
        )
        .unwrap();

        let cell = frame.cell(0, 0).unwrap();
        assert_eq!(cell.symbol, '@', "the ramp was not used");
        assert_eq!(cell.fg, crate::frame::Ink::None);
    }

    #[test]
    fn a_cache_answers_only_for_what_it_holds() {
        let mut cache = Cache::default();
        let key = CoverKey::new(
            "https://example.invalid/a.png",
            Size::new(24, 12),
            options(ArtworkMode::HalfBlock, ColourDepth::TrueColor),
        );
        assert!(cache.get(&key).is_none());

        let frame = Frame::new(24, 12);
        cache.put(key.clone(), frame.clone());
        assert_eq!(cache.get(&key), Some(frame));

        // A different size, a different colour depth or another cover is not
        // what was drawn.
        let mut other = key.clone();
        other.area = Size::new(12, 6);
        assert!(cache.get(&other).is_none());
        let mut other = key.clone();
        other.options.depth = ColourDepth::Colors256;
        assert!(cache.get(&other).is_none());
        let other = CoverKey::new(
            "https://example.invalid/b.png",
            Size::new(24, 12),
            key.options,
        );
        assert!(cache.get(&other).is_none());

        // Only what was put in comes back out; nothing else does.
        assert!(cache
            .get(&CoverKey::new(
                "https://example.invalid/c.png",
                Size::new(24, 12),
                key.options,
            ))
            .is_none());
    }
}
