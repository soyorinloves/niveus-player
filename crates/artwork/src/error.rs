//! What can go wrong on the way from a cover URL to a picture.

#[derive(Debug, thiserror::Error)]
pub enum ArtworkError {
    #[error("无法下载封面：{0}")]
    Download(#[from] reqwest::Error),
    #[error("封面地址为空")]
    EmptyUrl,
    #[error("封面地址返回了 HTTP {0}")]
    Status(u16),
    #[error("封面过大，已放弃（{0} 字节）")]
    TooLarge(usize),
    #[error("无法解析封面图片：{0}")]
    Decode(#[from] image::ImageError),
}
