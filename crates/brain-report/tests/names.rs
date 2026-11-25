use brain_report::{Door, Event};

#[test]
fn every_name_round_trips_and_matches_its_serde_form() {
    for e in Event::ALL {
        assert_eq!(Event::parse(e.as_str()), Some(*e));
        assert_eq!(
            serde_json::to_string(e).unwrap(),
            format!("\"{}\"", e.as_str())
        );
    }
    for d in Door::ALL {
        assert_eq!(Door::parse(d.as_str()), Some(*d));
        assert_eq!(
            serde_json::to_string(d).unwrap(),
            format!("\"{}\"", d.as_str())
        );
    }
    assert_eq!(Event::parse("note_title"), None);
}
