//! What can go wrong on the way from a cover URL to a picture.

#[derive(Debug, thiserror::Error)]
pub enum ArtworkError {
    #[error("cannot download the cover: {0}")]
    Download(#[from] reqwest::Error),
    #[error("the cover URL is empty")]
    EmptyUrl,
    #[error("the cover URL returned HTTP {0}")]
    Status(u16),
    #[error("the cover is too large; giving up ({0} bytes)")]
    TooLarge(usize),
    #[error("cannot decode the cover image: {0}")]
    Decode(#[from] image::ImageError),
}
