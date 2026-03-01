use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteRecord {
    pub time: f32,
    pub line_id: usize,
    pub note_id: usize,
    pub judgment: String,
    pub offset_ms: f32,
    #[serde(default)]
    pub hold_duration: Option<f32>, // Hold音符的持续时间（秒）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayData {
    pub chart_id: Option<i32>,
    pub chart_name: String,
    pub records: Vec<NoteRecord>,
    pub score: Option<i32>,
    pub accuracy: Option<f32>,
    pub max_combo: Option<u32>,
    pub timestamp: i64, // 游玩时间戳
}

impl ReplayData {
    pub fn new(chart_id: Option<i32>, chart_name: String) -> Self {
        Self {
            chart_id,
            chart_name,
            records: Vec::new(),
            score: None,
            accuracy: None,
            max_combo: None,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }

    pub fn add_record(&mut self, time: f32, line_id: usize, note_id: usize, judgment: String, offset_ms: f32) {
        self.records.push(NoteRecord {
            time,
            line_id,
            note_id,
            judgment,
            offset_ms,
            hold_duration: None,
        });
    }
    
    pub fn add_hold_record(&mut self, time: f32, line_id: usize, note_id: usize, judgment: String, offset_ms: f32, hold_duration: f32) {
        self.records.push(NoteRecord {
            time,
            line_id,
            note_id,
            judgment,
            offset_ms,
            hold_duration: Some(hold_duration),
        });
    }

    pub fn finalize(&mut self, score: i32, accuracy: f32, max_combo: u32) {
        self.score = Some(score);
        self.accuracy = Some(accuracy);
        self.max_combo = Some(max_combo);
    }
}
