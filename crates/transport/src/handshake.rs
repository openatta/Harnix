//! One handshake, before anything else on a connection.
//!
//! Protocol version, contribution API version and topology are settled once.
//! A mismatch is refused with a sentence saying which side is out of date —
//! never downgraded, because a half-compatible client produces bug reports
//! nobody can act on (§5.1).
//!
//! In a multi-channel topology the later channels carry the credential this
//! produced rather than negotiating again. A negotiation held twice is one
//! that will eventually produce two answers.

use harnix_contract::{codes, Handshake, RpcError, Subject, Topology};
use serde_json::{json, Value};

/// What a connection is after a successful handshake.
pub struct Session {
    pub subject: Subject,
    pub topology: Topology,
}

/// Read a client's `harnix.handshake` and decide.
pub fn negotiate(
    params: &Value,
    supported: &[Topology],
    subject: Subject,
) -> Result<Session, RpcError> {
    let client: Handshake = serde_json::from_value(params.clone())
        .map_err(|e| RpcError::invalid_params(format!("handshake: {e}")))?;
    if let Some(reason) = harnix_contract::handshake::refuse(&client, supported) {
        return Err(RpcError::new(codes::HANDSHAKE_REFUSED, reason));
    }
    Ok(Session {
        subject,
        topology: client.topology,
    })
}

/// The server's half of the handshake.
pub fn accepted(session: &Session, supported: &[Topology]) -> Value {
    json!({
        "protocol_version": harnix_contract::PROTOCOL_VERSION,
        "contrib_api_version": harnix_contract::CONTRIB_API_VERSION,
        "topology": session.topology,
        "topologies": supported,
        "subject": session.subject,
        "channels": ["unary", "session_events", "host_events", "out_of_band", "bulk"],
    })
}

/// A handshake both topologies hold the same way, from its body to the server's answer.
pub(crate) struct Opened {
    /// What every later channel of this subject carries.
    pub credential: String,
    pub session: std::sync::Arc<crate::session::Established>,
    /// The server's half, with the client this is and its credential.
    pub answer: Value,
}

/// Why a handshake was not established.
#[derive(Debug)]
pub(crate) enum Refused {
    /// Admission said no: the token, or off loopback the device's proof. One sentence for every
    /// reason (`admission::NOT_ADMITTED`).
    NotAdmitted(&'static str),
    /// The negotiation did: a version or topology this server does not speak, or one that is
    /// established somewhere else.
    Negotiation(RpcError),
}

/// Admit a handshake, negotiate it, and establish it in `topology`, the one the endpoint it
/// arrived at carries. `body` is the handshake with the token in it; `elsewhere` is what a client
/// asking for another topology is told.
///
/// `single_wire` and `user_agent` are as [`crate::session::Sessions::establish`] takes them.
pub(crate) async fn establish_from_body(
    state: &crate::AppState,
    body: &Value,
    topology: Topology,
    elsewhere: &'static str,
    single_wire: Option<tokio::sync::mpsc::Sender<String>>,
    user_agent: &str,
) -> Result<Opened, Refused> {
    let config = &state.config;
    let device = match crate::admission::admit(
        config.admission,
        &config.token,
        &state.devices,
        &state.challenges,
        body,
    )
    .await
    {
        crate::admission::Verdict::Admit { device } => device,
        crate::admission::Verdict::Refuse(reason) => return Err(Refused::NotAdmitted(reason)),
    };
    let subject = Subject::Device {
        id: device.unwrap_or_else(|| harnix_contract::LOCAL_DEVICE.into()),
    };
    let negotiated = negotiate(body, &config.topologies, subject).map_err(Refused::Negotiation)?;
    if negotiated.topology != topology {
        return Err(Refused::Negotiation(RpcError::new(codes::HANDSHAKE_REFUSED, elsewhere)));
    }
    let (credential, session) = state
        .sessions
        .establish(
            state.hub.as_ref(),
            negotiated.subject.clone(),
            negotiated.topology,
            single_wire,
            user_agent,
        )
        .await;
    let mut answer = accepted(&negotiated, &config.topologies);
    answer["client"] = json!(session.client.0);
    // What a module route (`/apps/<id>/`) is presented with, in either topology.
    answer["credential"] = json!(credential);
    Ok(Opened {
        credential,
        session,
        answer,
    })
}
