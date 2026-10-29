//! What the land's workers take from the program they run in ([`Host`], [`install`]): the folder their caches live
//! under and how their threads are scheduled. The game's client installs its own (its user data folder and its utility
//! class, naval-client's `qos`) before any land is asked for; a program that installs none keeps the caches under its
//! working directory and runs the workers as plain threads ([`Host::plain`]).

use std::path::PathBuf;
use std::sync::{LazyLock, OnceLock};

/// The program's side of the land's workers.
pub struct Host {
    /// The folder the caches live under (`surface-cache`, `osm-cache`): the game's user data folder.
    pub data_dir: fn() -> PathBuf,
    /// Starts a thread named `name` for work streamed in the background, below the frame loop and the simulation.
    pub spawn_utility: fn(name: &str, work: Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
    /// The calling thread's scheduling class as the host numbers it (none where it keeps none), for the threads it
    /// spawns to take ([`Inherit`]).
    pub thread_class: fn() -> Option<u32>,
    /// Puts the calling thread in a class `thread_class` read.
    pub apply_class: fn(Option<u32>),
}

impl Host {
    /// The working directory and plain threads: what a program that installs nothing gets.
    pub fn plain() -> Self {
        Self {
            data_dir: || std::env::current_dir().unwrap_or_default(),
            spawn_utility: |name, work| {
                std::thread::Builder::new()
                    .name(name.to_owned())
                    .spawn(work)
                    .map(drop)
            },
            thread_class: || None,
            apply_class: |_| {},
        }
    }
}

static INSTALLED: OnceLock<Host> = OnceLock::new();
static PLAIN: LazyLock<Host> = LazyLock::new(Host::plain);

/// Installs the program's side. The first install holds; until one, the land works as [`Host::plain`].
pub fn install(host: Host) {
    let _ = INSTALLED.set(host);
}

fn host() -> &'static Host {
    INSTALLED.get().unwrap_or(&PLAIN)
}

/// The folder the caches live under ([`Host::data_dir`]).
pub fn data_dir() -> PathBuf {
    (host().data_dir)()
}

/// Starts a thread named `name` for work streamed in the background ([`Host::spawn_utility`]). A pool it spawns for
/// its work takes its class with [`Inherit`].
pub fn spawn_utility<F>(name: &str, work: F) -> std::io::Result<()>
where
    F: FnOnce() + Send + 'static,
{
    (host().spawn_utility)(name, Box::new(work))
}

/// The calling thread's class, for the threads it spawns to take ([`Inherit::apply`] first thing in each).
#[derive(Clone, Copy, Debug)]
pub struct Inherit(Option<u32>);

impl Inherit {
    /// The calling thread's class.
    pub fn here() -> Self {
        Self((host().thread_class)())
    }

    /// Puts the calling thread in the class taken.
    pub fn apply(self) {
        (host().apply_class)(self.0)
    }
}
