mod coordinator;
mod model;

pub use coordinator::{ActivityCoordinator, ActivityGuard, ActivityKind, SwitchCheck};
pub use model::{ActiveWorkspacePhase, ActiveWorkspaceState, ActivitySnapshot};
