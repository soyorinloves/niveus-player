//! What can go wrong putting Niveus on the bus.

#[derive(Debug, thiserror::Error)]
pub enum MprisError {
    #[error("cannot connect to the D-Bus session bus: {0}")]
    Connect(#[source] zbus::Error),
    #[error("cannot register the MPRIS service name: {0}")]
    Name(#[source] zbus::Error),
    #[error("cannot publish the MPRIS interface: {0}")]
    Serve(#[source] zbus::Error),
}
