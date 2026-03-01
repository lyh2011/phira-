use super::fs_from_path;
use crate::{
    client::UserManager,
    dir, get_data,
    icons::Icons,
};
use anyhow::{Context, Result};
use prpr::{
    ext::SafeTexture,
    fs,
    replay::ReplayData,
    scene::{BasicPlayer, GameMode, LoadingScene, NextScene, Scene},
    time::TimeManager,
    ui::Ui,
};
use std::sync::Arc;
use tracing::info;

// 全局变量：临时禁用回放功能
// TODO: 修复回放模式的渲染拉伸问题后移除此变量
pub const REPLAY_DISABLED: bool = true;

pub struct ReplayScene {
    loading_scene: Box<LoadingScene>,
}

impl ReplayScene {
    pub async fn new(
        _icons: Arc<Icons>,
        _rank_icons: [SafeTexture; 8],
        replay_data: ReplayData,
    ) -> Result<Self> {
        info!("[REPLAY_SCENE] Creating ReplayScene for chart: {}", replay_data.chart_name);
        
        // 查找对应的本地铺面
        let local_chart = get_data()
            .charts
            .iter()
            .find(|c| {
                // 优先通过 chart_id 匹配
                if let Some(id) = replay_data.chart_id {
                    if c.info.id == Some(id) {
                        return true;
                    }
                }
                // 其次通过名称匹配
                c.info.name == replay_data.chart_name
            })
            .context(format!("找不到对应的铺面: {}", replay_data.chart_name))?;
        
        info!("[REPLAY_SCENE] Found local chart: {}", local_chart.local_path);
        
        // 创建文件系统
        let mut fs = fs_from_path(&local_chart.local_path)?;
        
        // 加载铺面信息
        let mut info = fs::load_info(fs.as_mut()).await?;
        info.id = local_chart.info.id;
        
        // 获取配置
        let mut config = get_data().config.clone();
        config.player_name = get_data()
            .me
            .as_ref()
            .map(|it| it.name.clone())
            .unwrap_or_else(|| "Guest".to_string());
        config.res_pack_path = {
            let id = get_data().respack_id;
            if id == 0 {
                None
            } else {
                Some(format!("{}/{}", dir::respacks()?, get_data().respacks[id - 1]))
            }
        };
        // 回放模式强制启用fix_aspect_ratio，避免竖直拉伸
        config.fix_aspect_ratio = true;
        
        // 预加载资源
        let preload = LoadingScene::load(fs.as_mut(), &info.illustration).await?;
        
        // 创建玩家信息（用于显示）
        let player = get_data().me.as_ref().map(|it| BasicPlayer {
            avatar: UserManager::get_avatar(it.id).flatten(),
            id: it.id,
            rks: it.rks,
            historic_best: 0,
        });
        
        info!("[REPLAY_SCENE] Creating LoadingScene with Normal mode");
        
        // 创建 LoadingScene，使用 Normal 模式
        let loading_scene = LoadingScene::new(
            GameMode::Normal,
            info,
            config,
            fs,
            player,
            None, // 不上传成绩
            None, // 不更新记录
            Some(preload),
            Some(replay_data), // 传递回放数据
        )
        .await?;
        
        info!("[REPLAY_SCENE] ReplayScene created successfully");
        
        Ok(Self {
            loading_scene: Box::new(loading_scene),
        })
    }
}

impl Scene for ReplayScene {
    fn enter(&mut self, tm: &mut TimeManager, target: Option<macroquad::prelude::RenderTarget>) -> Result<()> {
        self.loading_scene.enter(tm, target)
    }

    fn pause(&mut self, tm: &mut TimeManager) -> Result<()> {
        self.loading_scene.pause(tm)
    }

    fn resume(&mut self, tm: &mut TimeManager) -> Result<()> {
        self.loading_scene.resume(tm)
    }

    fn touch(&mut self, tm: &mut TimeManager, touch: &macroquad::prelude::Touch) -> Result<bool> {
        self.loading_scene.touch(tm, touch)
    }

    fn update(&mut self, tm: &mut TimeManager) -> Result<()> {
        self.loading_scene.update(tm)
    }

    fn render(&mut self, tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        self.loading_scene.render(tm, ui)
    }

    fn next_scene(&mut self, tm: &mut TimeManager) -> NextScene {
        self.loading_scene.next_scene(tm)
    }
}
