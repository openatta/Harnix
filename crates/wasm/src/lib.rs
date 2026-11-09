//! The WebAssembly runtime for Harnix applications.
//!
//! An application's backend logic and its agent tools are WebAssembly
//! components (the `harnix:app` world in `wit/app.wit`). This crate is the
//! whole of the boundary between them and Harnix: it owns the one wasmtime
//! engine a process has, loads the precompiled artifacts the store ships, and
//! runs each application in its own [`AppVm`].
//!
//! # One VM per application
//!
//! An [`AppVm`] is a boundary, not a process. It holds the application's
//! component, linked once; a **resident** backend instance — one long-lived
//! store with the application's state, its subscriptions and its timers,
//! entered by one call at a time; and a fresh, short-lived instance for every
//! tool call and event, so tool calls run concurrently and a bad one damages
//! nothing that outlives it. Memory is accounted per application across all of
//! them, calls are bounded by a deadline, and what keeps faulting — a tool, or
//! a backend that crashes again after its rebuild — is set aside rather than
//! retried forever.
//!
//! # What the host provides
//!
//! The crate depends on nothing else of Harnix's. What a component may ask of
//! the host — call a hub method, publish a topic, subscribe, reach the
//! network, store a value — goes through traits the host implements
//! ([`AppHost`], [`HttpFetcher`], [`KvStore`]), and every one of those
//! requests is checked against the application's declared [`Capabilities`]
//! here, before the host sees it. wasmtime's own types do not appear in the
//! public API, so nothing outside this crate names wasmtime.
//!
//! # Precompiled only
//!
//! The product build links no compiler: it deserializes artifacts the store
//! compiled for its [`WasmEngine::fingerprint`]. The `compile` feature adds
//! Cranelift for development, tests and the store's own build tool
//! (`harnix-wasm-compile`). Every artifact is compiled for the target's
//! baseline [`CPU`], so one compiled on one machine loads on every machine
//! of its target triple.

mod bindings;
mod engine;
mod health;
mod host;
mod kv;
mod services;
mod spec;
mod state;
mod storage;
mod sync;
mod vm;

pub use bindings::WIT_PACKAGE;
pub use engine::{AppComponent, WasmEngine, CPU, EPOCH_TICK, WASMTIME_VERSION};
pub use health::{HealthSnapshot, VmState, FAULT_LIMIT};
pub use kv::{FileKv, KvError, KvStore, MemoryKv, MAX_KEY_BYTES};
pub use services::{
    AppHost, HttpFetcher, HttpRequest, HttpResponse, NoNetwork, ProgressSink, Services,
};
pub use spec::{host_of, AppDirs, AppSpec, Capabilities, FsGrant, Limits};
pub use vm::{
    AppError, AppVm, CallFailure, HookDecision, ToolDef, ToolOutput, WeakAppVm, World,
    HEALTHY_PERIOD, MAX_SUBSCRIPTIONS, MAX_TIMERS,
};
