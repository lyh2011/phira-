//! Judgement system

use crate::{
    config::Config,
    core::{BadNote, Chart, NoteKind, Point, Resource, Vector, NOTE_WIDTH_RATIO_BASE},
    ext::{get_viewport, NotNanExt},
};
use macroquad::prelude::{
    utils::{register_input_subscriber, repeat_all_miniquad_input},
    *,
};
use miniquad::{EventHandler, MouseButton};
use once_cell::sync::Lazy;
use sasa::{PlaySfxParams, Sfx};
use serde::Serialize;
use std::{cell::RefCell, collections::HashMap, num::FpCategory};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, info};

// 全局变量：控制是否启用判定日志
static ENABLE_JUDGE_LOG: AtomicBool = AtomicBool::new(false);

// 设置判定日志开关
pub fn set_judge_log_enabled(enabled: bool) {
    ENABLE_JUDGE_LOG.store(enabled, Ordering::Relaxed);
}

// 获取判定日志开关状态
pub fn is_judge_log_enabled() -> bool {
    ENABLE_JUDGE_LOG.load(Ordering::Relaxed)
}

pub const FLICK_SPEED_THRESHOLD: f32 = 0.8;
pub const LIMIT_PERFECT: f32 = 0.08;
pub const LIMIT_GOOD: f32 = 0.16;
pub const LIMIT_BAD: f32 = 0.22;
pub const UP_TOLERANCE: f32 = 0.05;
pub const DIST_FACTOR: f32 = 0.2;

// Phigros judge mode constants (±80ms Perfect, ±160ms Good, ±180ms Bad)
pub const LIMIT_PERFECT_RELAXED: f32 = 0.08;
pub const LIMIT_GOOD_RELAXED: f32 = 0.16;
pub const LIMIT_BAD_RELAXED: f32 = 0.18;

// Phigros Flick判定窗口：perfectTimeRange × 1.75 = 0.08 × 1.75 = 0.14秒
pub const LIMIT_FLICK_RELAXED: f32 = 0.14;

// 严格判定 (±40ms Perfect, ±75ms Good, ±180ms Bad)
pub const LIMIT_PERFECT_STRICT: f32 = 0.04;
pub const LIMIT_GOOD_STRICT: f32 = 0.075;
pub const LIMIT_BAD_STRICT: f32 = 0.18;

const EARLY_OFFSET: f32 = 0.07;

// 写入判定日志（输出到logcat）
fn write_judge_log(message: &str) {
    // 检查是否启用日志
    if !is_judge_log_enabled() {
        return;
    }
    
    // 使用info!宏输出到logcat，标签为JUDGE
    info!("[JUDGE] {}", message);
}

impl Judge {
    #[inline]
    fn get_limit_perfect(config: &Config) -> f32 {
        if config.relaxed_judge {
            if config.strict_judge {
                LIMIT_PERFECT_STRICT
            } else {
                LIMIT_PERFECT_RELAXED
            }
        } else {
            LIMIT_PERFECT
        }
    }

    #[inline]
    fn get_limit_good(config: &Config) -> f32 {
        if config.relaxed_judge {
            if config.strict_judge {
                LIMIT_GOOD_STRICT
            } else {
                LIMIT_GOOD_RELAXED
            }
        } else {
            LIMIT_GOOD
        }
    }

    #[inline]
    fn get_limit_bad(config: &Config) -> f32 {
        if config.relaxed_judge {
            if config.strict_judge {
                LIMIT_BAD_STRICT
            } else {
                LIMIT_BAD_RELAXED
            }
        } else {
            LIMIT_BAD
        }
    }
}

#[derive(Debug, Clone)]
pub enum HitSound {
    None,
    Click,
    Flick,
    Drag,
    Custom(String),
}

impl HitSound {
    pub fn play(&self, res: &mut Resource) {
        match self {
            HitSound::None => {}
            HitSound::Click => play_sfx(&mut res.sfx_click, &res.config),
            HitSound::Flick => play_sfx(&mut res.sfx_flick, &res.config),
            HitSound::Drag => play_sfx(&mut res.sfx_drag, &res.config),
            HitSound::Custom(s) => {
                if let Some(sfx) = res.extra_sfxs.get_mut(s) {
                    play_sfx(sfx, &res.config);
                }
            }
        }
    }

    pub fn default_from_kind(kind: &NoteKind) -> Self {
        match kind {
            NoteKind::Click => HitSound::Click,
            NoteKind::Flick => HitSound::Flick,
            NoteKind::Drag => HitSound::Drag,
            NoteKind::Hold { .. } => HitSound::Click,
        }
    }
}

pub fn play_sfx(sfx: &mut Sfx, config: &Config) {
    if config.volume_sfx <= 1e-2 {
        return;
    }
    let _ = sfx.play(PlaySfxParams {
        amplifier: config.volume_sfx,
    });
}

#[cfg(all(not(target_os = "windows"), not(target_os = "ios")))]
fn get_uptime() -> f64 {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) };
    assert!(ret == 0);
    time.tv_sec as f64 + time.tv_nsec as f64 * 1e-9
}

#[cfg(target_os = "ios")]
fn get_uptime() -> f64 {
    use crate::objc::*;
    unsafe {
        let process_info: ObjcId = msg_send![class!(NSProcessInfo), processInfo];
        msg_send![process_info, systemUptime]
    }
}

#[cfg(target_os = "windows")]
fn get_uptime() -> f64 {
    use std::time::SystemTime;
    let start = SystemTime::UNIX_EPOCH;
    let now = SystemTime::now();
    let duration = now.duration_since(start).expect("Time went backwards");
    duration.as_secs() as f64 + duration.subsec_nanos() as f64 * 1e-9
}

pub struct FlickTracker {
    threshold: f32,
    last_point: Point,
    last_delta: Option<Vector>,
    last_time: f32,
    flicked: bool,
    stopped: bool,
    judged_this_swipe: bool, // 记录这次滑动是否已经判定过（用于Phigros模式）
    phigros_mode: bool, // 记录是否为Phigros模式
}

impl FlickTracker {
    pub fn new(_dpi: u32, time: f32, point: Point) -> Self {
        // TODO maybe a better approach?
        let dpi = 275;
        Self {
            threshold: FLICK_SPEED_THRESHOLD * dpi as f32 / 386.,
            last_point: point,
            last_delta: None,
            last_time: time,
            flicked: false,
            stopped: true,
            judged_this_swipe: false,
            phigros_mode: false, // 默认false，会在update时设置
        }
    }

    pub fn push(&mut self, time: f32, position: Point) {
        let delta = position - self.last_point;
        self.last_point = position;
        if let Some(last_delta) = &self.last_delta {
            let dt = time - self.last_time;
            let speed = delta.dot(last_delta) / dt;
            
            // 检测方向是否改变（速度投影为负表示方向相反）
            let direction_changed = speed < 0.;
            
            // 只在速度低于阈值时设置stopped = true
            if speed < self.threshold {
                self.stopped = true;
            }
            
            // 速度变慢或方向改变时，重置judged_this_swipe，允许下次滑动判定
            if self.stopped || direction_changed {
                if self.judged_this_swipe {
                    // 记录重置事件
                    let log_msg = format!(
                        "TRACKER_RESET: judged_this_swipe reset | stopped={} | direction_changed={} | speed={:.2}",
                        self.stopped, direction_changed, speed
                    );
                    write_judge_log(&log_msg);
                }
                self.judged_this_swipe = false;
            }
            
            // 只有在stopped状态且还没flicked时，才检测新的flick
            // 在Phigros模式下，如果已经判定过这次滑动，就不再触发新的flick
            if self.stopped && !self.flicked && (!self.phigros_mode || !self.judged_this_swipe) {
                let is_flick = delta.magnitude() / dt >= self.threshold * 2.;
                if is_flick {
                    self.flicked = true;
                }
            }
        }
        self.last_delta = Some(delta.normalize());
        self.last_time = time;
    }
}

#[derive(Debug)]
pub enum JudgeStatus {
    NotJudged,
    PreJudge,
    Judged,
    Hold(bool, f32, f32, bool, f32), // perfect, at, diff, pre-judge, up-time
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Serialize)]
pub enum Judgement {
    Perfect,
    Good,
    Bad,
    Miss,
}

#[cfg(not(feature = "closed"))]
#[derive(Default)]
pub(crate) struct JudgeInner {
    diffs: Vec<f32>,

    combo: u32,
    max_combo: u32,
    perfect_combo: u32, // 新增：Perfect Combo 计数
    counts: [u32; 4],
    num_of_notes: u32,
}

#[cfg(not(feature = "closed"))]
impl JudgeInner {
    pub fn new(num_of_notes: u32) -> Self {
        Self {
            diffs: Vec::new(),

            combo: 0,
            max_combo: 0,
            perfect_combo: 0, // 初始化
            counts: [0; 4],
            num_of_notes,
        }
    }

    pub fn commit(&mut self, what: Judgement, diff: f32) {
        use Judgement::*;
        if matches!(what, Judgement::Good) {
            self.diffs.push(diff);
        }
        self.counts[what as usize] += 1;
        match what {
            Perfect | Good => {
                self.combo += 1;
                if self.combo > self.max_combo {
                    self.max_combo = self.combo;
                }
                // 更新 Perfect Combo
                if matches!(what, Perfect) {
                    self.perfect_combo += 1;
                } else {
                    self.perfect_combo = 0;
                }
            }
            _ => {
                self.combo = 0;
                self.perfect_combo = 0;
            }
        }
    }

    pub fn reset(&mut self) {
        self.combo = 0;
        self.max_combo = 0;
        self.perfect_combo = 0;
        self.counts = [0; 4];
        self.diffs.clear();
    }

    pub fn accuracy(&self) -> f64 {
        (self.counts[0] as f64 + self.counts[1] as f64 * 0.65) / self.num_of_notes as f64
    }

    pub fn real_time_accuracy(&self) -> f64 {
        let cnt = self.counts.iter().sum::<u32>();
        if cnt == 0 {
            return 1.;
        }
        (self.counts[0] as f64 + self.counts[1] as f64 * 0.65) / cnt as f64
    }

    pub fn score(&self) -> u32 {
        const TOTAL: u32 = 1000000;
        if self.counts[0] == self.num_of_notes {
            TOTAL
        } else {
            let score = (0.9 * self.accuracy() + self.max_combo as f64 / self.num_of_notes as f64 * 0.1) * TOTAL as f64;
            score.round() as u32
        }
    }

    pub fn result(&self) -> PlayResult {
        let early = self.diffs.iter().filter(|it| **it < 0.).count() as u32;
        
        // 计算标准差
        let std = if !self.diffs.is_empty() {
            let mean = self.diffs.iter().sum::<f32>() / self.diffs.len() as f32;
            let variance = self.diffs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / self.diffs.len() as f32;
            variance.sqrt()
        } else {
            0.
        };
        
        PlayResult {
            score: self.score(),
            accuracy: self.accuracy(),
            max_combo: self.max_combo,
            num_of_notes: self.num_of_notes,
            counts: self.counts,
            early,
            late: self.diffs.len() as u32 - early,
            std,
        }
    }

    pub fn combo(&self) -> u32 {
        self.combo
    }

    pub fn perfect_combo(&self) -> u32 {
        self.perfect_combo
    }

    pub fn counts(&self) -> [u32; 4] {
        self.counts
    }
}

#[rustfmt::skip]
#[cfg(feature = "closed")]
pub mod inner;
#[cfg(feature = "closed")]
use inner::*;

#[repr(C)]
pub struct Judge {
    // notes of each line in order
    // LinkedList::drain_filter is unstable...
    pub notes: Vec<(Vec<u32>, usize)>,
    pub trackers: HashMap<u64, FlickTracker>,
    pub last_time: f32,

    key_down_count: u32,

    pub(crate) inner: JudgeInner,
    pub judgements: RefCell<Vec<(f32, u32, u32, Result<Judgement, bool>)>>,
    pub last_judge_offset: RefCell<Option<f32>>, // 最近一次判定的偏差（秒）
    pub last_good_early_late: RefCell<Option<bool>>, // 最近一次 Good 判定是否为 EARLY (true=EARLY, false=LATE)
    pub last_good_time: RefCell<f32>, // 最近一次 Good 判定的时间
    
    // Drag保护：记录最近的drag判定信息 (时间, 判定线ID, x位置)
    drag_history: RefCell<Vec<(f32, usize, f32)>>,
}

static SUBSCRIBER_ID: Lazy<usize> = Lazy::new(register_input_subscriber);
thread_local! {
    static TOUCHES: RefCell<(Vec<Touch>, i32, u32)> = RefCell::default();
}

impl Judge {
    pub fn new(chart: &Chart) -> Self {
        let notes = chart
            .lines
            .iter()
            .map(|line| {
                let mut idx: Vec<u32> = (0..(line.notes.len() as u32)).filter(|it| !line.notes[*it as usize].fake).collect();
                idx.sort_by_key(|id| line.notes[*id as usize].time.not_nan());
                (idx, 0)
            })
            .collect();
        Self {
            notes,
            trackers: HashMap::new(),
            last_time: 0.,

            key_down_count: 0,

            inner: JudgeInner::new(chart.lines.iter().map(|it| it.notes.iter().filter(|it| !it.fake).count() as u32).sum()),
            judgements: RefCell::new(Vec::new()),
            last_judge_offset: RefCell::new(None),
            last_good_early_late: RefCell::new(None),
            last_good_time: RefCell::new(0.),
            drag_history: RefCell::new(Vec::new()),
        }
    }

    pub fn reset(&mut self) {
        self.notes.iter_mut().for_each(|it| it.1 = 0);
        self.trackers.clear();
        self.inner.reset();
        *self.last_judge_offset.borrow_mut() = None;
        *self.last_good_early_late.borrow_mut() = None;
        *self.last_good_time.borrow_mut() = 0.;
        self.judgements.borrow_mut().clear();
    }

    pub fn commit(&mut self, t: f32, what: Judgement, line_id: u32, note_id: u32, diff: f32, note_kind: &NoteKind) {
        let log_msg = format!(
            "JUDGE_COMMIT: result={:?} | diff={:.1}ms | line_id={} | note_id={}",
            what,
            diff * 1000.0,
            line_id,
            note_id
        );
        info!("{}", log_msg);
        write_judge_log(&log_msg);
        
        self.judgements.borrow_mut().push((t, line_id, note_id, Ok(what)));
        self.inner.commit(what, diff);
        // 记录最近的判定偏差（不包括 Miss、Drag 和 Flick）
        if !matches!(what, Judgement::Miss) && !matches!(note_kind, NoteKind::Drag | NoteKind::Flick) {
            *self.last_judge_offset.borrow_mut() = Some(diff);
        }
        // 记录 Good 判定的 EARLY/LATE 信息（不包括 Drag 和 Flick）
        if matches!(what, Judgement::Good) && !matches!(note_kind, NoteKind::Drag | NoteKind::Flick) {
            *self.last_good_early_late.borrow_mut() = Some(diff < 0.0); // true=EARLY, false=LATE
            *self.last_good_time.borrow_mut() = t;
        }
    }

    #[inline]
    pub fn accuracy(&self) -> f64 {
        self.inner.accuracy()
    }

    #[inline]
    pub fn real_time_accuracy(&self) -> f64 {
        self.inner.real_time_accuracy()
    }

    #[inline]
    pub fn score(&self) -> u32 {
        self.inner.score()
    }

    pub(crate) fn on_new_frame() {
        let mut handler = Handler(Vec::new(), 0, 0);
        repeat_all_miniquad_input(&mut handler, *SUBSCRIBER_ID);
        handler.finalize();
        TOUCHES.with(|it| {
            *it.borrow_mut() = (handler.0, handler.1, handler.2);
        });
    }

    fn touch_transform(flip_x: bool) -> impl Fn(&mut Touch) {
        let vp = get_viewport();
        move |touch| {
            let p = touch.position;
            touch.position = vec2(
                (p.x - vp.0 as f32) / vp.2 as f32 * 2. - 1.,
                ((p.y - (screen_height() - (vp.1 + vp.3) as f32)) / vp.3 as f32 * 2. - 1.) / (vp.2 as f32 / vp.3 as f32),
            );
            if flip_x {
                touch.position.x *= -1.;
            }
        }
    }

    pub fn get_touches() -> Vec<Touch> {
        TOUCHES.with(|it| {
            let guard = it.borrow();
            let tr = Self::touch_transform(false);
            guard
                .0
                .iter()
                .cloned()
                .map(|mut it| {
                    tr(&mut it);
                    it
                })
                .collect()
        })
    }

    pub fn update(&mut self, res: &mut Resource, chart: &mut Chart, bad_notes: &mut Vec<BadNote>) {
        if res.config.autoplay() {
            self.auto_play_update(res, chart);
            return;
        }
        const X_DIFF_MAX: f32 = 0.21 / (16. / 9.) * 2.;
        let spd = res.config.speed;

        // Get judge limits based on mode
        let limit_perfect = Self::get_limit_perfect(&res.config);
        let limit_good = Self::get_limit_good(&res.config);
        let limit_bad = Self::get_limit_bad(&res.config);

        let uptime = get_uptime();

        let t = res.time;
        // TODO optimize
        let mut touches: HashMap<u64, Touch> = {
            let mut touches = touches();
            let btn = MouseButton::Left;
            let id = button_to_id(btn);
            if is_mouse_button_pressed(btn) {
                let p = mouse_position();
                touches.push(Touch {
                    id,
                    phase: TouchPhase::Started,
                    position: vec2(p.0, p.1),
                    time: f64::NEG_INFINITY,
                });
            } else if is_mouse_button_down(btn) {
                let p = mouse_position();
                touches.push(Touch {
                    id,
                    phase: TouchPhase::Moved,
                    position: vec2(p.0, p.1),
                    time: f64::NEG_INFINITY,
                });
            } else if is_mouse_button_released(btn) {
                let p = mouse_position();
                touches.push(Touch {
                    id,
                    phase: TouchPhase::Ended,
                    position: vec2(p.0, p.1),
                    time: f64::NEG_INFINITY,
                });
            }
            let tr = Self::touch_transform(res.config.flip_x());
            touches
                .into_iter()
                .map(|mut it| {
                    tr(&mut it);
                    (it.id, it)
                })
                .collect()
        };
        let (events, keys_down) = TOUCHES.with(|it| {
            let guard = it.borrow();
            (guard.0.clone(), guard.2)
        });
        self.key_down_count = self.key_down_count.saturating_add_signed(TOUCHES.with(|it| it.borrow().1));
        {
            fn to_local(Vec2 { x, y }: Vec2) -> Point {
                Point::new(x / screen_width() * 2. - 1., y / screen_height() * 2. - 1.)
            }
            let delta = (t / spd - self.last_time) as f64 / (events.len() + 1) as f64;
            let mut t = self.last_time as f64;
            for Touch {
                id,
                phase,
                position: p,
                time,
            } in events.into_iter()
            {
                t += delta;
                let t = t as f32;
                let p = to_local(p);
                match phase {
                    TouchPhase::Started => {
                        self.trackers.insert(id, FlickTracker::new(res.dpi, t, p));
                        // 设置Phigros模式标志
                        if let Some(tracker) = self.trackers.get_mut(&id) {
                            tracker.phigros_mode = res.config.relaxed_judge;
                        }
                        touches
                            .entry(id)
                            .or_insert_with(|| Touch {
                                id,
                                phase: TouchPhase::Started,
                                position: vec2(p.x, p.y),
                                time,
                            })
                            .phase = TouchPhase::Started;
                    }
                    TouchPhase::Moved | TouchPhase::Stationary => {
                        if let Some(tracker) = self.trackers.get_mut(&id) {
                            tracker.push(t, p);
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        self.trackers.remove(&id);
                    }
                }
            }
        }
        let touches: Vec<Touch> = touches
            .into_values()
            .map(|mut it| {
                it.time = if it.time.is_infinite() {
                    f64::NEG_INFINITY
                } else {
                    t as f64 - (uptime - it.time) * spd as f64
                };
                it
            })
            .collect();
        // pos[line][touch]
        let mut pos = Vec::<Vec<Option<Point>>>::with_capacity(chart.lines.len());
        for id in 0..pos.capacity() {
            chart.lines[id].object.set_time(t);
            let inv = chart.lines[id].now_transform(res, &chart.lines).try_inverse().unwrap();
            pos.push(
                touches
                    .iter()
                    .map(|touch| {
                        let p = touch.position;
                        let p = inv.transform_point(&Point::new(p.x, -p.y));
                        fn ok(f: f32) -> bool {
                            matches!(f.classify(), FpCategory::Zero | FpCategory::Subnormal | FpCategory::Normal)
                        }
                        if ok(p.x) && ok(p.y) {
                            Some(p)
                        } else {
                            None
                        }
                    })
                    .collect(),
            );
        }
        let time_of = |touch: &Touch| {
            if touch.time.is_infinite() {
                t
            } else {
                touch.time as f32
            }
        };
        let mut judgements = Vec::new();
        // clicks & flicks
        for (id, touch) in touches.iter().enumerate() {
            let click = touch.phase == TouchPhase::Started;
            let flick = matches!(touch.phase, TouchPhase::Moved | TouchPhase::Stationary) 
                && self.trackers.get_mut(&touch.id).is_some_and(|it| {
                    // 在Phigros模式下，如果这次滑动已经判定过，就不再触发
                    it.flicked && (!res.config.relaxed_judge || !it.judged_this_swipe)
                });
            if !(click || flick) {
                continue;
            }
            let t = time_of(touch);
            let mut closest = (None, X_DIFF_MAX, limit_bad, limit_bad + (X_DIFF_MAX / NOTE_WIDTH_RATIO_BASE - 1.).max(0.) * DIST_FACTOR);
            for (line_id, ((line, pos), (idx, st))) in chart.lines.iter_mut().zip(pos.iter()).zip(self.notes.iter_mut()).enumerate() {
                let Some(pos) = pos[id] else {
                    continue;
                };
                for id in &idx[*st..] {
                    let note = &mut line.notes[*id as usize];
                    if !matches!(note.judge, JudgeStatus::NotJudged | JudgeStatus::PreJudge) {
                        continue;
                    }
                    if !click && matches!(note.kind, NoteKind::Click | NoteKind::Hold { .. }) {
                        continue;
                    }
                    let dt = (note.time - t) / spd;
                    if dt >= closest.3 {
                        break;
                    }
                    // 在Phigros判定模式下不使用EARLY_OFFSET
                    let dt = if dt < 0. {
                        if res.config.relaxed_judge {
                            // Phigros模式：不使用EARLY_OFFSET，直接取绝对值
                            dt.abs()
                        } else {
                            // Phira原版：使用EARLY_OFFSET
                            (dt + EARLY_OFFSET).min(0.).abs()
                        }
                    } else {
                        dt
                    };
                    let x = &mut note.object.translation.0;
                    x.set_time(t);
                    let dist = (x.now() - pos.x).abs();
                    if dist > X_DIFF_MAX {
                        continue;
                    }
                    if dt
                        > if matches!(note.kind, NoteKind::Click) {
                            // Drag保护：只在Phigros判定模式且非严格判定时启用
                            if res.config.relaxed_judge && !res.config.strict_judge && dist > 0.9 {
                                limit_bad - limit_perfect * (dist - 0.9) * 0.5
                            } else {
                                limit_bad
                            }
                        } else {
                            limit_good
                        }
                    {
                        continue;
                    }
                    let dt = if matches!(note.kind, NoteKind::Flick | NoteKind::Drag) {
                        // Flick和Drag使用特殊的判定窗口
                        // Phigros模式：使用140ms窗口（perfectTimeRange × 1.75）
                        // Phira模式：使用160ms窗口（limit_good）
                        let flick_limit = if res.config.relaxed_judge {
                            LIMIT_FLICK_RELAXED
                        } else {
                            limit_good
                        };
                        dt + flick_limit
                    } else {
                        dt
                    };
                    let key = dt + (dist / NOTE_WIDTH_RATIO_BASE - 1.).max(0.) * DIST_FACTOR;
                    if key < closest.3 {
                        closest = (Some((line_id, *id)), dist, dt, key);
                    }
                }
            }
            if let (Some((line_id, id)), _, dt, _) = closest {
                let line = &mut chart.lines[line_id];
                if matches!(line.notes[id as usize].kind, NoteKind::Drag) {
                    debug!("reject by drag");
                    continue;
                }
                if click {
                    // click & hold
                    let note = &mut line.notes[id as usize];
                    if matches!(note.kind, NoteKind::Flick) {
                        continue;
                    }
                    if dt <= limit_bad || matches!(note.kind, NoteKind::Hold { .. }) {
                        match note.kind {
                            NoteKind::Click => {
                                note.judge = JudgeStatus::Judged;
                                
                                // Drag保护逻辑：只在Phigros判定模式且非严格判定时启用
                                let mut judgement = if dt <= limit_perfect {
                                    Judgement::Perfect
                                } else if dt <= limit_good {
                                    Judgement::Good
                                } else {
                                    Judgement::Bad
                                };
                                
                                if res.config.relaxed_judge && !res.config.strict_judge {
                                    // 清理过期的drag历史（超过150ms）
                                    let mut drag_hist = self.drag_history.borrow_mut();
                                    drag_hist.retain(|(drag_time, _, _)| (note.time - drag_time).abs() <= 0.15);
                                    
                                    // 检查是否有drag保护
                                    let note_x = {
                                        let mut x = note.object.translation.0.clone();
                                        x.set_time(note.time);
                                        x.now()
                                    };
                                    
                                    for (drag_time, drag_line_id, drag_x) in drag_hist.iter() {
                                        // 检查是否在同一判定线
                                        if *drag_line_id == line_id {
                                            let time_diff = (note.time - drag_time).abs();
                                            let pos_diff = (note_x - drag_x).abs();
                                            
                                            // 同一位置的判定：x坐标差距小于0.1
                                            if pos_diff <= 0.1 {
                                                if time_diff <= 0.08 {
                                                    // 80ms内：强制Perfect
                                                    judgement = Judgement::Perfect;
                                                    let log_msg = format!(
                                                        "DRAG_PROTECT_80MS: tap forced to Perfect | time_diff={:.1}ms | pos_diff={:.3}",
                                                        time_diff * 1000.0, pos_diff
                                                    );
                                                    info!("{}", log_msg);
                                                    write_judge_log(&log_msg);
                                                    break;
                                                } else if time_diff <= 0.15 && matches!(judgement, Judgement::Bad) {
                                                    // 150ms内：Bad提升为Good
                                                    judgement = Judgement::Good;
                                                    let log_msg = format!(
                                                        "DRAG_PROTECT_150MS: tap upgraded from Bad to Good | time_diff={:.1}ms | pos_diff={:.3}",
                                                        time_diff * 1000.0, pos_diff
                                                    );
                                                    info!("{}", log_msg);
                                                    write_judge_log(&log_msg);
                                                }
                                            }
                                        }
                                    }
                                }
                                
                                let log_msg = format!(
                                    "JUDGE_PATH_1_TOUCH: Click note | dt={:.1}ms | result={:?} | limit_p={:.1}ms | limit_g={:.1}ms | limit_b={:.1}ms | strict={} | phigros={}",
                                    dt * 1000.0,
                                    judgement,
                                    limit_perfect * 1000.0,
                                    limit_good * 1000.0,
                                    limit_bad * 1000.0,
                                    res.config.strict_judge,
                                    res.config.relaxed_judge
                                );
                                info!("{}", log_msg);
                                write_judge_log(&log_msg);
                                // 使用原版的方式：传入t，在commit时计算diff
                                judgements.push((judgement, line_id, id, Some(t)));
                            }
                            NoteKind::Hold { .. } => {
                                note.hitsound.play(res);
                                self.judgements.borrow_mut().push((t, line_id as _, id, Err(dt <= limit_perfect)));
                                note.judge = JudgeStatus::Hold(dt <= limit_perfect, t, t, false, f32::INFINITY);
                                
                                // Hold 音符按下时立即显示判定偏差和 EARLY/LATE
                                let diff = (t - note.time) / spd;
                                if dt <= limit_perfect {
                                    *self.last_judge_offset.borrow_mut() = Some(diff);
                                } else if dt <= limit_good {
                                    *self.last_judge_offset.borrow_mut() = Some(diff);
                                    *self.last_good_early_late.borrow_mut() = Some(diff < 0.0);
                                    *self.last_good_time.borrow_mut() = t;
                                }
                            }
                            _ => unreachable!(),
                        };
                    } else {
                        // prevent extra judgements
                        if matches!(note.judge, JudgeStatus::NotJudged) {
                            // 超过limit_bad的音符直接判定为Bad，不再重新判定
                            let log_msg = format!(
                                "JUDGE_PATH_2_TIMEOUT: Click note | dt={:.1}ms | result=Bad | limit_b={:.1}ms | strict={} | phigros={}",
                                dt * 1000.0,
                                limit_bad * 1000.0,
                                res.config.strict_judge,
                                res.config.relaxed_judge
                            );
                            info!("{}", log_msg);
                            write_judge_log(&log_msg);
                            line.notes[id as usize].judge = JudgeStatus::Judged;
                            judgements.push((Judgement::Bad, line_id, id, None));
                        }
                    }
                } else {
                    // flick
                    let note = &mut line.notes[id as usize];
                    note.judge = JudgeStatus::PreJudge;
                    
                    let log_msg = format!(
                        "FLICK_JUDGED: line_id={} | note_id={} | touch_id={} | dt={:.1}ms | phigros_mode={}",
                        line_id,
                        id,
                        touch.id,
                        dt * 1000.0,
                        res.config.relaxed_judge
                    );
                    info!("{}", log_msg);
                    write_judge_log(&log_msg);
                    
                    if let Some(tracker) = self.trackers.get_mut(&touch.id) {
                        // 重置flicked，允许下次检测
                        tracker.flicked = false;
                        
                        if res.config.relaxed_judge {
                            // Phigros模式：标记这次滑动已判定，防止重复判定
                            tracker.judged_this_swipe = true;
                            let log_msg = format!(
                                "PHIGROS_MODE: judged_this_swipe=true, no more flicks until speed drops"
                            );
                            info!("{}", log_msg);
                            write_judge_log(&log_msg);
                        }
                    }
                }
            }
        }
        for _ in 0..keys_down {
            // find the earliest not judged click / hold note
            if let Some((line_id, id)) = chart
                .lines
                .iter()
                .zip(self.notes.iter())
                .enumerate()
                .filter_map(|(line_id, (line, (idx, st)))| {
                    idx[*st..]
                        .iter()
                        .cloned()
                        .find(|id| {
                            let note = &line.notes[*id as usize];
                            matches!(note.judge, JudgeStatus::NotJudged) && matches!(note.kind, NoteKind::Click | NoteKind::Hold { .. })
                        })
                        .map(|id| (line_id, id))
                })
                .min_by_key(|(line_id, id)| chart.lines[*line_id].notes[*id as usize].time.not_nan())
            {
                let note = &mut chart.lines[line_id].notes[id as usize];
                let dt = (t - note.time).abs() / spd;
                if dt <= if matches!(note.kind, NoteKind::Click) { limit_bad } else { limit_good } {
                    match note.kind {
                        NoteKind::Click => {
                            note.judge = JudgeStatus::Judged;
                            
                            // Drag保护逻辑：只在Phigros判定模式且非严格判定时启用
                            let mut judgement = if dt <= limit_perfect {
                                Judgement::Perfect
                            } else if dt <= limit_good {
                                Judgement::Good
                            } else {
                                Judgement::Bad
                            };
                            
                            if res.config.relaxed_judge && !res.config.strict_judge {
                                // 清理过期的drag历史（超过150ms）
                                let mut drag_hist = self.drag_history.borrow_mut();
                                drag_hist.retain(|(drag_time, _, _)| (note.time - drag_time).abs() <= 0.15);
                                
                                // 检查是否有drag保护
                                let note_x = {
                                    let mut x = note.object.translation.0.clone();
                                    x.set_time(note.time);
                                    x.now()
                                };
                                
                                for (drag_time, drag_line_id, drag_x) in drag_hist.iter() {
                                    // 检查是否在同一判定线
                                    if *drag_line_id == line_id {
                                        let time_diff = (note.time - drag_time).abs();
                                        let pos_diff = (note_x - drag_x).abs();
                                        
                                        // 同一位置的判定：x坐标差距小于0.1
                                        if pos_diff <= 0.1 {
                                            if time_diff <= 0.08 {
                                                // 80ms内：强制Perfect
                                                judgement = Judgement::Perfect;
                                                let log_msg = format!(
                                                    "DRAG_PROTECT_80MS_KB: tap forced to Perfect | time_diff={:.1}ms | pos_diff={:.3}",
                                                    time_diff * 1000.0, pos_diff
                                                );
                                                info!("{}", log_msg);
                                                write_judge_log(&log_msg);
                                                break;
                                            } else if time_diff <= 0.15 && matches!(judgement, Judgement::Bad) {
                                                // 150ms内：Bad提升为Good
                                                judgement = Judgement::Good;
                                                let log_msg = format!(
                                                    "DRAG_PROTECT_150MS_KB: tap upgraded from Bad to Good | time_diff={:.1}ms | pos_diff={:.3}",
                                                    time_diff * 1000.0, pos_diff
                                                );
                                                info!("{}", log_msg);
                                                write_judge_log(&log_msg);
                                            }
                                        }
                                    }
                                }
                            }
                            
                            let log_msg = format!(
                                "JUDGE_PATH_3_KEYBOARD: Click note | dt={:.1}ms | result={:?} | limit_p={:.1}ms | limit_g={:.1}ms | limit_b={:.1}ms | strict={} | phigros={}",
                                dt * 1000.0,
                                judgement,
                                limit_perfect * 1000.0,
                                limit_good * 1000.0,
                                limit_bad * 1000.0,
                                res.config.strict_judge,
                                res.config.relaxed_judge
                            );
                            info!("{}", log_msg);
                            write_judge_log(&log_msg);
                            judgements.push((
                                judgement,
                                line_id,
                                id,
                                None,
                            ));
                        }
                        NoteKind::Hold { .. } => {
                            note.hitsound.play(res);
                            self.judgements.borrow_mut().push((t, line_id as _, id, Err(dt <= limit_perfect)));
                            note.judge = JudgeStatus::Hold(dt <= limit_perfect, t, (t - note.time) / spd, false, f32::INFINITY);
                            
                            // Hold 音符按下时立即显示判定偏差和 EARLY/LATE
                            let diff = (t - note.time) / spd;
                            if dt <= limit_perfect {
                                *self.last_judge_offset.borrow_mut() = Some(diff);
                            } else if dt <= limit_good {
                                *self.last_judge_offset.borrow_mut() = Some(diff);
                                *self.last_good_early_late.borrow_mut() = Some(diff < 0.0);
                                *self.last_good_time.borrow_mut() = t;
                            }
                        }
                        _ => unreachable!(),
                    };
                }
            } else {
                break;
            }
        }
        for (line_id, ((line, pos), (idx, st))) in chart.lines.iter_mut().zip(pos.iter()).zip(self.notes.iter()).enumerate() {
            line.object.set_time(t);
            for id in &idx[*st..] {
                let note = &mut line.notes[*id as usize];
                if let NoteKind::Hold { end_time, .. } = &note.kind {
                    if let JudgeStatus::Hold(.., ref mut pre_judge, ref mut up_time) = note.judge {
                        if (*end_time - t) / spd <= limit_bad {
                            *pre_judge = true;
                            continue;
                        }
                        let x = &mut note.object.translation.0;
                        x.set_time(t);
                        let x = x.now();
                        if self.key_down_count == 0 && !pos.iter().any(|it| it.is_some_and(|it| (it.x - x).abs() <= X_DIFF_MAX)) {
                            if t > *up_time + UP_TOLERANCE {
                                note.judge = JudgeStatus::Judged;
                                judgements.push((Judgement::Miss, line_id, *id, None));
                            } else if up_time.is_infinite() {
                                *up_time = t;
                            }
                        } else {
                            *up_time = f32::INFINITY;
                        }
                        continue;
                    }
                }
                if !matches!(note.judge, JudgeStatus::NotJudged) {
                    continue;
                }
                // process miss
                let dt = (t - note.time) / spd;
                if dt > limit_bad {
                    note.judge = JudgeStatus::Judged;
                    judgements.push((Judgement::Miss, line_id, *id, None));
                    continue;
                }
                if -dt > limit_bad {
                    break;
                }
                if !matches!(note.kind, NoteKind::Drag) && (self.key_down_count == 0 || !matches!(note.kind, NoteKind::Flick)) {
                    continue;
                }
                let dt = dt.abs();
                let x = &mut note.object.translation.0;
                x.set_time(t);
                let x = x.now();
                if self.key_down_count != 0
                    || pos.iter().any(|it| {
                        it.is_some_and(|it| {
                            let dx = (it.x - x).abs();
                            dx <= X_DIFF_MAX && dt <= (limit_bad - limit_perfect * (dx - 0.9).max(0.))
                        })
                    })
                {
                    note.judge = JudgeStatus::PreJudge;
                    
                    // 记录drag的位置和时间用于保护tap
                    if matches!(note.kind, NoteKind::Drag) {
                        self.drag_history.borrow_mut().push((note.time, line_id, x));
                        let log_msg = format!(
                            "DRAG_RECORDED: time={:.3}s | line_id={} | x={:.3}",
                            note.time, line_id, x
                        );
                        info!("{}", log_msg);
                        write_judge_log(&log_msg);
                    }
                }
            }
        }
        // process pre-judge
        for (line_id, (line, (idx, st))) in chart.lines.iter_mut().zip(self.notes.iter()).enumerate() {
            line.object.set_time(t);
            for id in &idx[*st..] {
                let note = &mut line.notes[*id as usize];
                if let JudgeStatus::Hold(perfect, .., diff, true, _) = note.judge {
                    if let NoteKind::Hold { end_time, .. } = &note.kind {
                        if *end_time <= t {
                            note.judge = JudgeStatus::Judged;
                            judgements.push((if perfect { Judgement::Perfect } else { Judgement::Good }, line_id, *id, Some(diff)));
                            continue;
                        }
                    }
                }
                // TODO adjust
                let ghost_t = t + limit_good;
                if matches!(note.kind, NoteKind::Click) {
                    if ghost_t < note.time {
                        break;
                    }
                } else if t < note.time {
                    continue;
                }
                if matches!(note.judge, JudgeStatus::PreJudge) {
                    let diff = if let JudgeStatus::Hold(.., diff, _, _) = note.judge {
                        Some(diff)
                    } else {
                        None
                    };
                    note.judge = JudgeStatus::Judged;
                    if matches!(note.kind, NoteKind::Click) {
                        // Click音符也需要根据时间偏差判定等级
                        let dt = (t - note.time).abs() / spd;
                        
                        // Drag保护逻辑：只在Phigros判定模式且非严格判定时启用
                        let mut judgement = if dt <= limit_perfect {
                            Judgement::Perfect
                        } else if dt <= limit_good {
                            Judgement::Good
                        } else {
                            Judgement::Bad
                        };
                        
                        if res.config.relaxed_judge && !res.config.strict_judge {
                            // 清理过期的drag历史（超过150ms）
                            let mut drag_hist = self.drag_history.borrow_mut();
                            drag_hist.retain(|(drag_time, _, _)| (note.time - drag_time).abs() <= 0.15);
                            
                            // 检查是否有drag保护
                            let note_x = {
                                let mut x = note.object.translation.0.clone();
                                x.set_time(note.time);
                                x.now()
                            };
                            
                            for (drag_time, drag_line_id, drag_x) in drag_hist.iter() {
                                // 检查是否在同一判定线
                                if *drag_line_id == line_id {
                                    let time_diff = (note.time - drag_time).abs();
                                    let pos_diff = (note_x - drag_x).abs();
                                    
                                    // 同一位置的判定：x坐标差距小于0.1
                                    if pos_diff <= 0.1 {
                                        if time_diff <= 0.08 {
                                            // 80ms内：强制Perfect
                                            judgement = Judgement::Perfect;
                                            let log_msg = format!(
                                                "DRAG_PROTECT_80MS_PRE: tap forced to Perfect | time_diff={:.1}ms | pos_diff={:.3}",
                                                time_diff * 1000.0, pos_diff
                                            );
                                            info!("{}", log_msg);
                                            write_judge_log(&log_msg);
                                            break;
                                        } else if time_diff <= 0.15 && matches!(judgement, Judgement::Bad) {
                                            // 150ms内：Bad提升为Good
                                            judgement = Judgement::Good;
                                            let log_msg = format!(
                                                "DRAG_PROTECT_150MS_PRE: tap upgraded from Bad to Good | time_diff={:.1}ms | pos_diff={:.3}",
                                                time_diff * 1000.0, pos_diff
                                            );
                                            info!("{}", log_msg);
                                            write_judge_log(&log_msg);
                                        }
                                    }
                                }
                            }
                        }
                        
                        let log_msg = format!(
                            "JUDGE_PATH_4_PREJUDGE: Click note | dt={:.1}ms | result={:?} | limit_p={:.1}ms | limit_g={:.1}ms | limit_b={:.1}ms | strict={} | phigros={}",
                            dt * 1000.0,
                            judgement,
                            limit_perfect * 1000.0,
                            limit_good * 1000.0,
                            limit_bad * 1000.0,
                            res.config.strict_judge,
                            res.config.relaxed_judge
                        );
                        info!("{}", log_msg);
                        write_judge_log(&log_msg);
                        judgements.push((judgement, line_id, *id, diff));
                    } else {
                        // Drag和Flick音符在PreJudge状态下判定为Perfect
                        judgements.push((Judgement::Perfect, line_id, *id, diff));
                    }
                }
            }
        }
        for (judgement, line_id, id, diff) in judgements {
            let line = &mut chart.lines[line_id];
            let note = &mut line.notes[id as usize];
            line.object.set_time(t);
            note.object.set_time(t);
            let line = &chart.lines[line_id];
            let note = &line.notes[id as usize];
            let line_tr = line.now_transform(res, &chart.lines);
            self.commit(
                t,
                judgement,
                line_id as _,
                id,
                if matches!(judgement, Judgement::Miss) {
                    0.25
                } else if matches!(note.kind, NoteKind::Drag | NoteKind::Flick) {
                    0.
                } else {
                    (diff.unwrap_or(t) - note.time) / spd
                },
                &note.kind,
            );
            if matches!(note.kind, NoteKind::Hold { .. }) {
                continue;
            }
            if match judgement {
                Judgement::Perfect => {
                    res.with_model(line_tr * note.object.now(res), |res| res.emit_at_origin(note.rotation(line), res.res_pack.info.fx_perfect()));
                    true
                }
                Judgement::Good => {
                    res.with_model(line_tr * note.object.now(res), |res| res.emit_at_origin(note.rotation(line), res.res_pack.info.fx_good()));
                    true
                }
                Judgement::Bad => {
                    if !matches!(note.kind, NoteKind::Hold { .. }) {
                        bad_notes.push(BadNote {
                            time: t,
                            kind: note.kind.clone(),
                            matrix: {
                                let mut mat = line_tr;
                                if !note.above {
                                    mat.append_nonuniform_scaling_mut(&Vector::new(1., -1.));
                                }
                                let incline_sin = line.incline.now_opt().map(|it| it.to_radians().sin()).unwrap_or_default();
                                mat *= note.now_transform(
                                    res,
                                    &line.ctrl_obj.borrow_mut(),
                                    (note.height - line.height.now()) / res.aspect_ratio * note.speed,
                                    incline_sin,
                                );
                                mat
                            },
                        });
                    }
                    false
                }
                _ => false,
            } {
                note.hitsound.play(res);
            }
        }
        for (line, (idx, st)) in chart.lines.iter().zip(self.notes.iter_mut()) {
            while idx
                .get(*st)
                .is_some_and(|id| matches!(line.notes[*id as usize].judge, JudgeStatus::Judged))
            {
                *st += 1;
            }
        }
        self.last_time = t / spd;
    }

    fn auto_play_update(&mut self, res: &mut Resource, chart: &mut Chart) {
        let t = res.time;
        let spd = res.config.speed;
        let mut judgements = Vec::new();
        for (line_id, (line, (idx, st))) in chart.lines.iter_mut().zip(self.notes.iter_mut()).enumerate() {
            for id in &idx[*st..] {
                let note = &mut line.notes[*id as usize];
                if let JudgeStatus::Hold(..) = note.judge {
                    if let NoteKind::Hold { end_time, .. } = note.kind {
                        if t >= end_time {
                            note.judge = JudgeStatus::Judged;
                            judgements.push((line_id, *id));
                            continue;
                        }
                    }
                }
                if !matches!(note.judge, JudgeStatus::NotJudged) {
                    continue;
                }
                if note.time > t {
                    break;
                }
                note.judge = if matches!(note.kind, NoteKind::Hold { .. }) {
                    note.hitsound.play(res);
                    self.judgements.borrow_mut().push((t, line_id as _, *id, Err(true)));
                    JudgeStatus::Hold(true, t, (t - note.time) / spd, false, f32::INFINITY)
                } else {
                    judgements.push((line_id, *id));
                    JudgeStatus::Judged
                };
            }
            while idx
                .get(*st)
                .is_some_and(|id| matches!(line.notes[*id as usize].judge, JudgeStatus::Judged))
            {
                *st += 1;
            }
        }
        for (line_id, id) in judgements.into_iter() {
            self.commit(t, Judgement::Perfect, line_id as _, id, 0., &chart.lines[line_id].notes[id as usize].kind);
            let (note_transform, note_hitsound) = {
                let line = &mut chart.lines[line_id];
                let note = &mut line.notes[id as usize];
                let nt = if matches!(note.kind, NoteKind::Hold { .. }) { t } else { note.time };
                line.object.set_time(nt);
                note.object.set_time(nt);
                (note.object.now(res), note.hitsound.clone())
            };
            let line = &chart.lines[line_id];
            res.with_model(line.now_transform(res, &chart.lines) * note_transform, |res| {
                res.emit_at_origin(line.notes[id as usize].rotation(line), res.res_pack.info.fx_perfect())
            });
            if !matches!(chart.lines[line_id].notes[id as usize].kind, NoteKind::Hold { .. }) {
                note_hitsound.play(res);
            }
        }
    }

    #[inline]
    pub fn result(&self) -> PlayResult {
        self.inner.result()
    }

    #[inline]
    pub fn combo(&self) -> u32 {
        self.inner.combo()
    }

    #[inline]
    pub fn perfect_combo(&self) -> u32 {
        self.inner.perfect_combo()
    }

    #[inline]
    pub fn counts(&self) -> [u32; 4] {
        self.inner.counts()
    }
}

struct Handler(Vec<Touch>, i32, u32);
impl Handler {
    fn finalize(&mut self) {
        if is_mouse_button_down(MouseButton::Left) {
            self.0.push(Touch {
                id: button_to_id(MouseButton::Left),
                phase: TouchPhase::Moved,
                position: mouse_position().into(),
                time: f64::NEG_INFINITY,
            });
        }
    }
}

fn button_to_id(button: MouseButton) -> u64 {
    u64::MAX
        - match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::Unknown => 3,
        }
}

impl EventHandler for Handler {
    fn update(&mut self, _: &mut miniquad::Context) {}
    fn draw(&mut self, _: &mut miniquad::Context) {}
    fn touch_event(&mut self, _: &mut miniquad::Context, phase: miniquad::TouchPhase, id: u64, x: f32, y: f32, time: f64) {
        self.0.push(Touch {
            id,
            phase: phase.into(),
            position: vec2(x, y),
            time,
        });
    }

    fn mouse_button_down_event(&mut self, _ctx: &mut miniquad::Context, button: MouseButton, x: f32, y: f32) {
        self.0.push(Touch {
            id: button_to_id(button),
            phase: TouchPhase::Started,
            position: vec2(x, y),
            time: f64::NEG_INFINITY,
        });
    }

    fn mouse_button_up_event(&mut self, _ctx: &mut miniquad::Context, button: MouseButton, x: f32, y: f32) {
        self.0.push(Touch {
            id: button_to_id(button),
            phase: TouchPhase::Ended,
            position: vec2(x, y),
            time: f64::NEG_INFINITY,
        });
    }

    fn key_down_event(&mut self, _ctx: &mut miniquad::Context, _keycode: KeyCode, _keymods: miniquad::KeyMods, repeat: bool) {
        if !repeat {
            self.1 += 1;
            self.2 += 1;
        }
    }

    fn key_up_event(&mut self, _ctx: &mut miniquad::Context, _keycode: KeyCode, _keymods: miniquad::KeyMods) {
        self.1 -= 1;
    }
}

#[derive(Default)]
pub struct PlayResult {
    pub score: u32,
    pub accuracy: f64,
    pub max_combo: u32,
    pub num_of_notes: u32,
    pub counts: [u32; 4],
    pub early: u32,
    pub late: u32,
    pub std: f32,
}

pub fn icon_index(score: u32, full_combo: bool) -> usize {
    match (score, full_combo) {
        (x, _) if x < 700000 => 0,
        (x, _) if x < 820000 => 1,
        (x, _) if x < 880000 => 2,
        (x, _) if x < 920000 => 3,
        (x, _) if x < 960000 => 4,
        (1000000, _) => 7,
        (_, false) => 5,
        (_, true) => 6,
    }
}
