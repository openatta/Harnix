//! Who a row is open to.
//!
//! Data, judged elsewhere (`harnix_authz::decide`): a row says how far it is open, and the one
//! place that decides reads it. Nothing is open unless a constructor or a `with_*` says so.

use std::collections::BTreeSet;

/// Which extensions a row is open to by its own declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Extensions {
    Any,
    Only(BTreeSet<String>),
}

/// How far one row is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    device: bool,
    kernel: bool,
    extensions: Extensions,
    /// Why it is refused to whoever it is not open to, when that is on purpose.
    refused: Option<&'static str>,
    /// Refused to everyone, the kernel and every grant included: the hub's own.
    hub_owned: bool,
    /// The extensions opened to it by name after it was declared: what a module's manifest
    /// grants, and what a runtime application's `hub` lists. The per-method grant, until rows
    /// require capabilities instead.
    granted: BTreeSet<String>,
}

impl Access {
    fn closed() -> Self {
        Self {
            device: false,
            kernel: false,
            extensions: Extensions::Only(BTreeSet::new()),
            refused: None,
            hub_owned: false,
            granted: BTreeSet::new(),
        }
    }

    /// Open to nobody until a `with_*` says otherwise.
    pub fn nobody() -> Self {
        Self::closed()
    }

    /// Any device, any extension, and the kernel.
    pub fn device() -> Self {
        Self::closed().with_device().with_kernel().with_extensions()
    }

    /// The kernel alone, and an extension granted it by name.
    pub fn kernel() -> Self {
        Self::closed().with_kernel()
    }

    /// Refused to everyone, with a reason a client can show; an extension may still be granted it
    /// by name.
    pub fn refused(why: &'static str) -> Self {
        Self::closed().refusing(why)
    }

    /// Refused to everyone, never grantable: the hub owns it, and reaching it would be a way
    /// around the hub.
    pub fn hub_owned(why: &'static str) -> Self {
        Self {
            hub_owned: true,
            ..Self::refused(why)
        }
    }

    pub fn with_device(mut self) -> Self {
        self.device = true;
        self
    }

    pub fn with_kernel(mut self) -> Self {
        self.kernel = true;
        self
    }

    /// Open to every extension.
    pub fn with_extensions(mut self) -> Self {
        self.extensions = Extensions::Any;
        self
    }

    /// Open to one more extension, by id.
    pub fn with_extension(mut self, id: impl Into<String>) -> Self {
        if let Extensions::Only(ids) = &mut self.extensions {
            ids.insert(id.into());
        }
        self
    }

    pub fn refusing(mut self, why: &'static str) -> Self {
        self.refused = Some(why);
        self
    }

    /// Open it to `ext` by name. Whether it may be is the table's to say.
    pub fn grant(&mut self, ext: &str) {
        self.granted.insert(ext.to_string());
    }

    /// Take back what [`Access::grant`] gave `ext`, and nothing else.
    pub fn revoke(&mut self, ext: &str) {
        self.granted.remove(ext);
    }

    pub fn opens_devices(&self) -> bool {
        self.device
    }

    pub fn opens_kernel(&self) -> bool {
        self.kernel
    }

    /// Whether every extension is let in by the row's own declaration.
    pub fn opens_every_extension(&self) -> bool {
        self.extensions == Extensions::Any
    }

    /// Whether `ext` is let in, by declaration or by grant. Not whether it may call: a row the
    /// hub owns lets in nobody, which is the deciding layer's to apply.
    pub fn opens_extension(&self, ext: &str) -> bool {
        let declared = match &self.extensions {
            Extensions::Any => true,
            Extensions::Only(ids) => ids.contains(ext),
        };
        declared || self.granted.contains(ext)
    }

    pub fn refusal(&self) -> Option<&'static str> {
        self.refused
    }

    pub fn is_hub_owned(&self) -> bool {
        self.hub_owned
    }

    /// Every extension let in, by declaration or grant, sorted: `*` for every extension.
    pub fn extensions(&self) -> Vec<String> {
        let mut all: BTreeSet<String> = self.granted.clone();
        match &self.extensions {
            Extensions::Any => {
                all.insert("*".into());
            }
            Extensions::Only(ids) => all.extend(ids.iter().cloned()),
        }
        all.into_iter().collect()
    }
}
