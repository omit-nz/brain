use brain_index::activity::Activity;
use brain_report::{Door, Event};
use jiff::civil::date;

fn store() -> (tempfile::TempDir, Activity) {
    let dir = tempfile::tempdir().unwrap();
    let a = Activity::open(&dir.path().join("activity.db")).unwrap();
    (dir, a)
}

#[test]
fn counts_add_up_by_day_kind_and_door() {
    let (_dir, a) = store();
    let (d1, d2) = (date(2026, 11, 25), date(2026, 11, 26));
    assert_eq!(a.first_day().unwrap(), None);
    a.count(d1, Event::Question, Some(Door::Cli)).unwrap();
    a.count(d1, Event::Question, Some(Door::Cli)).unwrap();
    a.count(d1, Event::Question, Some(Door::ClaudeCode))
        .unwrap();
    a.count(d1, Event::ErrorIndex, None).unwrap();
    a.count(d2, Event::Question, Some(Door::Cli)).unwrap();
    let mut day1 = a.day(d1).unwrap();
    day1.sort();
    assert_eq!(
        day1,
        vec![
            (Event::Question, Some(Door::ClaudeCode), 1),
            (Event::Question, Some(Door::Cli), 2),
            (Event::ErrorIndex, None, 1),
        ]
    );
    assert_eq!(
        a.day(d2).unwrap(),
        vec![(Event::Question, Some(Door::Cli), 1)]
    );
    assert_eq!(a.first_day().unwrap(), Some(d1));
}

#[test]
fn the_activity_store_is_private() {
    let (dir, a) = store();
    a.count(date(2026, 11, 25), Event::Question, None).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("activity.db"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
    #[cfg(windows)]
    assert!(dir.path().join("activity.db").is_file());
}

#[test]
fn a_gap_is_logged_and_filled_once() {
    let (_dir, a) = store();
    let t1: jiff::Timestamp = "2026-11-25T01:00:00Z".parse().unwrap();
    let t2: jiff::Timestamp = "2026-11-25T02:00:00Z".parse().unwrap();
    a.log_gap(t1, Door::Cli, "who signs the lease", Some("clients"))
        .unwrap();
    a.log_gap(t2, Door::ClaudeCode, "what is the wifi password", None)
        .unwrap();
    let open = a.open_gaps(10).unwrap();
    assert_eq!(open.len(), 2);
    assert_eq!(open[0].query, "what is the wifi password", "newest first");
    assert_eq!(open[1].area.as_deref(), Some("clients"));
    assert_eq!(open[1].door, Door::Cli);
    a.fill_gap(open[1].id, t2).unwrap();
    a.fill_gap(open[1].id, t2).unwrap();
    let still = a.open_gaps(10).unwrap();
    assert_eq!(still.len(), 1);
    assert_eq!(a.filled_since(t1).unwrap(), 1);
}
