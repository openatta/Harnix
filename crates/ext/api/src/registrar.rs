//! What a native extension declares through, in `register`.

use std::path::PathBuf;

use harnix_contract::s1::{Access, ExtId};
use harnix_contract::SpaceDecl;
use harnix_engine_api::{EngineContribution, InstanceTemplate, ToolSpec};

use crate::descriptor::{DescriptorBuilder, MethodBuilder, TopicBuilder};
use crate::http::RouteHandler;
use crate::ui::UiDecl;

/// One native extension's declarations, under its own id: `method("quote")` is `<id>.quote`,
/// `template("worker")` is `<id>.worker`, a route is under `/<id>/`. Only the extension host
/// makes one.
pub struct Registrar<'a, C> {
    id: &'a ExtId,
    config: &'a C,
    builder: &'a mut DescriptorBuilder,
}

impl<'a, C> Registrar<'a, C> {
    pub(crate) fn new(id: &'a ExtId, config: &'a C, builder: &'a mut DescriptorBuilder) -> Self {
        Self { id, config, builder }
    }

    pub fn id(&self) -> &ExtId {
        self.id
    }

    /// Its section of the profile, read already: what it declares may depend on it.
    pub fn config(&self) -> &C {
        self.config
    }

    /// `<id>.<name>`. Nobody but the kernel may call it unless its access says.
    pub fn method(&mut self, name: &str) -> MethodBuilder<'_> {
        let name = self.own(name);
        self.builder.method(&name)
    }

    /// `<id>.<name>`. Devices and every extension may subscribe unless its access says.
    pub fn topic(&mut self, name: &str) -> TopicBuilder<'_> {
        let name = self.own(name);
        self.builder.topic(&name)
    }

    /// A space this extension is the backend of: where its new sessions land and in what scene.
    pub fn space(&mut self, decl: SpaceDecl) {
        self.builder.space(decl);
    }

    /// A tool's spec, so the diagnostics can say which tools an extension gives. What runs is
    /// a [`Tool`](crate::engine::Tool) or [`ToolProvider`](crate::engine::ToolProvider) in
    /// [`Registrar::engine`].
    pub fn tool(&mut self, spec: ToolSpec) {
        self.builder.tool(spec);
    }

    /// HTTP under `/<id>/<prefix>`, same origin as the interface, admitted like any request and
    /// decided by `access`.
    pub fn route(&mut self, prefix: &str, access: Access, handler: impl RouteHandler) {
        self.builder.route(prefix, access, handler);
    }

    /// Files under `dir` at `/<id>/<prefix>`: read-only, no listing, nothing outside `dir`.
    pub fn static_dir(&mut self, prefix: &str, dir: PathBuf, access: Access) {
        self.builder.static_dir(prefix, dir, access);
    }

    /// The interface half: drawn by the shell, declared here.
    pub fn ui(&mut self) -> &mut UiDecl {
        self.builder.ui()
    }

    /// What it gives the `main` instance's engine: scenes, tool providers, prompt gates, a
    /// credential source.
    pub fn engine(&mut self) -> &mut EngineContribution {
        self.builder.engine()
    }

    /// What it gives every engine this process builds or spawns: tool providers, registered on
    /// each one's link. An attached daemon is somebody else's and gets none.
    pub fn engine_everywhere(&mut self) -> &mut EngineContribution {
        self.builder.engine_everywhere()
    }

    /// An instance template, `<id>.<its id>`.
    pub fn template(&mut self, mut template: InstanceTemplate) {
        template.id = self.own(&template.id);
        self.builder.template(template);
    }

    fn own(&self, name: &str) -> String {
        format!("{}.{name}", self.id)
    }
}
