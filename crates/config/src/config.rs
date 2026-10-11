//! User preferences read from `config.toml`.
//!
//! Only preferences live here. Playback mode, volume, position, queue and
//! history are runtime state and belong to `state.json`.

use crate::keymap::{Action, Key, Keymap};
use niveus_music_core::AudioQuality;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("无法读取配置文件: {0}")]
    Read(#[from] std::io::Error),
    #[error("配置文件解析失败: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("快捷键配置无效: {0}")]
    InvalidKeymap(String),
    #[error("无法写入配置文件: {0}")]
    Write(#[source] std::io::Error),
}

/// Something in `config.toml` that could not be used as written.
///
/// A bad file must never stop the program from starting (SPEC §25), but
/// silently substituting the default would leave the user looking at a key
/// that does not do what their file says, with nothing to explain why. The
/// loader therefore carries on and reports what it ignored, so the interface
/// can say so once the screen is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigNotice {
    /// A sentence for the user, in their language.
    pub message: String,
}

impl ConfigNotice {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigNotice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// How album art is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtworkMode {
    /// Two vertically-stacked samples per cell using `▀` with both colours.
    #[default]
    Halfblock,
    /// Brightness mapped onto a character ramp.
    Ascii,
}

impl ArtworkMode {
    pub fn name(self) -> &'static str {
        match self {
            ArtworkMode::Halfblock => "halfblock",
            ArtworkMode::Ascii => "ascii",
        }
    }
}

/// Manual override of the terminal's colour capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Auto,
    Truecolor,
    #[serde(rename = "256")]
    Colors256,
    #[serde(rename = "16")]
    Colors16,
    #[serde(rename = "mono")]
    Monochrome,
}

impl ColorMode {
    pub fn name(self) -> &'static str {
        match self {
            ColorMode::Auto => "auto",
            ColorMode::Truecolor => "truecolor",
            ColorMode::Colors256 => "256",
            ColorMode::Colors16 => "16",
            ColorMode::Monochrome => "mono",
        }
    }
}

/// How much of the now-playing pane the cover takes.
///
/// The three settings are proportions of the pane rather than fixed counts, so
/// a tall terminal gets a tall cover without anyone having to tune a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverSize {
    /// About a quarter of the pane.
    Small,
    #[default]
    Normal,
    /// Everything the pane can spare, once the track's own details and three
    /// lines of lyrics have been paid for.
    Large,
}

impl CoverSize {
    /// Every setting, in the order the Settings page steps through them.
    pub const ALL: [CoverSize; 3] = [CoverSize::Small, CoverSize::Normal, CoverSize::Large];

    pub fn name(self) -> &'static str {
        match self {
            CoverSize::Small => "small",
            CoverSize::Normal => "normal",
            CoverSize::Large => "large",
        }
    }

    /// The next setting, wrapping round; used when the value is stepped.
    pub fn next(self) -> Self {
        self.step(1)
    }

    /// The size `delta` steps away, wrapping round.
    pub fn step(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|size| *size == self)
            .expect("ALL lists every variant") as isize;
        Self::ALL[(index + delta).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

/// What the written file opens with.
///
/// Everything below it is optional, which is the point: an entry that is not
/// there is one the user has no opinion about, and the program is free to
/// choose.
const CONFIG_HEADER: &str = "\
# Niveus 用户偏好。只记录与默认值不同的项，没有列出的项使用程序内置的默认值。
#
# 播放模式、音量、播放位置、队列与历史属于运行状态，写在 state.json 里。

";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub keymap: Keymap,
    pub audio_quality: AudioQuality,
    pub artwork_mode: ArtworkMode,
    pub color_mode: ColorMode,
    pub cover_size: CoverSize,
}

/// The on-disk shape.
///
/// Note the file layout: `audio_quality`, `artwork_mode` and `color_mode` are
/// top-level preferences, while `[keybindings]` is its own table. Because TOML
/// tables must follow bare keys, this crate always writes `[keybindings]` last.
#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    keybindings: HashMap<String, String>,
    audio_quality: Option<String>,
    artwork_mode: Option<ArtworkMode>,
    color_mode: Option<ColorMode>,
    cover_size: Option<CoverSize>,
}

impl Config {
    /// Reads `config.toml`, falling back to defaults on any problem.
    ///
    /// A malformed configuration must never stop the program from starting, so
    /// every failure is logged and only the offending part is discarded.
    pub fn load_or_default(path: &Path) -> Config {
        Self::load(path).0
    }

    /// Reads `config.toml`, and says what it had to work around.
    ///
    /// The configuration itself is always usable — what cannot be used is
    /// dropped and the default stands in — but the parts that were dropped are
    /// handed back so the interface can mention them (SPEC §25).
    pub fn load(path: &Path) -> (Config, Vec<ConfigNotice>) {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %path.display(), "no config file, using defaults");
                (Config::default(), Vec::new())
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "cannot read config, using defaults");
                (
                    Config::default(),
                    vec![ConfigNotice::new(format!(
                        "无法读取配置文件，已使用默认设置：{e}"
                    ))],
                )
            }
        }
    }

    fn from_toml(text: &str, path: &Path) -> (Config, Vec<ConfigNotice>) {
        let parsed: ConfigFile = match toml::from_str(text) {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "config parse failed, using defaults");
                return (
                    Config::default(),
                    vec![ConfigNotice::new(format!(
                        "配置文件无法解析，已使用默认设置：{e}"
                    ))],
                );
            }
        };

        let mut config = Config::default();
        let mut notices = Vec::new();

        let keymap_errors = config.keymap.apply_overrides(&parsed.keybindings);
        for error in keymap_errors {
            tracing::warn!(path = %path.display(), error = %error, "ignoring keybinding entry");
            notices.push(ConfigNotice::new(format!("快捷键配置已忽略：{error}")));
        }

        if let Some(raw) = parsed.audio_quality.as_deref() {
            match AudioQuality::from_api_level(raw) {
                Some(quality) => config.audio_quality = quality,
                None => {
                    tracing::warn!(
                        value = raw,
                        "audio_quality is not one of standard/higher/exhigh/lossless, keeping default"
                    );
                    notices.push(ConfigNotice::new(format!(
                        "音质「{raw}」无法识别，已使用默认音质 {}",
                        config.audio_quality.label()
                    )));
                }
            }
        }

        if let Some(mode) = parsed.artwork_mode {
            config.artwork_mode = mode;
        }
        if let Some(mode) = parsed.color_mode {
            config.color_mode = mode;
        }
        if let Some(size) = parsed.cover_size {
            config.cover_size = size;
        }

        (config, notices)
    }

    /// Writes the preferences back to `config.toml`.
    ///
    /// Only preferences: playback mode, volume, position, queue and history are
    /// runtime state and belong to `state.json`.
    ///
    /// **Only what differs from the defaults is written.** A file that spelled
    /// out every setting would pin the user to the defaults of the day it was
    /// written: a later release could change a default and nobody who had ever
    /// opened Settings would ever see it — silently, and for good. What is not
    /// in the file is not a decision the user made, so it stays the program's
    /// to make. It also means a hand-written file is not flattened: anything
    /// the user did not change is simply not written back over.
    ///
    /// The bare keys go first and the keybindings table last, because a TOML
    /// table swallows every bare key that follows it.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let defaults = Config::default();
        let mut text = String::from(CONFIG_HEADER);

        if self.audio_quality != defaults.audio_quality {
            text.push_str(&format!(
                "audio_quality = \"{}\"\n",
                self.audio_quality.as_api_level()
            ));
        }
        if self.artwork_mode != defaults.artwork_mode {
            text.push_str(&format!(
                "artwork_mode = \"{}\"\n",
                self.artwork_mode.name()
            ));
        }
        if self.color_mode != defaults.color_mode {
            text.push_str(&format!("color_mode = \"{}\"\n", self.color_mode.name()));
        }
        if self.cover_size != defaults.cover_size {
            text.push_str(&format!("cover_size = \"{}\"\n", self.cover_size.name()));
        }

        let rebound: Vec<(Action, Key)> = self
            .keymap
            .iter()
            .filter(|(action, key)| defaults.keymap.key_for(*action) != Some(*key))
            .collect();
        if !rebound.is_empty() {
            text.push_str("\n[keybindings]\n");
            for (action, key) in rebound {
                text.push_str(&format!("{} = \"{}\"\n", action.name(), key.as_config()));
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Written even when there is nothing to say, so that a setting put back
        // to its default is actually removed from the file rather than left
        // behind to be read again on the next start.
        std::fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Action, Key};

    fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("niveus-config-test-{name}.toml"));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn defaults_match_the_spec() {
        let config = Config::default();
        assert_eq!(config.audio_quality, AudioQuality::Exhigh);
        assert_eq!(config.artwork_mode, ArtworkMode::Halfblock);
        assert_eq!(config.color_mode, ColorMode::Auto);
    }

    #[test]
    fn absent_file_yields_defaults() {
        let path = std::env::temp_dir().join("niveus-does-not-exist-0000.toml");
        let config = Config::load_or_default(&path);
        assert_eq!(config, Config::default());
    }

    #[test]
    fn preferences_and_overrides_are_applied() {
        let path = write_temp(
            "ok",
            r#"
audio_quality = "lossless"
artwork_mode = "ascii"
color_mode = "16"

[keybindings]
quit = "x"
"#,
        );
        let config = Config::load_or_default(&path);

        assert_eq!(config.audio_quality, AudioQuality::Lossless);
        assert_eq!(config.artwork_mode, ArtworkMode::Ascii);
        assert_eq!(config.color_mode, ColorMode::Colors16);
        assert_eq!(config.keymap.key_for(Action::Quit), Some(Key::Char('x')));
    }

    #[test]
    fn a_broken_file_still_starts_with_defaults() {
        let path = write_temp("broken", "this is not = = toml ][");
        assert_eq!(Config::load_or_default(&path), Config::default());
    }

    #[test]
    fn a_file_that_says_nothing_reports_nothing() {
        let path = write_temp("quiet", "cover_size = \"large\"\n");
        let (config, notices) = Config::load(&path);
        assert_eq!(config.cover_size, CoverSize::Large);
        assert!(notices.is_empty(), "{notices:?}");

        // And a file that is not there at all is not something to report.
        let missing = std::env::temp_dir().join("niveus-absent-0000.toml");
        let (_, notices) = Config::load(&missing);
        assert!(notices.is_empty(), "{notices:?}");
    }

    #[test]
    fn what_the_loader_had_to_ignore_is_reported_rather_than_only_logged() {
        // SPEC §25: an unusable configuration is not fatal, but the user has to
        // be able to find out why the key they wrote does nothing.
        let path = write_temp(
            "notices",
            r#"
audio_quality = "hires"

[keybindings]
quit = "x"
nonsense = "z"
next = "not-a-key"
"#,
        );
        let (config, notices) = Config::load(&path);

        // Everything that could be used was used.
        assert_eq!(config.keymap.key_for(Action::Quit), Some(Key::Char('x')));
        assert_eq!(config.keymap.key_for(Action::Next), Some(Key::Char('n')));
        assert_eq!(config.audio_quality, AudioQuality::Exhigh);

        assert_eq!(notices.len(), 3, "{notices:?}");
        let text = notices
            .iter()
            .map(|notice| notice.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("hires"), "{text}");
        assert!(text.contains("nonsense"), "{text}");
        assert!(text.contains("not-a-key"), "{text}");
    }

    #[test]
    fn an_unreadable_file_is_reported_and_defaults_are_used() {
        let path = write_temp("unreadable", "this is not = = toml ][");
        let (config, notices) = Config::load(&path);
        assert_eq!(config, Config::default());
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].message.contains("无法解析"), "{notices:?}");
        assert_eq!(notices[0].to_string(), notices[0].message);
    }

    #[test]
    fn an_unknown_quality_keeps_the_default() {
        let path = write_temp("bad-quality", "audio_quality = \"hires\"\n");
        let config = Config::load_or_default(&path);
        assert_eq!(config.audio_quality, AudioQuality::Exhigh);
    }

    /// A scratch file for the save tests.
    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("niveus-save-{name}-{}.toml", std::process::id()))
    }

    #[test]
    fn a_file_is_written_even_when_there_is_nothing_to_say() {
        // Written, not skipped: a setting put back to its default has to be
        // taken out of the file, or the next start reads the old value again.
        let path = scratch("defaults");
        let _ = std::fs::remove_file(&path);

        Config::default().save(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();

        // Nothing but the comment: every key in it would be a value frozen at
        // whatever the defaults happened to be that day.
        for line in written.lines() {
            let line = line.trim();
            assert!(
                line.is_empty() || line.starts_with('#'),
                "a default was written out: {line:?}"
            );
        }
        assert!(written.contains("只记录与默认值不同的项"), "{written}");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn only_a_changed_preference_is_written() {
        let path = scratch("one-preference");
        let _ = std::fs::remove_file(&path);

        let config = Config {
            cover_size: CoverSize::Large,
            ..Config::default()
        };
        config.save(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();

        assert!(written.contains("cover_size = \"large\""), "{written}");
        // The other three are still at their defaults, so they are the
        // program's to choose and do not belong in the file.
        for key in ["audio_quality", "artwork_mode", "color_mode"] {
            assert!(!written.contains(key), "{key} was written out: {written}");
        }
        assert!(!written.contains("[keybindings]"), "{written}");

        // And it reads back as what was written.
        assert_eq!(Config::load_or_default(&path).cover_size, CoverSize::Large);
        assert_eq!(
            Config::load_or_default(&path).audio_quality,
            Config::default().audio_quality
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn only_a_rebound_key_is_written() {
        let path = scratch("one-binding");
        let _ = std::fs::remove_file(&path);

        let mut config = Config::default();
        config.keymap.rebind(Action::Quit, Key::Char('x')).unwrap();
        config.save(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();

        assert!(written.contains("[keybindings]"), "{written}");
        assert!(written.contains("quit = \"x\""), "{written}");
        // The twenty-three that were not touched are absent, not restated.
        let bindings = written
            .lines()
            .filter(|line| line.contains(" = ") && !line.trim_start().starts_with('#'))
            .count();
        assert_eq!(
            bindings, 1,
            "more than the changed binding was written: {written}"
        );

        // And the change survives the round trip.
        let read_back = Config::load_or_default(&path);
        assert_eq!(read_back.keymap.key_for(Action::Quit), Some(Key::Char('x')));
        assert_eq!(
            read_back.keymap.key_for(Action::Next),
            Keymap::default().key_for(Action::Next),
            "an untouched binding did not fall back to its default"
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_setting_put_back_to_its_default_leaves_the_file() {
        // The reason the file is written even when empty: leaving the old entry
        // behind would undo the change on the next start.
        let path = scratch("reverted");
        let _ = std::fs::remove_file(&path);

        let mut config = Config {
            cover_size: CoverSize::Large,
            ..Config::default()
        };
        config.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("cover_size"));

        config.cover_size = CoverSize::Normal;
        config.save(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("cover_size"), "{written}");
        assert_eq!(Config::load_or_default(&path).cover_size, CoverSize::Normal);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn keybindings_that_trade_places_survive_the_round_trip() {
        // The file is a set of assignments, not a sequence: applying them one
        // at a time against the defaults would reject the second of a pair that
        // only makes sense together, and which one that is would depend on the
        // order a hash map happened to hand them over in.
        let path = scratch("trade");
        let _ = std::fs::remove_file(&path);

        let mut config = Config::default();
        // Free `q` first, then give it to another action: the state a user
        // reaches by rebinding twice in Settings.
        config.keymap.rebind(Action::Quit, Key::Char('x')).unwrap();
        config.keymap.rebind(Action::Next, Key::Char('q')).unwrap();
        config.save(&path).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("quit = \"x\""), "{written}");
        assert!(written.contains("next = \"q\""), "{written}");

        let read_back = Config::load_or_default(&path);
        assert_eq!(read_back.keymap.key_for(Action::Quit), Some(Key::Char('x')));
        assert_eq!(read_back.keymap.key_for(Action::Next), Some(Key::Char('q')));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_saved_file_reads_back_as_the_same_configuration() {
        // What Settings writes has to be what the next start reads.
        let path =
            std::env::temp_dir().join(format!("niveus-config-save-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut config = Config {
            audio_quality: AudioQuality::Lossless,
            artwork_mode: ArtworkMode::Ascii,
            color_mode: ColorMode::Colors256,
            cover_size: CoverSize::Large,
            ..Config::default()
        };
        config.keymap.rebind(Action::Quit, Key::Char('x')).unwrap();
        config.save(&path).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        // Bare keys first: a table swallows everything after it.
        let table = written.find("[keybindings]").expect("the table is written");
        let bare = written
            .find("audio_quality")
            .expect("preferences are written");
        assert!(
            bare < table,
            "a preference fell inside the keybindings table"
        );
        assert!(written.contains("cover_size = \"large\""), "{written}");

        let read_back = Config::load_or_default(&path);
        assert_eq!(read_back, config, "the file does not say what was written");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_cover_size_is_read_from_the_file() {
        let path = write_temp("cover-size", "cover_size = \"large\"\n");
        assert_eq!(Config::load_or_default(&path).cover_size, CoverSize::Large);

        // An absent one keeps the default rather than failing the file.
        let path = write_temp("no-cover-size", "audio_quality = \"higher\"\n");
        assert_eq!(Config::load_or_default(&path).cover_size, CoverSize::Normal);

        // And it steps round in the documented order.
        assert_eq!(CoverSize::Small.next(), CoverSize::Normal);
        assert_eq!(CoverSize::Normal.next(), CoverSize::Large);
        assert_eq!(CoverSize::Large.next(), CoverSize::Small);
    }

    #[test]
    fn readme_style_layout_puts_preferences_before_the_table() {
        // Mirrors SPEC §28's example, with the table last so TOML parses it as
        // written.
        let path = write_temp(
            "spec-layout",
            r#"
audio_quality = "higher"
artwork_mode = "halfblock"
color_mode = "auto"

[keybindings]
playlist = "1"
queue = "u"
"#,
        );
        let config = Config::load_or_default(&path);
        assert_eq!(config.audio_quality, AudioQuality::Higher);
        assert_eq!(
            config.keymap.key_for(Action::ToggleQueue),
            Some(Key::Char('u'))
        );
    }
}
