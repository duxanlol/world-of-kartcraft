//! The driver and what it reports.

use std::f32::consts::{FRAC_PI_2, TAU};

use ctr_kart::{Class, Collision, Event, Input, Kart, Modifiers, State, TICK};
use glam::{Vec2, Vec3};

use crate::route::{left_angle, xz, Path, Route};

/// Kartcraft's lu per yard at kart scale 1: 0.9144 m a yard over 0.025 m an lu.
pub const LU_PER_YARD: f32 = 0.9144 / 0.025;
/// Speed units (1/256 lu a frame) to lu/s.
const SPEED_LUS: f32 = 30.0 / 256.0;
/// Angle units (4096ths of a turn) a frame to rad/s.
const ANGLE_RADS: f32 = TAU / 4096.0 * 30.0;

/// The pursuit point: this many seconds ahead at the current speed, within these bounds (lu).
const LOOK_SECS: f32 = 0.4;
const LOOK_MIN: f32 = 140.0;
const LOOK_MAX: f32 = 900.0;
/// The share of the class's full-lock yaw rate a corner is planned at, and the deceleration
/// braking is planned at (lu/s²; the brake gives about 900).
const CORNER_YAW_SHARE: f32 = 0.85;
const PLAN_DECEL: f32 = 650.0;
/// Bends tighter than this radius (lu) are planned at a lower share of the yaw rate.
const TIGHT_RADIUS: f32 = 900.0;
const TIGHT_YAW_SHARE: f32 = 0.6;
/// The share of the slide's full turn a bend planned for slides is taken at.
const DRIFT_YAW_SHARE: f32 = 0.8;
/// A powerslide is started for a bend turning at least this much (radians) within the next
/// [`SLIDE_LOOK_SECS`], no tighter than [`SLIDE_MIN_RADIUS`], at this share of top speed or more
/// (CTR keeps a drift only above half the class speed).
const SLIDE_MIN_TURN: f32 = 1.0;
const SLIDE_LOOK_SECS: f32 = 1.2;
const SLIDE_MIN_RADIUS: f32 = 600.0;
const SLIDE_MIN_SPEED: f32 = 0.7;
/// A powerslide is let go once less than this much bend is left in the next 0.6 s.
const SLIDE_END_TURN: f32 = 0.25;
/// Boost once the meter is this full (it fills in 30 frames; the red zone is the last 15 and a
/// press later in it gives more).
const BOOST_AT: f32 = 0.86;
/// With no meter to press, a shoulder tap at least this often (frames) keeps a long powerslide
/// from spinning out (CTR spins out after 60 frames steering into it without a press).
const SLIDE_TAP_FRAMES: u32 = 40;
/// Progress the stuck check counts, lu, and how long a recovery reverses, frames.
const PROGRESS_LU: f32 = 80.0;
const REVERSE_FRAMES: u32 = 36;
/// Hopping a low obstacle, as a player does: a face standing ahead at [`HOP_LOW`] yards up and
/// none at [`HOP_HIGH`] (the hop's apex is 1.4 yd, 51 lu), within the distance the kart covers
/// in [`HOP_LEAD`] seconds (no nearer than [`HOP_REACH`].0 yards, no farther than .1), a face
/// steeper than a ramp ([`HOP_WALL`], its normal's up) and no hop in the last [`HOP_EVERY`]
/// seconds (CTR's 384 ms cooldown and a little).
const HOP_LOW: f32 = 0.3;
const HOP_HIGH: f32 = 1.6;
const HOP_LEAD: f32 = 0.2;
const HOP_REACH: (f32, f32) = (1.5, 5.0);
const HOP_WALL: f32 = 0.5;
const HOP_EVERY: f32 = 0.5;

/// How the driver takes its powerslides: the defaults are the course tester's, a decent player's;
/// a showcase run slides harder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlideTuning {
    /// A slide starts for a bend turning at least this much (radians) within the next
    /// `look_secs`, no tighter than `min_radius` (lu), at `min_speed` of top speed or more.
    pub min_turn: f32,
    pub look_secs: f32,
    pub min_radius: f32,
    pub min_speed: f32,
    /// It is let go once less than this much bend is left in the next 0.6 s.
    pub end_turn: f32,
    /// Frames after a slide before another may start.
    pub rest_frames: u32,
    /// Boost once the meter is this full.
    pub boost_at: f32,
    /// Let a long slide go once its three boosts are spent, and hop into it again.
    pub rehop: bool,
    /// Boosts taken a slide, up to the three CTR gives: fewer keeps a slide slower, and so tighter.
    pub max_boosts: u8,
    /// Plan the speeds for sliding through the bends, and hold a slide to the speed its bend
    /// allows (off the gas), so a tight bend is slid slower rather than run wide.
    pub drift_plan: bool,
}

impl Default for SlideTuning {
    fn default() -> Self {
        Self {
            min_turn: SLIDE_MIN_TURN,
            look_secs: SLIDE_LOOK_SECS,
            min_radius: SLIDE_MIN_RADIUS,
            min_speed: SLIDE_MIN_SPEED,
            end_turn: SLIDE_END_TURN,
            rest_frames: 20,
            boost_at: BOOST_AT,
            rehop: false,
            max_boosts: 3,
            drift_plan: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DriveOpts {
    pub class: Class,
    /// Give up after this many seconds.
    pub time_limit: f32,
    /// Farther than this from the route is off the track, yards.
    pub off_track_yards: f32,
    /// lu per yard, for the yard-valued options.
    pub lu_per_yard: f32,
    /// A gate also counts on coming this near its centre, yards (the game's 8).
    pub checkpoint_radius_yards: f32,
    /// Seconds without progress along the route before backing out.
    pub stuck_secs: f32,
    /// Give up after this many recoveries with no gate passed between them.
    pub max_recoveries: u32,
    /// Hop a low obstacle ahead (a kerb, a fence, a log) rather than drive into it.
    pub hops: bool,
    /// Powerslide in long bends.
    pub powerslides: bool,
    /// Seconds between trace samples.
    pub trace_every: f32,
    /// How the powerslides are taken.
    pub slide: SlideTuning,
}

impl Default for DriveOpts {
    fn default() -> Self {
        Self {
            class: Class::Balanced,
            time_limit: 600.0,
            off_track_yards: 12.0,
            lu_per_yard: LU_PER_YARD,
            checkpoint_radius_yards: 8.0,
            stuck_secs: 2.0,
            max_recoveries: 5,
            hops: true,
            powerslides: true,
            trace_every: 0.25,
            slide: SlideTuning::default(),
        }
    }
}

/// One trace sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TracePoint {
    pub t: f32,
    pub position: Vec3,
    /// lu/s.
    pub speed: f32,
    pub grounded: bool,
}

/// How a course drove. Lengths are lu, speeds lu/s, times seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DriveReport {
    pub finished: bool,
    /// Dropped far below the route: off the world.
    pub fell: bool,
    /// Gave up after [`DriveOpts::max_recoveries`] recoveries without a gate.
    pub gave_up: bool,
    pub time: f32,
    pub lap_times: Vec<f32>,
    pub gates_passed: u32,
    pub distance: f32,
    pub avg_speed: f32,
    pub max_speed: f32,
    /// Share of the time at 95 % of the class's top speed or more, percent.
    pub top_speed_pct: f32,
    /// Walls touched: rubs begun and head-on slams.
    pub wall_contacts: u32,
    /// Where each wall contact happened, lu.
    pub wall_contact_points: Vec<Vec3>,
    pub slams: u32,
    pub stuck_recoveries: u32,
    /// Hops over low obstacles ([`DriveOpts::hops`]).
    pub hops: u32,
    pub off_track_events: u32,
    pub off_track_time: f32,
    pub airtime: f32,
    pub longest_air: f32,
    pub powerslides: u32,
    pub boosts: u32,
    /// Climb summed over the grounded driving.
    pub elevation_gain: f32,
    /// The steepest ground driven on, degrees.
    pub max_slope_deg: f32,
    pub trace: Vec<TracePoint>,
}

impl DriveReport {
    /// The report as JSON, with the trace (as `[t, x, y, z, speed, grounded]` rows) if `trace`.
    pub fn to_json(&self, trace: bool) -> serde_json::Value {
        let mut v = serde_json::json!({
            "finished": self.finished,
            "fell": self.fell,
            "gave_up": self.gave_up,
            "time": self.time,
            "lap_times": self.lap_times,
            "gates_passed": self.gates_passed,
            "distance": self.distance,
            "avg_speed": self.avg_speed,
            "max_speed": self.max_speed,
            "top_speed_pct": self.top_speed_pct,
            "wall_contacts": self.wall_contacts,
            "slams": self.slams,
            "stuck_recoveries": self.stuck_recoveries,
            "hops": self.hops,
            "off_track_events": self.off_track_events,
            "off_track_time": self.off_track_time,
            "airtime": self.airtime,
            "longest_air": self.longest_air,
            "powerslides": self.powerslides,
            "boosts": self.boosts,
            "elevation_gain": self.elevation_gain,
            "max_slope_deg": self.max_slope_deg,
        });
        if trace {
            v["trace"] = self
                .trace
                .iter()
                .map(|p| {
                    serde_json::json!([
                        p.t,
                        p.position.x,
                        p.position.y,
                        p.position.z,
                        p.speed,
                        p.grounded
                    ])
                })
                .collect();
        }
        v
    }
}

/// Drives `route` over `world` from a standing start at its first waypoint, facing the second,
/// until it finishes, falls off the world, gives up or runs out of time.
pub fn drive(world: &Collision, route: &Route, opts: &DriveOpts) -> DriveReport {
    let mut report = DriveReport::default();
    if route.waypoints.len() < 2 {
        return report;
    }
    let path = Path::new(&route.waypoints, route.is_loop());
    if path.len() < 2 {
        return report;
    }
    let top = opts.class.stats().speed as f32 * SPEED_LUS;

    let start = path.pts[0];
    let first = xz(path.pts[1] - path.pts[0]);
    let heading = (first.x.atan2(first.y) / TAU * 4096.0).round() as i32;
    let ground = world
        .ground(start + Vec3::Y * 200.0, 800.0)
        .map_or(start, |h| h.point);
    // A little above the ground: the first ticks settle the kart onto it, as the session does.
    let mut kart = Kart::new(opts.class, ground + Vec3::Y * 4.0, heading);
    let mods = Modifiers::default();

    let mut driver = Driver::new(&path, opts.class, opts.slide);

    let floor = route
        .waypoints
        .iter()
        .map(|p| p.y)
        .fold(f32::INFINITY, f32::min)
        - 3000.0;
    let off_lu = opts.off_track_yards * opts.lu_per_yard;
    let radius = opts.checkpoint_radius_yards * opts.lu_per_yard;
    let ticks = (opts.time_limit / TICK).ceil() as u64;
    let n_gates = route.gates.len();
    let mut next_gate = usize::from(n_gates > 1);
    let mut laps_done = 0;
    let mut lap_start = 0.0;

    let (mut seg, mut s_prev, mut wraps) = (0usize, 0.0f32, 0i64);
    let mut best = 0.0f32;
    let mut best_at = 0.0f32;
    let mut recoveries_since_gate = 0;
    let mut off = false;
    let mut air = 0.0f32;
    let mut last_ground_y: Option<f32> = None;
    let mut at_top = 0u64;
    let mut speed_sum = 0.0f64;
    let mut next_sample = 0.0f32;
    let mut pos = kart.pose().position;
    let mut t = 0.0f32;
    let mut hopped_at = f32::NEG_INFINITY;

    for tick in 0..ticks {
        t = tick as f32 * TICK;
        let (i, s, dist) = path.nearest(pos, seg, 25, 75);
        seg = i;
        if path.closed {
            if s - s_prev < -path.length / 2.0 {
                wraps += 1;
            } else if s - s_prev > path.length / 2.0 {
                wraps -= 1;
            }
        }
        s_prev = s;
        let progress = wraps as f32 * path.length + s;
        if progress > best + PROGRESS_LU {
            best = progress;
            best_at = t;
        }

        if dist > off_lu && !off {
            off = true;
            report.off_track_events += 1;
        } else if off && dist < off_lu * 0.8 {
            off = false;
        }
        if off {
            report.off_track_time += TICK;
        }

        if driver.recover == 0 && t - best_at > opts.stuck_secs {
            report.stuck_recoveries += 1;
            recoveries_since_gate += 1;
            if recoveries_since_gate > opts.max_recoveries {
                report.gave_up = true;
                break;
            }
            driver.recover = REVERSE_FRAMES;
            driver.slide = Slide::Off(0);
        }
        if driver.recover == 1 {
            // Out of the recovery: a fresh 2 s to make progress.
            best_at = t;
            best = best.min(progress);
        }

        if t >= next_sample {
            report.trace.push(TracePoint {
                t,
                position: pos,
                speed: kart.speed().abs() as f32 * SPEED_LUS,
                grounded: kart.grounded(),
            });
            next_sample += opts.trace_every;
        }

        if opts.hops && driver.recover == 0 && kart.grounded() && t - hopped_at > HOP_EVERY {
            let y = opts.lu_per_yard;
            let h = kart.pose().heading as f32 / 4096.0 * TAU;
            let ahead = Vec3::new(h.sin(), 0.0, h.cos());
            let v = kart.speed().abs() as f32 * SPEED_LUS;
            let reach = (v * HOP_LEAD).clamp(HOP_REACH.0 * y, HOP_REACH.1 * y);
            let low = world
                .ray(pos + Vec3::Y * HOP_LOW * y, ahead, reach)
                .is_some_and(|hit| hit.normal.y.abs() < HOP_WALL);
            if low
                && world
                    .ray(pos + Vec3::Y * HOP_HIGH * y, ahead, reach + y)
                    .is_none()
            {
                driver.hop = true;
                hopped_at = t;
                report.hops += 1;
            }
        }
        let input = driver.input(&kart, pos, seg, s);
        kart.tick(world, &input, &mods);
        let here = kart.pose().position;
        for e in kart.events.drain(..) {
            match e {
                Event::WallRub => {
                    report.wall_contacts += 1;
                    report.wall_contact_points.push(here);
                }
                Event::Slam { .. } => {
                    report.wall_contacts += 1;
                    report.wall_contact_points.push(here);
                    report.slams += 1;
                }
                Event::DriftStart { .. } => report.powerslides += 1,
                Event::Boost { .. } => report.boosts += 1,
                _ => {}
            }
        }

        let now = kart.pose().position;
        let speed = kart.speed().abs() as f32 * SPEED_LUS;
        report.distance += now.distance(pos);
        report.max_speed = report.max_speed.max(speed);
        speed_sum += f64::from(speed);
        at_top += u64::from(speed >= top * 0.95);
        if kart.grounded() {
            report.longest_air = report.longest_air.max(air);
            air = 0.0;
            // The ground under the kart, not the kart: a hop's first frames still count as on it.
            if let Some(hit) = world.ground(now + Vec3::Y * 30.0, 120.0) {
                let slope = hit.normal.y.clamp(-1.0, 1.0).acos().to_degrees();
                report.max_slope_deg = report.max_slope_deg.max(slope);
                if let Some(y) = last_ground_y {
                    report.elevation_gain += (hit.point.y - y).max(0.0);
                }
                last_ground_y = Some(hit.point.y);
            }
        } else {
            air += TICK;
            report.airtime += TICK;
        }

        if n_gates > 0 && route.gates[next_gate].passes(pos, now, radius) {
            report.gates_passed += 1;
            recoveries_since_gate = 0;
            if route.is_loop() {
                if next_gate == 0 {
                    report.lap_times.push(t + TICK - lap_start);
                    lap_start = t + TICK;
                    laps_done += 1;
                }
                next_gate = (next_gate + 1) % n_gates;
            } else if next_gate + 1 == n_gates {
                report.finished = true;
            } else {
                next_gate += 1;
            }
            if laps_done >= route.laps && route.is_loop() {
                report.finished = true;
            }
        }
        pos = now;
        t += TICK;
        if report.finished {
            break;
        }
        if pos.y < floor {
            report.fell = true;
            break;
        }
    }

    report.longest_air = report.longest_air.max(air);
    report.time = t;
    let frames = (t / TICK).round().max(1.0);
    report.avg_speed = (speed_sum / f64::from(frames)) as f32;
    report.top_speed_pct = at_top as f32 / frames * 100.0;
    report
}

/// The fastest speed (lu/s) to be doing at each point: what the curvature allows at `yaw` rad/s,
/// lowered so each corner can be braked for from the points before it.
fn speed_profile(path: &Path, top: f32, yaw: f32, tight_share: f32) -> Vec<f32> {
    let n = path.len();
    // The path's curvature is averaged over a few hundred lu, which spreads a hairpin's turn
    // out: plan each point at the sharpest curvature near it, and tight bends (under
    // [`TIGHT_RADIUS`]) at [`TIGHT_YAW_SHARE`] of the yaw, so the kart is slow before it turns in.
    let mut v: Vec<f32> = (0..n as isize)
        .map(|i| {
            let k = (-2..=2)
                .map(|d| path.curv[path.idx(i + d)].abs())
                .fold(1e-6f32, f32::max);
            let share = if k > 1.0 / TIGHT_RADIUS {
                tight_share
            } else {
                1.0
            };
            (yaw * share / k).min(top)
        })
        .collect();
    let reach = 2.0 * PLAN_DECEL * path.step;
    for _ in 0..if path.closed { 2 } else { 1 } {
        for i in (0..n).rev() {
            let j = if i + 1 < n {
                i + 1
            } else if path.closed {
                0
            } else {
                continue;
            };
            v[i] = v[i].min((v[j] * v[j] + reach).sqrt());
        }
    }
    v
}

/// The powerslide in hand.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slide {
    /// Not sliding; frames before another may start.
    Off(u32),
    /// Hopped into a bend, holding R1 until the landing starts the slide.
    Hop { frames: u32, left: bool },
    /// Sliding; frames since a shoulder tap.
    On {
        left: bool,
        since_tap: u32,
        /// Boosts pressed in the red this slide.
        boosts: u8,
    },
}

pub(crate) struct Driver<'a> {
    path: &'a Path,
    profile: Vec<f32>,
    top: f32,
    /// Full-lock yaw rate, rad/s.
    yaw_max: f32,
    /// The class's powerslide base turn, angle units a frame.
    drift_base: f32,
    slide: Slide,
    l1_prev: bool,
    /// Frames of reversing left.
    recover: u32,
    /// Hop this frame: a low obstacle ahead.
    hop: bool,
    tuning: SlideTuning,
}

impl<'a> Driver<'a> {
    /// A driver of `class` along `path`, sliding as `tuning` says.
    pub(crate) fn new(path: &'a Path, class: Class, tuning: SlideTuning) -> Self {
        let stats = class.stats();
        let top = stats.speed as f32 * SPEED_LUS;
        let yaw_max = stats.turn_rate as f32 * ANGLE_RADS;
        // Planned for slides, a bend is taken at the slide's own turn (its base and 22 more
        // steering into it, spec 5.2), no slower: a tight one at a slide's speed, not a crawl.
        let profile = if tuning.drift_plan {
            let drift_yaw = (stats.drift_turn_base + 22) as f32 * ANGLE_RADS;
            speed_profile(path, top, drift_yaw * DRIFT_YAW_SHARE, 1.0)
        } else {
            speed_profile(path, top, yaw_max * CORNER_YAW_SHARE, TIGHT_YAW_SHARE)
        };
        Self {
            path,
            profile,
            top,
            yaw_max,
            drift_base: stats.drift_turn_base as f32,
            slide: Slide::Off(0),
            l1_prev: false,
            recover: 0,
            hop: false,
            tuning,
        }
    }

    /// This frame's controls, the kart at `pos`, nearest the path on segment `seg`, `s` lu along.
    pub(crate) fn input(&mut self, kart: &Kart, pos: Vec3, seg: usize, s: f32) -> Input {
        let pose = kart.pose();
        let v = kart.speed().abs() as f32 * SPEED_LUS;
        let h = pose.heading as f32 / 4096.0 * TAU;
        let facing = Vec2::new(h.sin(), h.cos());
        let vel = xz(pose.velocity);
        let fwd = if vel.length() > 200.0 && kart.forward_speed() > 0 {
            vel.normalize()
        } else {
            facing
        };
        let look = (v * LOOK_SECS).clamp(LOOK_MIN, LOOK_MAX);
        let target = self.path.at(s + look);
        let alpha = left_angle(fwd, xz(target - pos));
        // Pure pursuit: the arc through the pursuit point, as a yaw rate (+ left).
        let omega = if alpha.abs() > FRAC_PI_2 {
            alpha.signum() * self.yaw_max * 4.0
        } else {
            2.0 * v.max(300.0) * alpha.sin() / look
        };

        if self.recover > 0 {
            self.recover -= 1;
            self.l1_prev = false;
            // Reversing turns like a car (CTR flips the steer): steer away to swing the nose in.
            return Input {
                steer: alpha.signum(),
                down: true,
                ..Default::default()
            };
        }

        let step = self.path.step;
        let ahead = |secs: f32, min: f32| ((v * secs).max(min) / step).ceil() as usize;
        let lead = ahead(0.25, step);
        let target_v = (0..=lead)
            .map(|j| self.profile[self.path.idx((seg + j) as isize)])
            .fold(self.top, f32::min);

        let mut input = Input {
            gas: true,
            steer: -stick(omega / self.yaw_max * 1.2),
            ..Default::default()
        };

        self.slide = match self.slide {
            Slide::Off(wait) => {
                let n = ahead(self.tuning.look_secs, 600.0);
                let bend = self.path.turn(seg, n);
                let sharpest = self.path.sharpest(seg, n);
                if wait == 0
                    && kart.state == State::Normal
                    && kart.grounded()
                    && v >= self.top * self.tuning.min_speed
                    && bend.abs() >= self.tuning.min_turn
                    && sharpest < 1.0 / self.tuning.min_radius
                    && alpha * bend >= 0.0
                {
                    Slide::Hop {
                        frames: 0,
                        left: bend > 0.0,
                    }
                } else {
                    Slide::Off(wait.saturating_sub(1))
                }
            }
            Slide::Hop { frames, left } => {
                if kart.state == State::Drifting {
                    Slide::On {
                        left,
                        since_tap: 0,
                        boosts: 0,
                    }
                } else if frames > 25 {
                    Slide::Off(30)
                } else {
                    Slide::Hop {
                        frames: frames + 1,
                        left,
                    }
                }
            }
            Slide::On {
                left,
                since_tap,
                boosts,
            } => {
                let rest = self.path.turn(seg, ahead(0.6, 300.0));
                let dir = if left { 1.0 } else { -1.0 };
                // Three boosts are all a slide gives (`boostsAttempted`): with `rehop`, a long bend
                // is let go once they are spent and hopped into afresh for three more.
                let spent = self.tuning.rehop
                    && boosts >= self.tuning.max_boosts
                    && (boosts < 3 || kart.turbo_meter().is_none());
                if kart.state != State::Drifting
                    || rest * dir < self.tuning.end_turn
                    || alpha * dir < -0.35
                    || target_v < self.top * 0.5
                    || spent
                {
                    Slide::Off(if spent { 0 } else { self.tuning.rest_frames })
                } else {
                    Slide::On {
                        left,
                        since_tap: since_tap + 1,
                        boosts,
                    }
                }
            }
        };

        match &mut self.slide {
            Slide::Off(_) => {
                if v > target_v + 60.0 {
                    input.gas = false;
                    input.brake = true;
                } else if v > target_v {
                    input.gas = false;
                }
            }
            Slide::Hop { left, .. } => {
                input.r1 = true;
                input.steer = if *left { -1.0 } else { 1.0 };
            }
            Slide::On {
                left,
                since_tap,
                boosts,
            } => {
                input.r1 = true;
                let dir = if *left { 1.0 } else { -1.0 };
                // CTR's slide turns `base` units a frame steering neutral, up to 22 more steering
                // into it and down to 10 fewer counter-steering (spec 5.2).
                let units = omega * dir / ANGLE_RADS;
                let into = if units >= self.drift_base {
                    (units - self.drift_base) / 22.0
                } else {
                    (units - self.drift_base) / 10.0
                };
                input.steer = -dir * stick(into * 1.2);
                // Too fast for the slide to hold the bend: off the gas (never under the half
                // of the class's speed a slide needs, `VehPhysProc.c`'s drift exit).
                if self.tuning.drift_plan && v > target_v + 40.0 && v > self.top * 0.56 {
                    input.gas = false;
                }
                let meter = kart.turbo_meter();
                let tap = match meter {
                    Some((filled, red)) => {
                        red && filled >= self.tuning.boost_at && *boosts < self.tuning.max_boosts
                    }
                    None => *since_tap >= SLIDE_TAP_FRAMES,
                };
                if tap && !self.l1_prev {
                    input.l1 = true;
                    *since_tap = 0;
                    *boosts += u8::from(meter.is_some());
                }
            }
        }
        self.l1_prev = input.l1;
        // A low obstacle ahead: a tap of the hop, outside a slide (which holds it already).
        if std::mem::take(&mut self.hop) && matches!(self.slide, Slide::Off(_)) {
            input.r1 = true;
        }
        input
    }
}

/// The stick deflection (−1 to 1) that asks the steer for `fraction` of its full strength,
/// inverting CTR's two-segment stick curve (dead zone 48, a fifth of the strength over the next
/// 47 counts, then 0.8 over 47 more, full at 127; `VehPhysJoystick_GetStrength`).
fn stick(fraction: f32) -> f32 {
    let f = fraction.abs().min(1.0);
    let v = if f < 0.02 {
        return 0.0;
    } else if f <= 0.2 {
        f * 235.0
    } else if f <= 0.74 {
        47.0 + (f - 0.2) * 58.75
    } else {
        79.0
    };
    ((48.0 + v + 0.5) / 127.0).min(1.0) * fraction.signum()
}
