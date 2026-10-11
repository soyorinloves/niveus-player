//! What can go wrong putting Niveus on the bus.

#[derive(Debug, thiserror::Error)]
pub enum MprisError {
    #[error("无法连接 D-Bus 会话总线：{0}")]
    Connect(#[source] zbus::Error),
    #[error("无法注册 MPRIS 服务名：{0}")]
    Name(#[source] zbus::Error),
    #[error("无法发布 MPRIS 接口：{0}")]
    Serve(#[source] zbus::Error),
}
