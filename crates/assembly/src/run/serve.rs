//! Step 6: serving — the token, the interface, the transport's router and its listeners.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use harnix_contract::Topology;
use harnix_hub::Hub;

use super::admission::Admission;
use super::register::Held;
use super::state_files::write_token;
use super::Basis;
use crate::cli::Cli;
use crate::pairing;
use crate::host::Record;

/// A process that serves: the listeners' task, how to stop it, and the channels it has open.
pub(super) struct Serving {
    pub server: tokio::task::JoinHandle<anyhow::Result<()>>,
    /// Stops the listeners taking connections.
    pub stop: tokio::sync::oneshot::Sender<()>,
    pub channels: harnix_transport::Channels,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn serve(
    cli: &Cli,
    basis: &Basis,
    app_name: &str,
    held: &Held,
    records: &[Record],
    hub: &Arc<Hub>,
    admission: &Admission,
    brand_icon: Option<std::path::PathBuf>,
) -> anyhow::Result<Serving> {
    let Basis {
        profile,
        host,
        tls,
        roots,
        state_lock,
        ..
    } = basis;
    let host = *host;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let token_file = roots.state.join("token");
    if let Err(e) = write_token(&token_file, &token) {
        tracing::warn!(error = %e, "could not write the token file");
    }

    let face = face(cli.headless, profile.ui.dir.clone());
    let links = Arc::new(harnix_transport::LaunchLinks::default());
    let browser = if profile.transport.launch_link {
        harnix_transport::BrowserAdmission::LaunchLink
    } else {
        harnix_transport::BrowserAdmission::PageToken
    };
    let routes: Vec<harnix_transport::ExtRoutes> = records
        .iter()
        .filter(|r| r.is_enabled())
        .filter_map(|r| r.descriptor.as_ref())
        .filter(|d| !d.routes().is_empty())
        .map(|d| harnix_transport::ExtRoutes {
            id: d.id().to_string(),
            router: crate::host::routes::router(d.routes().to_vec()),
        })
        .collect();
    let bulk: Arc<dyn harnix_transport::BulkStore> = admission.seats.bulk.clone();
    let max_upload_bytes = harnix_contract::MAX_UPLOAD_BYTES;
    let extensions = harnix_transport::Extensions {
        routes,
        route_gate: Some(admission.router.clone()),
        packages: admission.dynamic.as_ref().map(|host| host.routes.router()),
    };
    let (router, statics, channels) = harnix_transport::router(
        admission.router.clone(),
        hub.clone(),
        bulk,
        held.paired.clone(),
        harnix_transport::Config {
            topologies: profile.transport.topologies.clone(),
            token: token.clone(),
            face: face.clone(),
            tls: tls.clone(),
            // Decided by where this listens, not by a setting. A reachable
            // listener needs a paired device; loopback does not have one to
            // ask for yet, and does not need one (concept §6.3).
            admission: if host.is_loopback() {
                harnix_transport::Admission::Token
            } else {
                harnix_transport::Admission::PairedDevice
            },
            max_upload_bytes,
            browser,
            links: links.clone(),
            extra_origins: profile.transport.extra_origins.clone(),
        },
        extensions,
    );

    statics.set_brand_icon(brand_icon);
    let _ = held.channels.set(channels.clone());

    let addrs = listen_addrs(host, profile.transport.port);
    let addr = addrs[0];

    // Before anything is served: a reachable node with no paired device has
    // no way to acquire one, because pairing is a method and methods need
    // admission. This is where that circle is broken.
    pairing::bootstrap(&held.paired, !host.is_loopback());

    let scheme = if cli.tls_cert.is_some() {
        "https"
    } else {
        "http"
    };
    // Where it listens, once it does: to the extensions (ipdrun-14) and into the lock file
    // (ipdrun-09).
    let told = {
        let listen = admission.listen.clone();
        let state_lock = state_lock.clone();
        move |bound: Vec<SocketAddr>| {
            let Some(first) = bound.first() else { return };
            let url = format!("{scheme}://{first}/");
            state_lock.listening(&url);
            listen.send_replace(Some(harnix_kernel::seat::Listening { addrs: bound, url }));
        }
    };
    // Shutting down stops the listener first, so it is handed the signal here.
    let (stop, serving_stops) = tokio::sync::oneshot::channel::<()>();
    let tls = tls.clone();
    let server = tokio::spawn(async move {
        harnix_transport::serve_with(&addrs, router, tls, told, async move {
            let _ = serving_stops.await;
        })
        .await
    });
    println!(
        "\n  {} → {scheme}://{addr}/\n  token → {}\n  topology → {}\n",
        app_name,
        token_file.display(),
        profile
            .transport
            .topologies
            .iter()
            .map(Topology::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    );
    for record in records.iter().filter(|r| !r.is_enabled()) {
        println!(
            "  extension `{}` disabled at {}: {}\n",
            record.manifest.id(),
            record.status.phase.unwrap_or("?"),
            record.status.reason.clone().unwrap_or_default()
        );
    }
    if browser == harnix_transport::BrowserAdmission::LaunchLink {
        // One link for whoever is reading this console; it is single-use and
        // short-lived, so scrollback is not a leak.
        let code = links.mint(std::time::SystemTime::now());
        println!(
            "  open → {scheme}://{addr}/auth/{code}  (one use, {}s)\n",
            harnix_transport::LINK_TTL.as_secs()
        );
    }
    Ok(Serving {
        server,
        stop,
        channels,
    })
}

/// Which interface, said once and logged, because "I edited a file and
/// nothing changed" is the failure this choice produces when it is silent.
fn face(headless: bool, dir: Option<std::path::PathBuf>) -> harnix_transport::Face {
    let face = match (headless, dir) {
        (true, _) => harnix_transport::Face::Headless,
        (false, Some(dir)) => harnix_transport::Face::Directory(dir),
        (false, None) => harnix_transport::Face::Embedded,
    };
    match &face {
        harnix_transport::Face::Embedded => tracing::info!("serving the interface compiled in"),
        harnix_transport::Face::Directory(dir) => {
            tracing::info!(dir = %dir.display(), "serving the interface from disk, not the one compiled in")
        }
        harnix_transport::Face::Headless => tracing::info!("headless: no interface, RPC only"),
    }
    face
}

/// Both loopback families, or exactly what was asked for. `localhost`
/// resolves to `::1` first in every current browser, so a v4-only listener
/// is a page that will not open.
fn listen_addrs(host: IpAddr, port: u16) -> Vec<SocketAddr> {
    let mut addrs = vec![SocketAddr::new(host, port)];
    if host.is_loopback() && host.is_ipv4() {
        addrs.push(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port));
    }
    addrs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IPv4 loopback is served on both families; anything else, as asked.
    #[test]
    fn loopback_is_served_on_both_families() {
        let v4 = IpAddr::from([127, 0, 0, 1]);
        assert_eq!(
            listen_addrs(v4, 4000),
            vec![SocketAddr::new(v4, 4000), SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 4000)]
        );
        let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
        assert_eq!(listen_addrs(v6, 4000), vec![SocketAddr::new(v6, 4000)]);
        let any = IpAddr::from([0, 0, 0, 0]);
        assert_eq!(listen_addrs(any, 4000), vec![SocketAddr::new(any, 4000)]);
    }

    /// `--headless` wins over a `[ui] dir`; without either, the interface compiled in.
    #[test]
    fn headless_wins_over_a_directory() {
        let dir = std::path::PathBuf::from("/ui");
        assert!(matches!(face(true, Some(dir.clone())), harnix_transport::Face::Headless));
        assert!(matches!(face(false, Some(dir)), harnix_transport::Face::Directory(_)));
        assert!(matches!(face(false, None), harnix_transport::Face::Embedded));
    }
}
