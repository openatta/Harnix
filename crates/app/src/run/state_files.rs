//! What this process keeps in its state directory beside the ledger: the paired devices and the
//! token, both readable by their owner only.

use harnix_authz::Devices;

/// The paired devices, beside the token in the state directory.
pub(super) const DEVICES_FILE: &str = "devices.json";

/// The devices paired before this start, and every change written back.
///
/// A file that is not there is no devices. One that does not parse is set aside as
/// `devices.json.broken` and the node starts with none — on a reachable listener it then prints a
/// pairing code, which is the one way back in. One that cannot be read stops the start: taken as
/// empty, the next change would write over it.
pub(super) fn load_devices(path: &std::path::Path) -> anyhow::Result<Devices> {
    let paired: Vec<harnix_authz::Device> = match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(paired) => paired,
            Err(e) => {
                let broken = path.with_extension("json.broken");
                tracing::warn!(error = %e, kept = %broken.display(), "the paired devices could not be read; starting with none");
                std::fs::rename(path, &broken)?;
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => anyhow::bail!("could not read {}: {e}", path.display()),
    };
    let file = path.to_path_buf();
    Ok(Devices::new(paired).persist_with(Box::new(move |devices| {
        if let Err(e) = write_devices(&file, devices) {
            tracing::warn!(error = %e, file = %file.display(), "could not write the paired devices");
        }
    })))
}

/// Replace the devices file whole, readable by its owner only: it holds no secret — public keys —
/// but it decides who is let in, so it is kept like the token.
fn write_devices(path: &std::path::Path, devices: &[harnix_authz::Device]) -> std::io::Result<()> {
    let body = serde_json::to_vec_pretty(devices)?;
    let temp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&temp);
    write_private(&temp, &body)?;
    std::fs::rename(&temp, path)
}

/// Write the token readable by its owner only (IPDrun N-302).
///
/// The token admits a client to everything a person at this machine can do;
/// with launch links it is also what mints them. The default umask leaves a
/// fresh file readable by every account. An existing file is narrowed too,
/// since `create` does not change the mode of a file that is already there.
pub(super) fn write_token(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    write_private(path, token.as_bytes())
}

/// Write `bytes` to a file only its owner can read, narrowing a file that is already there.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        options.mode(0o600);
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    options.open(path)?.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> harnix_authz::Device {
        harnix_authz::Device {
            id: id.into(),
            label: "laptop".into(),
            public_key: "pk".into(),
            paired_at: "1".into(),
            last_seen: None,
        }
    }

    /// Paired devices survive a restart, in a file only the owner can read; a broken one is set
    /// aside rather than trusted or written over.
    #[test]
    fn paired_devices_are_kept_across_starts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICES_FILE);
        assert!(load_devices(&path).unwrap().list().is_empty(), "no file is no devices");

        let devices = load_devices(&path).unwrap();
        devices.begin_pairing("ABC123", "laptop", std::time::SystemTime::now());
        let paired = devices
            .complete_pairing("ABC123", "pk", std::time::SystemTime::now(), "1")
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{mode:o}");
        }
        let again = load_devices(&path).unwrap();
        assert!(again.is_paired(&paired.id));
        assert!(again.revoke(&paired.id));
        assert!(load_devices(&path).unwrap().list().is_empty(), "the revocation was kept");

        write_devices(&path, &[device("dev-a")]).unwrap();
        assert!(load_devices(&path).unwrap().is_paired("dev-a"));

        std::fs::write(&path, "not json").unwrap();
        assert!(load_devices(&path).unwrap().list().is_empty());
        assert!(path.with_extension("json.broken").exists());

        // Unreadable — a directory where the file should be — stops the start.
        std::fs::remove_file(&path).ok();
        std::fs::create_dir(&path).unwrap();
        assert!(load_devices(&path).is_err());
    }
}
