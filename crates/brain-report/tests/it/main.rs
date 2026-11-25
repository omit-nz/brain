//! The report's contract: what `sign` makes, `accept` takes, and every way `accept` refuses.

use brain_report::{Count, Door, Event, Refused, Report, accept, sign};
use ed25519_dalek::SigningKey;
use jiff::civil::{Date, date};

const TODAY: Date = date(2026, 11, 25);

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn report(day: Date, counts: Vec<Count>) -> Report {
    Report::new(&key(), day, "0.1.1", 1, counts)
}

fn one() -> Vec<Count> {
    vec![Count {
        event: Event::Question,
        door: Some(Door::Cli),
        n: 3,
    }]
}

#[test]
fn a_signed_report_is_accepted() {
    let r = report(date(2026, 11, 24), one());
    let (body, signature) = sign(&r, &key());
    assert_eq!(accept(&body, &signature, TODAY), Ok(r));
}

#[test]
fn accept_refuses_a_changed_byte() {
    let (mut body, signature) = sign(&report(date(2026, 11, 24), one()), &key());
    let at = body.windows(3).position(|w| w == b":3}").unwrap() + 1;
    body[at] = b'4';
    assert_eq!(accept(&body, &signature, TODAY), Err(Refused::BadSignature));
    let other = SigningKey::from_bytes(&[8; 32]);
    let (body, _) = sign(&report(date(2026, 11, 24), one()), &key());
    let (_, wrong) = sign(&report(date(2026, 11, 24), one()), &other);
    assert_eq!(accept(&body, &wrong, TODAY), Err(Refused::BadSignature));
    assert_eq!(
        accept(&body, "not base64!", TODAY),
        Err(Refused::BadSignature)
    );
}

#[test]
fn accept_refuses_unknown_fields() {
    let r = report(date(2026, 11, 24), one());
    let mut v = serde_json::to_value(&r).unwrap();
    v["note"] = "the gate code".into();
    let body = serde_json::to_vec(&v).unwrap();
    let signature = sign_bytes(&body);
    assert_eq!(accept(&body, &signature, TODAY), Err(Refused::Malformed));
    let mut v = serde_json::to_value(&r).unwrap();
    v["counts"][0]["query"] = "the gate code".into();
    let body = serde_json::to_vec(&v).unwrap();
    assert_eq!(
        accept(&body, &sign_bytes(&body), TODAY),
        Err(Refused::Malformed)
    );
    let mut v = serde_json::to_value(&r).unwrap();
    v["v"] = 2.into();
    let body = serde_json::to_vec(&v).unwrap();
    assert_eq!(
        accept(&body, &sign_bytes(&body), TODAY),
        Err(Refused::Malformed)
    );
    let mut v = serde_json::to_value(&r).unwrap();
    v["workspace"] = "AAAA".into();
    let body = serde_json::to_vec(&v).unwrap();
    assert_eq!(
        accept(&body, &sign_bytes(&body), TODAY),
        Err(Refused::BadKey)
    );
}

/// A signature by the test key over `body` exactly, as `sign` makes one.
fn sign_bytes(body: &[u8]) -> String {
    use base64::Engine;
    use ed25519_dalek::Signer;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key().sign(body).to_bytes())
}

#[test]
fn accept_refuses_a_day_out_of_range() {
    for (day, ok) in [
        (date(2026, 11, 25), true),
        (date(2026, 11, 26), false),
        (date(2026, 10, 26), true),
        (date(2026, 10, 25), false),
    ] {
        let (body, signature) = sign(&report(day, one()), &key());
        let got = accept(&body, &signature, TODAY);
        if ok {
            assert!(got.is_ok(), "{day}: {got:?}");
        } else {
            assert_eq!(got, Err(Refused::DayOutOfRange), "{day}");
        }
    }
}

#[test]
fn accept_refuses_a_body_over_64_kib() {
    let (mut body, signature) = sign(&report(date(2026, 11, 24), one()), &key());
    body.resize(65_537, b' ');
    assert_eq!(accept(&body, &signature, TODAY), Err(Refused::TooLarge));
}

#[test]
fn accept_refuses_more_than_500_counts() {
    let count = |n| Count {
        event: Event::Question,
        door: None,
        n,
    };
    let day = date(2026, 11, 24);
    // 500 is under the cap; repeated pairs then make it malformed, never too many.
    let (body, signature) = sign(&report(day, vec![count(1); 500]), &key());
    assert_eq!(accept(&body, &signature, TODAY), Err(Refused::Malformed));
    let (body, signature) = sign(&report(day, vec![count(1); 501]), &key());
    assert_eq!(accept(&body, &signature, TODAY), Err(Refused::TooMany));
    let (body, signature) = sign(&report(day, vec![count(10_000_000)]), &key());
    assert!(accept(&body, &signature, TODAY).is_ok());
    let (body, signature) = sign(&report(day, vec![count(10_000_001)]), &key());
    assert_eq!(accept(&body, &signature, TODAY), Err(Refused::TooMany));
}

#[test]
fn accept_refuses_a_weak_key() {
    use base64::Engine;
    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    // The identity point as the key and a signature R = identity, s = 0: plain verify takes it
    // for any message, verify_strict does not.
    let mut identity = [0u8; 32];
    identity[0] = 1;
    let mut v = serde_json::to_value(report(date(2026, 11, 24), one())).unwrap();
    v["workspace"] = b64(&identity).into();
    let body = serde_json::to_vec(&v).unwrap();
    let mut signature = [0u8; 64];
    signature[0] = 1;
    assert_eq!(
        accept(&body, &b64(&signature), TODAY),
        Err(Refused::BadSignature)
    );
}

#[test]
fn a_body_of_exactly_64_kib_is_accepted() {
    let (body, _) = sign(&report(date(2026, 11, 24), one()), &key());
    let mut body = body;
    body.resize(65_536, b' ');
    let signature = sign_bytes(&body);
    assert!(accept(&body, &signature, TODAY).is_ok());
}

/// `report` with `edit` applied to its JSON, signed by the test key.
fn edited(edit: impl FnOnce(&mut serde_json::Value)) -> (Vec<u8>, String) {
    let mut v = serde_json::to_value(report(date(2026, 11, 24), one())).unwrap();
    edit(&mut v);
    let body = serde_json::to_vec(&v).unwrap();
    let signature = sign_bytes(&body);
    (body, signature)
}

#[test]
fn accept_refuses_what_one_install_cannot_report() {
    type Edit = Box<dyn FnOnce(&mut serde_json::Value)>;
    let cases: [(&str, Edit); 4] = [
        ("people over 1", Box::new(|v| v["people"] = 2.into())),
        (
            "a version over 64 characters",
            Box::new(|v| v["version"] = "9".repeat(65).into()),
        ),
        (
            "one event and door twice",
            Box::new(|v| {
                let c = v["counts"][0].clone();
                v["counts"].as_array_mut().unwrap().push(c);
            }),
        ),
        (
            "notes_due through a door",
            Box::new(|v| {
                v["counts"][0]["event"] = "notes_due".into();
            }),
        ),
    ];
    for (what, edit) in cases {
        let (body, signature) = edited(edit);
        assert_eq!(
            accept(&body, &signature, TODAY),
            Err(Refused::Malformed),
            "{what}"
        );
    }
    let (body, signature) = edited(|v| v["version"] = "9".repeat(64).into());
    assert!(accept(&body, &signature, TODAY).is_ok());
}
