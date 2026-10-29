//! A theatre scene laid again on another chart (step C's re-lay, decision C7): how what one scene held is carried into
//! the next by its kind. The client's scene is a chart about a place `S` near its camera; within a chart a re-centre is a
//! translation (`naval_sim::earth::SceneShift`), and between two charts nothing is: a point goes through its place, a
//! vector through the two charts' Jacobians at its holder, a heading through the turn between their grids there. The
//! client makes the map (`naval_client::theatre::scene_chart::SceneRelay`); the histories that carry across a re-lay
//! (wakes, torpedo tracks, vapour trails, released smoke) take it through this trait, so they need no chart of their own.
//!
//! Every value is the scene's: metres `[x, y, z]` (x east, y up, z south on the scene chart's grid, about its render
//! origin), a heading in radians clockwise from −z.
pub trait SceneRelay {
    /// A point of the old scene in the new one: its place, on the new scene's chart less its render origin; its height
    /// kept.
    fn point(&self, p: [f64; 3]) -> [f64; 3];
    /// A vector of the old scene held at its point `at` (a velocity, a direction, a wind) on the new scene's axes there:
    /// its level part through the old chart's inverse Jacobian and the new one's, its up component as it is.
    fn vector(&self, at: [f64; 3], v: [f64; 3]) -> [f64; 3];
    /// A heading of the old scene held at its point `at` (a body's, a wake's) as a heading of the new one: the true
    /// heading it names there on the new scene's grid.
    fn heading(&self, at: [f64; 3], heading: f64) -> f64;

    /// A level direction `[x, z]` held at `at`, carried as [`SceneRelay::vector`] carries it.
    fn direction(&self, at: [f64; 3], d: [f64; 2]) -> [f64; 2] {
        let v = self.vector(at, [d[0], 0.0, d[1]]);
        [v[0], v[2]]
    }

    /// A level point `[x, z]` at sea level, carried as [`SceneRelay::point`] carries it.
    fn point2(&self, p: [f64; 2]) -> [f64; 2] {
        let q = self.point([p[0], 0.0, p[1]]);
        [q[0], q[2]]
    }

    /// The turn about the up axis that carries an orientation held at `at` (an emitter's, a camera's): what a heading
    /// along the old scene's −z there gains, radians clockwise. A rotation about +y by `−turn` applies it.
    fn turn(&self, at: [f64; 3]) -> f64 {
        self.heading(at, 0.0)
    }
}
