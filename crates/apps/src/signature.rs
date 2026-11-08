//! `SIGNATURES`: who vouches for a package, and for exactly which of its bytes.
//!
//! A package names every file it carries with its sha256, and two parties sign that list:
//!
//! - its **publisher**, with an ed25519 key of their own, over the files they made (the manifest,
//!   `ui/`, `agent/`, the portable component, and any compiled artifact they built themselves);
//! - the **store**, which checked the publisher, compiled the component for each engine it
//!   serves, and countersigns the whole list — the publisher's signature included, and the
//!   compiled artifacts it added (the only files it may add).
//!
//! ```json
//! {
//!   "format": "harnix-app-signatures@1",
//!   "files": {"harnix-app.toml": "<sha256>", "wasm/tools.<engine>.cwasm": "<sha256>", …},
//!   "publisher": {"id": "acme", "key": "<hex>", "files": ["harnix-app.toml", …], "signature": "<hex>"},
//!   "store": {"key": "<hex>", "signature": "<hex>"}
//! }
//! ```
//!
//! What a package is trusted as ([`Trust`]) is decided here and nowhere else: `store` when the
//! countersignature verifies under a store key this Harnix carries; `trusted` when the publisher's
//! signature covers every file and its key is one the profile names for that publisher; and
//! nothing else is signed. A package with no `SIGNATURES` is `unsigned`, which only a process with
//! `[apps] dev = true` runs. Every file is checked against its sha256 — one too many, one missing
//! or one changed is refused, naming it — and the compiled artifact's bytes are checked once more
//! as they are read to be loaded, so what is deserialized is what was signed.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The file, at the package's root. Not in its own list.
pub const FILE: &str = "SIGNATURES";
pub const FORMAT: &str = "harnix-app-signatures@1";

/// How far a package is trusted, as its install record keeps it: what runs it without dev mode,
/// and (later) whether its interface runs in the page or in an isolated frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Countersigned by a store this Harnix carries the key of.
    Store,
    /// Signed, in full, by a publisher the profile trusts.
    Trusted,
    /// No signatures at all: only with `[apps] dev = true`.
    Unsigned,
}

impl Trust {
    pub fn as_str(self) -> &'static str {
        match self {
            Trust::Store => "store",
            Trust::Trusted => "trusted",
            Trust::Unsigned => "unsigned",
        }
    }

    pub fn signed(self) -> bool {
        self != Trust::Unsigned
    }
}

impl fmt::Display for Trust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PublicKey(pub [u8; 32]);

impl PublicKey {
    /// 64 hex digits; whitespace around them is ignored, so a key file may end in a newline.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let bytes =
            hex::decode(text).map_err(|_| format!("`{text}` is not a key (64 hex digits)"))?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| format!("`{text}` is not a key (64 hex digits)"))?;
        Ok(Self(bytes))
    }

    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }

    fn verifies(&self, message: &[u8], signature_hex: &str) -> bool {
        let Ok(signature) = hex::decode(signature_hex) else {
            return false;
        };
        UnparsedPublicKey::new(&ED25519, self.0)
            .verify(message, &signature)
            .is_ok()
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.hex())
    }
}

/// An ed25519 signing key: its 32-byte seed, kept as 64 hex digits in a key file.
pub struct SecretKey {
    pair: Ed25519KeyPair,
}

impl SecretKey {
    pub fn generate() -> Result<(Self, String), String> {
        let mut seed = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut seed)
            .map_err(|_| "no randomness to make a key with".to_string())?;
        let hex = hex::encode(seed);
        Ok((Self::from_seed(&seed)?, hex))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let seed =
            hex::decode(text.trim()).map_err(|_| "a key file holds 64 hex digits".to_string())?;
        Self::from_seed(&seed)
    }

    fn from_seed(seed: &[u8]) -> Result<Self, String> {
        let pair = Ed25519KeyPair::from_seed_unchecked(seed)
            .map_err(|e| format!("not an ed25519 key: {e}"))?;
        Ok(Self { pair })
    }

    pub fn public(&self) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(self.pair.public_key().as_ref());
        PublicKey(bytes)
    }

    fn sign(&self, message: &[u8]) -> String {
        hex::encode(self.pair.sign(message).as_ref())
    }
}

/// The keys a process verifies with.
#[derive(Debug, Clone, Default)]
pub struct Keys {
    /// The stores whose countersignature makes a package `store`.
    pub store: Vec<PublicKey>,
    /// `[apps.trusted_publishers]`: a publisher's id, and the keys that sign as it.
    pub trusted: BTreeMap<String, Vec<PublicKey>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signatures {
    pub format: String,
    /// Every file of the package but this one, by its path in the package: its sha256, hex.
    pub files: BTreeMap<String, String>,
    pub publisher: PublisherSignature,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<StoreSignature>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherSignature {
    /// The manifest's `[app] publisher`.
    pub id: String,
    pub key: String,
    /// The files it signed: all of them, or all but the artifacts a store added.
    pub files: Vec<String>,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSignature {
    pub key: String,
    pub signature: String,
}

/// What a signature is over: the application, the signer, and each file's sha256, in path order.
/// Text, so that what was signed can be read.
fn payload<'a>(
    role: &str,
    app: (&str, &str),
    signer: &str,
    files: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> Vec<u8> {
    let mut text = format!("{FORMAT} {role}\napp {} {}\n{signer}\n", app.0, app.1);
    for (path, sha) in files {
        text.push_str(sha);
        text.push_str("  ");
        text.push_str(path);
        text.push('\n');
    }
    text.into_bytes()
}

impl Signatures {
    pub fn parse(text: &str) -> Result<Self, String> {
        let signatures: Signatures =
            serde_json::from_str(text).map_err(|e| format!("{FILE} does not read: {e}"))?;
        if signatures.format != FORMAT {
            return Err(format!(
                "{FILE} is `{}`, and this Harnix reads `{FORMAT}`",
                signatures.format
            ));
        }
        Ok(signatures)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serializes")
    }

    fn publisher_payload(&self, app: (&str, &str)) -> Result<Vec<u8>, String> {
        let mut covered = BTreeMap::new();
        for path in &self.publisher.files {
            let sha = self.files.get(path).ok_or_else(|| {
                format!("the publisher signed `{path}`, which the package does not list")
            })?;
            covered.insert(path, sha);
        }
        Ok(payload(
            "publisher",
            app,
            &format!("publisher {} {}", self.publisher.id, self.publisher.key),
            covered,
        ))
    }

    fn store_payload(&self, app: (&str, &str)) -> Vec<u8> {
        payload(
            "store",
            app,
            &format!(
                "publisher {} {} {}",
                self.publisher.id, self.publisher.key, self.publisher.signature
            ),
            &self.files,
        )
    }

    /// The publisher signs every file in `files`.
    pub fn sign(
        app: (&str, &str),
        publisher: &str,
        key: &SecretKey,
        files: BTreeMap<String, String>,
    ) -> Self {
        let mut signatures = Signatures {
            format: FORMAT.into(),
            publisher: PublisherSignature {
                id: publisher.into(),
                key: key.public().hex(),
                files: files.keys().cloned().collect(),
                signature: String::new(),
            },
            files,
            store: None,
        };
        let message = signatures
            .publisher_payload(app)
            .expect("every file it signs is listed");
        signatures.publisher.signature = key.sign(&message);
        signatures
    }

    /// The store countersigns what the publisher signed, and the compiled artifacts it added:
    /// `files` is every file of the package now. The publisher's signature is checked first; a
    /// store does not vouch for what nobody signed.
    pub fn countersign(
        &self,
        app: (&str, &str),
        key: &SecretKey,
        files: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let publisher_key = PublicKey::parse(&self.publisher.key)?;
        if !publisher_key.verifies(&self.publisher_payload(app)?, &self.publisher.signature) {
            return Err("the publisher's signature does not verify".into());
        }
        let mut next = Signatures {
            files,
            store: None,
            ..self.clone()
        };
        for path in &next.publisher.files {
            if next.files.get(path) != self.files.get(path) {
                return Err(format!("`{path}` is not the file the publisher signed"));
            }
        }
        check_added(&next)?;
        let message = next.store_payload(app);
        next.store = Some(StoreSignature {
            key: key.public().hex(),
            signature: key.sign(&message),
        });
        Ok(next)
    }
}

/// What a store may add to what the publisher signed: compiled artifacts, nothing else.
fn check_added(signatures: &Signatures) -> Result<(), String> {
    let covered: BTreeSet<&String> = signatures.publisher.files.iter().collect();
    for path in signatures.files.keys().filter(|p| !covered.contains(p)) {
        if !(path.starts_with("wasm/") && path.ends_with(".cwasm") && !path[5..].contains('/')) {
            return Err(format!(
                "`{path}` is not signed by the publisher, and a store adds only compiled \
                 components (wasm/<name>.<engine>.cwasm)"
            ));
        }
    }
    Ok(())
}

/// The sha256 of `bytes`, hex.
pub fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A package's signatures, checked: what it is trusted as, and the sha256 of each of its files
/// (what a reader checks an artifact's bytes against as it loads them).
#[derive(Debug, Clone)]
pub struct Verified {
    pub trust: Trust,
    /// The publisher's key, when it is signed.
    pub publisher_key: Option<PublicKey>,
    pub files: BTreeMap<String, String>,
}

impl Verified {
    /// Check `bytes`, read from the package's `path`, against what was signed. Unsigned, there is
    /// nothing to check.
    pub fn check(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        if !self.trust.signed() {
            return Ok(());
        }
        match self.files.get(path) {
            Some(sha) if *sha == sha256(bytes) => Ok(()),
            Some(_) => Err(format!(
                "`{path}` is not the file that was signed (its sha256 differs)"
            )),
            None => Err(format!("`{path}` is not one of the files that were signed")),
        }
    }
}

/// Check `signatures` (the package's `SIGNATURES`, if it has one) against the files it has now
/// (`present`: path → sha256, `SIGNATURES` not among them), for the application the manifest says
/// it is (`app`: id and version; `publisher`: its `[app] publisher`).
pub fn verify(
    signatures: Option<&Signatures>,
    present: &BTreeMap<String, String>,
    app: (&str, &str),
    publisher: &str,
    keys: &Keys,
) -> Result<Verified, String> {
    let Some(s) = signatures else {
        return Ok(Verified {
            trust: Trust::Unsigned,
            publisher_key: None,
            files: present.clone(),
        });
    };
    if let Some(path) = present.keys().find(|p| !s.files.contains_key(*p)) {
        return Err(format!(
            "the package carries `{path}`, which is not one of the files that were signed"
        ));
    }
    for (path, sha) in &s.files {
        match present.get(path) {
            None => return Err(format!("the package is missing `{path}`, which was signed")),
            Some(actual) if actual != sha => {
                return Err(format!(
                    "`{path}` is not the file that was signed: its sha256 is {actual}, and {sha} was signed"
                ))
            }
            Some(_) => {}
        }
    }
    if s.publisher.id != publisher {
        return Err(format!(
            "the package is signed as the publisher `{}`, and its manifest says `{publisher}`",
            s.publisher.id
        ));
    }
    let publisher_key =
        PublicKey::parse(&s.publisher.key).map_err(|e| format!("the publisher's key: {e}"))?;
    if !publisher_key.verifies(&s.publisher_payload(app)?, &s.publisher.signature) {
        return Err(format!(
            "the publisher's signature does not verify (publisher `{}`, key {})",
            s.publisher.id, s.publisher.key
        ));
    }
    let covers_all = s.publisher.files.len() == s.files.len();
    let verified = |trust| Verified {
        trust,
        publisher_key: Some(publisher_key),
        files: s.files.clone(),
    };
    let trusted_publisher = keys
        .trusted
        .get(publisher)
        .is_some_and(|ks| ks.contains(&publisher_key));
    if let Some(store) = &s.store {
        check_added(s)?;
        let store_key =
            PublicKey::parse(&store.key).map_err(|e| format!("the store's key: {e}"))?;
        if keys.store.contains(&store_key) {
            if !store_key.verifies(&s.store_payload(app), &store.signature) {
                return Err(format!(
                    "the store's countersignature does not verify (store key {})",
                    store.key
                ));
            }
            return Ok(verified(Trust::Store));
        }
        if trusted_publisher && covers_all {
            return Ok(verified(Trust::Trusted));
        }
        return Err(format!(
            "the package is countersigned by a store this Harnix does not trust (key {}), and its \
             publisher `{publisher}` is not one the profile trusts ([apps.trusted_publishers])",
            store.key
        ));
    }
    if !covers_all {
        return Err(
            "the publisher did not sign every file, and no store countersigned the rest".into(),
        );
    }
    if trusted_publisher {
        return Ok(verified(Trust::Trusted));
    }
    Err(format!(
        "the package is signed by the publisher `{publisher}` (key {}), which is not one the profile \
         trusts ([apps.trusted_publishers]), and no store countersigned it",
        s.publisher.key
    ))
}

/* ── the files of a package, as signed ─────────────────────────────────── */

/// Every file of a zip but `SIGNATURES`, with its sha256; and `SIGNATURES`, if it has one.
pub fn of_zip(path: &Path) -> Result<(BTreeMap<String, String>, Option<Signatures>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("reading the package: {e}"))?;
    let mut files = BTreeMap::new();
    let mut signatures = None;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("reading the package: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(|e| format!("`{name}`: {e}"))?;
        if name == FILE {
            let text = String::from_utf8(bytes).map_err(|_| format!("{FILE} is not text"))?;
            signatures = Some(Signatures::parse(&text)?);
        } else if files.insert(name.clone(), sha256(&bytes)).is_some() {
            return Err(format!("the package has `{name}` twice"));
        }
    }
    Ok((files, signatures))
}

/// The same of a package unpacked at `root`.
pub fn of_dir(root: &Path) -> Result<(BTreeMap<String, String>, Option<Signatures>), String> {
    let mut walked = Vec::new();
    crate::package::walk(root, root, &mut walked)
        .map_err(|e| format!("{}: {e}", root.display()))?;
    let mut files = BTreeMap::new();
    let mut signatures = None;
    for (relative, path) in walked {
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if relative == FILE {
            let text = String::from_utf8(bytes).map_err(|_| format!("{FILE} is not text"))?;
            signatures = Some(Signatures::parse(&text)?);
        } else {
            files.insert(relative, sha256(&bytes));
        }
    }
    Ok((files, signatures))
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: (&str, &str) = ("acme.notes", "1.0.0");

    fn key(byte: u8) -> SecretKey {
        SecretKey::from_seed(&[byte; 32]).unwrap()
    }

    fn files(cwasm: bool) -> BTreeMap<String, String> {
        let mut files = BTreeMap::from([
            ("harnix-app.toml".to_string(), sha256(b"manifest")),
            ("ui/notes.js".to_string(), sha256(b"module")),
            ("wasm/tools.wasm".to_string(), sha256(b"portable")),
        ]);
        if cwasm {
            files.insert(
                "wasm/tools.wasmtime-1+x+abc.cwasm".into(),
                sha256(b"compiled"),
            );
        }
        files
    }

    fn keys(store: &[&SecretKey], trusted: &[(&str, &SecretKey)]) -> Keys {
        let mut keys = Keys {
            store: store.iter().map(|k| k.public()).collect(),
            ..Keys::default()
        };
        for (id, k) in trusted {
            keys.trusted
                .entry(id.to_string())
                .or_default()
                .push(k.public());
        }
        keys
    }

    #[test]
    fn keys_round_trip_through_their_text() {
        let (secret, seed) = SecretKey::generate().unwrap();
        let again = SecretKey::parse(&format!("{seed}\n")).unwrap();
        assert_eq!(secret.public(), again.public());
        assert_eq!(
            PublicKey::parse(&secret.public().hex()).unwrap(),
            secret.public()
        );
        assert!(PublicKey::parse("abcd").is_err());
        assert!(SecretKey::parse("not hex").is_err());
    }

    #[test]
    fn unsigned_is_unsigned_and_checks_nothing() {
        let verified = verify(None, &files(true), APP, "acme", &Keys::default()).unwrap();
        assert_eq!(verified.trust, Trust::Unsigned);
        assert!(verified.check("ui/notes.js", b"anything").is_ok());
    }

    #[test]
    fn a_store_countersignature_is_store_trust() {
        let (publisher, store) = (key(1), key(2));
        let signed = Signatures::sign(APP, "acme", &publisher, files(false));
        // The store adds the artifact it compiled, and countersigns the lot.
        let countersigned = signed.countersign(APP, &store, files(true)).unwrap();
        let verified = verify(
            Some(&countersigned),
            &files(true),
            APP,
            "acme",
            &keys(&[&store], &[]),
        )
        .unwrap();
        assert_eq!(verified.trust, Trust::Store);
        assert_eq!(verified.publisher_key, Some(publisher.public()));
        assert!(verified
            .check("wasm/tools.wasmtime-1+x+abc.cwasm", b"compiled")
            .is_ok());
        let err = verified
            .check("wasm/tools.wasmtime-1+x+abc.cwasm", b"other")
            .unwrap_err();
        assert!(err.contains("sha256 differs"), "{err}");

        // Through JSON and back, as it is kept in the package.
        let again = Signatures::parse(&countersigned.to_json()).unwrap();
        assert_eq!(again, countersigned);
    }

    #[test]
    fn a_trusted_publisher_signs_for_itself_and_nobody_else_does() {
        let publisher = key(1);
        let signed = Signatures::sign(APP, "acme", &publisher, files(true));
        let trusted = keys(&[], &[("acme", &publisher)]);
        assert_eq!(
            verify(Some(&signed), &files(true), APP, "acme", &trusted)
                .unwrap()
                .trust,
            Trust::Trusted
        );

        let err = verify(Some(&signed), &files(true), APP, "acme", &Keys::default()).unwrap_err();
        assert!(err.contains("not one the profile trusts"), "{err}");
        // Trusted as another publisher is not trusted as this one.
        let elsewhere = keys(&[], &[("other", &publisher)]);
        assert!(verify(Some(&signed), &files(true), APP, "acme", &elsewhere).is_err());
        // A key of the publisher's name that is not its key.
        let impostor = keys(&[], &[("acme", &key(9))]);
        assert!(verify(Some(&signed), &files(true), APP, "acme", &impostor).is_err());
    }

    #[test]
    fn what_was_not_signed_is_refused_naming_it() {
        let (publisher, store) = (key(1), key(2));
        let countersigned = Signatures::sign(APP, "acme", &publisher, files(false))
            .countersign(APP, &store, files(true))
            .unwrap();
        let k = keys(&[&store], &[]);
        let check = |s: &Signatures, present: &BTreeMap<String, String>, says: &str| {
            let err = verify(Some(s), present, APP, "acme", &k).unwrap_err();
            assert!(err.contains(says), "wanted `{says}`, got: {err}");
        };

        let mut changed = files(true);
        changed.insert("ui/notes.js".into(), sha256(b"tampered"));
        check(
            &countersigned,
            &changed,
            "`ui/notes.js` is not the file that was signed",
        );

        let mut extra = files(true);
        extra.insert("ui/extra.js".into(), sha256(b"x"));
        check(
            &countersigned,
            &extra,
            "`ui/extra.js`, which is not one of the files",
        );

        let mut missing = files(true);
        missing.remove("wasm/tools.wasm");
        check(&countersigned, &missing, "missing `wasm/tools.wasm`");

        let mut bad_store = countersigned.clone();
        let sig = &mut bad_store.store.as_mut().unwrap().signature;
        sig.replace_range(0..2, if sig.starts_with("00") { "11" } else { "00" });
        check(
            &bad_store,
            &files(true),
            "the store's countersignature does not verify",
        );

        let mut bad_publisher = countersigned.clone();
        bad_publisher.publisher.signature = hex::encode([7u8; 64]);
        check(
            &bad_publisher,
            &files(true),
            "the publisher's signature does not verify",
        );

        // Signed as another application, or as another publisher than the manifest says.
        assert!(verify(
            Some(&countersigned),
            &files(true),
            ("acme.notes", "1.0.1"),
            "acme",
            &k
        )
        .is_err());
        assert!(verify(Some(&countersigned), &files(true), APP, "other", &k)
            .unwrap_err()
            .contains("its manifest says `other`"));

        // A store this Harnix does not carry the key of is nobody.
        let err = verify(
            Some(&countersigned),
            &files(true),
            APP,
            "acme",
            &keys(&[&key(3)], &[]),
        )
        .unwrap_err();
        assert!(err.contains("a store this Harnix does not trust"), "{err}");
    }

    #[test]
    fn a_store_adds_compiled_components_and_nothing_else() {
        let (publisher, store) = (key(1), key(2));
        let signed = Signatures::sign(APP, "acme", &publisher, files(false));
        let mut more = files(true);
        more.insert("ui/added-by-the-store.js".into(), sha256(b"x"));
        let err = signed.countersign(APP, &store, more).unwrap_err();
        assert!(
            err.contains("a store adds only compiled components"),
            "{err}"
        );

        // Nor may it change what the publisher signed.
        let mut changed = files(true);
        changed.insert("ui/notes.js".into(), sha256(b"changed"));
        assert!(signed
            .countersign(APP, &store, changed)
            .unwrap_err()
            .contains("not the file the publisher signed"));

        // And a publisher's signature that does not cover everything is not enough on its own.
        let partial = Signatures {
            files: files(true),
            ..signed.clone()
        };
        let err = verify(
            Some(&partial),
            &files(true),
            APP,
            "acme",
            &keys(&[], &[("acme", &publisher)]),
        )
        .unwrap_err();
        assert!(err.contains("did not sign every file"), "{err}");
    }
}
