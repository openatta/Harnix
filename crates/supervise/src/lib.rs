//! Child processes, kept: spawned by fork + exec, handed only the fds they are given, bound to
//! this process's life by their stdin, ended with everything they left behind, and started again
//! on a schedule until it is plain that they will not stay up.
//!
//! The crate knows nothing of what the child is. `harnix-instance` keeps an Agix daemon with it,
//! and the extension host keeps extension processes with it.

mod child;
mod policy;
pub mod reap;
mod supervisor;

pub use child::{Child, ChildSpec, Sandbox};
pub use policy::{Decision, RestartPolicy, Restarts};
pub use supervisor::{ChildEvent, Supervisor};
