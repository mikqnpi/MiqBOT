use crate::pb::bridge_v1 as pb;

pub struct SpeechPolicy {
    silence_gap_ms: u64,
    duplicate_cooldown_ms: u64,
    last_line: Option<String>,
    last_line_ms: u64,
}

impl SpeechPolicy {
    pub fn new(silence_gap_ms: u64, duplicate_cooldown_ms: u64) -> Self {
        Self {
            silence_gap_ms,
            duplicate_cooldown_ms,
            last_line: None,
            last_line_ms: 0,
        }
    }

    pub fn line_from_telemetry(&mut self, now_ms: u64, telemetry: &pb::TelemetryFrame) -> Option<String> {
        let dim = match pb::Dimension::from_i32(telemetry.dimension).unwrap_or(pb::Dimension::DimensionUnspecified) {
            pb::Dimension::DimensionOverworld => "オーバーワールド",
            pb::Dimension::DimensionNether => "ネザー",
            pb::Dimension::DimensionEnd => "エンド",
            pb::Dimension::DimensionOther => "特殊ディメンション",
            pb::Dimension::DimensionUnspecified => "不明な場所",
        };

        let line = format!(
            "いま{}、体力{}で空腹{}。足元を確認しながら行くよ。",
            dim,
            telemetry.hp,
            telemetry.hunger
        );

        self.accept_line(now_ms, line)
    }

    pub fn filler_if_needed(&mut self, now_ms: u64, last_spoken_ms: u64) -> Option<String> {
        if now_ms.saturating_sub(last_spoken_ms) < self.silence_gap_ms {
            return None;
        }

        self.accept_line(now_ms, "えっと、次の一手を考えてる。安全第一で進むね。".to_string())
    }

    fn accept_line(&mut self, now_ms: u64, line: String) -> Option<String> {
        if let Some(last) = &self.last_line {
            if *last == line && now_ms.saturating_sub(self.last_line_ms) < self.duplicate_cooldown_ms {
                return None;
            }
        }

        self.last_line = Some(line.clone());
        self.last_line_ms = now_ms;
        Some(line)
    }
}
