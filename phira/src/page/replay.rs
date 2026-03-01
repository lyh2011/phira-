use super::{NextPage, Page, SharedState};
use crate::{dir, get_data, save_data};
use anyhow::Result;
use macroquad::prelude::*;
use prpr::{
    core::BOLD_FONT,
    ext::{semi_white, RectExt, SafeTexture},
    replay::ReplayData,
    scene::show_message,
    ui::{DRectButton, Scroll, Ui},
};
use std::{borrow::Cow, fs, path::PathBuf};

pub struct ReplayPage {
    scroll: Scroll,
    replays: Vec<(PathBuf, ReplayData)>,
    play_btns: Vec<DRectButton>,
    delete_btns: Vec<DRectButton>,
}

impl ReplayPage {
    pub fn new() -> Result<Self> {
        let mut replays = Vec::new();
        let replay_dir = format!("{}/replays", dir::root()?);
        if let Ok(entries) = fs::read_dir(&replay_dir) {
            for entry in entries.flatten() {
                if let Ok(data) = fs::read(entry.path()) {
                    if let Ok(replay) = serde_json::from_slice::<ReplayData>(&data) {
                        replays.push((entry.path(), replay));
                    }
                }
            }
        }
        replays.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
        
        let play_btns = (0..replays.len()).map(|_| DRectButton::new()).collect();
        let delete_btns = (0..replays.len()).map(|_| DRectButton::new()).collect();
        
        Ok(Self {
            scroll: Scroll::new(),
            replays,
            play_btns,
            delete_btns,
        })
    }
}

impl Page for ReplayPage {
    fn label(&self) -> Cow<'static, str> {
        "Replays".into()
    }

    fn update(&mut self, _focus: bool, _s: &mut SharedState) -> Result<()> {
        Ok(())
    }

    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        if self.scroll.touch(touch, s.t) {
            return Ok(true);
        }
        
        for (i, btn) in self.delete_btns.iter_mut().enumerate() {
            if btn.touch(touch, s.t) {
                if let Some((path, _)) = self.replays.get(i) {
                    let _ = fs::remove_file(path);
                    self.replays.remove(i);
                    self.play_btns.remove(i);
                    self.delete_btns.remove(i);
                    show_message("Replay deleted").ok();
                }
                return Ok(true);
            }
        }
        
        Ok(false)
    }

    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        let top = -ui.top;
        let mut h = 0.1;
        
        self.scroll.size((ui.top * 2., ui.top * 2.));
        self.scroll.render(ui, |ui| {
            for (i, (_, replay)) in self.replays.iter().enumerate() {
                let r = Rect::new(-0.9, h, 1.8, 0.15);
                ui.fill_rect(r, semi_white(0.1));
                
                let chart_name = replay.chart_path.split('/').last().unwrap_or(&replay.chart_path);
                ui.text(chart_name)
                    .pos(r.x + 0.02, r.y + 0.04)
                    .size(0.5)
                    .draw();
                
                let info = format!("Score: {} | Acc: {:.2}% | Combo: {}", 
                    replay.score, replay.accuracy * 100.0, replay.max_combo);
                ui.text(&info)
                    .pos(r.x + 0.02, r.y + 0.09)
                    .size(0.35)
                    .color(semi_white(0.7))
                    .draw();
                
                let delete_r = Rect::new(r.right() - 0.25, r.y + 0.025, 0.2, 0.1);
                self.delete_btns[i].render_text(ui, delete_r, s.t, "Delete", 0.4, true);
                
                h += 0.17;
            }
            (1.8, h)
        });
        
        Ok(())
    }
}
