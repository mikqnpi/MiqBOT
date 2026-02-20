use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

pub struct AudioPlayer {
    output_dir: PathBuf,
    fallback_wav_path: PathBuf,
}

impl AudioPlayer {
    pub fn new(output_dir: impl Into<PathBuf>, fallback_wav_path: impl Into<PathBuf>) -> Result<Self> {
        let output_dir = output_dir.into();
        std::fs::create_dir_all(&output_dir).context("create audio output_dir")?;

        Ok(Self {
            output_dir,
            fallback_wav_path: fallback_wav_path.into(),
        })
    }

    pub fn play_or_fallback(&self, wav_bytes: &[u8]) -> Result<PathBuf> {
        let utterance_id = Uuid::new_v4().to_string();
        let wav_path = self.output_dir.join(format!("{utterance_id}.wav"));
        std::fs::write(&wav_path, wav_bytes).with_context(|| format!("write wav: {}", wav_path.display()))?;

        if self.try_play(&wav_path).is_ok() {
            return Ok(wav_path);
        }

        std::fs::write(&self.fallback_wav_path, wav_bytes)
            .with_context(|| format!("write fallback wav: {}", self.fallback_wav_path.display()))?;
        Ok(self.fallback_wav_path.clone())
    }

    fn try_play(&self, wav_path: &Path) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            let escaped = wav_path.display().to_string().replace("'", "''");
            let cmd = format!("(New-Object Media.SoundPlayer '{escaped}').Play()");
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &cmd])
                .spawn()
                .context("spawn windows sound player")?;
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        {
            Command::new("afplay")
                .arg(wav_path)
                .spawn()
                .context("spawn afplay")?;
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        {
            if Command::new("aplay").arg(wav_path).spawn().is_ok() {
                return Ok(());
            }
            if Command::new("paplay").arg(wav_path).spawn().is_ok() {
                return Ok(());
            }
            anyhow::bail!("no supported linux audio player command found (aplay/paplay)");
        }

        #[allow(unreachable_code)]
        anyhow::bail!("audio playback not implemented for this platform")
    }
}
