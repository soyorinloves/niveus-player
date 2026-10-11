//! What the desktop asks Niveus to do.

/// A control request that arrived from outside the program.
///
/// These are requests, not instructions: the application treats each one
/// exactly as it would treat the matching key press, so a desktop pressing
/// "play" and a listener pressing `Space` take the same path through the
/// program and produce the same result.
#[derive(Debug, Clone, PartialEq)]
pub enum MprisCommand {
    Play,
    Pause,
    /// Play if paused, pause if playing.
    PlayPause,
    Stop,
    Next,
    Previous,
    /// Move the position by this many microseconds, either way.
    Seek(i64),
    /// Jump to a position in microseconds.
    SetPosition(i64),
    /// Set the volume, 0–100.
    SetVolume(u8),
    Quit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_carries_what_it_needs() {
        // The variants differ only in their payloads, which is what the
        // application has to act on; this pins down that they are not confused
        // for one another.
        assert_ne!(MprisCommand::Seek(1), MprisCommand::SetPosition(1));
        assert_ne!(MprisCommand::Play, MprisCommand::PlayPause);
        assert_eq!(
            MprisCommand::Seek(-5_000_000),
            MprisCommand::Seek(-5_000_000)
        );
    }
}
