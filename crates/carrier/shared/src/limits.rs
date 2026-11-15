//! What a package is started held to ([`Limits`]): its consented manifest's, and the product's
//! settings, which only shorten how long its backend idles.

use std::time::Duration;

use harnix_package::manifest::Manifest;
use harnix_wasm_port::Limits;

/// `manifest`'s limits; an on-demand backend idling `idle` when the product says, in place of its
/// `idle_secs`.
pub fn limits(manifest: &Manifest, idle: Option<Duration>) -> Limits {
    let component = manifest.component.as_ref();
    let defaults = Limits::default();
    Limits {
        memory_mb: component.map_or(defaults.memory_mb, |c| c.memory_mb),
        timeout_ms: component.map_or(defaults.timeout_ms, |c| c.timeout_ms),
        storage_mb: manifest.capabilities.storage_mb,
        resident: manifest.backend.resident.clone(),
        idle_ms: idle.unwrap_or(Duration::from_secs(manifest.backend.idle_secs)).as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use harnix_package::manifest::RESIDENT;

    use super::*;

    #[test]
    fn the_resident_names_agree_with_the_manifests() {
        assert_eq!(RESIDENT, ["always", "on-demand"]);
        assert!(Limits { resident: RESIDENT[1].into(), ..Limits::default() }.on_demand());
        assert_eq!(Limits::default().resident, RESIDENT[0]);
    }
}
