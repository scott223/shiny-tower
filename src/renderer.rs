use crate::messages::{RendererWorkerMessage, SimulationRendererMessage};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Receives worker updates and simulation ticks; rendering is currently a placeholder.
pub fn run(
    _to_worker: Sender<RendererWorkerMessage>,
    from_worker: Receiver<RendererWorkerMessage>,
    from_simulation: Receiver<SimulationRendererMessage>,
) {
    eprintln!("renderer placeholder started; rendering is disabled");
    let mut worker_stopped = false;
    let mut simulation_stopped = false;

    // Poll both channels so one source cannot block updates from the other.
    while !worker_stopped || !simulation_stopped {
        match from_worker.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(message @ RendererWorkerMessage::RoomBuilt { .. }) => {
                eprintln!("renderer received room-built update: {message:?}");
            }
            Ok(message @ RendererWorkerMessage::FloorSegmentBuilt { .. }) => {
                eprintln!("renderer received floor-segment-built update: {message:?}");
            }
            Ok(RendererWorkerMessage::RoomRemoved(room_id)) => {
                eprintln!("renderer received room-removed update: {room_id:?}");
            }
            Ok(RendererWorkerMessage::Shutdown) => worker_stopped = true,
            Ok(RendererWorkerMessage::UiInput(_)) => {
                eprintln!("unexpected UI input received by renderer");
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => worker_stopped = true,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }

        loop {
            match from_simulation.try_recv() {
                Ok(SimulationRendererMessage::Tick {
                    tick,
                    date,
                    time,
                    cash,
                }) => {
                    eprintln!(
                        "simulation tick {tick}: time {:02}:{:02} ({:?}), day of week {:?}, quarter Q{} (year {}), cash {cash}",
                        time.hour, time.minute, time.period, date.day_type, date.quarter, date.year
                    );
                }
                Ok(SimulationRendererMessage::Shutdown) => simulation_stopped = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    simulation_stopped = true;
                    break;
                }
            }
        }
    }
}
