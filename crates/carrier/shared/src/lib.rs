//! What the two package carriers have in common: a running package and the roster of them, the
//! interface entries the page is told of, a package's tools as a session sees them, and the routes
//! under `/ext/<id>/`. The carriers differ in how a call reaches the package (an instance in this
//! process, or a process of its own over the extension wire) and keep that to themselves; this
//! crate names neither, and neither carrier names the other.

pub mod frames;
mod limits;
pub mod registry;
pub mod routes;
pub mod sync;
pub mod tools;
mod ui_stamp;

pub use limits::limits;
pub use harnix_package::{server_name, tool_name};
pub use registry::{Listener, Loaded, OnProgress, Roster, DEFAULT_TIMEOUT};
pub use routes::{Hosted, McpEndpoint, UiFiles, FRAME_CSP};
pub use tools::{MAX_NOTE, NAME as TOOLS_NAME};
pub use ui_stamp::ui_stamp;
