use super::{NextPage, Page, SharedState};
use anyhow::Result;
use macroquad::prelude::*;
use prpr::{
    scene::{NextScene, Scene},
    time::TimeManager,
    ui::Ui,
};
use std::borrow::Cow;

pub struct SceneWrapperPage {
    scene: Box<dyn Scene>,
    tm: TimeManager,
    label: String,
}

impl SceneWrapperPage {
    pub fn new(scene: Box<dyn Scene>, label: String) -> Self {
        Self {
            scene,
            tm: TimeManager::default(),
            label,
        }
    }
}

impl Page for SceneWrapperPage {
    fn label(&self) -> Cow<'static, str> {
        // 回放模式下不显示label
        "".into()
    }

    fn can_play_bgm(&self) -> bool {
        false
    }

    fn enter(&mut self, s: &mut SharedState) -> Result<()> {
        self.tm.reset();
        self.tm.update(s.t as _);
        // Scene 不需要 render target（在 phira 中都传 None）
        self.scene.enter(&mut self.tm, None)
    }

    fn update(&mut self, s: &mut SharedState) -> Result<()> {
        self.tm.update(s.t as _);
        self.scene.update(&mut self.tm)
    }

    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        self.tm.update(s.t as _);
        self.scene.touch(&mut self.tm, touch)
    }

    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        self.tm.update(s.t as _);
        self.scene.render(&mut self.tm, ui)
    }

    fn next_page(&mut self) -> NextPage {
        match self.scene.next_scene(&mut self.tm) {
            NextScene::Pop => {
                tracing::info!("[SCENE_WRAPPER] Scene requested Pop, returning to previous page");
                NextPage::Pop
            }
            NextScene::Replace(new_scene) => {
                tracing::info!("[SCENE_WRAPPER] Scene requested Replace, switching to new scene");
                // 替换当前场景
                self.scene = new_scene;
                // 调用新场景的 enter（phira 中都传 None）
                if let Err(e) = self.scene.enter(&mut self.tm, None) {
                    tracing::error!("[SCENE_WRAPPER] Failed to enter new scene: {}", e);
                }
                NextPage::None
            }
            NextScene::PopWithResult(err) => {
                tracing::error!("[SCENE_WRAPPER] Scene failed with error: {:?}", err);
                NextPage::Pop
            }
            _ => NextPage::None,
        }
    }
}
