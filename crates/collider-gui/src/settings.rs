//! User settings, saved as TOML in the config directory (`~/.config/collider/gui.toml`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use collider_core::Quality;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where downloads are saved.
    pub output_dir: PathBuf,
    pub quality: QualityPref,
    /// Preferred audio language or rendition name; empty means the stream's default.
    pub audio_lang: String,
    pub container: Container,
    /// Parallel segment downloads per job.
    pub concurrency: usize,
    pub retries: u32,
    pub timeout_secs: u64,
    /// How often a watched stream is checked until it goes live.
    pub watch_interval_secs: u64,
    pub headers: Vec<Header>,
    pub ui_scale: f32,
}

impl Default for Settings {
    fn default() -> Self {
        let videos = dirs::video_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Videos")))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            output_dir: videos.join("collider"),
            quality: QualityPref::Best,
            audio_lang: String::new(),
            container: Container::Mp4,
            concurrency: 8,
            retries: 5,
            timeout_secs: 30,
            watch_interval_secs: 20,
            headers: Vec::new(),
            ui_scale: 1.0,
        }
    }
}

/// Mirrors [`Quality`], stored as `"best"`, `"worst"` or a height such as `"720"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum QualityPref {
    Best,
    Worst,
    MaxHeight(u64),
}

impl QualityPref {
    pub fn to_quality(self) -> Quality {
        match self {
            Self::Best => Quality::Best,
            Self::Worst => Quality::Worst,
            Self::MaxHeight(h) => Quality::MaxHeight(h),
        }
    }
}

impl TryFrom<String> for QualityPref {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        match s.parse::<Quality>() {
            Ok(Quality::Best) => Ok(Self::Best),
            Ok(Quality::Worst) => Ok(Self::Worst),
            Ok(Quality::MaxHeight(h)) => Ok(Self::MaxHeight(h)),
            Err(_) => Err(format!("invalid quality '{s}'")),
        }
    }
}

impl From<QualityPref> for String {
    fn from(q: QualityPref) -> String {
        match q {
            QualityPref::Best => "best".into(),
            QualityPref::Worst => "worst".into(),
            QualityPref::MaxHeight(h) => h.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Mp4,
    Mkv,
}

impl Container {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub enabled: bool,
    pub name: String,
    pub value: String,
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("collider").join("gui.toml"))
    }

    /// The settings saved at `path`, or the defaults when there are none (or they cannot
    /// be read).
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::from_toml(&text))
            .unwrap_or_default()
    }

    pub fn from_toml(text: &str) -> Self {
        toml::from_str::<Settings>(text)
            .map(Settings::clamped)
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    fn clamped(mut self) -> Self {
        self.concurrency = self.concurrency.clamp(1, 32);
        self.retries = self.retries.min(20);
        self.timeout_secs = self.timeout_secs.clamp(5, 600);
        self.watch_interval_secs = self.watch_interval_secs.clamp(5, 3600);
        self.ui_scale = self.ui_scale.clamp(0.75, 2.0);
        self
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    /// Enabled headers with a name, as `(name, value)` pairs for the HTTP client.
    pub fn http_headers(&self) -> Vec<(String, String)> {
        self.headers
            .iter()
            .filter(|h| h.enabled && !h.name.trim().is_empty())
            .map(|h| (h.name.trim().to_string(), h.value.trim().to_string()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bad_values() {
        let s = Settings {
            quality: QualityPref::MaxHeight(720),
            container: Container::Mkv,
            headers: vec![Header {
                enabled: true,
                name: "Referer".into(),
                value: "https://example.com".into(),
            }],
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&s).unwrap();
        assert!(text.contains("quality = \"720\""), "{text}");
        assert_eq!(Settings::from_toml(&text), s);

        let partial = Settings::from_toml("concurrency = 99\nquality = \"worst\"\n");
        assert_eq!(partial.concurrency, 32);
        assert_eq!(partial.quality, QualityPref::Worst);
        assert_eq!(partial.retries, Settings::default().retries);
        assert_eq!(
            Settings::from_toml("quality = \"sideways\""),
            Settings::default()
        );
    }

    #[test]
    fn saves_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("collider").join("gui.toml");
        let s = Settings {
            retries: 3,
            ..Settings::default()
        };
        s.save_to(&path).unwrap();
        let back = Settings::from_toml(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(back.retries, 3);
    }
}
