//! Capabilities: named privileges, and the kernel's catalogue of them.
//!
//! A capability is all a privilege is. There is no first-party kind of extension: the system
//! space's power is a few of these, and the same grant gives another extension the same power.
//! An extension declares its own beside the kernel's (`<its id>.<name>`), requires them on its
//! rows, and the assembly grants them like any other.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// One named privilege.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capability(Cow<'static, str>);

impl Capability {
    pub const fn new(name: &'static str) -> Self {
        Self(Cow::Borrowed(name))
    }

    /// One named at runtime: read from a manifest, or off the wire.
    pub fn named(name: impl Into<String>) -> Self {
        Self(Cow::Owned(name.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A capability as its definer declares it: its name, who may be granted it, and what holding it
/// means, for the diagnostics page and a package's disclosure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySpec {
    name: &'static str,
    native: bool,
    package: bool,
    disclosure: &'static str,
}

impl CapabilitySpec {
    /// Grantable to native extensions; [`CapabilitySpec::package`] opens it to packages too.
    pub const fn new(name: &'static str, disclosure: &'static str) -> Self {
        Self { name, native: true, package: false, disclosure }
    }

    /// A package may ask for it as well.
    pub const fn package(mut self) -> Self {
        self.package = true;
        self
    }

    /// Native extensions only (the default, said out loud).
    pub const fn native_only(mut self) -> Self {
        self.native = true;
        self.package = false;
        self
    }

    /// Packages only: what a native extension does not need asked, it is given by being native.
    pub const fn package_only(mut self) -> Self {
        self.native = false;
        self.package = true;
        self
    }

    pub const fn capability(&self) -> Capability {
        Capability::new(self.name)
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn disclosure(&self) -> &'static str {
        self.disclosure
    }

    pub fn is_native(&self) -> bool {
        self.native
    }

    pub fn is_package(&self) -> bool {
        self.package
    }
}

/// The kernel's capabilities. Their names are the kernel's words; only the skeleton adds to them.
pub mod caps {
    use super::{Capability, CapabilitySpec};

    /// Stop and restart any instance not the kernel's own; build from any template.
    pub const INSTANCES_ADMIN: Capability = Capability::new("instances.admin");
    /// Read the inside of any session: its context, tasks, task output, goal.
    pub const ENGINE_INSPECT: Capability = Capability::new("engine.inspect");
    /// Change the inside of any session: tasks, messages, compaction, model, name, goal; and trust a
    /// project, which changes what every session in it may do.
    pub const ENGINE_CONTROL: Capability = Capability::new("engine.control");
    pub const ENGINE_CONFIG_READ: Capability = Capability::new("engine.config.read");
    /// Write the engine's settings and providers, reload them, hand them to inheriting instances.
    pub const ENGINE_CONFIG_WRITE: Capability = Capability::new("engine.config.write");
    /// Install, remove, switch and reload the engine's plugins.
    pub const ENGINE_PLUGINS: Capability = Capability::new("engine.plugins");
    /// Export a session, and import a bundle as new sessions.
    pub const ENGINE_EXPORT: Capability = Capability::new("engine.export");
    pub const ENGINE_MCP: Capability = Capability::new("engine.mcp");
    pub const RECORDINGS_READ: Capability = Capability::new("recordings.read");
    pub const TRANSCRIPTS_READ: Capability = Capability::new("transcripts.read");
    /// Versions, uptime, memory, threads, the connected clients; each extension's log.
    pub const PROCESS_OBSERVE: Capability = Capability::new("process.observe");
    pub const DEVICES_MANAGE: Capability = Capability::new("devices.manage");
    /// The host's services: registering extensions while the process runs.
    pub const EXTENSIONS_MANAGE: Capability = Capability::new("extensions.manage");
    pub const UI_PAGE: Capability = Capability::new("ui.page");
    pub const EVENTS_ALL_TOOLS: Capability = Capability::new("events.all_tools");

    /// Every one of them, as the kernel declares it.
    pub const KERNEL: &[CapabilitySpec] = &[
        CapabilitySpec::new("instances.admin", "停、重启任何不属于内核的实例；按任何模板建实例"),
        CapabilitySpec::new("engine.inspect", "读任何会话的内部：上下文、任务、任务输出、目标"),
        CapabilitySpec::new("engine.control", "改任何会话的内部：任务、消息、压缩、清空、模型、名字、目标；读、改项目的信任"),
        CapabilitySpec::new("engine.config.read", "读引擎设置与 provider"),
        CapabilitySpec::new("engine.config.write", "写引擎设置与 provider，重读设置，交给继承它的实例"),
        CapabilitySpec::new("engine.plugins", "装、卸、启停、重读 Agix 插件"),
        CapabilitySpec::new("engine.export", "导出、导入会话"),
        CapabilitySpec::new("engine.mcp", "给引擎加 MCP 服务器"),
        CapabilitySpec::new("recordings.read", "读模型调用录像与用量"),
        CapabilitySpec::new("transcripts.read", "读会话转录"),
        CapabilitySpec::new("process.observe", "版本、运行时长、内存、线程、已连接的客户端；各扩展的日志"),
        CapabilitySpec::new("devices.manage", "配对码、列设备、撤销设备"),
        CapabilitySpec::new("extensions.manage", "运行期登记扩展，用宿主的服务"),
        CapabilitySpec::new("ui.page", "进页面的 ES 模块").package(),
        CapabilitySpec::new("events.all_tools", "看到每个会话的每一次工具调用").package_only(),
    ];

    /// The kernel's declaration of `cap`, when it is the kernel's.
    pub fn kernel(cap: &Capability) -> Option<&'static CapabilitySpec> {
        KERNEL.iter().find(|spec| spec.name() == cap.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::caps;

    #[test]
    fn every_named_kernel_capability_is_in_the_catalogue_once() {
        for cap in [
            caps::INSTANCES_ADMIN,
            caps::ENGINE_INSPECT,
            caps::ENGINE_CONTROL,
            caps::ENGINE_CONFIG_READ,
            caps::ENGINE_CONFIG_WRITE,
            caps::ENGINE_PLUGINS,
            caps::ENGINE_EXPORT,
            caps::ENGINE_MCP,
            caps::RECORDINGS_READ,
            caps::TRANSCRIPTS_READ,
            caps::PROCESS_OBSERVE,
            caps::DEVICES_MANAGE,
            caps::EXTENSIONS_MANAGE,
            caps::UI_PAGE,
            caps::EVENTS_ALL_TOOLS,
        ] {
            assert!(caps::kernel(&cap).is_some(), "{cap}");
        }
        let mut names: Vec<_> = caps::KERNEL.iter().map(|s| s.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), caps::KERNEL.len());
        assert!(!caps::kernel(&caps::EVENTS_ALL_TOOLS).unwrap().is_native());
        assert!(caps::kernel(&caps::UI_PAGE).unwrap().is_package());
    }
}
