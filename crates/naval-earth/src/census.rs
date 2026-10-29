//! The earth censuses' counters, compiled only with the cargo feature `earth-census` (off in play and in every gate):
//! what the sphere is asked for, counted where it is asked. naval-sim's `earth::census` re-exports them, with its
//! `take` the land's and the route planner's counts beside these.
//!
//! - **Participants per `calc` site.** naval-sim's `earth::calc()` is `#[track_caller]` in this build and opens its
//!   site here ([`Open`]); every participant the frame projects (`sphere::Frame`'s `coords`, `delta`, `delta3`,
//!   `tangent`, `to_tangent`, `to_tangent2`) is measured from the anchor (the chord, metres) and counted under the
//!   innermost open site. A frame used outside any `calc` (a launch frame's) counts under `(no calc)`. Sites beyond
//!   150 km, the distance the frame is trusted to (D5), move to D6's long-range forms or are justified.
//! - **The costly operations per tick** (each measured; the 8 % step budget is about 68,000 projections a tick):
//!   neighbour projections, carries (and those that turned something), chart fixes (`Chart::forward`), the chart's
//!   inverse and Jacobian, frames built.
//! - **Saturations and far answers** (D5, D6, D9), which must be zero: a frame position beyond the near side, a level
//!   tangent read on the far side, a place clamped to the pole limit, a far answer (`is_far` places given a bearing,
//!   a carry or a step).
//! - **Distances beyond the series** (`distancesBeyondSeries`, D6): ranges past 1,000 km of chord, which
//!   `sphere::distance` takes in its exact `atan2` form. Counted, not a fault, and not zero: a world reaches 15,000 km
//!   about its origin, and every range it takes past 1,000 km of chord is one.
//! - **The fields** (C9, C10: `weather::Field::at`, `sea::Field`'s samples): evaluations by what was asked and by
//!   caller ([`field`]).
//!
//! Ticks are the battle's (`Battle::step_observed`) and the theatre's (`Theatre::step`). [`take`] returns every
//! thread's counts and starts again: the equality harness takes one per run (`census/<run>.json`), through naval-sim.
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::Location;
use std::sync::{Arc, Mutex};

/// What is counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Projection,
    Carry,
    CarryTurned,
    ChartFix,
    ChartInverse,
    Jacobian,
    Frame,
    PositionSaturated,
    FarSideTangent,
    PoleSaturated,
    FarAnswer,
    DistanceFar,
    BattleTick,
    TheatreTick,
}

const OPS: [(Op, &str); 14] = [
    (Op::Projection, "neighbourProjections"),
    (Op::Carry, "carries"),
    (Op::CarryTurned, "carriesTurned"),
    (Op::ChartFix, "chartFixes"),
    (Op::ChartInverse, "chartInverses"),
    (Op::Jacobian, "chartJacobians"),
    (Op::Frame, "framesBuilt"),
    (Op::PositionSaturated, "positionSaturated"),
    (Op::FarSideTangent, "farSideTangents"),
    (Op::PoleSaturated, "poleSaturated"),
    (Op::FarAnswer, "farAnswers"),
    (Op::DistanceFar, "distancesBeyondSeries"),
    (Op::BattleTick, "battleTicks"),
    (Op::TheatreTick, "theatreTicks"),
];

/// The frame is trusted to this far from its anchor (D5).
pub const TRUSTED_M: f64 = 150_000.0;

#[derive(Clone, Copy, Default)]
struct Site {
    opened: u64,
    participants: u64,
    beyond: u64,
    max_m: f64,
}

#[derive(Default)]
struct State {
    ops: [u64; OPS.len()],
    sites: BTreeMap<SiteKey, Site>,
    /// The fields' evaluations (step C's C9 and C10), by what was asked and the caller that asked it.
    field: BTreeMap<(&'static str, SiteKey), u64>,
}

type SiteKey = (&'static str, u32);

static REGISTRY: Mutex<Vec<Arc<Mutex<State>>>> = Mutex::new(Vec::new());

thread_local! {
    static LOCAL: Arc<Mutex<State>> = {
        let state = Arc::new(Mutex::new(State::default()));
        REGISTRY.lock().unwrap().push(state.clone());
        state
    };
    static OPEN: RefCell<Vec<SiteKey>> = const { RefCell::new(Vec::new()) };
    static PAUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `f` run with this thread's counts standing still: a census's own work (the harness's second serialization of a
/// frame for the wire census) is not the simulation's.
pub fn paused<R>(f: impl FnOnce() -> R) -> R {
    let was = PAUSED.with(|p| p.replace(true));
    let r = f();
    PAUSED.with(|p| p.set(was));
    r
}

fn counting() -> bool {
    !PAUSED.with(|p| p.get())
}

const NO_CALC: SiteKey = ("(no calc)", 0);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    LOCAL.with(|s| f(&mut s.lock().unwrap()))
}

/// Count one `op`.
#[inline]
pub fn count(op: Op) {
    if counting() {
        with(|s| s.ops[op as usize] += 1);
    }
}

/// A `calc` opened at `site`: counted, and its site the one participants are counted under until it closes.
pub struct Open;

impl Open {
    pub fn new(site: &'static Location<'static>) -> Self {
        let key = (site.file(), site.line());
        OPEN.with(|o| o.borrow_mut().push(key));
        if counting() {
            with(|s| s.sites.entry(key).or_default().opened += 1);
        }
        Open
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        OPEN.with(|o| o.borrow_mut().pop());
    }
}

/// A participant at `p` brought into the frame at `anchor` (both unit vectors): one projection, its distance the
/// chord between them.
#[inline]
pub fn participant(anchor: [f64; 3], p: [f64; 3]) {
    if !counting() {
        return;
    }
    let d = [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]];
    let metres = super::sphere::R * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let key = OPEN.with(|o| o.borrow().last().copied().unwrap_or(NO_CALC));
    with(|s| {
        s.ops[Op::Projection as usize] += 1;
        let site = s.sites.entry(key).or_default();
        site.participants += 1;
        if metres > TRUSTED_M {
            site.beyond += 1;
        }
        if metres > site.max_m {
            site.max_m = metres;
        }
    });
}

/// One evaluation of a field (`weather::Field::at`, `sea::Field`'s samples: `what`), counted under the caller that
/// asked for it (`site`: the sampling methods and the `environment::Environment` wrappers are `#[track_caller]` in
/// this build, so it is the first caller outside them).
#[inline]
pub fn field(what: &'static str, site: &'static Location<'static>) {
    if counting() {
        with(|s| {
            *s.field
                .entry((what, (site.file(), site.line())))
                .or_default() += 1
        });
    }
}

/// Every thread's counts since the last take, and a fresh start: the sphere's operations, the `calc` sites and the
/// fields' evaluations (naval-sim's `earth::census::take` adds the land's and the route planner's beside them).
pub fn take() -> Value {
    let mut total = State::default();
    for state in REGISTRY.lock().unwrap().iter() {
        let state = std::mem::take(&mut *state.lock().unwrap());
        for (i, n) in state.ops.iter().enumerate() {
            total.ops[i] += n;
        }
        for (key, site) in state.sites {
            let t = total.sites.entry(key).or_default();
            t.opened += site.opened;
            t.participants += site.participants;
            t.beyond += site.beyond;
            t.max_m = t.max_m.max(site.max_m);
        }
        for (key, n) in state.field {
            *total.field.entry(key).or_default() += n;
        }
    }
    let ops: serde_json::Map<String, Value> = OPS
        .iter()
        .map(|(op, name)| ((*name).to_string(), json!(total.ops[*op as usize])))
        .collect();
    let mut sites: Vec<(SiteKey, Site)> = total.sites.into_iter().collect();
    sites.sort_by(|a, b| b.1.max_m.total_cmp(&a.1.max_m).then(a.0.cmp(&b.0)));
    let sites: Vec<Value> = sites
        .into_iter()
        .map(|((file, line), s)| {
            json!({"site": format!("{file}:{line}"), "opened": s.opened, "participants": s.participants,
                "beyond150km": s.beyond, "maxM": s.max_m})
        })
        .collect();
    // The fields' evaluations per caller, the most asked first (the plan's §7.5 "Field": per tick, by the ticks above).
    let mut field: Vec<((&'static str, SiteKey), u64)> = total.field.into_iter().collect();
    field.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let field: Vec<Value> = field
        .into_iter()
        .map(|((what, (file, line)), n)| json!({"field": what, "site": format!("{file}:{line}"), "evaluations": n}))
        .collect();
    json!({"ops": ops, "sites": sites, "field": field})
}
