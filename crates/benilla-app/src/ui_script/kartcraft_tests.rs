//! World of Kartcraft's race windows (`assets/ui/KartcraftFrames.xml`) on the production load:
//! the Race Finder queues and answers over the addon channel, the ready check is the stock popup,
//! and the kart's race and score events reach their frames.

use benilla_ui::script::{ScriptValue, UiScript};

use crate::local_state::test_env::ENV_LOCK;

/// The production load with the layer, and no third-party addon.
fn loaded() -> UiScript {
    let (s, failures) = super::layer_tests::production_load_with("kartcraft", false, "", |_| {});
    assert!(failures.is_empty(), "load failures: {failures:#?}");
    s
}

/// The kart's courses as the client sends them: the three of today, then `n - 3` more.
fn courses(s: &mut UiScript, n: usize) {
    let mut list = vec![
        "sprint\tNorthshire Sprint".to_string(),
        "elwynn\tElwynn Loop".to_string(),
        "stormwind\tRoad to Stormwind".to_string(),
    ];
    list.extend((4..=n).map(|i| format!("c{i}\tCourse {i}")));
    s.fire_event(
        "BENILLA_KART_COURSES",
        vec![ScriptValue::Str(list.join("\n"))],
    );
    s.tick(0.0);
}

/// The addon lines the VM sent since the last call, as `prefix\tmessage`.
fn sent(s: &mut UiScript) -> Vec<String> {
    s.take_addon_sends().into_iter().map(|a| a.text).collect()
}

/// The server's line to us, as the client hands it to Lua.
fn from_server(s: &mut UiScript, message: &str) {
    s.fire_event(
        "CHAT_MSG_ADDON",
        vec![
            ScriptValue::Str("KARTCRAFT".into()),
            ScriptValue::Str(message.into()),
            ScriptValue::Str("PARTY".into()),
            ScriptValue::Str("Probesix".into()),
        ],
    );
    s.tick(0.0);
}

#[test]
fn the_finder_queues_keeps_its_place_and_answers_the_ready_check() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);

    // I opens it, in the stock widgets, with "Any course" ticked and the courses named.
    assert!(!s
        .eval::<bool>("return BenillaKartFinderFrame:IsVisible() == 1")
        .unwrap());
    s.fire_event("BENILLA_KART_FINDER_TOGGLE", Vec::new());
    s.tick(0.0);
    assert!(s
        .eval::<bool>("return BenillaKartFinderFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartFinderFrameRow2Text:GetText()")
            .unwrap(),
        "Elwynn Loop"
    );
    assert!(s
        .eval::<bool>("return BenillaKartFinderFrameAny:GetChecked() == 1")
        .unwrap());

    // Elwynn Loop alone, then Find race: the queue line goes up.
    s.run("BenillaKartFinderFrameRow2:Click() BenillaKartFinderFrameQueue:Click()")
        .unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tQ+\telwynn"]);
    assert_eq!(
        s.eval::<String>("return BenillaKartFinderFrameQueue:GetText()")
            .unwrap(),
        "Leave queue"
    );

    // The heartbeat every two seconds, and the server's status in the window.
    s.tick(2.1);
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tW"]);
    from_server(&mut s, "R\tQs\t2\t75");
    let status: String = s
        .eval("return BenillaKartFinderFrameStatus:GetText()")
        .unwrap();
    assert!(
        status.contains("Elwynn Loop") && status.contains("1:15, 2 waiting"),
        "{status}"
    );

    // The ready check is the stock popup; its Accept answers the server.
    from_server(&mut s, "R\tPop\t7\telwynn\t3\t20");
    assert!(s
        .eval::<bool>("return StaticPopup1:IsVisible() == 1")
        .unwrap());
    let text: String = s.eval("return StaticPopup1Text:GetText()").unwrap();
    assert!(
        text.contains("Elwynn Loop") && text.contains("3 racers"),
        "{text}"
    );
    s.run("StaticPopup1Button1:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tY\t7"]);

    // The race's roster with us in it ends the finder's part.
    from_server(&mut s, "R\tL\t3\tGoblin,Probesix");
    assert!(s
        .eval::<bool>("return BenillaKartFinder.queued == nil and BenillaKartFinder.check == nil")
        .unwrap());
    assert!(
        !s.eval::<bool>("return BenillaKartFinderFrame:IsVisible() == 1")
            .unwrap(),
        "the finder closes once the race has us"
    );
    s.tick(2.1);
    assert!(sent(&mut s).is_empty(), "no heartbeat once the race has us");
}

#[test]
fn a_declined_check_leaves_the_queue_and_a_racer_is_told_why() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.run("BenillaKartFinder_Queue()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tQ+\tany"]);

    from_server(&mut s, "R\tPop\t9\tsprint\t2\t20");
    s.run("StaticPopup1Button2:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tN\t9"]);
    assert!(s
        .eval::<bool>("return BenillaKartFinder.queued == nil")
        .unwrap());

    // Queued while racing: the server refuses, and the window says why.
    s.run("BenillaKartFinder_Queue()").unwrap();
    from_server(&mut s, "R\tQx\tracing");
    s.run("BenillaKartFinderFrame:Show()").unwrap();
    s.tick(0.0);
    assert_eq!(
        s.eval::<String>("return BenillaKartFinderFrameStatus:GetText()")
            .unwrap(),
        "You're in a race: finish it, or leave it with F6, first."
    );
}

#[test]
fn the_kart_s_race_and_score_reach_their_frames() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.fire_event(
        "BENILLA_KART_RACE",
        vec![
            ScriptValue::Str("3".into()),
            ScriptValue::Str("1. Goblin  1:02.40\n2. Probesix  1:03.10".into()),
        ],
    );
    s.tick(0.0);
    assert_eq!(
        s.eval::<String>("return BenillaKartRaceBanner:GetText()")
            .unwrap(),
        "3"
    );
    assert!(s
        .eval::<String>("return BenillaKartRaceResults:GetText()")
        .unwrap()
        .starts_with("1. Goblin"));

    let n = |v: f64| ScriptValue::Number(v);
    let t = |v: &str| ScriptValue::Str(v.into());
    s.fire_event(
        "BENILLA_KART_SCORE",
        vec![
            t("Triple Turbo x3"),
            n(1.0),
            n(1.0),
            n(1.0),
            t("+1,250"),
            n(0.35),
            n(1.0),
            n(0.35),
            t("+120 XP"),
            t("+2 Karting (40)"),
            n(0.5),
        ],
    );
    s.tick(0.0);
    assert!(s
        .eval::<bool>("return BenillaKartScoreFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartScorePoints:GetText()")
            .unwrap(),
        "+1,250"
    );
    assert!(
        (s.eval::<f64>("return BenillaKartScoreFrame:GetAlpha()")
            .unwrap()
            - 0.5)
            .abs()
            < 0.01
    );
    s.fire_event("BENILLA_KART_SCORE", Vec::new());
    s.tick(0.0);
    assert!(!s
        .eval::<bool>("return BenillaKartScoreFrame:IsVisible() == 1")
        .unwrap());
}

#[test]
fn forty_courses_scroll_and_a_tick_follows_its_course() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 40);
    s.run("BenillaKartFinderFrame:Show()").unwrap();
    s.tick(0.0);

    // Eight rows over the forty, and the bar to range over them.
    let shown: u32 = s
        .eval(
            "local n = 0 for i = 1, BENILLA_KART_ROWS do \
             if getglobal('BenillaKartFinderFrameRow' .. i):IsVisible() then n = n + 1 end end \
             return n",
        )
        .unwrap();
    assert_eq!(shown, 8);
    assert!(s
        .eval::<bool>("return BenillaKartFinderFrameScrollScrollBar:IsVisible() == 1")
        .unwrap());
    let first = |s: &UiScript| -> String {
        s.eval("return BenillaKartFinderFrameRow1Text:GetText()")
            .unwrap()
    };
    assert_eq!(first(&s), "Northshire Sprint");

    // The wheel moves the list.
    for _ in 0..12 {
        s.run("BenillaKartFinder_OnMouseWheel(-1)").unwrap();
        s.tick(0.0);
    }
    assert_ne!(first(&s), "Northshire Sprint", "the wheel scrolled nothing");

    // A tick down the list stays with its course when the list scrolls back.
    let key: String = s
        .eval("BenillaKartFinderFrameRow1:Click() return BenillaKartFinderFrameRow1.key")
        .unwrap();
    for _ in 0..12 {
        s.run("BenillaKartFinder_OnMouseWheel(1)").unwrap();
        s.tick(0.0);
    }
    assert_eq!(first(&s), "Northshire Sprint");
    assert!(!s
        .eval::<bool>("return BenillaKartFinderFrameRow1:GetChecked() == 1")
        .unwrap());
    s.run("BenillaKartFinderFrameQueue:Click()").unwrap();
    assert_eq!(sent(&mut s), [format!("KARTCRAFT\tR\tQ+\t{key}")]);
}

#[test]
fn the_search_narrows_the_list() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 40);
    s.run("BenillaKartFinderFrame:Show()").unwrap();
    s.tick(0.0);
    let visible = |s: &UiScript| -> Vec<String> {
        s.eval(
            "local out = {} for i = 1, BENILLA_KART_ROWS do \
             local row = getglobal('BenillaKartFinderFrameRow' .. i) \
             if row:IsVisible() then table.insert(out, getglobal(row:GetName() .. 'Text'):GetText()) end \
             end return out",
        )
        .unwrap()
    };

    // Any case, anywhere in the name.
    s.run("BenillaKartFinderFrameSearch:SetText('COURSE 4')")
        .unwrap();
    s.tick(0.0);
    assert_eq!(visible(&s), ["Course 4", "Course 40"]);
    s.run("BenillaKartFinderFrameSearch:SetText('elwyn')")
        .unwrap();
    s.tick(0.0);
    assert_eq!(visible(&s), ["Elwynn Loop"]);

    // A tick made under a search queues; clearing it brings the whole list back.
    s.run("BenillaKartFinderFrameRow1:Click()").unwrap();
    s.run("BenillaKartFinderFrameSearch:SetText('')").unwrap();
    s.tick(0.0);
    assert_eq!(visible(&s).len(), 8);
    s.run("BenillaKartFinderFrameQueue:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tQ+\telwynn"]);
}

#[test]
fn closing_while_queued_minimizes_to_the_queue_button() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    s.run("BenillaKartFinderFrame:Show()").unwrap();
    s.tick(0.0);
    let shown = |s: &UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };

    // Not queued: Close closes, and nothing stays on screen.
    assert_eq!(
        s.eval::<String>("return BenillaKartFinderFrameClose:GetText()")
            .unwrap(),
        "Close"
    );
    s.run("BenillaKartFinderFrameClose:Click()").unwrap();
    s.tick(0.0);
    assert!(!shown(&s, "BenillaKartFinderFrame") && !shown(&s, "BenillaKartQueueButton"));

    // Queued: Close reads Minimize, and the queue button shows the server's status.
    s.run("BenillaKartFinderFrame:Show() BenillaKartFinderFrameQueue:Click()")
        .unwrap();
    from_server(&mut s, "R\tQs\t3\t65");
    assert_eq!(
        s.eval::<String>("return BenillaKartFinderFrameClose:GetText()")
            .unwrap(),
        "Minimize"
    );
    s.run("BenillaKartFinderFrameClose:Click()").unwrap();
    s.tick(0.0);
    assert!(!shown(&s, "BenillaKartFinderFrame"));
    assert!(shown(&s, "BenillaKartQueueButton"));
    assert_eq!(
        s.eval::<String>("return BenillaKartQueueButtonText:GetText()")
            .unwrap(),
        "Race Finder: 1:05, 3 waiting"
    );

    // A click opens the window again; leaving the queue retires the button.
    s.run("BenillaKartQueueButton:Click()").unwrap();
    s.tick(0.0);
    assert!(shown(&s, "BenillaKartFinderFrame") && !shown(&s, "BenillaKartQueueButton"));
    s.run("BenillaKartFinderFrameQueue:Click() BenillaKartFinderFrame:Hide()")
        .unwrap();
    s.tick(0.0);
    assert!(!shown(&s, "BenillaKartQueueButton"));
}

#[test]
fn the_results_are_a_board_and_the_widgets_move() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let race = |s: &mut UiScript, banner: &str, results: &str| {
        s.fire_event(
            "BENILLA_KART_RACE",
            vec![
                ScriptValue::Str(banner.into()),
                ScriptValue::Str(results.into()),
            ],
        );
        s.tick(0.0);
    };
    let shown = |s: &UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };

    // No results, no board; the finish shows one as tall as its lines.
    race(&mut s, "GO!", "");
    assert!(!shown(&s, "BenillaKartResultsFrame"));
    race(
        &mut s,
        "1ST — 1:02.40",
        "1. Probesix  1:02.40\n2. Goblin  1:03.10\n3. Gnome  1:05.00",
    );
    assert!(shown(&s, "BenillaKartResultsFrame"));
    assert_eq!(
        s.eval::<String>("return BenillaKartResultsFrameTitle:GetText()")
            .unwrap(),
        "Race Results"
    );
    assert_eq!(
        s.eval::<f64>("return BenillaKartResultsFrame:GetHeight()")
            .unwrap(),
        44.0 + 3.0 * 16.0
    );
    race(&mut s, "", "");
    assert!(!shown(&s, "BenillaKartResultsFrame"));

    // The finder, its queue button and the board drag, and a dragged one keeps its place.
    for frame in [
        "BenillaKartFinderFrame",
        "BenillaKartQueueButton",
        "BenillaKartResultsFrame",
    ] {
        assert!(
            s.eval::<bool>(&format!("return {frame}:IsMovable() == 1"))
                .unwrap(),
            "{frame} is not movable"
        );
    }
    s.run("BenillaKartResultsFrame:Show() BenillaKartResultsFrame:StartMoving() BenillaKartResultsFrame:StopMovingOrSizing()")
        .unwrap();
    assert!(s
        .eval::<bool>("return BenillaKartResultsFrame:IsUserPlaced() == 1")
        .unwrap());
}
