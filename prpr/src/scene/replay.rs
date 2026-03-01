use super::{draw_background, NextScene, Scene};
use crate::{
    config::Config,
    core::{BadNote, Chart, Effect, Resource},
    ext::SafeTexture,
    fs::FileSystem,
    info::ChartInfo,
    judge::Judge,
    replay::ReplayData,
    time::TimeManager,
    ui::Ui,
};
use anyhow::Result;
use macroquad::prelude::*;
use sasa::{Music, MusicParams};
use std::ops::DerefMut;

pub struct ReplayScene {
    pub res: Resource,
    pub chart: Chart,
    pub judge: Judge,
    pub music: Music,
    
    state: ReplayState,
    last_update_time: f64,
    should_exit: bool,
    next_scene: Option<NextScene>,
    effects: Vec<Effect>,
    bad_notes: Vec<BadNote>,
}

enum ReplayState {
    Starting,
    BeforeMusic,
    Playing,
    Ending,
}

impl ReplayScene {
    const BEFORE_TIME: f32 = 0.7;
    const WAIT_TIME: f32 = 0.5;
    const AFTER_TIME: f32 = 0.7;

    pub async fn new(
        info: ChartInfo,
        mut config: Config,
        mut fs: Box<dyn FileSystem>,
        background: SafeTexture,
        illustration: SafeTexture,
        replay_data: ReplayData,
    ) -> Result<Self> {
        config.mods.remove(crate::config::Mods::AUTOPLAY);
        
        let (mut chart, _, _) = crate::scene::GameScene::load_chart(fs.deref_mut(), &info).await?;
        let effects = std::mem::take(&mut chart.extra.global_effects);
        
        let mut res = Resource::new(
            config,
            info,
            fs,
            None,
            background,
            illustration,
            chart.extra.effects.is_empty() && effects.is_empty(),
        )
        .await?;
        
        chart.hitsounds.drain().for_each(|(name, clip)| {
            if let Ok(clip) = res.create_sfx(clip) {
                res.extra_sfxs.insert(name, clip);
            }
        });
        
        let mut judge = Judge::new(&chart);
        judge.set_replay_data(replay_data);
        
        let music = res.audio.create_music(
            res.music.clone(),
            MusicParams {
                amplifier: res.config.volume_music as _,
                playback_rate: res.config.speed as _,
                ..Default::default()
            },
        )?;
        
        Ok(Self {
            res,
            chart,
            judge,
            music,
            state: ReplayState::Starting,
            last_update_time: 0.,
            should_exit: false,
            next_scene: None,
            effects,
            bad_notes: Vec::new(),
        })
    }
}

impl Scene for ReplayScene {
    fn enter(&mut self, tm: &mut TimeManager, _target: Option<RenderTarget>) -> Result<()> {
        tm.reset();
        self.last_update_time = tm.now();
        Ok(())
    }

    fn update(&mut self, tm: &mut TimeManager) -> Result<()> {
        let t = tm.now() as f32;
        
        let time = match self.state {
            ReplayState::Starting => {
                if t >= Self::BEFORE_TIME {
                    self.state = ReplayState::BeforeMusic;
                }
                self.res.alpha = (t / Self::BEFORE_TIME).min(1.);
                0.
            }
            ReplayState::BeforeMusic => {
                if t >= 0. {
                    self.music.seek_to(0.)?;
                    if !tm.paused() {
                        self.music.play()?;
                    }
                    self.state = ReplayState::Playing;
                }
                0.
            }
            ReplayState::Playing => {
                if t > self.res.track_length + Self::WAIT_TIME {
                    self.state = ReplayState::Ending;
                }
                t
            }
            ReplayState::Ending => {
                let dt = t - self.res.track_length - Self::WAIT_TIME;
                if dt >= Self::AFTER_TIME {
                    self.should_exit = true;
                }
                self.res.alpha = 1. - (dt / Self::AFTER_TIME).min(1.);
                self.res.track_length
            }
        };
        
        self.res.time = time;
        self.judge.update(&mut self.res, &mut self.chart, &mut self.bad_notes);
        self.chart.update(&mut self.res);
        
        for e in &mut self.effects {
            e.update(&self.res);
        }
        
        self.last_update_time = tm.now();
        Ok(())
    }

    fn render(&mut self, tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        draw_background(*self.res.background);
        
        self.chart.render(ui, &mut self.res);
        
        ui.scope(|ui| {
            ui.dx(-1.);
            ui.dy(-1. / self.res.aspect_ratio);
            
            let _t = tm.now() as f32;
            let top = -1. / self.res.aspect_ratio;
            
            ui.text("REPLAY")
                .pos(0., top + 0.05)
                .anchor(0.5, 0.)
                .size(0.6)
                .color(Color::new(1., 1., 0., 0.8))
                .draw();
            
            let score = format!("{:07}", self.judge.score());
            ui.text(&score)
                .pos(0.9, top + 0.05)
                .anchor(1., 0.)
                .size(0.7)
                .draw();
        });
        
        Ok(())
    }

    fn next_scene(&mut self, _tm: &mut TimeManager) -> NextScene {
        if self.should_exit {
            NextScene::Pop
        } else {
            NextScene::None
        }
    }
}
