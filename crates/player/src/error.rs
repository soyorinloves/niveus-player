//! Failure modes of playback control.
//!
//! Deliberately free of any mention of mpv, sockets or JSON: the caller answers
//! a `PlayerError`, not an IPC failure, and swapping the player must not change
//! what callers match on.

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("未找到 mpv：播放需要 mpv，请用你的发行版包管理器安装")]
    MpvNotFound,
    #[error("无法启动播放器：{0}")]
    Spawn(#[source] std::io::Error),
    #[error("无法连接播放器：{0}")]
    IpcConnection(#[source] std::io::Error),
    #[error("播放器通信失败：{0}")]
    IpcProtocol(String),
    #[error("播放器拒绝执行该操作：{0}")]
    Rejected(String),
    #[error("播放器已关闭")]
    Closed,
}
