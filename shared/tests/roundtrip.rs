use fleetview_shared::crypto::{cert_fingerprint, secrets_match, MasterIdentity};
use fleetview_shared::discovery::DiscoveryBeacon;
use fleetview_shared::protocol::{AgentMsg, MasterMsg};
use fleetview_shared::PROTOCOL_VERSION;

#[test]
fn identity_sign_and_verify() {
    let id = MasterIdentity::generate();
    let pk = id.public_key_b64().unwrap();
    let msg = b"hello fleet";
    let sig = id.sign(msg).unwrap();
    assert!(fleetview_shared::crypto::verify_signature(&pk, msg, &sig));
    assert!(!fleetview_shared::crypto::verify_signature(
        &pk,
        b"tampered",
        &sig
    ));
}

#[test]
fn beacon_signature_roundtrip() {
    let id = MasterIdentity::generate();
    let bytes = DiscoveryBeacon::signing_bytes(PROTOCOL_VERSION, 48620, "abcd", "Master", 123);
    let sig = id.sign(&bytes).unwrap();
    let beacon = DiscoveryBeacon {
        protocol: PROTOCOL_VERSION,
        control_port: 48620,
        cert_fingerprint: "abcd".into(),
        master_name: "Master".into(),
        issued_at_ms: 123,
        signature: sig,
    };
    let pk = id.public_key_b64().unwrap();
    assert!(fleetview_shared::crypto::verify_signature(
        &pk,
        &beacon.canonical_bytes(),
        &beacon.signature
    ));
}

#[test]
fn fingerprint_is_stable_hex() {
    let fp = cert_fingerprint(b"some-der-bytes");
    assert_eq!(fp.len(), 64);
    assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(secrets_match(&fp, &cert_fingerprint(b"some-der-bytes")));
}

#[test]
fn protocol_messagepack_roundtrip() {
    let msg = MasterMsg::FileChunk {
        transfer_id: "t1".into(),
        data: vec![0u8, 1, 2, 3, 255],
    };
    let bytes = rmp_serde::to_vec_named(&msg).unwrap();
    let back: MasterMsg = rmp_serde::from_slice(&bytes).unwrap();
    match back {
        MasterMsg::FileChunk { transfer_id, data } => {
            assert_eq!(transfer_id, "t1");
            assert_eq!(data, vec![0u8, 1, 2, 3, 255]);
        }
        _ => panic!("wrong variant"),
    }

    let frame = AgentMsg::ScreenFrame {
        command_id: Some("c".into()),
        width: 4,
        height: 2,
        jpeg: vec![9, 8, 7],
    };
    let bytes = rmp_serde::to_vec_named(&frame).unwrap();
    let _back: AgentMsg = rmp_serde::from_slice(&bytes).unwrap();
}
