use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;
use std::time::Instant;

use crate::config::Appearance;
use crate::layout::{Bounds, Layout, Scene};
use crate::window::DockItem;

#[derive(Default)]
struct Motion {
    zoom: f32,
    velocity: f32,
    launched: Option<Instant>,
    action: Option<(Activation, Instant)>,
    press: f32,
    position: Option<f32>,
    position_velocity: f32,
}

#[derive(Clone, Copy)]
pub enum Activation {
    Focus,
    Minimize,
}

impl Motion {
    fn position(&mut self, target: f32, dt: Option<f32>) -> (f32, bool) {
        let current = self.position.get_or_insert(target);
        let Some(dt) = dt else {
            *current = target;
            self.position_velocity = 0.0;
            return (target, false);
        };
        let rate = 24.0;
        let error = *current - target;
        let impulse = (self.position_velocity + rate * error) * dt;
        let decay = (-rate * dt).exp();
        *current = target + (error + impulse) * decay;
        self.position_velocity = (self.position_velocity - rate * impulse) * decay;
        let moving = (*current - target).abs() > 0.05 || self.position_velocity.abs() > 0.1;
        if !moving {
            *current = target;
            self.position_velocity = 0.0;
        }
        (*current, moving)
    }

    fn zoom(&mut self, target: f32, dt: Option<f32>) -> bool {
        let Some(dt) = dt else {
            self.zoom = target;
            self.velocity = 0.0;
            return false;
        };
        // Analytic critically damped spring, independent of refresh rate.
        let rate = 22.0;
        let error = self.zoom - target;
        let impulse = (self.velocity + rate * error) * dt;
        let decay = (-rate * dt).exp();
        self.zoom = target + (error + impulse) * decay;
        self.velocity = (self.velocity - rate * impulse) * decay;
        let moving = (self.zoom - target).abs() > 0.001 || self.velocity.abs() > 0.02;
        if !moving {
            self.zoom = target;
            self.velocity = 0.0;
        }
        moving
    }

    fn lift(&mut self, now: Instant, speed: f32) -> f32 {
        let Some(start) = self.launched else {
            return 0.0;
        };
        let progress = (now - start).as_secs_f32() * speed / 0.46;
        if progress >= 1.0 {
            self.launched = None;
            return 0.0;
        }
        (progress * PI * 2.0).sin().abs() * (1.0 - progress) * 22.0
    }

    fn action(&mut self, now: Instant, speed: f32) -> (f32, f32) {
        let Some((action, start)) = self.action else {
            return (0.0, 0.0);
        };
        let progress = (now - start).as_secs_f32() * speed / 0.30;
        if progress >= 1.0 {
            self.action = None;
            return (0.0, 0.0);
        }
        let pulse = (progress * PI).sin();
        match action {
            Activation::Focus => (pulse * 9.0, pulse * 0.06),
            Activation::Minimize => (-pulse * 11.0, -pulse * 0.24),
        }
    }
}

pub struct Animation {
    motion: HashMap<String, Motion>,
    last_frame: Option<Instant>,
    slide: Motion,
    speed: f32,
    launch_bounce: bool,
}

impl Animation {
    pub fn new(hidden: bool, appearance: Appearance) -> Self {
        Self {
            motion: HashMap::new(),
            last_frame: None,
            slide: Motion {
                zoom: if hidden { 1.0 } else { 0.0 },
                ..Default::default()
            },
            speed: appearance.animation_speed_percent as f32 / 100.0,
            launch_bounce: appearance.launch_bounce,
        }
    }

    pub fn retain(&mut self, targets: &HashSet<String>) {
        self.motion.retain(|target, _| targets.contains(target));
    }

    pub fn launch(&mut self, target: &str) {
        if self.launch_bounce && self.speed > 0.0 {
            self.motion.entry(target.to_owned()).or_default().launched = Some(Instant::now());
        }
    }

    pub fn activate(&mut self, target: &str, action: Activation) {
        if self.speed > 0.0 {
            self.motion.entry(target.to_owned()).or_default().action =
                Some((action, Instant::now()));
        }
    }

    pub fn frame(
        &mut self,
        layout: Layout,
        items: &[DockItem],
        cursor: Option<f32>,
        pressed: Option<usize>,
        hidden: bool,
        drag: Option<(usize, usize, f32)>,
    ) -> (Scene, bool) {
        let now = Instant::now();
        let dt = (self.speed > 0.0).then(|| {
            self.last_frame
                .map_or(1.0 / 60.0, |last| (now - last).as_secs_f32().min(0.05))
                * self.speed
        });
        let size = layout.icon_size();
        let gap = layout.gap();
        let mut moving = self.slide.zoom(if hidden { 1.0 } else { 0.0 }, dt);
        let slide =
            (layout.height - layout.panel_top() - 3.0).max(0.0) * self.slide.zoom.clamp(0.0, 1.0);
        let mut extra = 0.0;
        let mut icons = Vec::with_capacity(items.len());
        let mut scales = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let slot = drag.map_or(index, |(from, to, _)| {
                if index == from {
                    to
                } else if from < to && index > from && index <= to {
                    index - 1
                } else if to < from && index >= to && index < from {
                    index + 1
                } else {
                    index
                }
            });
            let (left, top, _) = layout.icon(slot);
            let zoom = cursor.map_or(0.0, |x| {
                let radius = (size + gap) * 2.1;
                let distance = ((x - left - size / 2.0).abs() / radius).min(1.0);
                layout.dock.zoom * (1.0 + (distance * PI).cos()) / 2.0
            });
            let motion = self.motion.entry(item.pin.target.clone()).or_default();
            moving |= motion.zoom(zoom, dt);
            let (lift, scale) = motion.action(now, self.speed);
            let lift = (lift + motion.lift(now, self.speed)) * layout.dock.factor;
            let press = if pressed == Some(index) { 0.12 } else { 0.0 };
            motion.press = dt.map_or(press, |dt| {
                press + (motion.press - press) * (-28.0 * dt).exp()
            });
            if (motion.press - press).abs() < 0.001 {
                motion.press = press;
            }
            scales.push(1.0 + scale - motion.press);
            moving |= motion.launched.is_some() || motion.action.is_some() || motion.press != press;
            let growth = size * motion.zoom.max(0.0);
            icons.push(Bounds {
                x: left,
                y: top - lift,
                width: growth,
                height: size,
            });
            extra += growth;
        }
        let room = (layout.width - layout.base_width()).max(0.0);
        let fit = if extra > room { room / extra } else { 1.0 };
        let width = layout.base_width() + extra * fit;
        let mut offset = -extra * fit / 2.0;
        let mut order: Vec<_> = (0..items.len()).collect();
        if let Some((from, to, _)) = drag {
            if from < to {
                order[from..=to].rotate_left(1);
            } else {
                order[to..=from].rotate_right(1);
            }
        }
        for index in order {
            let icon = &mut icons[index];
            let scale = scales[index];
            let growth = icon.width * fit;
            icon.x += offset;
            icon.y = (icon.y - growth).max(0.0) + slide;
            icon.width = size + growth;
            icon.height = icon.width;
            let inset = icon.width * (1.0 - scale) / 2.0;
            icon.x += inset;
            icon.y += inset;
            icon.width *= scale;
            icon.height *= scale;
            offset += growth;
        }
        for (index, (item, icon)) in items.iter().zip(&mut icons).enumerate() {
            let motion = self.motion.get_mut(&item.pin.target).unwrap();
            if let Some((_, _, x)) = drag.filter(|(from, _, _)| *from == index) {
                icon.x = x - icon.width / 2.0;
                icon.y = (icon.y - 6.0 * layout.dock.factor).max(0.0);
                motion.position = Some(icon.x);
                motion.position_velocity = 0.0;
            } else {
                let (x, active) = motion.position(icon.x, dt);
                icon.x = x;
                moving |= active;
            }
        }
        self.last_frame = moving.then_some(now);
        (
            Scene {
                panel: Bounds {
                    x: (layout.width - width) / 2.0,
                    y: layout.panel_top() + slide,
                    width,
                    height: layout.panel_height(),
                },
                icons,
            },
            moving,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reordered_icons_travel_and_settle_without_snapping() {
        let mut motion = Motion::default();
        assert_eq!(motion.position(100.0, Some(1.0 / 60.0)), (100.0, false));
        let (x, moving) = motion.position(200.0, Some(1.0 / 60.0));
        assert!(moving && x > 100.0 && x < 200.0);
        for _ in 0..120 {
            motion.position(200.0, Some(1.0 / 60.0));
        }
        assert_eq!(motion.position(200.0, Some(1.0 / 60.0)), (200.0, false));
        // Animation-off settings must never keep a frame pump alive.
        assert_eq!(motion.position(50.0, None), (50.0, false));
    }
}
