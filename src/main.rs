mod economy;
mod elevator;
pub mod messages;
mod people;
mod renderer;
mod room;
mod runtime;
mod simulation;
mod tower;
mod worker;

use economy::Economy;
use people::People;
use tower::Tower;

#[derive(Debug, Default)]
/// Mutable simulation data shared by the simulation and worker threads.
pub struct GameState {
    /// Number of simulation ticks completed.
    pub simulation_tick: u64,
    /// Tower layout and its rooms.
    pub building: Tower,
    /// Current funds and recurring finances.
    pub economy: Economy,
    /// People living in or visiting the tower.
    pub people: People,
}

/// Entry point that starts the simulation runtime.
fn main() {
    runtime::run();
}
