//! The `when` language of SDK 1.10.0 (events-and-triggers.md §4.3): the
//! tests of the atoms it adds; those of the first seven are in
//! conditions.rs.
use super::*;
use serde_json::json;

#[test]
fn the_new_atoms_read_the_wire_state_in_the_sdk_names() {
    let empty = Facts::new(&Value::Null, &Value::Null, "vestra", "system");
    assert_eq!((empty.duration, empty.volume, empty.rate), (None, 100., 1.));
    assert_eq!((empty.audio_track, empty.audio_device), (None, None));
    assert!(!empty.looping && !empty.maximized && !empty.pip && !empty.muted);
    let full = Facts::new(
        &json!({"phase":"ready","duration":61.5,"rate":2,
            "output":{"volume":40,"muted":true},
            "audioTracks":[{"trackId":1,"selected":false},{"trackId":3,"selected":true}],
            "audioOutput":{"requested":null,"active":"wasapi/x"},
            "loop":{"a":1,"b":2,"count":null}}),
        &json!({"maximized":true,"pictureInPicture":{"supported":true,"active":true}}),
        "vestra",
        "system",
    );
    assert_eq!(full.duration, Some(61.5));
    assert_eq!((full.volume, full.rate, full.muted), (40., 2., true));
    assert_eq!(full.audio_track, Some(3));
    assert_eq!(full.audio_device.as_deref(), Some("wasapi/x"));
    assert!(full.looping && full.maximized && full.pip);
}

#[test]
fn malformed_new_atoms_are_refused() {
    for bad in [
        json!({"duration":{}}),
        json!({"duration":5}),
        json!({"volume":{"min":2,"max":1}}),
        json!({"rate":{"min":1,"max":1}}),
        json!({"time":{"at":3}}),
        json!({"audioTrack":"2"}),
        json!({"subtitleTrack":2}),
        json!({"audioDevice":""}),
        json!({"muted":"yes"}),
        json!({"position":{"ranges":[[5,2]]}}),
        json!({"position":{"ranges":[[-1,2]]}}),
        json!({"position":{"ranges":[[1,2,3]]}}),
        json!({"position":{"ranges":[[1,2]],"lead":-1}}),
        json!({"position":{"ranges":[[1,2]],"source":3}}),
        json!({"position":{"lead":1}}),
        json!({"all":[{"position":{"ranges":[[1,2]]}},{"position":{"ranges":[[3,4]]}}]}),
    ] {
        assert!(Condition::parse(&bad).is_err(), "{bad}");
    }
}
/// A position list is one node however long; 1000 ranges (the timeline's
/// number), more is `limit`.
#[test]
fn a_position_list_is_one_node_and_holds_1000_ranges() {
    let ranges = |n: usize| {
        (0..n)
            .map(|i| json!([i as f64, i as f64 + 0.5]))
            .collect::<Vec<_>>()
    };
    let list = json!({"all":[{"phase":["ready"]},{"position":{"ranges":ranges(MAX_RANGES)}}]});
    assert!(Condition::parse(&list).is_ok());
    let refusal =
        Condition::parse_coded(&json!({"position":{"ranges":ranges(MAX_RANGES + 1)}})).unwrap_err();
    assert_eq!(refusal.code, "limit");
    assert_eq!(MAX_RANGES, 1000);
}

/// The new atoms work in a command's state; position and time do not.
#[test]
fn a_command_takes_the_new_atoms_but_not_position_or_time() {
    assert!(
        Conditions::parse(&json!({"enabled":{"all":[{"duration":{"min":1}},{"muted":false}]}}))
            .unwrap()
            .is_some()
    );
    for bad in [
        json!({"enabled":{"time":{"min":1}}}),
        json!({"checked":{"not":{"position":{"ranges":[[1,2]]}}}}),
    ] {
        let error = Conditions::parse(&bad).unwrap_err();
        assert!(error.contains("for a trigger"), "{error}");
    }
}

#[test]
fn a_range_list_matches_the_earliest_start_and_names_its_boundaries() {
    let list = RangeList::new(
        vec![
            (10., Some(20.)),
            (15., Some(30.)),
            (40., None),
            (50., Some(51.)),
        ],
        0.,
    );
    assert_eq!(list.matched(9.99), None);
    assert_eq!(list.matched(10.), Some(0));
    assert_eq!(list.matched(16.), Some(0));
    assert_eq!(list.matched(20.), Some(1));
    assert_eq!(list.matched(30.), None);
    assert_eq!(list.matched(45.), Some(2));
    // Open at the end: every later position is in it.
    assert_eq!(list.matched(50.5), Some(2));
    assert_eq!(list.next_boundary(0.), Some(10.));
    assert_eq!(list.next_boundary(10.), Some(15.));
    assert_eq!(list.next_boundary(16.), Some(20.));
    assert_eq!(list.next_boundary(30.), Some(40.));
    assert_eq!(list.next_boundary(51.), None);
    assert_eq!(list.boundaries_between(9., 20.), [10., 15., 20.]);
    assert_eq!(list.boundaries_between(20., 20.), Vec::<f64>::new());
    // A lead moves every start earlier; the ends stay.
    let lead = RangeList::new(vec![(10., Some(20.))], 2.);
    assert_eq!(lead.matched(8.), Some(0));
    assert_eq!(lead.matched(7.9), None);
    assert_eq!(lead.next_boundary(0.), Some(8.));
    assert_eq!(lead.range(0), json!({"start":10.,"end":20.,"index":0}));
}

/// The facts of one row of the agreement table: its defaults, overridden.
fn table_facts(defaults: &Value, row: &Value) -> (Facts, Moment, Option<u64>) {
    let mut f = defaults.as_object().unwrap().clone();
    for (key, value) in row.as_object().unwrap() {
        f.insert(key.clone(), value.clone());
    }
    let text = |key: &str| f[key].as_str().map(str::to_owned);
    let facts = Facts {
        phase: text("phase").unwrap(),
        paused: f["paused"] == true,
        kind: text("kind").unwrap(),
        can: f["can"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect(),
        fullscreen: f["fullscreen"] == true,
        theme: text("theme").unwrap(),
        language: text("language").unwrap(),
        duration: f["duration"].as_f64(),
        volume: f["volume"].as_f64().unwrap(),
        rate: f["rate"].as_f64().unwrap(),
        muted: f["muted"] == true,
        audio_track: f["audioTrack"].as_i64(),
        audio_device: text("audioDevice"),
        looping: f["looping"] == true,
        maximized: f["maximized"] == true,
        pip: f["pip"] == true,
        subtitle_track: text("subtitleTrack"),
    };
    let moment = Moment {
        source: f["sourceId"].as_u64().unwrap(),
        position: f["position"].as_f64(),
        time: f["time"].as_f64().unwrap(),
    };
    (facts, moment, f["bound"].as_u64())
}

/// The SDK–host agreement table (design §4.11): every row holds here as
/// the SDK's `evaluate` says (its copy is checked equal by
/// src/tests/sdk/host-agreement.test.ts).
#[test]
fn the_agreement_table_holds() {
    let table: Value = serde_json::from_str(include_str!("../condition-table.json")).unwrap();
    let rows = table["rows"].as_array().unwrap();
    assert!(rows.len() >= 40);
    for row in rows {
        let condition = Condition::parse(&row["when"]).unwrap_or_else(|e| panic!("{row}: {e}"));
        let (facts, moment, bound) = table_facts(&table["defaults"], &row["facts"]);
        let truth = condition.eval(&facts, Some(&moment), bound);
        assert_eq!(truth.holds, row["holds"] == true, "{row}");
        assert_eq!(
            truth.range,
            row["range"].as_u64().map(|r| r as usize),
            "{row}"
        );
    }
}

#[test]
fn groups_and_time_bounds() {
    let parse = |value: Value| Condition::parse(&value).unwrap();
    assert_eq!(parse(json!({"paused":true})).groups(), groups::SOURCE);
    assert_eq!(
        parse(json!({"all":[{"fullscreen":true},{"theme":"vestra"}]})).groups(),
        groups::WINDOW | groups::APPEARANCE
    );
    assert_eq!(
        parse(json!({"not":{"position":{"ranges":[]}}})).groups(),
        groups::POSITION | groups::SOURCE
    );
    assert_eq!(
        parse(json!({"subtitleTrack":"embedded:1"})).groups(),
        groups::SUBTITLES
    );
    let time = parse(json!({"any":[{"time":{"min":100,"max":200}},{"time":{"min":150}}]}));
    assert_eq!(time.groups(), groups::TIME);
    assert_eq!(time.next_time(0.), Some(100.));
    assert_eq!(time.next_time(120.), Some(150.));
    assert_eq!(time.next_time(160.), Some(200.));
    assert_eq!(time.next_time(200.), None);
}
