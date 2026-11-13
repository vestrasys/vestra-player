use super::*;

/// One mpv at a time in this module (as processing.rs's tests).
fn one_mpv_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static MPV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    MPV.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The player's mpv without a window: its own filters in the chain.
fn mpv() -> Mpv {
    Mpv::builder()
        .unwrap()
        .set_property("vo", "null")
        .unwrap()
        .set_property("ao", "null")
        .unwrap()
        .set_property("vf", crate::playback::source_geometry::INITIAL_FILTERS)
        .unwrap()
        .build()
        .unwrap()
}

fn labels(mpv: &Mpv, stage: &str) -> Vec<String> {
    chain(mpv, stage)
        .unwrap()
        .iter()
        .map(|filter| label_of(filter).to_owned())
        .collect()
}

/// A request as the capability forwards it (checked there first, as here;
/// the caller's manifest id added).
fn ask(
    pass: &mut Passthrough,
    mpv: &Mpv,
    owner: u64,
    method: &str,
    mut params: Value,
) -> (Result<Value, RpcError>, Option<Value>) {
    if let Err(error) = crate::capabilities::mpv_check(method, &params) {
        return (Err(error), None);
    }
    params["plugin"] = if owner == 1 { "aa.first" } else { "bb.second" }.into();
    pass.request(mpv, owner, method, &params, Instant::now())
}

fn command(pass: &mut Passthrough, mpv: &Mpv, owner: u64, args: Value) -> Result<Value, RpcError> {
    ask(pass, mpv, owner, "mpv.command", json!({ "args": args })).0
}

/// Two plugins use it: owner 1 is `aa.first`, owner 2 `bb.second` (`ask`).
fn two_plugins() -> Passthrough {
    Passthrough::default()
}

const ORIENTATION: [&str; 2] = ["vestra-orientation", "vestra-orientation-metadata"];

#[test]
fn json_and_nodes_convert_both_ways() {
    let value = json!({"a":[1, 2.5, true, null, "x"], "b":{"c":-3}});
    assert_eq!(from_node(&to_node(&value)), value);
    assert_eq!(to_node(&json!(5)), Node::Int64(5));
    assert_eq!(to_node(&json!(5.0)), Node::Double(5.));
    assert_eq!(to_node(&json!(0.5)), Node::Double(0.5));
    assert_eq!(from_node(&Node::Double(f64::NAN)), Value::Null);
    assert_eq!(from_node(&Node::ByteArray(vec![1, 2])), Value::Null);
    assert_eq!(from_data(&PropertyData::None), Value::Null);
    assert_eq!(from_data(&PropertyData::Flag(true)), json!(true));
}

#[test]
fn only_the_named_labels_entries_may_differ() {
    let filter = |label: &str, name: &str| {
        Node::Map(vec![
            ("label".into(), Node::String(label.into())),
            ("name".into(), Node::String(name.into())),
        ])
    };
    let host = filter("vestra-owned", "lavfi");
    let mine = filter("mine", "hflip");
    let changed = filter("mine", "vflip");
    let other = filter("", "hflip");
    let only_host = std::slice::from_ref(&host);
    let only_mine = std::slice::from_ref(&mine);
    assert!(only_label_changed(only_host, only_host, "mine"));
    assert!(only_label_changed(
        only_host,
        &[host.clone(), mine.clone()],
        "mine"
    ));
    assert!(only_label_changed(
        &[mine.clone(), host.clone()],
        only_host,
        "mine"
    ));
    assert!(only_label_changed(
        &[host.clone(), mine.clone()],
        &[host.clone(), changed],
        "mine"
    ));
    // A filter without the label came, or another one went or changed.
    assert!(!only_label_changed(
        only_host,
        &[host.clone(), mine.clone(), other],
        "mine"
    ));
    assert!(!only_label_changed(
        &[host.clone(), mine.clone()],
        only_mine,
        "mine"
    ));
    assert!(!only_label_changed(
        only_host,
        &[filter("vestra-owned", "null"), mine.clone()],
        "mine"
    ));
    // The same filter twice is two entries.
    assert!(!only_label_changed(
        only_host,
        &[host.clone(), host.clone()],
        "mine"
    ));
}

#[test]
fn commands_and_properties_reach_mpv_and_its_errors_come_back() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let mut pass = two_plugins();
    // A property mpv has, read, written and read again; the write stays.
    let (set, _) = ask(
        &mut pass,
        &mpv,
        1,
        "mpv.set",
        json!({"name":"brightness","value":7}),
    );
    assert_eq!(set.unwrap(), Value::Null);
    let (got, _) = ask(&mut pass, &mpv, 1, "mpv.get", json!({"name":"brightness"}));
    assert_eq!(got.unwrap(), json!({"value":7.0}));
    // A command with typed arguments, as a node array.
    let ran = command(&mut pass, &mpv, 1, json!(["add", "brightness", 2])).unwrap();
    assert_eq!(ran, json!({"result":null}));
    let ran = command(&mut pass, &mpv, 1, json!(["set", "contrast", "3"])).unwrap();
    assert_eq!(ran, json!({"result":null}));
    assert_eq!(mpv.get_property::<i64>("brightness").unwrap(), 9);
    assert_eq!(mpv.get_property::<i64>("contrast").unwrap(), 3);
    // A command that returns something.
    let text = command(
        &mut pass,
        &mpv,
        1,
        json!(["expand-text", "b=${brightness}"]),
    )
    .unwrap();
    assert_eq!(text, json!({"result":"b=9"}));
    // mpv's own errors, in its words.
    let (missing, _) = ask(
        &mut pass,
        &mpv,
        1,
        "mpv.get",
        json!({"name":"no-such-property"}),
    );
    let missing = missing.unwrap_err();
    assert_eq!(
        (missing.code.as_str(), missing.message.as_str()),
        ("backend", "mpv: property not found")
    );
    let unknown = command(&mut pass, &mpv, 1, json!(["no-such-command"])).unwrap_err();
    assert_eq!(unknown.code, "backend");
    assert!(unknown.message.starts_with("mpv: "), "{}", unknown.message);
    let (bad, _) = ask(
        &mut pass,
        &mpv,
        1,
        "mpv.set",
        json!({"name":"brightness","value":"much"}),
    );
    assert_eq!(bad.unwrap_err().code, "backend");
    // What the rules refuse never reaches mpv.
    let refused = command(&mut pass, &mpv, 1, json!(["set", "pause", "yes"])).unwrap_err();
    assert_eq!(refused.code, "refused");
    assert!(!mpv.get_property::<bool>("pause").unwrap());
    // The stop of the plugin rolls no property write back.
    pass.stopped(&mpv, 1);
    assert_eq!(mpv.get_property::<i64>("brightness").unwrap(), 9);
}

#[test]
fn a_plugin_changes_only_its_own_labelled_filters() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let mut pass = two_plugins();
    assert_eq!(labels(&mpv, "vf"), ORIENTATION);
    // `add` appends, `pre` prepends, a label in the chain is replaced in place.
    command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@mine:lavfi=[hflip]"]),
    )
    .unwrap();
    command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@more:lavfi=[vflip]"]),
    )
    .unwrap();
    command(
        &mut pass,
        &mpv,
        2,
        json!(["vf", "pre", "@theirs:lavfi=[null]"]),
    )
    .unwrap();
    let all = ["theirs", ORIENTATION[0], ORIENTATION[1], "mine", "more"];
    assert_eq!(labels(&mpv, "vf"), all);
    command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@mine:lavfi=[null]"]),
    )
    .unwrap();
    assert_eq!(labels(&mpv, "vf"), all);
    assert!(pass.holds(1) && pass.holds(2) && !pass.holds(3));
    // Another plugin's label.
    for args in [
        json!(["vf", "add", "@mine:lavfi=[vflip]"]),
        json!(["vf", "remove", "@mine"]),
        json!(["vf", "toggle", "@mine:lavfi=[null]"]),
        json!(["vf-command", "mine", "x", "1"]),
    ] {
        let error = command(&mut pass, &mpv, 2, args).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("conflict", "the filter label @mine belongs to aa.first")
        );
    }
    // The same label in the other chain is another label.
    command(
        &mut pass,
        &mpv,
        2,
        json!(["af", "add", "@mine:lavfi=[anull]"]),
    )
    .unwrap();
    assert_eq!(labels(&mpv, "af"), ["mine"]);
    // The player's own labels, and the chain as a whole, by any spelling.
    for args in [
        json!(["vf", "remove", "@vestra-orientation"]),
        json!(["vf", "add", "@vestra-orientation:lavfi=[null]"]),
        json!(["vf", "set", "@mine:lavfi=[null]"]),
        json!(["vf", "clr", ""]),
        json!(["vf", "add", "hflip"]),
        json!(["set", "vf", ""]),
        json!(["change-list", "vf", "clr", ""]),
        json!(["vf-command", "vestra-owned", "x", "1"]),
    ] {
        let error = command(&mut pass, &mpv, 1, args.clone()).unwrap_err();
        assert_eq!(error.code, "refused", "{args}");
    }
    let (set, _) = ask(
        &mut pass,
        &mpv,
        1,
        "mpv.set",
        json!({"name":"vf","value":""}),
    );
    assert_eq!(set.unwrap_err().code, "refused");
    assert_eq!(labels(&mpv, "vf"), all);
    // A second filter behind the labelled one: the chain is as it was.
    let before = chain(&mpv, "vf").unwrap();
    let hidden = command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@new:lavfi=[hflip],vflip"]),
    )
    .unwrap_err();
    assert_eq!(
        (hidden.code.as_str(), hidden.message.as_str()),
        (
            "refused",
            "mpv command \"vf\" is refused: a plugin changes only its own labelled filters (vf/af add, pre, toggle or remove of @label:…, a label not starting with vestra-)"
        )
    );
    assert_eq!(chain(&mpv, "vf").unwrap(), before);
    // The label it did not get is nobody's.
    command(
        &mut pass,
        &mpv,
        2,
        json!(["vf", "add", "@new:lavfi=[null]"]),
    )
    .unwrap();
    command(&mut pass, &mpv, 2, json!(["vf", "remove", "@new"])).unwrap();
    // Removed or toggled off, a label is free again.
    command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "toggle", "@more:lavfi=[vflip]"]),
    )
    .unwrap();
    command(&mut pass, &mpv, 1, json!(["vf", "remove", "@mine"])).unwrap();
    assert_eq!(
        labels(&mpv, "vf"),
        ["theirs", ORIENTATION[0], ORIENTATION[1]]
    );
    command(
        &mut pass,
        &mpv,
        2,
        json!(["vf", "add", "@mine:lavfi=[null]"]),
    )
    .unwrap();
    assert!(!pass.holds(1));
    // mpv refuses a filter it does not know: nothing changes, nothing is owned.
    let unknown = command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@odd:no-such-filter"]),
    )
    .unwrap_err();
    assert_eq!(unknown.code, "backend");
    assert!(!pass.holds(1));
    // A stopped plugin's filters leave both chains; the others stay.
    command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@last:lavfi=[null]"]),
    )
    .unwrap();
    pass.stopped(&mpv, 2);
    assert_eq!(labels(&mpv, "vf"), [ORIENTATION[0], ORIENTATION[1], "last"]);
    assert!(labels(&mpv, "af").is_empty());
    assert!(pass.holds(1) && !pass.holds(2));
    pass.stopped(&mpv, 1);
    assert_eq!(labels(&mpv, "vf"), ORIENTATION);
}

#[test]
fn an_observer_gets_the_value_at_once_then_changes_at_most_every_interval() {
    let _one = one_mpv_at_a_time();
    let mut mpv = mpv();
    let mut pass = two_plugins();
    mpv.set_property("brightness", 1_i64).unwrap();
    let start = Instant::now();
    let (answer, first) =
        pass.request(&mpv, 1, "mpv.observe", &json!({"name":"brightness"}), start);
    assert_eq!(answer.unwrap(), Value::Null);
    assert_eq!(first, Some(json!({"name":"brightness","value":1.0})));
    // A property without a value now: null.
    let (_, none) = pass.request(&mpv, 1, "mpv.observe", &json!({"name":"time-pos"}), start);
    assert_eq!(none, Some(json!({"name":"time-pos","value":null})));
    assert!(pass.holds(1) && !pass.holds(2));
    // mpv's own first notification repeats the value: not sent again.
    let drain = |mpv: &mut Mpv, pass: &mut Passthrough| {
        let mut ours = 0;
        while let Some(event) = mpv.wait_event(0.2) {
            if let rsmpv::Event::PropertyChange { userdata, data, .. } = &event
                && pass.changed(*userdata, data)
            {
                ours += 1;
            }
        }
        ours
    };
    assert!(drain(&mut mpv, &mut pass) >= 1);
    assert!(pass.due(start + INTERVAL).is_empty());
    assert_eq!(pass.due_in(start + INTERVAL), None);
    // Changes within the interval: the newest value, once it is over, to
    // the observer only.
    mpv.set_property("brightness", 2_i64).unwrap();
    mpv.set_property("brightness", 3_i64).unwrap();
    assert!(drain(&mut mpv, &mut pass) >= 1);
    assert!(pass.due(start + INTERVAL / 2).is_empty());
    assert_eq!(pass.due_in(start + INTERVAL / 2), Some(INTERVAL / 2));
    let later = start + INTERVAL;
    assert_eq!(
        pass.due(later),
        [(1, json!({"name":"brightness","value":3.0}))]
    );
    assert!(pass.due(later).is_empty());
    // A second observer of the same property shares mpv's observation and
    // gets the current value first; both get the next change.
    let (_, second) = pass.request(&mpv, 2, "mpv.observe", &json!({"name":"brightness"}), later);
    assert_eq!(second, Some(json!({"name":"brightness","value":3.0})));
    mpv.set_property("brightness", 4_i64).unwrap();
    assert!(drain(&mut mpv, &mut pass) >= 1);
    let next = later + INTERVAL;
    assert_eq!(
        pass.due(next),
        [
            (1, json!({"name":"brightness","value":4.0})),
            (2, json!({"name":"brightness","value":4.0}))
        ]
    );
    // A value that went back to the one it got last is nothing new.
    mpv.set_property("brightness", 5_i64).unwrap();
    mpv.set_property("brightness", 4_i64).unwrap();
    drain(&mut mpv, &mut pass);
    assert!(pass.due(next + INTERVAL).is_empty());
    // Unobserved, or its plugin stopped: nothing more for it.
    let _ = pass.request(
        &mpv,
        1,
        "mpv.unobserve",
        &json!({"name":"brightness"}),
        next,
    );
    mpv.set_property("brightness", 6_i64).unwrap();
    drain(&mut mpv, &mut pass);
    assert_eq!(
        pass.due(next + 2 * INTERVAL),
        [(2, json!({"name":"brightness","value":6.0}))]
    );
    pass.stopped(&mpv, 2);
    pass.stopped(&mpv, 1);
    assert!(!pass.holds(1) && !pass.holds(2));
    mpv.set_property("brightness", 7_i64).unwrap();
    assert_eq!(drain(&mut mpv, &mut pass), 0);
    assert!(pass.due(next + 3 * INTERVAL).is_empty());
}

#[test]
fn a_plugin_observes_at_most_the_limit_of_properties() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let mut pass = two_plugins();
    let now = Instant::now();
    let observe = |pass: &mut Passthrough, owner: u64, name: String| {
        pass.request(&mpv, owner, "mpv.observe", &json!({ "name": name }), now)
            .0
    };
    for n in 0..mpv_rules::OBSERVED_MAX {
        assert!(
            observe(&mut pass, 1, format!("user-data/p{n}")).is_ok(),
            "{n}"
        );
    }
    let over = observe(&mut pass, 1, "user-data/more".into()).unwrap_err();
    assert_eq!(
        (over.code.as_str(), over.message.as_str()),
        ("limit", "a plugin observes at most 64 mpv properties")
    );
    // One it observes already is no new one; another plugin has its own count.
    assert!(observe(&mut pass, 1, "user-data/p0".into()).is_ok());
    assert!(observe(&mut pass, 2, "user-data/more".into()).is_ok());
    // One less, one more.
    let _ = pass.request(
        &mpv,
        1,
        "mpv.unobserve",
        &json!({"name":"user-data/p0"}),
        now,
    );
    assert!(observe(&mut pass, 1, "user-data/more".into()).is_ok());
}

/// What the worker's loop asks before it does anything for the
/// pass-through: nothing is observed and nothing waits until a plugin
/// uses it, and again once it let go.
#[test]
fn nothing_is_watched_or_waiting_until_a_plugin_observes() {
    let _one = one_mpv_at_a_time();
    let mut mpv = mpv();
    let mut pass = two_plugins();
    let start = Instant::now();
    let idle = |pass: &Passthrough| {
        !pass.watching() && !pass.waiting() && pass.due_in(start).is_none() && !pass.holds(1)
    };
    assert!(idle(&pass));
    // Requests that hold nothing leave it so: commands, reads, writes, an
    // unobserve of nothing, the stop of a plugin that holds nothing.
    command(&mut pass, &mpv, 1, json!(["set", "brightness", "3"])).unwrap();
    ask(&mut pass, &mpv, 1, "mpv.get", json!({"name":"brightness"}))
        .0
        .unwrap();
    ask(
        &mut pass,
        &mpv,
        1,
        "mpv.set",
        json!({"name":"contrast","value":1}),
    )
    .0
    .unwrap();
    ask(
        &mut pass,
        &mpv,
        1,
        "mpv.unobserve",
        json!({"name":"brightness"}),
    )
    .0
    .unwrap();
    pass.stopped(&mpv, 1);
    assert!(pass.due(start).is_empty());
    assert!(idle(&pass));
    // A property change of the worker's own observations is not its.
    assert!(!pass.changed(1, &PropertyData::Flag(true)));
    assert!(!pass.changed(2, &PropertyData::Flag(true)));
    assert!(idle(&pass));
    // Observed: watched; a value waits only between its change and its push.
    let _ = pass.request(&mpv, 1, "mpv.observe", &json!({"name":"brightness"}), start);
    assert!(pass.watching() && !pass.waiting() && pass.holds(1));
    mpv.set_property("brightness", 9_i64).unwrap();
    while let Some(event) = mpv.wait_event(0.2) {
        if let rsmpv::Event::PropertyChange { userdata, data, .. } = &event {
            pass.changed(*userdata, data);
        }
    }
    assert!(pass.waiting());
    assert_eq!(pass.due_in(start + INTERVAL / 4), Some(INTERVAL * 3 / 4));
    assert_eq!(pass.due(start + INTERVAL).len(), 1);
    assert!(!pass.waiting() && pass.due_in(start + INTERVAL).is_none());
    // Let go: as at the start.
    pass.stopped(&mpv, 1);
    assert!(idle(&pass));
    // A labelled filter is held without anything watched.
    command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@mine:lavfi=[null]"]),
    )
    .unwrap();
    assert!(pass.holds(1) && !pass.watching() && !pass.waiting());
    pass.stopped(&mpv, 1);
    assert!(idle(&pass));
    assert_eq!(labels(&mpv, "vf"), ORIENTATION);
}

/// A labelled filter that left its chain behind the pass-through's back
/// is nobody's again: at the next filter command of any plugin, and when
/// the worker is asked what a plugin holds.
#[test]
fn a_label_whose_filter_left_the_chain_is_let_go() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let mut pass = two_plugins();
    command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@mine:lavfi=[null]"]),
    )
    .unwrap();
    command(
        &mut pass,
        &mpv,
        1,
        json!(["af", "add", "@loud:lavfi=[anull]"]),
    )
    .unwrap();
    assert!(pass.holds(1) && pass.labelled());
    // Gone from the video chain without a request of its plugin.
    mpv.command(&["vf", "remove", "@mine"]).unwrap();
    assert!(pass.holds(1), "not looked at yet");
    // Another plugin may take the label: no conflict with a holder of nothing.
    command(
        &mut pass,
        &mpv,
        2,
        json!(["vf", "add", "@mine:lavfi=[null]"]),
    )
    .unwrap();
    let error = command(&mut pass, &mpv, 1, json!(["vf", "remove", "@mine"])).unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        ("conflict", "the filter label @mine belongs to bb.second")
    );
    // The audio label too, when the worker asks (`owner.resources`).
    mpv.command(&["af", "remove", "@loud"]).unwrap();
    pass.refresh(&mpv);
    assert!(!pass.holds(1) && pass.holds(2));
    // A filter message to a label that is gone finds no holder either.
    mpv.command(&["vf", "remove", "@mine"]).unwrap();
    let gone = command(&mut pass, &mpv, 1, json!(["vf-command", "mine", "x", "1"])).unwrap_err();
    assert_eq!(gone.code, "backend");
    assert!(!pass.holds(2) && !pass.labelled());
}

/// mpv's `all` addresses every filter of a chain, the player's too: it is
/// no plugin's label, in a filter message or as a filter's own.
#[test]
fn the_label_all_and_underscore_names_reach_no_filter_of_the_players() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let mut pass = two_plugins();
    let before = chain(&mpv, "vf").unwrap();
    for args in [
        json!(["vf-command", "all", "angle", "1"]),
        json!(["af-command", "all", "volume", "0"]),
        json!(["vf_command", "all", "angle", "1"]),
        json!(["vf_command", "vestra-orientation", "angle", "1"]),
        json!(["vf", "add", "@all:lavfi=[null]"]),
        json!(["af", "pre", "@all"]),
        json!(["change_list", "vf", "clr", ""]),
        json!(["cycle_values", "pause", "yes", "no"]),
    ] {
        let error = command(&mut pass, &mpv, 1, args.clone()).unwrap_err();
        assert_eq!(error.code, "refused", "{args}");
    }
    assert_eq!(chain(&mpv, "vf").unwrap(), before);
    assert!(!mpv.get_property::<bool>("pause").unwrap());
    // mpv itself reads `_` as `-` in a command's name (why the rules do):
    // a command that is not refused runs under either spelling.
    command(
        &mut pass,
        &mpv,
        1,
        json!(["cycle_values", "brightness", "1", "2"]),
    )
    .unwrap();
    // A label in another case is a label like any other.
    command(
        &mut pass,
        &mpv,
        1,
        json!(["vf", "add", "@ALL:lavfi=[null]"]),
    )
    .unwrap();
    assert!(pass.holds(1));
    pass.stopped(&mpv, 1);
    assert_eq!(chain(&mpv, "vf").unwrap(), before);
}
