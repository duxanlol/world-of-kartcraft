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
        s.eval::<String>("return BenillaKartFinderFrameCourse2Text:GetText()")
            .unwrap(),
        "Elwynn Loop"
    );
    assert!(s
        .eval::<bool>("return BenillaKartFinderFrameAny:GetChecked() == 1")
        .unwrap());

    // Elwynn Loop alone, then Find race: the queue line goes up.
    s.run("BenillaKartFinderFrameCourse2:Click() BenillaKartFinderFrameQueue:Click()")
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
