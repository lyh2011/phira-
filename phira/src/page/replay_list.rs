use super::{local_illustration, Illustration, NextPage, Page, SharedState};
use crate::{
    charts_view::{ChartDisplayItem, ChartsView},
    dir, get_data,
    icons::Icons,
    page::BLACK_TEXTURE,
};
use anyhow::Result;
use chrono::{Local, TimeZone};
use macroquad::prelude::*;
use prpr::{
    ext::{poll_future, SafeTexture, semi_black, ScaleType},
    replay::ReplayData,
    scene::NextScene,
    ui::{button_hit, DRectButton, Ui},
};
use std::{borrow::Cow, collections::HashMap, pin::Pin, sync::Arc};
use tracing::info;

type LocalTask<T> = Option<Pin<Box<dyn std::future::Future<Output = T>>>>;

pub struct ReplayListPage {
    view: ChartsView,
    icons: Arc<Icons>,
    rank_icons: [SafeTexture; 8],
    next_page: Option<NextPage>,
    next_scene_task: LocalTask<Result<NextScene>>,
    
    // 文件夹导航
    current_folder: Option<String>, // None = 根目录，Some(chart_name) = 在某个谱面的回放列表中
    back_btn: DRectButton,
    
    // 临时存储被点击的回放文件名
    clicked_replay_file: Option<String>,
}

impl ReplayListPage {
    pub fn new(icons: Arc<Icons>, rank_icons: [SafeTexture; 8]) -> Result<Self> {
        let view = ChartsView::new(Arc::clone(&icons), rank_icons.clone());
        
        Ok(Self {
            view,
            icons,
            rank_icons,
            next_page: None,
            next_scene_task: None,
            current_folder: None,
            back_btn: DRectButton::new(),
            clicked_replay_file: None,
        })
    }
    
    fn load_replays(&mut self, t: f32) {
        let mut items = Vec::new();
        // 不添加"谱面合集"按钮
        
        if let Some(chart_name) = &self.current_folder {
            // 在某个谱面的文件夹中，显示该谱面的所有回放
            self.load_chart_replays(chart_name, &mut items);
        } else {
            // 在根目录，显示所有谱面的文件夹
            self.load_chart_folders(&mut items);
        }
        
        self.view.set(t, items);
    }
    
    fn load_chart_folders(&self, items: &mut Vec<ChartDisplayItem>) {
        // 读取所有回放文件，按谱面分组
        let replay_dir = match dir::root() {
            Ok(root) => std::path::Path::new(&root).join("data").join("replays"),
            Err(_) => return,
        };
        
        if !replay_dir.exists() {
            return;
        }
        
        // 按谱面名称分组回放，同时记录第一个回放（用于获取chart_id）
        let mut chart_replays: HashMap<String, (Vec<ReplayData>, Option<i32>)> = HashMap::new();
        
        // 读取所有回放文件
        if let Ok(entries) = std::fs::read_dir(&replay_dir) {
            for entry in entries.flatten() {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(replay) = serde_json::from_str::<ReplayData>(&content) {
                        let entry = chart_replays.entry(replay.chart_name.clone())
                            .or_insert_with(|| (Vec::new(), replay.chart_id));
                        entry.0.push(replay);
                    }
                }
            }
        }
        
        // 按谱面名称排序
        let mut sorted_charts: Vec<_> = chart_replays.into_iter().collect();
        sorted_charts.sort_by(|a, b| a.0.cmp(&b.0));
        
        // 获取本地铺面数据，用于匹配背景图
        let local_charts = &get_data().charts;
        
        // 为每个谱面创建一个文件夹项
        for (chart_name, (replays, chart_id)) in sorted_charts {
            let count = replays.len();
            
            // 尝试找到对应的本地铺面
            let local_chart = local_charts.iter().find(|c| {
                // 优先通过 chart_id 匹配
                if let Some(id) = chart_id {
                    if c.info.id == Some(id) {
                        return true;
                    }
                }
                // 其次通过名称匹配
                c.info.name == chart_name
            });
            
            // 加载铺面背景或使用默认黑色背景
            let mut illu = if let Some(chart) = local_chart {
                local_illustration(chart.local_path.clone(), BLACK_TEXTURE.clone(), false)
            } else {
                Illustration::from_done(BLACK_TEXTURE.clone())
            };
            
            // 通知开始加载背景图片
            illu.notify();
            
            // 创建文件夹的 ChartItem
            let folder_chart = super::ChartItem {
                info: super::BriefChartInfo {
                    id: chart_id,
                    uploader: None,
                    name: chart_name.clone(),
                    level: "Folder".to_string(),
                    difficulty: count as f32,
                    charter: format!("{} 个回放", count),
                    composer: String::new(),
                    illustrator: String::new(),
                    created: None,
                    updated: None,
                    chart_updated: None,
                    intro: String::new(),
                    has_unlock: false,
                },
                local_path: local_chart.map(|c| c.local_path.clone()),
                illu,
                chart_type: super::ChartType::Imported,
                folder: Some(chart_name),
            };
            
            items.push(ChartDisplayItem::new(Some(folder_chart), None));
        }
    }
    
    fn load_chart_replays(&self, chart_name: &str, items: &mut Vec<ChartDisplayItem>) {
        let replay_dir = match dir::root() {
            Ok(root) => std::path::Path::new(&root).join("data").join("replays"),
            Err(_) => return,
        };
        
        if !replay_dir.exists() {
            return;
        }
        
        let mut replays = Vec::new();
        
        // 读取该谱面的所有回放
        if let Ok(entries) = std::fs::read_dir(&replay_dir) {
            for entry in entries.flatten() {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(replay) = serde_json::from_str::<ReplayData>(&content) {
                        if replay.chart_name == chart_name {
                            replays.push((entry.file_name().to_string_lossy().to_string(), replay));
                        }
                    }
                }
            }
        }
        
        // 按时间排序（最新的在前）
        replays.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
        
        // 为每个回放创建一个项
        for (filename, replay) in replays {
            // 格式化时间为 HH:MM YY-MM-DD
            let dt = Local.timestamp_opt(replay.timestamp, 0).single();
            let time_str = if let Some(dt) = dt {
                dt.format("%H:%M %y-%m-%d").to_string()
            } else {
                "Unknown".to_string()
            };
            
            // 只显示准确率
            let level_str = if let Some(acc) = replay.accuracy {
                format!("{:.2}%", acc * 100.0)
            } else {
                "N/A".to_string()
            };
            
            let illu = Illustration::from_done(BLACK_TEXTURE.clone());
            
            // 创建回放的 ChartItem
            // 使用 local_path 字段存储文件名（用于后续加载）
            let replay_chart = super::ChartItem {
                info: super::BriefChartInfo {
                    id: None,
                    uploader: None,
                    name: time_str,
                    level: level_str,
                    difficulty: -1.0, // 设置为负数以隐藏难度显示
                    charter: String::new(),
                    composer: String::new(),
                    illustrator: String::new(),
                    created: None,
                    updated: None,
                    chart_updated: None,
                    intro: String::new(),
                    has_unlock: false,
                },
                local_path: Some(filename), // 存储文件名在 local_path 中
                illu,
                chart_type: super::ChartType::Imported,
                folder: None,
            };
            
            items.push(ChartDisplayItem::new(Some(replay_chart), None));
        }
    }
}

impl Page for ReplayListPage {
    fn label(&self) -> Cow<'static, str> {
        if self.current_folder.is_some() {
            "回放记录".into()
        } else {
            "回放列表".into()
        }
    }

    fn enter(&mut self, s: &mut SharedState) -> Result<()> {
        self.load_replays(s.t);
        Ok(())
    }

    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        let t = s.t;
        
        // 返回按钮
        if self.current_folder.is_some() && self.back_btn.touch(touch, t) {
            button_hit();
            info!("[REPLAY_LIST] Back button clicked, returning to root");
            self.current_folder = None;
            self.load_replays(t);
            return Ok(true);
        }
        
        // 调用 view.touch 处理点击
        info!("[REPLAY_LIST] Calling view.touch, current_folder: {:?}", self.current_folder);
        let view_handled = self.view.touch(touch, s.t, s.rt)?;
        info!("[REPLAY_LIST] view.touch returned: {}, transiting: {}", view_handled, self.view.transiting());
        
        // 检查是否点击了文件夹
        if let Some(clicked_folder) = self.view.clicked_folder.take() {
            info!("[REPLAY_LIST] Folder clicked: {}", clicked_folder);
            self.current_folder = Some(clicked_folder);
            self.load_replays(t);
            return Ok(true);
        }
        
        // 如果在文件夹内且 view 正在转场，说明点击了回放项
        // 我们需要取消 transit 并自己处理
        if self.current_folder.is_some() && self.view.transiting() {
            info!("[REPLAY_LIST] In folder and transiting, canceling transit");
            // 取消 ChartsView 的转场
            self.view.on_result(t, false);
            
            // 从 clicked_chart_path 获取文件名
            if let Some(filename) = self.view.clicked_chart_path.take() {
                info!("[REPLAY_LIST] Got filename from clicked_chart_path: {}", filename);
                
                // 读取回放文件
                let replay_dir = match dir::root() {
                    Ok(root) => std::path::Path::new(&root).join("data").join("replays"),
                    Err(e) => {
                        use prpr::scene::show_message;
                        let msg = format!("无法访问回放目录: {}", e);
                        info!("[REPLAY_LIST] Error: {}", msg);
                        show_message(msg).error();
                        return Ok(true);
                    }
                };
                
                let replay_path = replay_dir.join(&filename);
                info!("[REPLAY_LIST] Reading replay file: {:?}", replay_path);
                
                match std::fs::read_to_string(&replay_path) {
                    Ok(content) => {
                        info!("[REPLAY_LIST] File read successfully, parsing JSON");
                        match serde_json::from_str::<ReplayData>(&content) {
                            Ok(replay_data) => {
                                info!("[REPLAY_LIST] Replay data parsed, chart: {}, records: {}", 
                                    replay_data.chart_name, replay_data.records.len());
                                
                                // 创建回放场景
                                use crate::scene::ReplayScene;
                                
                                let icons = Arc::clone(&self.icons);
                                let rank_icons = self.rank_icons.clone();
                                
                                info!("[REPLAY_LIST] Creating async task for ReplayScene");
                                self.next_scene_task = Some(Box::pin(async move {
                                    info!("[REPLAY_LIST] Async task started, creating ReplayScene");
                                    let result = ReplayScene::new(icons, rank_icons, replay_data).await;
                                    match &result {
                                        Ok(_) => info!("[REPLAY_LIST] ReplayScene created successfully"),
                                        Err(e) => info!("[REPLAY_LIST] ReplayScene creation failed: {}", e),
                                    }
                                    result.map(|scene| {
                                        info!("[REPLAY_LIST] Wrapping scene in NextScene::Overlay");
                                        NextScene::Overlay(Box::new(scene))
                                    })
                                }));
                            },
                            Err(e) => {
                                use prpr::scene::show_message;
                                let msg = format!("回放文件格式错误: {}", e);
                                info!("[REPLAY_LIST] Error: {}", msg);
                                show_message(msg).error();
                            }
                        }
                    },
                    Err(e) => {
                        use prpr::scene::show_message;
                        let msg = format!("无法读取回放文件: {}", e);
                        info!("[REPLAY_LIST] Error: {}", msg);
                        show_message(msg).error();
                    }
                }
            } else {
                info!("[REPLAY_LIST] No clicked_chart_path found!");
            }
            
            return Ok(true);
        }
        
        info!("[REPLAY_LIST] Returning view_handled: {}", view_handled);
        Ok(view_handled)
    }

    fn update(&mut self, s: &mut SharedState) -> Result<()> {
        self.view.update(s.t)?;
        
        // 处理异步场景加载任务
        if let Some(task) = &mut self.next_scene_task {
            if let Some(res) = poll_future(task.as_mut()) {
                info!("[REPLAY_LIST] Async task completed");
                match res {
                    Ok(next_scene) => {
                        info!("[REPLAY_LIST] Creating SceneWrapperPage");
                        // 将 Scene 包装为 Page
                        match next_scene {
                            NextScene::Overlay(scene) => {
                                use super::SceneWrapperPage;
                                self.next_page = Some(NextPage::Overlay(Box::new(SceneWrapperPage::new(scene, "回放".to_string()))));
                                info!("[REPLAY_LIST] next_page set to Overlay");
                            }
                            _ => {
                                info!("[REPLAY_LIST] Unexpected NextScene type");
                            }
                        }
                    }
                    Err(e) => {
                        use prpr::scene::show_message;
                        let msg = format!("加载回放失败: {}", e);
                        info!("[REPLAY_LIST] Error: {}", msg);
                        show_message(msg).error();
                    }
                }
                self.next_scene_task = None;
            }
        }
        
        Ok(())
    }

    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        let t = s.t;
        
        // 渲染返回按钮
        if self.current_folder.is_some() {
            s.render_fader(ui, |ui| {
                let r = Rect::new(-0.9, ui.top + 0.02, 0.15, 0.07);
                self.back_btn.render_shadow(ui, r, t, |ui, path| {
                    ui.fill_path(&path, semi_black(0.4));
                    let ir = Rect::new(r.x + 0.01, r.y + 0.01, 0.05, 0.05);
                    ui.fill_rect(ir, (*self.icons.back, ir, ScaleType::Fit));
                    ui.text("返回").pos(r.x + 0.07, r.center().y).anchor(0., 0.5).size(0.5).draw();
                });
            });
        }
        
        s.render_fader(ui, |ui| {
            let r = ui.content_rect();
            self.view.render(ui, r, t);
        });
        Ok(())
    }

    fn next_page(&mut self) -> NextPage {
        self.next_page.take().unwrap_or_default()
    }
}
