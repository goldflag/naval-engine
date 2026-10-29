//! The film camera for play (Graphics → Film camera; native/docs/photo.md, "The film camera in play"): the frame as a
//! camera records it rather than as the eye sees it, by photo mode's own parts.
//!
//! - **The shutter**: the scene blurred by what moves while it is open ([`PlayCamera::exposure_s`]). Photo mode stops
//!   time and takes the frozen moment's bodies; play takes temporal AA's motion instead: the camera's from the scene's
//!   depth and the last frame's view-projection, the moving surfaces' own from the motion pass (the hulls and
//!   aircraft), scaled from a frame's interval to the exposure. The shutter's tiles, neighbours and gather are photo
//!   mode's (`crate::shutter`). Without temporal AA there is no motion pass and no blur.
//! - **The lens**: a coated lens's veiling glare of the sun, a halo about it and a plateau over the frame (photo mode's
//!   coated lens, `crate::photo_optics`, without its ghosts and spikes: the veil alone).
//! - **The colour fringes** (Graphics → Chromatic aberration, a row of its own): photo mode's lateral chromatic
//!   aberration, red magnified and blue shrunk against green toward the corners, by the lens the view's field makes
//!   ([`PlayCamera::fringe_shift`]).
//!
//! Motion blur is off in every preset and chromatic aberration on; a preset change keeps both rows. Photo mode's camera
//! replaces it while photo mode is up.

use crate::camera::Camera;
use crate::photo_optics::{Coating, FlareSun, Optics};

/// The film camera's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayCamera {
    /// Seconds the shutter is open (0 draws no blur).
    pub exposure_s: f32,
    /// The coated lens's veiling glare of the sun.
    pub veil: bool,
    /// Lateral chromatic aberration, 0–1 (photo mode's `chromatic`).
    pub fringes: f32,
}

impl PlayCamera {
    /// The camera the Graphics row turns on: a sixtieth of a second, a newsreel camera's order (24 frames a second at
    /// a half-turn shutter is a forty-eighth), and the coated lens's veil.
    pub const FILM: PlayCamera = PlayCamera {
        exposure_s: 1.0 / 60.0,
        veil: true,
        fringes: 0.0,
    };

    /// The colour fringes alone (Graphics → Chromatic aberration with Motion blur off): no shutter, no veil, photo mode's
    /// fringes at full strength.
    pub const FRINGES: PlayCamera = PlayCamera {
        exposure_s: 0.0,
        veil: false,
        fringes: 1.0,
    };

    /// The share of a frame's travel the shutter draws, for frames `interval_s` apart (0 without an interval).
    pub fn travel_share(&self, interval_s: f32) -> f32 {
        if interval_s > 0.0 && self.exposure_s > 0.0 {
            self.exposure_s / interval_s
        } else {
            0.0
        }
    }

    /// The colour fringes' shift at the frame's corners, over its half-diagonal, through the lens `camera`'s field
    /// makes (`crate::photo_optics::chromatic_shift`): about 0.23 % on the default 52°, more on a long lens.
    pub fn fringe_shift(&self, camera: &Camera) -> f32 {
        crate::photo_optics::chromatic_shift(self.fringes, focal_mm(camera))
    }
}

/// The focal length that draws `camera`'s field on a full frame's 24 mm height.
fn focal_mm(camera: &Camera) -> f32 {
    12.0 / (camera.fov_y_degrees.to_radians() * 0.5).tan().max(1e-3)
}

/// The lens the veil is drawn through: photo mode's coated lens, round, at `camera`'s field (a full frame's 24 mm
/// height), without its chromatic aberration or distortion, taking `sun` (the light the eye's glare takes, when up) and
/// no point lights (the coated lens scatters only the sun's light).
pub(crate) fn veil_lens(camera: &Camera, sun: Option<FlareSun>) -> Optics {
    Optics {
        coating: Coating::Coated,
        blades: 0,
        aperture: 4.0,
        focal_mm: focal_mm(camera),
        chromatic: 0.0,
        distortion: false,
        sun,
        lights: crate::lights::PointLights::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shutter_draws_its_exposure_s_share_of_a_frame_s_travel() {
        // At 60 frames a second a sixtieth draws the whole frame's travel; at 120, twice it.
        assert!((PlayCamera::FILM.travel_share(1.0 / 60.0) - 1.0).abs() < 1e-5);
        assert!((PlayCamera::FILM.travel_share(1.0 / 120.0) - 2.0).abs() < 1e-5);
        assert_eq!(PlayCamera::FILM.travel_share(0.0), 0.0);
        let closed = PlayCamera {
            exposure_s: 0.0,
            veil: true,
            fringes: 0.0,
        };
        assert_eq!(closed.travel_share(1.0 / 60.0), 0.0);
        assert_eq!(PlayCamera::FRINGES.travel_share(1.0 / 60.0), 0.0);
    }

    #[test]
    fn the_fringes_follow_the_lens_the_field_makes() {
        let camera = |fov_y_degrees| Camera {
            fov_y_degrees,
            ..Camera::default()
        };
        assert_eq!(PlayCamera::FILM.fringe_shift(&camera(52.0)), 0.0);
        let chase = PlayCamera::FRINGES.fringe_shift(&camera(crate::camera::DEFAULT_FOV_Y_DEGREES));
        let normal = PlayCamera::FRINGES.fringe_shift(&camera(20.0));
        let long = PlayCamera::FRINGES.fringe_shift(&camera(4.0));
        assert!((0.002..0.003).contains(&chase), "{chase}");
        assert!(normal < chase && normal < long, "{normal} {chase} {long}");
    }
}
