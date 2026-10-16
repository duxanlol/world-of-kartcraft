//! World of Kartcraft's windows (`assets/ui/KartcraftFrames.xml`) on the production load: the
//! minimap button and its menu, the Race Finder queuing and answering over the addon channel, the
//! ready check as the stock popup, the kart's race and score events reaching their frames, the
//! settings window writing its CVars, and the garage's classes.

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

/// The addon lines the VM sent since the last call, as `prefix\tmessage`, but the open finder's
/// background poll for the queue and the lobbies.
fn sent(s: &mut UiScript) -> Vec<String> {
    s.take_addon_sends()
        .into_iter()
        .map(|a| a.text)
        .filter(|t| t != "KARTCRAFT\tR\tLobby")
        .collect()
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

    // The minimap button opens it, in the stock widgets, with "Any course" ticked and the courses named.
    assert!(!s
        .eval::<bool>("return BenillaKartFinderFrame:IsVisible() == 1")
        .unwrap());
    s.run("BenillaKartMinimapButton:Click()").unwrap();
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
        "You're in a race: finish it, or leave it with /leaverace, first."
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
fn in_the_kart_the_bars_and_unit_frames_step_aside_and_come_back() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let shown = |s: &mut UiScript, frame: &str| {
        s.eval::<bool>(&format!("return {frame}:IsVisible() == 1"))
            .unwrap()
    };
    let seated = |s: &mut UiScript, on: bool| {
        s.fire_event(
            "BENILLA_KART_SEATED",
            vec![ScriptValue::Number(f64::from(u8::from(on)))],
        );
        s.tick(0.0);
    };
    s.tick(0.0);
    assert!(shown(&mut s, "MainMenuBar") && shown(&mut s, "PlayerFrame"));
    assert!(shown(&mut s, "ChatFrame1") && shown(&mut s, "MinimapCluster"));

    seated(&mut s, true);
    assert!(!shown(&mut s, "MainMenuBar") && !shown(&mut s, "PlayerFrame"));
    // The stock code's own Show() cannot bring a frame back while we drive.
    s.eval::<()>("PlayerFrame:Show()").unwrap();
    s.tick(0.0);
    assert!(!shown(&mut s, "PlayerFrame"));
    // Chat and the minimap stay.
    assert!(shown(&mut s, "ChatFrame1") && shown(&mut s, "MinimapCluster"));

    seated(&mut s, false);
    assert!(shown(&mut s, "MainMenuBar") && shown(&mut s, "PlayerFrame"));
    assert_eq!(
        s.eval::<String>("return PlayerFrame:GetParent():GetName()")
            .unwrap(),
        "UIParent"
    );
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
fn closing_while_queued_minimizes_to_the_minimap_button() {
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
    let tooltip =
        |s: &UiScript| -> String { s.eval("return BenillaKartMinimapButton.tooltip").unwrap() };

    // Not queued: Close closes, and the minimap button has no queue to tell or glow for.
    assert_eq!(
        s.eval::<String>("return BenillaKartFinderFrameClose:GetText()")
            .unwrap(),
        "Close"
    );
    s.run("BenillaKartFinderFrameClose:Click()").unwrap();
    s.tick(0.0);
    assert!(!shown(&s, "BenillaKartFinderFrame"));
    assert_eq!(tooltip(&s), "World of Kartcraft");
    assert!(!shown(&s, "BenillaKartMinimapButtonGlow"));

    // Queued: Close reads Minimize, and the minimap button tells the server's status and glows.
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
    assert_eq!(tooltip(&s), "Race Finder: 1:05, 3 waiting");
    assert!(shown(&s, "BenillaKartMinimapButtonGlow"));
    s.run("BenillaKartMinimap_Tooltip()").unwrap();
    assert!(
        s.eval::<bool>(
            "return GameTooltip:IsVisible() == 1 \
             and GameTooltipTextLeft2:GetText() == 'Race Finder: 1:05, 3 waiting'"
        )
        .unwrap(),
        "the tooltip tells the queue"
    );

    // A click opens the window again; leaving the queue puts the glow out.
    s.run("BenillaKartMinimapButton:Click()").unwrap();
    s.tick(0.0);
    assert!(shown(&s, "BenillaKartFinderFrame"));
    s.run("BenillaKartFinderFrameQueue:Click() BenillaKartFinderFrame:Hide()")
        .unwrap();
    s.tick(0.0);
    assert!(!shown(&s, "BenillaKartMinimapButtonGlow"));
    assert_eq!(tooltip(&s), "World of Kartcraft");
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

    // The finder, the settings and the board drag, and a dragged one keeps its place.
    for frame in [
        "BenillaKartFinderFrame",
        "BenillaKartSettingsFrame",
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

#[test]
fn leaverace_leaves_everything_and_the_answer_clears_the_finder() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);

    // Queued, with a ready check up.
    s.run("BenillaKartFinderFrame:Show()").unwrap();
    s.tick(0.0);
    s.run("BenillaKartFinderFrameQueue:Click()").unwrap();
    from_server(&mut s, "R\tPop\t7\tsprint\t2\t20");
    sent(&mut s);

    // /leaverace (and /lr) asks the server to let us go from everything.
    s.run("SlashCmdList[\"BENILLA_LEAVERACE\"](\"\")").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tX"]);
    assert_eq!(
        s.eval::<String>("return SLASH_BENILLA_LEAVERACE2").unwrap(),
        "/lr"
    );

    // Its answer clears the queue and the ready check.
    from_server(&mut s, "R\tX");
    assert!(s
        .eval::<bool>("return BenillaKartFinder.queued == nil and BenillaKartFinder.check == nil")
        .unwrap());
}

#[test]
fn racewait_asks_the_server_for_the_finders_wait() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    sent(&mut s);

    // The seconds, stripped of anything else; bare, it asks for the wait in force.
    s.run("SlashCmdList[\"BENILLA_RACEWAIT\"](\" 5s\")")
        .unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tFillWait\t5"]);
    s.run("SlashCmdList[\"BENILLA_RACEWAIT\"](\"\")").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tFillWait\t"]);
}

#[test]
fn kartshow_asks_the_server_for_the_showcase_runs() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    sent(&mut s);

    // Bare, the list; with a name (trimmed, any case), that run; "stop" stops it.
    s.run("SlashCmdList[\"BENILLA_KARTSHOW\"](\"\")").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tShow"]);
    s.run("SlashCmdList[\"BENILLA_KARTSHOW\"](\"  Mirage \")")
        .unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tShow\tmirage"]);
    s.run("SlashCmdList[\"BENILLA_KARTSHOW\"](\"stop\")")
        .unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tShow\tstop"]);
    assert_eq!(
        s.eval::<String>("return SLASH_BENILLA_KARTSHOW1").unwrap(),
        "/kartshow"
    );
}

#[test]
fn a_finder_race_offers_the_way_back_and_new_bests_reach_the_chat() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    s.run(
        "BenillaTestSaid = {} \
         DEFAULT_CHAT_FRAME.AddMessage = function(self, m) table.insert(BenillaTestSaid, m) end",
    )
    .unwrap();
    let said =
        |s: &UiScript| -> String { s.eval("return table.concat(BenillaTestSaid, '|')").unwrap() };
    let me: String = s.eval("return UnitName('player')").unwrap();

    // A course record by anyone is told; a personal best only when it is ours.
    from_server(&mut s, "R\tC\t4\tstormwind\t0\t1:2;3:4");
    from_server(&mut s, "R\tF\t4\tGoblin\t1\t62400\tr");
    from_server(&mut s, "R\tF\t4\tGnome\t2\t65000\tp");
    from_server(&mut s, "R\tF\t4\tElf\t3\t66000\t");
    from_server(&mut s, &format!("R\tF\t4\t{me}\t4\t70010\tp"));
    assert_eq!(
        said(&s),
        "Goblin set a new course record on Road to Stormwind: 1:02.40!|\
         New personal best on Road to Stormwind: 1:10.01."
    );

    // Out of the race the finder took us to: the stock popup offers the way back.
    from_server(&mut s, "R\tRet");
    assert!(s
        .eval::<bool>("return StaticPopup1:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return StaticPopup1Text:GetText()")
            .unwrap(),
        "Return to where you were before the race?\n\n|cff999999Controller: Triangle / Y to return, \
         Square / X to stay|r"
    );
    assert_eq!(
        s.eval::<String>("return StaticPopup1Button1:GetText()")
            .unwrap(),
        "Return"
    );
    sent(&mut s);
    s.run("StaticPopup1Button1:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tBack"]);
    assert!(!s
        .eval::<bool>("return StaticPopup1:IsVisible() == 1")
        .unwrap());

    // Stay says so, and the server forgets the spot.
    from_server(&mut s, "R\tRet");
    s.run("StaticPopup1Button2:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tStay"]);
}

/// After a time trial the way out offers another go, by mouse or by Options / Menu ("TA"): beside
/// Return and Stay when the track browser took us there ("Ret" naming the course), alone when
/// nothing did ("Again"). After a Race Finder race only the way back is offered.
#[test]
fn the_way_out_of_a_time_trial_offers_a_retry() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    let visible = |s: &UiScript, frame: &str| -> bool {
        s.eval::<bool>(&format!("return {frame}:IsVisible() == 1"))
            .unwrap()
    };
    let text = |s: &UiScript| -> String { s.eval("return StaticPopup1Text:GetText()").unwrap() };
    let owns = |s: &UiScript| -> String { s.eval("return BenillaPad_Owns()").unwrap() };

    // The track browser's: Return, Stay and Retry, which neither returns nor stays.
    from_server(&mut s, "R\tRet\tstormwind");
    assert!(visible(&s, "StaticPopup1"));
    assert_eq!(
        text(&s),
        "Return to where you were before the time trial?\n\n|cff999999Controller: Triangle / Y to \
         return, Square / X to stay, Options / Menu to retry|r"
    );
    assert_eq!(
        s.eval::<String>(
            "return StaticPopup1Button1:GetText() .. '/' .. StaticPopup1Button2:GetText()"
        )
        .unwrap(),
        "Return/Stay"
    );
    assert!(visible(&s, "BenillaKartRetryButton"));
    sent(&mut s);
    s.run("BenillaKartRetryButton:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tTA\tstormwind"]);
    assert!(!visible(&s, "StaticPopup1"));
    assert!(!visible(&s, "BenillaKartRetryButton"));

    // Options / Menu retries while it is up, as Triangle and Square answer.
    from_server(&mut s, "R\tRet\tstormwind");
    assert_eq!(owns(&s), "PAD4 PAD3 PADFORWARD");
    assert!(s
        .eval::<bool>("return BenillaPad_OnButton('PADFORWARD') == true")
        .unwrap());
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tTA\tstormwind"]);
    assert!(!visible(&s, "BenillaKartRetryButton"));
    // Return still returns, and takes Retry with it.
    from_server(&mut s, "R\tRet\tstormwind");
    s.run("StaticPopup1Button1:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tBack"]);
    assert!(!visible(&s, "BenillaKartRetryButton"));

    // The Race Marshal's, with nowhere to go back to: Retry or Close.
    from_server(&mut s, "R\tAgain\tstormwind");
    assert_eq!(
        text(&s),
        "Run the time trial again?\n\n|cff999999Controller: Options / Menu to retry, Square / X to \
         close|r"
    );
    assert!(!visible(&s, "BenillaKartRetryButton"));
    assert_eq!(owns(&s), "PADFORWARD PAD3");
    s.run("StaticPopup1Button1:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tTA\tstormwind"]);
    from_server(&mut s, "R\tAgain\tstormwind");
    s.run("StaticPopup1Button2:Click()").unwrap();
    assert!(sent(&mut s).is_empty(), "Close says nothing");
    assert!(!visible(&s, "StaticPopup1"));

    // A Race Finder race: the plain way back, no Retry.
    from_server(&mut s, "R\tRet");
    assert_eq!(
        text(&s),
        "Return to where you were before the race?\n\n|cff999999Controller: Triangle / Y to return, \
         Square / X to stay|r"
    );
    assert!(!visible(&s, "BenillaKartRetryButton"));
    assert_eq!(owns(&s), "PAD4 PAD3");
}

/// In a time trial under way the Kartcraft menu (Options on the controller) restarts it, as the
/// Restart binding does ("TR"); out of one its entry is greyed and does nothing.
#[test]
fn the_kartcraft_menu_restarts_a_time_trial_under_way() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    let me: String = s.eval("return UnitName('player')").unwrap();
    let entry: String = s
        .eval(
            "for i = 1, table.getn(BENILLA_KART_MENU) do \
             if BENILLA_KART_MENU[i][1] == 'Restart time trial' then return 'BenillaKartMenuFrameEntry' .. i end end",
        )
        .unwrap();
    let enabled = |s: &mut UiScript| -> bool {
        s.run("BenillaKartMenuFrame:Hide() BenillaKart_Open(BenillaKartMenuFrame)")
            .unwrap();
        s.eval(&format!("return {entry}:IsEnabled() == 1")).unwrap()
    };
    assert!(!enabled(&mut s), "no race: greyed");

    // On the roster of time trial 7, the kart racing it.
    from_server(&mut s, &format!("R\tL\t7\t{me}"));
    from_server(&mut s, "R\tTT\t7");
    s.fire_event(
        "BENILLA_KART_TRACKMAP",
        vec![
            ScriptValue::Str("Road to Stormwind".into()),
            ScriptValue::Str("3".into()),
            ScriptValue::Str("".into()),
        ],
    );
    assert!(enabled(&mut s));
    sent(&mut s);
    s.run(&format!("{entry}:Click()")).unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tTR\t7"]);
    assert!(!s
        .eval::<bool>("return BenillaKartMenuFrame:IsVisible() == 1")
        .unwrap());

    // A race that is not a time trial: greyed again.
    from_server(&mut s, &format!("R\tL\t8\t{me}"));
    assert!(!enabled(&mut s));
    // The race over: greyed.
    from_server(&mut s, &format!("R\tL\t9\t{me}"));
    from_server(&mut s, "R\tTT\t9");
    s.fire_event(
        "BENILLA_KART_TRACKMAP",
        vec![
            ScriptValue::Str("".into()),
            ScriptValue::Str("0".into()),
            ScriptValue::Str("".into()),
        ],
    );
    assert!(!enabled(&mut s));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn a_time_trial_s_new_bests_are_told_as_time_trial_bests() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    s.run(
        "BenillaTestSaid = {} \
         DEFAULT_CHAT_FRAME.AddMessage = function(self, m) table.insert(BenillaTestSaid, m) end",
    )
    .unwrap();
    let said =
        |s: &UiScript| -> String { s.eval("return table.concat(BenillaTestSaid, '|')").unwrap() };
    let me: String = s.eval("return UnitName('player')").unwrap();

    // Race 7 is a time trial: its record and our best are the time trials'.
    from_server(&mut s, &format!("R\tL\t7\t{me}"));
    from_server(&mut s, "R\tTT\t7");
    from_server(&mut s, "R\tC\t7\tsprint\t0\t1:2;3:4");
    from_server(&mut s, &format!("R\tF\t7\t{me}\t1\t61000\tr"));
    // A race on the same course afterwards is a race again.
    from_server(&mut s, "R\tC\t8\tsprint\t0\t1:2;3:4");
    from_server(&mut s, &format!("R\tF\t8\t{me}\t1\t60500\tp"));
    from_server(&mut s, "R\tL\t9\tGnome");
    from_server(&mut s, "R\tTT\t9");
    from_server(&mut s, "R\tC\t9\tsprint\t0\t1:2;3:4");
    from_server(&mut s, "R\tF\t9\tGnome\t1\t59000\tp");
    from_server(&mut s, "R\tTT\t10");
    from_server(&mut s, "R\tC\t10\tsprint\t0\t1:2;3:4");
    from_server(&mut s, &format!("R\tF\t10\t{me}\t1\t58000\tp"));
    assert_eq!(
        said(&s),
        format!(
            "{me} set a new time trial record on Northshire Sprint: 1:01.00!|\
             New personal best on Northshire Sprint: 1:00.50.|\
             New time trial best on Northshire Sprint: 0:58.00."
        ),
        "another's time trial best is theirs alone"
    );
}

/// The labels of the minimap button's menu, opened afresh.
fn menu(s: &mut UiScript) -> Vec<String> {
    s.run("CloseDropDownMenus() BenillaKartMinimapButton:Click('RightButton')")
        .unwrap();
    s.tick(0.0);
    s.eval(
        "local out = {} for i = 1, UIDROPDOWNMENU_MAXBUTTONS do \
         local b = getglobal('DropDownList1Button' .. i) \
         if b and b:IsVisible() then table.insert(out, b:GetText()) end \
         end return out",
    )
    .unwrap()
}

/// The open menu's entry labelled `text`, clicked.
fn pick(s: &mut UiScript, text: &str) {
    s.run(&format!(
        "for i = 1, UIDROPDOWNMENU_MAXBUTTONS do \
         local b = getglobal('DropDownList1Button' .. i) \
         if b and b:IsVisible() and b:GetText() == '{text}' then b:Click() return end \
         end error('no menu entry {text}')"
    ))
    .unwrap();
    s.tick(0.0);
}

#[test]
fn the_minimap_button_sits_on_the_ring_and_its_menu_works() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    let shown = |s: &UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };

    // On the minimap, at its ring, with the stock tracking border round an install icon.
    assert!(shown(&s, "BenillaKartMinimapButton"));
    assert!(s
        .eval::<bool>("return BenillaKartMinimapButton:GetParent() == Minimap")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartMinimapButtonBorder:GetTexture()")
            .unwrap()
            .to_ascii_lowercase(),
        "interface\\minimap\\minimap-trackingborder"
    );
    let ring = |s: &UiScript| -> (f64, f64) {
        let v: Vec<f64> = s
            .eval(
                "local x, y = BenillaKartMinimapButton:GetCenter() \
                 local cx, cy = Minimap:GetCenter() \
                 return { x - cx, y - cy }",
            )
            .unwrap();
        (
            v[0].hypot(v[1]),
            v[1].atan2(v[0]).to_degrees().rem_euclid(360.0),
        )
    };
    let (radius, angle) = ring(&s);
    assert!((radius - 80.0).abs() < 0.5, "radius {radius}");
    assert!((angle - 190.0).abs() < 0.5, "angle {angle}");

    // Moved round the ring, it stays on it and keeps its angle in KartMinimapAngle.
    s.run("BenillaKartSettings_Set('KartMinimapAngle', 90.2, 0) BenillaKartMinimap_Place()")
        .unwrap();
    let (radius, angle) = ring(&s);
    assert!((radius - 80.0).abs() < 0.5 && (angle - 90.0).abs() < 0.5);
    assert_eq!(s.cvar("KartMinimapAngle").as_deref(), Some("90"));

    // A left click toggles the finder.
    s.run("BenillaKartMinimapButton:Click('LeftButton')")
        .unwrap();
    assert!(shown(&s, "BenillaKartFinderFrame"));
    s.run("BenillaKartMinimapButton:Click('LeftButton')")
        .unwrap();
    assert!(!shown(&s, "BenillaKartFinderFrame"));

    // The right click's menu; Leave queue only while queued.
    assert_eq!(
        menu(&mut s),
        [
            "World of Kartcraft",
            "Race Finder",
            "Race mode",
            "Track browser",
            "Replays...",
            "Show Race Marshals",
            "Leave race",
            "Unstuck",
            "Courier jobs...",
            "Guide...",
            "Kart garage...",
            "Skin shop...",
            "Kartcraft settings...",
            "Kart camera...",
            "Controller test...",
            "Hide minimap buttons",
        ]
    );
    pick(&mut s, "Race Finder");
    assert!(shown(&s, "BenillaKartFinderFrame"));
    s.run("BenillaKartFinderFrameQueue:Click() BenillaKartFinderFrame:Hide()")
        .unwrap();
    sent(&mut s);
    assert!(menu(&mut s).contains(&"Leave queue".to_string()));
    pick(&mut s, "Leave queue");
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tQ-"]);

    // Leave race asks the server to let us go from everything, as /leaverace does.
    menu(&mut s);
    pick(&mut s, "Leave race");
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tX"]);

    menu(&mut s);
    pick(&mut s, "Kart garage...");
    assert!(shown(&s, "BenillaKartGarageFrame"));
    s.run("BenillaKartGarageFrame:Hide()").unwrap();

    menu(&mut s);
    pick(&mut s, "Kartcraft settings...");
    assert!(shown(&s, "BenillaKartSettingsFrame"));
    s.run("BenillaKartSettingsFrame:Hide()").unwrap();

    // Hidden from the menu; the settings window's box brings it back.
    menu(&mut s);
    pick(&mut s, "Hide minimap buttons");
    assert!(!shown(&s, "BenillaKartMinimapButton"));
    assert!(!shown(&s, "BenillaKartShopMinimapButton"));
    assert_eq!(s.cvar("KartMinimapButton").as_deref(), Some("0"));
    s.run("SlashCmdList['BENILLA_KARTCRAFT']('')").unwrap();
    assert!(!s
        .eval::<bool>("return BenillaKartSettingsFrameMinimapButton:GetChecked() == 1")
        .unwrap());
    s.run("BenillaKartSettingsFrameMinimapButton:Click()")
        .unwrap();
    assert!(shown(&s, "BenillaKartMinimapButton"));
    assert!(shown(&s, "BenillaKartShopMinimapButton"));
    assert_eq!(s.cvar("KartMinimapButton").as_deref(), Some("1"));

    // The shop's own button: on the ring at its own angle, a click opens the shop.
    s.run("BenillaKartSettings_Set('KartShopMinimapAngle', 270, 0) BenillaKartShopButton_Place()")
        .unwrap();
    let (x, y): (f32, f32) = s
        .eval(
            "local x, y = BenillaKartShopMinimapButton:GetCenter() \
             local cx, cy = Minimap:GetCenter() return x - cx, y - cy",
        )
        .unwrap();
    assert!(x.abs() < 0.5 && (y + 80.0).abs() < 0.5, "({x}, {y})");
    s.run("BenillaKartShopMinimapButton:Click('LeftButton')")
        .unwrap();
    assert!(shown(&s, "BenillaKartShopFrame"));
}

#[test]
fn the_settings_window_sets_the_kart_s_cvars() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.take_cvar_changes();
    let shown = |s: &UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };

    // /kartcraft opens it at the CVars' values, and opening it writes nothing.
    s.run("SlashCmdList['BENILLA_KARTCRAFT']('')").unwrap();
    s.tick(0.0);
    assert!(shown(&s, "BenillaKartSettingsFrame"));
    assert_eq!(
        s.eval::<String>("return SLASH_BENILLA_KARTCRAFT1").unwrap(),
        "/kartcraft"
    );
    for (name, label) in [
        ("ArrowShow", "Show the guide arrow in a race"),
        ("ChaseCamera", "CTR chase camera (off: your WoW camera)"),
        ("Music", "Play CTR race music"),
        ("MinimapButton", "Show the minimap buttons"),
        ("Ghost", "Race the ghost of your best run"),
    ] {
        let frame = format!("BenillaKartSettingsFrame{name}");
        assert!(
            s.eval::<bool>(&format!("return {frame}:GetChecked() == 1"))
                .unwrap(),
            "{name} starts on"
        );
        assert_eq!(
            s.eval::<String>(&format!("return {frame}Text:GetText()"))
                .unwrap(),
            label
        );
    }
    assert_eq!(
        s.eval::<f64>("return BenillaKartSettingsFrameArrowSize:GetValue()")
            .unwrap(),
        1.0
    );
    // The checkpoint gates' height opens at its default, and the slider writes its CVar.
    assert_eq!(
        s.eval::<f64>("return BenillaKartSettingsFrameGateHeight:GetValue()")
            .unwrap(),
        8.0
    );
    assert_eq!(
        s.eval::<String>("return BenillaKartSettingsFrameGateHeightText:GetText()")
            .unwrap(),
        "Checkpoint gate height"
    );
    s.run("BenillaKartSettingsFrameGateHeight:SetValue(3)")
        .unwrap();
    assert_eq!(
        s.eval::<f64>("return tonumber(GetCVar('KartGateHeight'))")
            .unwrap(),
        3.0
    );
    s.run("BenillaKartSettingsFrameGateHeight:SetValue(8)")
        .unwrap();
    s.take_cvar_changes();
    // The ground types start off: every ground drives as asphalt until asked.
    assert!(!s
        .eval::<bool>("return BenillaKartSettingsFrameTerrain:GetChecked() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartSettingsFrameTerrainText:GetText()")
            .unwrap(),
        "Ground types slow you off-road (CTR)"
    );
    // The far camera starts off: CTR starts on its near one.
    assert!(!s
        .eval::<bool>("return BenillaKartSettingsFrameFarCamera:GetChecked() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartSettingsFrameFarCameraText:GetText()")
            .unwrap(),
        "Far chase camera (off: near)"
    );
    assert!(
        s.take_cvar_changes().is_empty(),
        "showing the window wrote a CVar"
    );

    // Each control writes its CVar, which the host hears.
    s.run(
        "BenillaKartSettingsFrameArrowShow:Click() \
         BenillaKartSettingsFrameChaseCamera:Click() \
         BenillaKartSettingsFrameMusic:Click() \
         BenillaKartSettingsFrameGhost:Click() \
         BenillaKartSettingsFrameTerrain:Click() \
         BenillaKartSettingsFrameRumble:Click() \
         BenillaKartSettingsFrameFarCamera:Click() \
         BenillaKartSettingsFrameArrowSize:SetValue(1.5) \
         BenillaKartSettingsFrameMusicVolume:SetValue(0.25)",
    )
    .unwrap();
    for (name, value) in [
        ("KartArrowShow", "0"),
        ("KartChaseCamera", "0"),
        ("KartMusic", "0"),
        ("KartGhost", "0"),
        ("KartTerrain", "1"),
        ("KartRumble", "0"),
        ("KartFarCamera", "1"),
        ("KartArrowSize", "1.5"),
        ("KartMusicVolume", "0.25"),
    ] {
        assert_eq!(s.cvar(name).as_deref(), Some(value), "{name}");
    }
    let changes = s.take_cvar_changes();
    assert!(
        changes
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case("KartChaseCamera") && v == "0"),
        "the host hears the camera: {changes:?}"
    );

    // Move arrow: the window steps aside for the arrow's box, which places the arrow while it is
    // up; the wheel sizes it, Done brings the window back.
    s.run("BenillaKartSettingsFrameMoveArrow:Click()").unwrap();
    s.tick(0.0);
    assert!(!shown(&s, "BenillaKartSettingsFrame") && shown(&s, "BenillaKartArrowMover"));
    assert_eq!(s.cvar("KartArrowMove").as_deref(), Some("1"));
    let at: Vec<f64> = s
        .eval(
            "local x, y = BenillaKartArrowMover:GetCenter() \
             return { x / UIParent:GetWidth(), 1 - y / UIParent:GetHeight() }",
        )
        .unwrap();
    assert!(
        (at[0] - 0.5).abs() < 0.01 && (at[1] - 0.22).abs() < 0.01,
        "the box stands where the arrow is: {at:?}"
    );
    s.run(
        "BenillaKartArrowMover:ClearAllPoints() \
         BenillaKartArrowMover:SetPoint('CENTER', 'UIParent', 'TOPLEFT', \
             UIParent:GetWidth() / 4, -UIParent:GetHeight() * 3 / 4) \
         BenillaKartArrowMover_Save() BenillaKartArrowMover_Wheel(1)",
    )
    .unwrap();
    assert_eq!(s.cvar("KartArrowX").as_deref(), Some("0.25"));
    assert_eq!(s.cvar("KartArrowY").as_deref(), Some("0.75"));
    assert_eq!(s.cvar("KartArrowSize").as_deref(), Some("1.65"));
    s.run("BenillaKartArrowMoverDone:Click()").unwrap();
    s.tick(0.0);
    assert_eq!(s.cvar("KartArrowMove").as_deref(), Some("0"));
    assert!(shown(&s, "BenillaKartSettingsFrame") && !shown(&s, "BenillaKartArrowMover"));

    // Reset arrow puts it back where it starts.
    s.run("BenillaKartSettingsFrameResetArrow:Click()").unwrap();
    for (name, value) in [
        ("KartArrowX", "0.5"),
        ("KartArrowY", "0.22"),
        ("KartArrowSize", "1"),
    ] {
        assert_eq!(s.cvar(name).as_deref(), Some(value), "{name}");
    }
}

/// The chase camera's tuning as the VM's camera CVars say, read as the host reads them.
fn camera_tuning(s: &UiScript) -> kartcraft::camera::CameraTuning {
    use crate::kart::settings::{camera_field, CAMERA_KNOBS};
    let value = |name: &str| s.cvar(name).unwrap_or_else(|| panic!("{name} is a CVar"));
    let mut t = kartcraft::camera::CameraTuning::default();
    for k in &CAMERA_KNOBS {
        *camera_field(&mut t, k.cvar).unwrap() = value(k.cvar).parse().unwrap();
    }
    t.travel = value("KartCamTravel") != "0";
    t.wall_pull = value("KartCamWallPull") != "0";
    t
}

#[test]
fn the_camera_window_tunes_the_chase_camera() {
    use kartcraft::camera::CameraTuning;
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.take_cvar_changes();
    let shown = |s: &UiScript| -> bool {
        s.eval("return BenillaKartCameraFrame:IsVisible() == 1")
            .unwrap()
    };
    let preset = |s: &UiScript| -> String {
        s.eval("return BenillaKartCameraFramePresetText:GetText()")
            .unwrap()
    };
    let label = |s: &UiScript, name: &str| -> String {
        s.eval(&format!(
            "return BenillaKartCameraFrame{name}Text:GetText()"
        ))
        .unwrap()
    };

    // /kartcam opens it at the CVars' values, CTR classic's, and opening it writes nothing.
    assert_eq!(
        s.eval::<String>("return SLASH_BENILLA_KARTCAM1").unwrap(),
        "/kartcam"
    );
    s.run("SlashCmdList['BENILLA_KARTCAM']('')").unwrap();
    s.tick(0.0);
    assert!(shown(&s));
    assert_eq!(preset(&s), "CTR classic");
    assert_eq!(camera_tuning(&s), CameraTuning::CTR);
    assert_eq!(label(&s, "Distance"), "Distance at speed: 6.1 yd");
    assert_eq!(label(&s, "Fov"), "Field of view: 73.7\u{b0}");
    assert_eq!(label(&s, "Swing"), "Powerslide swing: 100%");
    assert_eq!(label(&s, "TurnSmoothing"), "Turn smoothing: none");
    assert_eq!(label(&s, "YawOffset"), "Angle round the kart: behind");
    assert_eq!(
        s.eval::<f64>("return BenillaKartCameraFrameDistance:GetValue()")
            .unwrap(),
        224.0
    );
    for (name, on) in [
        ("ChaseCamera", true),
        ("FarCamera", false),
        ("Travel", false),
        ("WallPull", false),
    ] {
        assert_eq!(
            s.eval::<bool>(&format!(
                "return BenillaKartCameraFrame{name}:GetChecked() == 1"
            ))
            .unwrap(),
            on,
            "{name}"
        );
    }
    assert!(
        s.take_cvar_changes().is_empty(),
        "showing the window wrote a CVar"
    );
    // Each slider spans the range the host clamps to.
    let ranges: Vec<(String, f64, f64)> = s
        .eval::<String>(
            "local out = '' \
             for i = 1, table.getn(BENILLA_KART_CAMERA_SLIDERS) do \
               local k = BENILLA_KART_CAMERA_SLIDERS[i] \
               local lo, hi = getglobal('BenillaKartCameraFrame' .. k[1]):GetMinMaxValues() \
               out = out .. k[2] .. ' ' .. lo .. ' ' .. hi .. '\\n' \
             end return out",
        )
        .unwrap()
        .lines()
        .map(|line| {
            let f: Vec<&str> = line.split(' ').collect();
            (
                f[0].to_string(),
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
            )
        })
        .collect();
    let knobs = &crate::kart::settings::CAMERA_KNOBS;
    assert_eq!(ranges.len(), knobs.len());
    for k in knobs {
        let (_, lo, hi) = ranges
            .iter()
            .find(|r| r.0 == k.cvar)
            .unwrap_or_else(|| panic!("{} has no slider", k.cvar));
        assert_eq!((*lo as f32, *hi as f32), (k.min, k.max), "{}", k.cvar);
    }

    // A preset from the dropdown writes every camera CVar, which the host hears.
    let choose = |s: &mut UiScript, text: &str| {
        s.run("BenillaKartCameraFramePresetButton:Click()").unwrap();
        s.tick(0.0);
        pick(s, text);
    };
    choose(&mut s, "Kartcraft");
    assert_eq!(camera_tuning(&s), CameraTuning::KARTCRAFT);
    assert_eq!(preset(&s), "Kartcraft");
    assert_eq!(label(&s, "Fov"), "Field of view: 45.0\u{b0}");
    assert!(s
        .eval::<bool>("return BenillaKartCameraFrameWallPull:GetChecked() == 1")
        .unwrap());
    let changes = s.take_cvar_changes();
    assert!(
        changes
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case("KartCamFov") && v == "45"),
        "the host hears the preset: {changes:?}"
    );
    choose(&mut s, "Smooth follow");
    assert_eq!(camera_tuning(&s), CameraTuning::SMOOTH);
    assert_eq!(preset(&s), "Smooth follow");
    assert!(s
        .eval::<bool>("return BenillaKartCameraFrameTravel:GetChecked() == 1")
        .unwrap());

    // A slider writes its CVar at once and the preset reads Custom; so does a box.
    s.run("BenillaKartCameraFrameDistance:SetValue(300)")
        .unwrap();
    assert_eq!(s.cvar("KartCamDistance").as_deref(), Some("300"));
    assert_eq!(label(&s, "Distance"), "Distance at speed: 8.2 yd");
    assert_eq!(preset(&s), "Custom");
    s.run("BenillaKartCameraFramePitch:SetValue(20) BenillaKartCameraFrameYawOffset:SetValue(-30)")
        .unwrap();
    assert_eq!(s.cvar("KartCamPitch").as_deref(), Some("20"));
    assert_eq!(
        label(&s, "YawOffset"),
        "Angle round the kart: 30\u{b0} left"
    );
    assert_eq!(camera_tuning(&s).pitch, 20.0);
    s.run("BenillaKartCameraFrameTravel:Click() BenillaKartCameraFrameFarCamera:Click()")
        .unwrap();
    assert_eq!(s.cvar("KartCamTravel").as_deref(), Some("0"));
    assert_eq!(s.cvar("KartFarCamera").as_deref(), Some("1"));

    // Reset puts every camera CVar back to CTR classic's, the default; near or far is the
    // player's own.
    s.run("BenillaKartCameraFrameReset:Click()").unwrap();
    assert_eq!(camera_tuning(&s), CameraTuning::CTR);
    assert_eq!(preset(&s), "CTR classic");
    assert_eq!(
        s.eval::<f64>("return BenillaKartCameraFramePitch:GetValue()")
            .unwrap(),
        8.75,
        "the slider sits on its step nearest CTR's 8.79"
    );
    assert_eq!(s.cvar("KartFarCamera").as_deref(), Some("1"));

    // Escape closes it, as it closes any UISpecialFrames window, and the menu does not open.
    s.run("ToggleGameMenu()").unwrap();
    assert!(!shown(&s));
    assert!(!s
        .eval::<bool>("return GameMenuFrame:IsVisible() == 1")
        .unwrap());

    // The settings window's button and the minimap button's menu open it.
    s.run("BenillaKartSettings_Toggle() BenillaKartSettingsFrameCamera:Click()")
        .unwrap();
    assert!(shown(&s));
    s.run("BenillaKartCameraFrame:Hide() BenillaKartSettingsFrame:Hide()")
        .unwrap();
    menu(&mut s);
    pick(&mut s, "Kart camera...");
    assert!(shown(&s));
    s.run("BenillaKartCameraFrame:Hide()").unwrap();

    // Not over the game menu.
    s.run("ToggleGameMenu(1)").unwrap();
    assert!(s
        .eval::<bool>("return GameMenuFrame:IsVisible() == 1")
        .unwrap());
    s.run("SlashCmdList['BENILLA_KARTCAM']('') BenillaKartCamera_Toggle()")
        .unwrap();
    assert!(!shown(&s), "opened over the game menu");
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The garage's event as the client sends it, at `skill`, with three racers' karts.
fn garage(s: &mut UiScript, skill: u16) {
    s.fire_event(
        "BENILLA_KART_GARAGE",
        vec![
            ScriptValue::Number(f64::from(skill)),
            ScriptValue::Str(crate::kart::class_list()),
            ScriptValue::Str(crate::kart::racer_list(&["crash", "coco", "tiny"])),
        ],
    );
    s.tick(0.0);
}

#[test]
fn the_garage_opens_every_class_and_picks_a_kart() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    garage(&mut s, 1);
    s.take_cvar_changes();
    let eval_bool = |s: &UiScript, lua: &str| -> bool { s.eval(&format!("return {lua}")).unwrap() };
    let text = |s: &UiScript, frame: &str| -> String {
        s.eval(&format!("return {frame}:GetText()")).unwrap()
    };

    // /garage opens it at the CVars, writing nothing: Turning ticked, every class open.
    s.run("SlashCmdList['BENILLA_GARAGE']('')").unwrap();
    s.tick(0.0);
    assert!(eval_bool(&s, "BenillaKartGarageFrame:IsVisible() == 1"));
    assert_eq!(
        s.eval::<String>("return SLASH_BENILLA_GARAGE1").unwrap(),
        "/garage"
    );
    assert_eq!(text(&s, "BenillaKartGarageFrameSkill"), "Karting skill: 1");
    let names: Vec<String> = (1..=4)
        .map(|i| text(&s, &format!("BenillaKartGarageFrameClass{i}Text")))
        .collect();
    assert_eq!(names, ["Turning", "Balanced", "Acceleration", "Speed"]);
    assert!(eval_bool(
        &s,
        "BenillaKartGarageFrameClass1:GetChecked() == 1"
    ));
    assert!(eval_bool(
        &s,
        "BenillaKartGarageFrameClass1:IsEnabled() == 1"
    ));
    assert_eq!(
        text(&s, "BenillaKartGarageFrameClass1About"),
        "Speed 1/4, acceleration 3/4, turning 4/4"
    );
    for i in 2..=4 {
        assert!(
            eval_bool(
                &s,
                &format!("BenillaKartGarageFrameClass{i}:IsEnabled() == 1")
            ),
            "class {i} is open at skill 1"
        );
        assert!(text(&s, &format!("BenillaKartGarageFrameClass{i}About")).starts_with("Speed "));
    }
    assert!(
        s.take_cvar_changes().is_empty(),
        "showing the garage wrote a CVar"
    );

    // The fastest class is picked at skill 1.
    s.run("BenillaKartGarage_PickClass(4)").unwrap();
    assert_eq!(s.cvar("KartClass").as_deref(), Some("speed"));
    s.run("BenillaKartGarageFrameClass3:Click()").unwrap();
    assert_eq!(s.cvar("KartClass").as_deref(), Some("accel"));
    assert!(eval_bool(
        &s,
        "BenillaKartGarageFrameClass3:GetChecked() == 1"
    ));
    assert!(!eval_bool(
        &s,
        "BenillaKartGarageFrameClass1:GetChecked() == 1"
    ));

    // At any skill the pick is what the kart drives.
    s.run("BenillaKartGarageFrameClass4:Click()").unwrap();
    garage(&mut s, 300);
    assert_eq!(s.cvar("KartClass").as_deref(), Some("speed"));
    assert!(eval_bool(
        &s,
        "BenillaKartGarageFrameClass4:GetChecked() == 1"
    ));

    // The karts by racer, ours ticked; a click picks another, which the host hears.
    assert_eq!(
        text(&s, "BenillaKartGarageFrameRacer2Text"),
        "Coco Bandicoot"
    );
    assert!(eval_bool(
        &s,
        "BenillaKartGarageFrameRacer1:GetChecked() == 1"
    ));
    assert!(!eval_bool(
        &s,
        "BenillaKartGarageFrameRacer4:IsShown() == 1"
    ));
    s.take_cvar_changes();
    s.run("BenillaKartGarageFrameRacer2:Click()").unwrap();
    assert_eq!(s.cvar("KartRacer").as_deref(), Some("coco"));
    assert!(eval_bool(
        &s,
        "BenillaKartGarageFrameRacer2:GetChecked() == 1"
    ));
    assert!(!eval_bool(
        &s,
        "BenillaKartGarageFrameRacer1:GetChecked() == 1"
    ));
    assert!(s
        .take_cvar_changes()
        .iter()
        .any(|(n, v)| n.eq_ignore_ascii_case("KartRacer") && v == "coco"));

    s.run("SlashCmdList['BENILLA_GARAGE']('')").unwrap();
    assert!(!eval_bool(&s, "BenillaKartGarageFrame:IsVisible() == 1"));
}

#[test]
fn the_race_marshal_opens_the_finder() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    let open = |s: &UiScript| -> bool {
        s.eval("return BenillaKartFinderFrame:IsVisible() == 1")
            .unwrap()
    };
    assert!(!open(&s));
    from_server(&mut s, "R\tFinder");
    assert!(open(&s));
    // Again: it stays open.
    from_server(&mut s, "R\tFinder");
    assert!(open(&s));
}

/// The kart's controls are the layer's `Bindings.xml`: Key Bindings lists them as a "World of
/// Kartcraft" section, each row by its name, and a row's binding runs the window it names.
#[test]
fn key_bindings_lists_the_kart_s_controls_and_they_run() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.run("BenillaKeyBindings_BuildSections()").unwrap();
    let (label, names): (String, String) = s
        .eval(
            r#"for _, sec in ipairs(BenillaKeyBindingsPage.sections) do
                if sec.token == "BINDING_HEADER_KARTCRAFT" then
                    local names = {}
                    for _, i in ipairs(sec.indices) do
                        local command = GetBinding(i)
                        table.insert(names, BenillaKeyBindings_String("BINDING_NAME_" .. command, command))
                    end
                    return BenillaKeyBindings_String(sec.token, sec.fallback), table.concat(names, ",")
                end
            end
            return "", """#,
        )
        .unwrap();
    assert_eq!(label, "World of Kartcraft");
    assert_eq!(
        names,
        "Accelerate,Brake / Reverse,Steer Left,Steer Right,Reverse,Hop / Powerslide,\
         Boost (Second Shoulder),Fire Item,Toggle Camera Distance,Look Back,Wumpa Juice,Turbo Boost,Super Engine,Aku Aku,\
         Summon / Dismiss Kart,Race Finder,Leave Race,Unstuck,Restart Time Trial,Courier Jobs,Kartcraft Settings,Kart Garage,\
         Clean Screen (hide all interface),Free Camera (fly, park, off),Next Kart,\
         Kartcraft Guide,Kartcraft Menu"
    );
    s.run("RunBinding('KARTCRAFT_GARAGE')").unwrap();
    assert!(s
        .eval::<bool>("return BenillaKartGarageFrame:IsVisible() == 1")
        .unwrap());
    s.run("RunBinding('KARTCRAFT_RACEFINDER')").unwrap();
    assert!(s
        .eval::<bool>("return BenillaKartFinderFrame:IsVisible() == 1")
        .unwrap());
    // Unstuck asks the server as the menu's entry does.
    sent(&mut s);
    s.run("RunBinding('KARTCRAFT_UNSTUCK')").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT	R	U"]);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// The race banner is HUD text at LOW: it draws before every DIALOG window, and the windows' own
/// ground (the 0.6-alpha dialog tile under a 0.7 dim) keeps it from reading through.
#[test]
fn the_race_banner_draws_under_the_kart_windows() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.fire_event(
        "BENILLA_KART_RACE",
        vec![
            ScriptValue::Str("Road to Stormwind starts here".into()),
            ScriptValue::Str(String::new()),
        ],
    );
    s.tick(0.0);
    for window in [
        "BenillaKartGarageFrame",
        "BenillaKartFinderFrame",
        "BenillaKartSettingsFrame",
        "BenillaKartCameraFrame",
    ] {
        s.run(&format!("{window}:Show()")).unwrap();
        s.tick(0.0);
        s.resolve();
        let list = s.extract();
        let at = |pred: &dyn Fn(&benilla_ui::script::ExtractedQuad) -> bool| {
            list.iter()
                .position(pred)
                .unwrap_or_else(|| panic!("{window}: an entry is not drawn"))
        };
        let banner = at(&|q| {
            matches!(&q.content, benilla_ui::script::QuadContent::Text { text: Some(t), .. }
                if t.starts_with("Road to Stormwind"))
        });
        let ground = at(&|q| {
            matches!(q.content, benilla_ui::script::QuadContent::Frame)
                && s.target_owner_name(q.target).as_deref() == Some(window)
        });
        assert!(
            banner < ground,
            "{window}: the banner draws before the window"
        );
        assert_eq!(
            benilla_ui::order::unpack(list[banner].z).strata,
            benilla_ui::order::Strata::Low.index()
        );
        // The dim: a colour texture over the window's backdrop, before its art.
        let dim = at(&|q| {
            matches!(
                q.content,
                benilla_ui::script::QuadContent::Texture {
                    path: None,
                    color: Some(_),
                    ..
                }
            ) && s.target_owner_name(q.target).as_deref() == Some(window)
        });
        let benilla_ui::script::QuadContent::Texture { color: Some(c), .. } = list[dim].content
        else {
            unreachable!()
        };
        let a = f64::from(c[3]);
        // 1 - (1 - 0.6) * (1 - a): the window's ground covers the HUD at 0.88.
        assert!(1.0 - 0.4 * (1.0 - a) >= 0.88 - 1e-6, "{window}: ground {a}");
        s.run(&format!("{window}:Hide()")).unwrap();
    }
}

/// The game menu closes the kart windows as it closes any UISpecialFrames window, from ESC or the
/// micro button (`UIParent.lua:1465-1497`), and while it holds the centre they do not open over
/// it, as a stock panel does not (`UIParent.lua:654-656`).
#[test]
fn the_game_menu_closes_the_kart_windows_and_they_do_not_open_over_it() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let visible = |s: &UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };

    // ESC with the garage open closes the garage and stops there.
    s.run("BenillaKartGarage_Toggle()").unwrap();
    assert!(visible(&s, "BenillaKartGarageFrame"));
    s.run("ToggleGameMenu()").unwrap();
    assert!(
        !visible(&s, "BenillaKartGarageFrame"),
        "ESC closed the garage"
    );
    assert!(!visible(&s, "GameMenuFrame"), "…and did not open the menu");

    // The micro button's click closes the finder and opens the menu.
    s.run("BenillaKartFinder_Toggle()").unwrap();
    assert!(visible(&s, "BenillaKartFinderFrame"));
    s.run("ToggleGameMenu(1)").unwrap();
    assert!(!visible(&s, "BenillaKartFinderFrame"));
    assert!(visible(&s, "GameMenuFrame"));

    // With the menu up, no binding, button or Race Marshal opens one over it.
    s.run(
        "BenillaKartGarage_Toggle() BenillaKartSettings_Toggle() BenillaKartFinder_Toggle() \
         BenillaKartMinimapButton:Click()",
    )
    .unwrap();
    from_server(&mut s, "R	Finder");
    for window in [
        "BenillaKartGarageFrame",
        "BenillaKartSettingsFrame",
        "BenillaKartFinderFrame",
    ] {
        assert!(!visible(&s, window), "{window} opened over the game menu");
    }

    // The menu's ESC closes it, and the garage opens again.
    s.run("ToggleGameMenu()").unwrap();
    assert!(!visible(&s, "GameMenuFrame"));
    s.run("BenillaKartGarage_Toggle()").unwrap();
    assert!(visible(&s, "BenillaKartGarageFrame"));
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Two tracks in Elwynn Forest, as the client sends them: a 3-lap circuit and a sprint.
fn tracks(s: &mut UiScript) {
    let list = [
        "t1\tCrystal Lake Loop\tElwynn Forest\t0\t3\tA 3-lap circuit in Elwynn Forest.\t-9000.0,-100.0;-9100.0,0.0;-9050.0,100.0",
        "t2\tGoldshire Dash\tElwynn Forest\t0\t0\tA sprint in Elwynn Forest.\t-8950.0,-150.0;-9150.0,150.0",
    ];
    s.fire_event(
        "BENILLA_KART_TRACKS",
        vec![ScriptValue::Str(list.join("\n"))],
    );
    s.tick(0.0);
}

/// A map with Elwynn Forest on it: the catalog, and a projection of the tracks' corner of it.
fn elwynn_map(s: &mut UiScript) {
    use benilla_ui::script::{WorldMapContinentView, WorldMapZoneView};
    let zone = |name: &str| WorldMapZoneView {
        name: name.into(),
        ..Default::default()
    };
    s.set_world_map_catalog(vec![
        WorldMapContinentView {
            name: "Kalimdor".into(),
            zones: vec![zone("Durotar")],
            ..Default::default()
        },
        WorldMapContinentView {
            name: "Eastern Kingdoms".into(),
            zones: vec![zone("Elwynn Forest")],
            ..Default::default()
        },
    ]);
    s.set_world_loc_projector(Box::new(|selection, map, x, y| {
        let (u, v) = ((250.0 - y) / 500.0, (-8900.0 - x) / 300.0);
        (selection.0 == 2
            && selection.1 == 1
            && map == 0
            && (0.0..=1.0).contains(&u)
            && (0.0..=1.0).contains(&v))
        .then_some((u, v))
    }));
}

/// How many of the route's dots are on the map.
fn dots_shown(s: &UiScript) -> u32 {
    s.eval(
        "local n = 0 for i = 1, table.getn(BenillaKartRoute.dots) do \
         if BenillaKartRoute.dots[i]:IsVisible() then n = n + 1 end end return n",
    )
    .unwrap()
}

#[test]
fn the_track_browser_shows_a_route_on_the_map_and_clears_it() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    tracks(&mut s);
    elwynn_map(&mut s);

    // The Race Marshal's "Browse the tracks" opens it at her course.
    from_server(&mut s, "R\tTracks\tt2");
    assert!(s
        .eval::<bool>("return BenillaKartTracksFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartTracksFrameTitle:GetText()")
            .unwrap(),
        "Goldshire Dash"
    );
    assert_eq!(
        s.eval::<String>("return BenillaKartTracksFrameText:GetText()")
            .unwrap(),
        "A sprint in Elwynn Forest."
    );

    // Show on map: the world map opens at Elwynn Forest, with the trail and its start.
    s.run("BenillaKartTracksFrameShow:Click()").unwrap();
    s.tick(0.0);
    assert!(s
        .eval::<bool>("return WorldMapFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<(i64, i64)>("return GetCurrentMapContinent(), GetCurrentMapZone()")
            .unwrap(),
        (2, 1)
    );
    let sprint = dots_shown(&s);
    assert!(sprint > 10, "the sprint's trail: {sprint} dots");
    assert!(s
        .eval::<bool>("return BenillaKartRouteStart:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartTracksFrameShow:GetText()")
            .unwrap(),
        "Shown on the map"
    );

    // Another track takes the first one's place, not a second route beside it.
    s.run("BenillaKartTrackBrowser_Select('t1') BenillaKartTracksFrameShow:Click()")
        .unwrap();
    s.tick(0.0);
    assert_eq!(
        s.eval::<String>("return BenillaKartRoute.key").unwrap(),
        "t1"
    );
    let circuit = dots_shown(&s);
    assert!(
        circuit > 10 && circuit != sprint,
        "the circuit's own trail: {circuit} dots"
    );

    // Closing the map takes the route away, and the browser is back on the screen; opening the
    // map again shows no route.
    s.run("ToggleWorldMap()").unwrap();
    s.tick(0.0);
    assert!(s
        .eval::<bool>("return BenillaKartTracksFrame:IsVisible() == 1")
        .unwrap());
    assert!(!s
        .eval::<bool>("return WorldMapFrame:IsVisible() == 1")
        .unwrap());
    assert!(s
        .eval::<bool>("return BenillaKartRoute.key == nil")
        .unwrap());
    s.run("ToggleWorldMap()").unwrap();
    s.tick(0.0);
    assert_eq!(dots_shown(&s), 0, "no route left over");
    assert!(!s
        .eval::<bool>("return BenillaKartRouteStart:IsVisible() == 1")
        .unwrap());

    // A map opened by hand closes the open windows, as the stock map does; the browser opens
    // again, and its search narrows the list by name or zone.
    s.run("ToggleWorldMap() BenillaKartTrackBrowser_Open()")
        .unwrap();
    s.tick(0.0);
    s.run("BenillaKartTracksFrameSearch:SetText('dash')")
        .unwrap();
    s.tick(0.0);
    assert!(s.eval::<bool>("return BenillaKartTracksFrameRow1:IsVisible() == 1 and not BenillaKartTracksFrameRow2:IsVisible()").unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartTracksFrameRow1Name:GetText()")
            .unwrap(),
        "Goldshire Dash"
    );
}

#[test]
fn unstuck_asks_the_server_and_says_why_it_waits() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.run("SlashCmdList['BENILLA_UNSTUCK']('')").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tU"]);
    // The server's two refusals reach the chat.
    s.run(
        "BenillaKartSaid = {} local said = BenillaKart_Say \
         BenillaKart_Say = function(t) table.insert(BenillaKartSaid, t) said(t) end",
    )
    .unwrap();
    from_server(&mut s, "R\tUx\t17");
    from_server(&mut s, "R\tUn");
    assert_eq!(
        s.eval::<Vec<String>>("return BenillaKartSaid").unwrap(),
        [
            "Unstuck: you can use it again in 17 seconds.",
            "Unstuck: no course starts within 300 yards.",
        ]
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

#[test]
fn the_race_map_draws_the_track_and_the_racers_on_it() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let track = |s: &mut UiScript, name: &str, points: &str| {
        s.fire_event(
            "BENILLA_KART_TRACKMAP",
            vec![
                ScriptValue::Str(name.into()),
                ScriptValue::Number(3.0),
                ScriptValue::Str(points.into()),
            ],
        );
        s.tick(0.0);
    };
    let shown = |s: &UiScript| -> bool {
        s.eval("return BenillaKartMapFrame:IsVisible() == 1")
            .unwrap()
    };

    // A square circuit, 100 yd a side: the frame shows, titled, a trail of dots round it.
    track(&mut s, "Elwynn Loop", "0,0;100,0;100,100;0,100");
    assert!(shown(&s));
    assert_eq!(
        s.eval::<String>("return BenillaKartMapFrameTitle:GetText()")
            .unwrap(),
        "Elwynn Loop"
    );
    let route: u32 = s
        .eval(
            "local n = 0 for i = 1, table.getn(BenillaKartMap.route) do \
             if BenillaKartMap.route[i]:IsVisible() then n = n + 1 end end return n",
        )
        .unwrap();
    assert!(route > 40, "only {route} dots of route");

    // Us at the start, a rival beside us, one far off at a checkpoint: dimmed.
    s.fire_event(
        "BENILLA_KART_RACERS",
        vec![ScriptValue::Str(
            "Probesix\t0\t0\t1\t1\t1\nGoblin\t0\t10\t2\t0\t1\nGnome\t100\t100\t3\t0\t0".into(),
        )],
    );
    s.tick(0.0);
    let (x, y): (f64, f64) = (
        s.eval("local _, _, _, x = BenillaKartMap.me:GetPoint(1) return x")
            .unwrap(),
        s.eval("local _, _, _, _, y = BenillaKartMap.me:GetPoint(1) return y")
            .unwrap(),
    );
    assert!(
        (0.0..=172.0).contains(&x) && (-172.0..=0.0).contains(&y),
        "me at {x},{y}"
    );
    assert!(s
        .eval::<bool>("return BenillaKartMap.me:IsVisible() == 1")
        .unwrap());
    // Slot 1 is us; 2 the rival, 3 the far one.
    assert_eq!(
        s.eval::<f64>("return BenillaKartMap.racers[2]:GetAlpha()")
            .unwrap(),
        1.0
    );
    assert!(
        s.eval::<f64>("return BenillaKartMap.racers[3]:GetAlpha()")
            .unwrap()
            < 0.5
    );
    assert_eq!(
        s.eval::<String>("return BenillaKartMap.labels[3]:GetText()")
            .unwrap(),
        "3"
    );

    // A smaller field hides the rest; no race, no map.
    s.fire_event(
        "BENILLA_KART_RACERS",
        vec![ScriptValue::Str("Goblin\t0\t10\t1\t0\t1".into())],
    );
    s.tick(0.0);
    assert!(!s
        .eval::<bool>("return BenillaKartMap.me:IsVisible() == 1")
        .unwrap());
    assert!(!s
        .eval::<bool>("return BenillaKartMap.racers[3]:IsVisible() == 1")
        .unwrap());
    track(&mut s, "", "");
    assert!(!shown(&s));
    assert!(s
        .eval::<bool>("return BenillaKartMapFrame:IsMovable() == 1")
        .unwrap());
}

#[test]
fn the_results_board_stands_clear_of_the_race_map_and_the_banner() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    // The race map up, then the tallest board: eight racers, three laps, the record, our best
    // and the ground types (14 lines), as the smoke run's finish showed them over the map.
    s.fire_event(
        "BENILLA_KART_TRACKMAP",
        vec![
            ScriptValue::Str("Elwynn Loop".into()),
            ScriptValue::Number(3.0),
            ScriptValue::Str("0,0;100,0;100,100;0,100".into()),
        ],
    );
    let mut lines: Vec<String> = (1..=8)
        .map(|i| format!("{i}. Racer{i}  1:0{i}.00"))
        .collect();
    lines.extend((1..=3).map(|i| format!("Lap {i}: 0:20.00")));
    lines.extend([
        "Course record: 1:00.00 by Racer1".to_string(),
        "Your best: 1:00.00".to_string(),
        "Ground types: on".to_string(),
    ]);
    s.fire_event(
        "BENILLA_KART_RACE",
        vec![
            ScriptValue::Str("1ST — 1:01.00".into()),
            ScriptValue::Str(lines.join("\n")),
        ],
    );
    s.tick(0.0);
    let rect = |s: &UiScript, frame: &str| -> [f64; 4] {
        ["GetLeft", "GetRight", "GetBottom", "GetTop"]
            .map(|m| s.eval::<f64>(&format!("return {frame}:{m}()")).unwrap())
    };
    for frame in ["BenillaKartMapFrame", "BenillaKartResultsFrame"] {
        assert!(
            s.eval::<bool>(&format!("return {frame}:IsVisible() == 1"))
                .unwrap(),
            "{frame} is not up"
        );
    }
    let [ml, mr, mb, mt] = rect(&s, "BenillaKartMapFrame");
    let [rl, rr, rb, rt] = rect(&s, "BenillaKartResultsFrame");
    assert!(
        rr <= ml || mr <= rl || rt <= mb || mt <= rb,
        "the board {:?} overlaps the map {:?}",
        [rl, rr, rb, rt],
        [ml, mr, mb, mt]
    );
    // Whole on screen, and under the banner's line.
    let (w, h): (f64, f64) = (
        s.eval("return UIParent:GetWidth()").unwrap(),
        s.eval("return UIParent:GetHeight()").unwrap(),
    );
    assert!(
        rl >= 0.0 && rr <= w && rb >= 0.0 && rt <= h,
        "{:?}",
        [rl, rr, rb, rt]
    );
    let banner_bottom: f64 = s.eval("return BenillaKartRaceBanner:GetBottom()").unwrap();
    assert!(
        rt <= banner_bottom,
        "board top {rt} over the banner at {banner_bottom}"
    );
}

#[test]
fn race_mode_is_asked_of_the_server_and_its_answer_ticks_the_menu() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let ticked = |s: &mut UiScript| -> bool {
        menu(s);
        s.eval(
            "for i = 1, UIDROPDOWNMENU_MAXBUTTONS do \
             local b = getglobal('DropDownList1Button' .. i) \
             if b and b:IsVisible() and b:GetText() == 'Race mode' then \
             return getglobal(b:GetName() .. 'Check'):IsVisible() == 1 end end return false",
        )
        .unwrap()
    };
    s.run(
        "BenillaKartSaid = {} local said = BenillaKart_Say \
         BenillaKart_Say = function(t) table.insert(BenillaKartSaid, t) said(t) end",
    )
    .unwrap();

    // Off until the server says on; the menu asks, the answer ticks it.
    assert!(!ticked(&mut s));
    pick(&mut s, "Race mode");
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tM\t1"]);
    from_server(&mut s, "R\tM\t1");
    assert!(ticked(&mut s));
    // On, the entry asks for it off.
    pick(&mut s, "Race mode");
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tM\t0"]);
    from_server(&mut s, "R\tM\t0");
    assert!(!ticked(&mut s));
    // Refused in combat: still off.
    from_server(&mut s, "R\tMx");
    assert!(!ticked(&mut s));
    assert_eq!(
        s.eval::<Vec<String>>("return BenillaKartSaid").unwrap(),
        [
            "Race mode on: nobody can target or attack you, and you can't attack anyone.",
            "Race mode off.",
            "Race mode: not while you're in combat.",
        ]
    );
}

#[test]
fn the_controller_test_lights_the_held_buttons_and_says_when_it_is_open() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let cvar = |s: &mut UiScript, name: &str| -> String {
        s.eval(&format!("return GetCVar('{name}')")).unwrap()
    };
    s.run("SlashCmdList[\"BENILLA_KARTPAD\"](\"\")").unwrap();
    assert!(s
        .eval::<bool>("return BenillaKartPadFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(cvar(&mut s, "KartPadTest"), "1");
    assert_eq!(
        s.eval::<String>("return BenillaKartPadFrameDevice:GetText()")
            .unwrap(),
        "No controller found: connect one and press a button."
    );

    let n = ScriptValue::Number;
    s.fire_event(
        "BENILLA_KART_PAD",
        vec![
            ScriptValue::Str("DualSense Wireless Controller".into()),
            ScriptValue::Str("South RightTrigger".into()),
            n(-1.0),
            n(0.5),
            n(0.0),
            n(0.0),
            n(0.0),
            n(1.0),
        ],
    );
    let green = |s: &mut UiScript, token: &str| -> bool {
        s.eval(&format!(
            "local r, g = BenillaKartPadFrame.rows['{token}'].name:GetTextColor() \
             return g > 0.9 and r < 0.5"
        ))
        .unwrap()
    };
    assert!(green(&mut s, "South") && green(&mut s, "RightTrigger"));
    assert!(!green(&mut s, "East") && !green(&mut s, "LeftTrigger"));
    assert_eq!(
        s.eval::<String>("return BenillaKartPadFrameDevice:GetText()")
            .unwrap(),
        "DualSense Wireless Controller"
    );
    let sticks: String = s
        .eval("return BenillaKartPadFrame.sticks:GetText()")
        .unwrap();
    assert!(
        sticks.starts_with("Left stick -1.00, +0.50") && sticks.ends_with("R2 1.00"),
        "{sticks}"
    );
    // Every row names what the button does, or that it does nothing.
    assert_eq!(
        s.eval::<String>("return BenillaKartPadFrame.rows['South'].what:GetText()")
            .unwrap(),
        "Gas"
    );

    // A rumble button shakes once and leaves the CVar ready for the next.
    s.run("BenillaKartPadFrameStrong:Click()").unwrap();
    assert_eq!(cvar(&mut s, "KartPadRumble"), "0");
    s.run("BenillaKartPadFrame:Hide()").unwrap();
    assert_eq!(cvar(&mut s, "KartPadTest"), "0");
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_world_channel_is_joined_at_login_until_the_player_leaves_it() {
    use benilla_ui::script::ChannelCommand;
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let joins = |s: &mut UiScript| -> Vec<String> {
        s.take_channel_commands()
            .into_iter()
            .filter_map(|c| match c {
                ChannelCommand::Join { name, .. } => Some(name),
                _ => None,
            })
            .collect()
    };
    let notice = |s: &mut UiScript, what: &str| {
        let mut args = vec![ScriptValue::Str(what.into())];
        args.extend((2..=8).map(|_| ScriptValue::Str(String::new())));
        args.push(ScriptValue::Str("World".into()));
        s.fire_event("CHAT_MSG_CHANNEL_NOTICE", args);
    };
    s.run("SetCVar('KartWorldChat', '1') BenillaKartWorldJoined = nil")
        .unwrap();
    joins(&mut s);

    // On by default: the login joins it into the main window; a zone-in does not again.
    s.fire_event("PLAYER_ENTERING_WORLD", Vec::new());
    assert_eq!(joins(&mut s), ["World"]);
    assert!(s
        .eval::<bool>(
            "for _, c in DEFAULT_CHAT_FRAME.channelList do if c == 'World' then return true end end \
             return false"
        )
        .unwrap());
    s.fire_event("PLAYER_ENTERING_WORLD", Vec::new());
    assert!(joins(&mut s).is_empty());
    // Left: the next session's login keeps out of it; a /join brings it back.
    notice(&mut s, "YOU_LEFT");
    s.run("BenillaKartWorldJoined = nil").unwrap();
    s.fire_event("PLAYER_ENTERING_WORLD", Vec::new());
    assert!(joins(&mut s).is_empty());
    notice(&mut s, "YOU_JOINED");
    assert_eq!(
        s.eval::<String>("return GetCVar('KartWorldChat')").unwrap(),
        "1"
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_controller_drives_the_kartcraft_menu_and_answers_its_popups() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let press = |s: &mut UiScript, key: &str| -> bool {
        let taken = s
            .eval(&format!("return BenillaPad_OnButton('{key}') == true"))
            .unwrap();
        s.tick(0.0);
        taken
    };
    let owns = |s: &mut UiScript| -> String { s.eval("return BenillaPad_Owns()").unwrap() };
    let focus = |s: &mut UiScript| -> String {
        s.eval("return BenillaPad.focus and BenillaPad.focus:GetName() or ''")
            .unwrap()
    };
    let shown = |s: &mut UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };

    // Nothing open: a press is the binding's, and the kart's buttons are its own.
    assert!(!press(&mut s, "PADDDOWN"));
    assert_eq!(owns(&mut s), "");

    // The menu: the first press finds the top entry, the D-pad moves, Cross chooses.
    s.run("BenillaKartMenu_Toggle()").unwrap();
    s.tick(0.0);
    assert_eq!(owns(&mut s), "ALL");
    assert!(press(&mut s, "PADDDOWN"));
    assert_eq!(focus(&mut s), "BenillaKartMenuFrameEntry1");
    assert!(press(&mut s, "PADDDOWN"));
    assert_eq!(focus(&mut s), "BenillaKartMenuFrameEntry2");
    assert!(press(&mut s, "PADDUP"));
    // Down to the fourth, Race Finder (after Find a race and the two challenges).
    for _ in 0..3 {
        assert!(press(&mut s, "PADDDOWN"));
    }
    assert_eq!(focus(&mut s), "BenillaKartMenuFrameEntry4");
    assert!(press(&mut s, "PAD1"));
    assert!(!shown(&mut s, "BenillaKartMenuFrame"));
    assert!(
        shown(&mut s, "BenillaKartFinderFrame"),
        "Race Finder chosen"
    );
    // Its window takes the controller in turn; Circle closes it and gives the pad back.
    assert!(press(&mut s, "PADDDOWN"));
    assert!(focus(&mut s).starts_with("BenillaKartFinderFrame"));
    assert!(press(&mut s, "PAD2"));
    assert!(!shown(&mut s, "BenillaKartFinderFrame"));
    assert_eq!(owns(&mut s), "");
    assert_eq!(focus(&mut s), "");

    // The way back takes Triangle and Square only, while it is up: Cross, likely still held
    // from driving, answers nothing.
    sent(&mut s);
    s.run("StaticPopup_Show('BENILLA_KART_RETURN')").unwrap();
    assert_eq!(owns(&mut s), "PAD4 PAD3");
    assert!(!press(&mut s, "PAD1"));
    assert!(press(&mut s, "PAD4"));
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tBack"]);
    assert_eq!(owns(&mut s), "");
    s.run("StaticPopup_Show('BENILLA_KART_RETURN')").unwrap();
    assert!(press(&mut s, "PAD3"));
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tStay"]);
    // The race-ready popup keeps Cross and Circle.
    s.run("StaticPopup_Show('BENILLA_KART_READY', 'Elwynn Loop', 2)")
        .unwrap();
    assert_eq!(owns(&mut s), "PAD1 PAD2");
    s.run("StaticPopup_Hide('BENILLA_KART_READY')").unwrap();
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_menu_names_the_challenges_and_opens_their_course() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.fire_event(
        "BENILLA_KART_COURSE_NAMES",
        vec![ScriptValue::Str(
            "stormwind\tRoad to Stormwind\nsprint\tNorthshire Sprint".into(),
        )],
    );
    sent(&mut s);
    s.run("BenillaKartMenu_Toggle()").unwrap();
    // Opening the menu asks the server; its answer names the two courses.
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tCH"]);
    from_server(&mut s, "R\tCH\tstormwind\tsprint\t0\tGoblin\t62400\t0\t\t0");
    let entry = |s: &mut UiScript, i: u32| -> String {
        s.eval(&format!("return BenillaKartMenuFrameEntry{i}:GetText()"))
            .unwrap()
    };
    assert_eq!(entry(&mut s, 2), "Daily: Road to Stormwind");
    assert_eq!(entry(&mut s, 3), "Weekly: Northshire Sprint");
    // Choosing the day's opens the track browser on its course, for the time trial.
    s.run("BenillaKartMenuFrameEntry2:Click()").unwrap();
    assert!(s
        .eval::<bool>("return BenillaKartTracksFrame:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartTrackBrowser.selected")
            .unwrap(),
        "stormwind"
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_garage_shows_the_look_and_wears_only_what_is_earned() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let text = |s: &mut UiScript, name: &str| -> String {
        s.eval(&format!("return {name}:GetText() or ''")).unwrap()
    };
    sent(&mut s);
    s.run("BenillaKartGarage_Toggle()").unwrap();
    // Opening the garage asks for our numbers.
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tCO"]);
    from_server(&mut s, "R\tCC\tP\tStock:0:0;Copper:0:10;Gold:1:25");
    from_server(&mut s, "R\tCC\tT\tNone:0:0;Sparks:0:25");
    from_server(&mut s, "R\tCO\t12,3,0,0,0,0\t0\t0");
    assert_eq!(
        text(&mut s, "BenillaKartGarageFramePaintName"),
        "Stock (worn)"
    );
    assert_eq!(text(&mut s, "BenillaKartGarageFramePaintNeed"), "");
    // Copper is earned (12 races of 10): stepping onto it wears it.
    s.run("BenillaKartGarageFramePaintNext:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tCE\tP\t1"]);
    assert_eq!(text(&mut s, "BenillaKartGarageFramePaintName"), "Copper");
    // Gold is not (3 wins of 25): shown with what it needs, and not asked for.
    s.run("BenillaKartGarageFramePaintNext:Click()").unwrap();
    assert!(sent(&mut s).is_empty());
    assert_eq!(text(&mut s, "BenillaKartGarageFramePaintName"), "Gold");
    assert_eq!(
        text(&mut s, "BenillaKartGarageFramePaintNeed"),
        "Needs 25 wins (3 so far)"
    );
    // Round past the end to Stock; the trail row reads its own list.
    s.run("BenillaKartGarageFrameTrailPrev:Click()").unwrap();
    assert_eq!(text(&mut s, "BenillaKartGarageFrameTrailName"), "Sparks");
    assert_eq!(
        text(&mut s, "BenillaKartGarageFrameTrailNeed"),
        "Needs 25 races (12 so far)"
    );
    // The server's answer is what we wear, and the kart's CVars follow it.
    from_server(&mut s, "R\tCO\t12,3,0,0,0,0\t1\t0");
    assert_eq!(
        text(&mut s, "BenillaKartGarageFramePaintName"),
        "Copper (worn)"
    );
    assert_eq!(
        s.eval::<String>("return GetCVar('KartPaint')").unwrap(),
        "1"
    );
    assert_eq!(
        s.eval::<String>("return GetCVar('KartTrail')").unwrap(),
        "0"
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_settings_pick_the_controller_among_those_connected() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.run("SetCVar('KartPads', '0000:0000:|Controller 0000:0000;054c:0ce6:PS5 Controller|PS5 Controller')")
        .unwrap();
    s.run("BenillaKartSettings_Toggle()").unwrap();
    let label = |s: &mut UiScript| -> String {
        s.eval("return UIDropDownMenu_GetText(BenillaKartSettingsFramePad) or ''")
            .unwrap()
    };
    assert_eq!(label(&mut s), "Any controller");
    s.run("BenillaKartSettingsFramePadButton:Click()").unwrap();
    s.tick(0.0);
    pick(&mut s, "PS5 Controller");
    assert_eq!(
        s.eval::<String>("return GetCVar('KartPad')").unwrap(),
        "054c:0ce6:PS5 Controller"
    );
    assert_eq!(label(&mut s), "PS5 Controller");
    // Chosen but unplugged: the window says every pad drives meanwhile.
    s.run("SetCVar('KartPads', '0000:0000:|Controller 0000:0000') BenillaKartSettings_Refresh()")
        .unwrap();
    assert_eq!(label(&mut s), "Not connected (any used)");
    // Back to any.
    s.run("BenillaKartSettingsFramePadButton:Click()").unwrap();
    s.tick(0.0);
    pick(&mut s, "Any controller");
    assert_eq!(s.eval::<String>("return GetCVar('KartPad')").unwrap(), "");
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn a_track_is_rated_after_a_race_and_a_closed_one_leaves_the_browser() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let shown = |s: &mut UiScript, frame: &str| {
        s.eval::<bool>(&format!("return {frame}:IsVisible() == 1"))
            .unwrap()
    };
    s.fire_event(
        "BENILLA_KART_TRACKS",
        vec![ScriptValue::Str(
            "t1\tGoldshire Loop\tElwynn Forest\t0\t3\tA loop.\t0.0,0.0;10.0,10.0\nt2\tCrystal Lake\tElwynn Forest\t0\t0\tA sprint.\t0.0,0.0;10.0,10.0"
                .into(),
        )],
    );
    sent(&mut s);
    // The server asks after a race: a good vote goes back at once.
    from_server(&mut s, "R\tRateAsk\tt1");
    assert!(shown(&mut s, "BenillaKartRateFrame"));
    s.run("BenillaKartRateFrameGood:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tRate\tt1\t1\t"]);
    assert!(!shown(&mut s, "BenillaKartRateFrame"));
    // A bad one asks why, then sends it.
    from_server(&mut s, "R\tRateAsk\tt2");
    s.run("BenillaKartRateFrameBad:Click()").unwrap();
    assert!(sent(&mut s).is_empty(), "nothing until the reason");
    assert!(shown(&mut s, "BenillaKartRateFrameReason1"));
    assert!(!shown(&mut s, "BenillaKartRateFrameGood"));
    s.run("BenillaKartRateFrameReason1:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tRate\tt2\t-1\tstuck"]);
    // Not now sends nothing.
    from_server(&mut s, "R\tRateAsk\tt1");
    s.run("BenillaKartRateFrameSkip:Click()").unwrap();
    assert!(sent(&mut s).is_empty());
    // The controller: Start a good track, Select a bad one (its guide says so), then the reason
    // with the D-pad and Cross as any window's buttons; Circle is not now.
    let press = |s: &mut UiScript, key: &str| -> bool {
        s.eval(&format!("return BenillaPad_OnButton('{key}') == true"))
            .unwrap()
    };
    let guide = |s: &mut UiScript| -> String {
        s.eval::<String>("return BenillaPad_Owns()").unwrap();
        s.eval("return BenillaPadLegendFrameText:GetText() or ''")
            .unwrap()
    };
    s.run("SetCVar('KartPads', '1|Wireless Controller')")
        .unwrap();
    from_server(&mut s, "R\tRateAsk\tt1");
    assert!(guide(&mut s).starts_with("Start: good track"));
    assert!(press(&mut s, "PADFORWARD"));
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tRate\tt1\t1\t"]);
    assert!(!shown(&mut s, "BenillaKartRateFrame"));
    from_server(&mut s, "R\tRateAsk\tt2");
    assert!(press(&mut s, "PADBACK"));
    assert!(sent(&mut s).is_empty(), "the reason first");
    assert!(shown(&mut s, "BenillaKartRateFrameReason1"));
    assert!(
        !guide(&mut s).contains("Start: good track"),
        "the reasons' own guide"
    );
    assert!(press(&mut s, "PADFORWARD"), "the window's, but no vote now");
    assert!(sent(&mut s).is_empty());
    s.run("BenillaKartRateFrame:Hide()").unwrap();
    from_server(&mut s, "R\tRateAsk\tt1");
    assert!(press(&mut s, "PAD2"));
    assert!(!shown(&mut s, "BenillaKartRateFrame"), "Circle: not now");
    assert!(sent(&mut s).is_empty());
    // The browser asks which tracks are closed, and hides them.
    s.run("BenillaKartTrackBrowser_Open()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tOff"]);
    from_server(&mut s, "R\tOff\tt2");
    let keys: String = s
        .eval("local out = {} for _, t in ipairs(BenillaKartTrackBrowser_Shown()) do table.insert(out, t.key) end return table.concat(out, ',')")
        .unwrap();
    assert_eq!(keys, "t1");
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_replays_window_lists_the_kept_races_and_plays_one() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let shown = |s: &mut UiScript, frame: &str| {
        s.eval::<bool>(&format!("return {frame}:IsVisible() == 1"))
            .unwrap()
    };
    s.run("SetCVar('KartReplays','000000000002-sprint|Northshire Sprint: 1st of 2, 1:02.40;000000000001-stormwind|Road to Stormwind: did not finish')")
        .unwrap();
    s.run("BenillaKartMenu_Toggle()").unwrap();
    let entry = |s: &mut UiScript, i: u32| -> String {
        s.eval(&format!("return BenillaKartMenuFrameEntry{i}:GetText()"))
            .unwrap()
    };
    assert_eq!(entry(&mut s, 6), "Replays");
    s.run("BenillaKartMenuFrameEntry6:Click()").unwrap();
    assert!(shown(&mut s, "BenillaKartReplaysFrame"));
    let row = |s: &mut UiScript, i: u32| -> (String, bool) {
        (
            s.eval(&format!(
                "return BenillaKartReplaysFrameEntry{i}:GetText() or ''"
            ))
            .unwrap(),
            s.eval::<bool>(&format!(
                "return BenillaKartReplaysFrameEntry{i}:IsVisible() == 1"
            ))
            .unwrap(),
        )
    };
    assert_eq!(
        row(&mut s, 1),
        ("Northshire Sprint: 1st of 2, 1:02.40".to_string(), true)
    );
    assert_eq!(row(&mut s, 2).0, "Road to Stormwind: did not finish");
    assert!(!row(&mut s, 3).1, "only the kept ones");
    // Choosing one plays it; the menu then offers to stop it.
    s.run("BenillaKartReplaysFrameEntry2:Click()").unwrap();
    assert_eq!(
        s.eval::<String>("return GetCVar('KartReplay')").unwrap(),
        "000000000001-stormwind"
    );
    assert!(!shown(&mut s, "BenillaKartReplaysFrame"));
    s.run("BenillaKartMenu_Toggle()").unwrap();
    assert_eq!(entry(&mut s, 6), "Stop replay");
    s.run("BenillaKartMenuFrameEntry6:Click()").unwrap();
    assert_eq!(
        s.eval::<String>("return GetCVar('KartReplay')").unwrap(),
        "0"
    );
    // None kept: the window says so.
    s.run("SetCVar('KartReplays', '') BenillaKartReplaysFrame:Show()")
        .unwrap();
    assert!(shown(&mut s, "BenillaKartReplaysFrameEmpty"));
    assert!(!row(&mut s, 1).1);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn race_mode_is_asked_again_until_the_server_says_it_is_on() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.run("SetCVar('KartRaceMode', '1') BenillaKartRaceMode = nil")
        .unwrap();
    sent(&mut s);
    s.fire_event("PLAYER_ENTERING_WORLD", Vec::new());
    // Race mode, and (every login) what we wear on the kart.
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tM\t1", "KARTCRAFT\tR\tCO"]);
    // No answer: not again at once, again 5 s on.
    s.tick(1.0);
    assert!(sent(&mut s).is_empty());
    s.tick(4.5);
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tM\t1"]);
    // On now: no more asking.
    from_server(&mut s, "R\tM\t1");
    s.tick(6.0);
    assert!(sent(&mut s).is_empty());
}

#[test]
fn race_mode_is_asked_for_at_every_login_until_turned_off() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    // Race mode's asks at a login; the cosmetics' (every login) aside.
    let login = |s: &mut UiScript| {
        s.fire_event("PLAYER_ENTERING_WORLD", Vec::new());
        sent(s)
            .into_iter()
            .filter(|l| l != "KARTCRAFT\tR\tCO")
            .collect::<Vec<_>>()
    };
    s.run("SetCVar('KartRaceMode', '1') BenillaKartRaceMode = nil")
        .unwrap();
    sent(&mut s);

    // On by default: the first login asks; once on, a zone-in does not ask again.
    assert_eq!(login(&mut s), ["KARTCRAFT\tR\tM\t1"]);
    from_server(&mut s, "R\tM\t1");
    assert!(login(&mut s).is_empty());
    // Refused in combat at login: asked again once the fight is over.
    s.run("BenillaKartRaceMode = nil").unwrap();
    assert_eq!(login(&mut s), ["KARTCRAFT\tR\tM\t1"]);
    from_server(&mut s, "R\tMx");
    s.fire_event("PLAYER_REGEN_ENABLED", Vec::new());
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tM\t1"]);
    from_server(&mut s, "R\tM\t1");
    // Turned off (the menu entry's toggle): kept off at the next login.
    s.run("BenillaKart_RaceModeToggle()").unwrap();
    sent(&mut s);
    from_server(&mut s, "R\tM\t0");
    assert_eq!(
        s.eval::<String>("return GetCVar('KartRaceMode')").unwrap(),
        "0"
    );
    assert!(login(&mut s).is_empty());
    s.run("SetCVar('KartRaceMode', '1')").unwrap();
}

#[test]
fn the_finder_shows_the_queue_and_the_hosted_races_and_joins_one() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    let text = |s: &UiScript, name: &str| -> String {
        s.eval(&format!("return {name}:GetText() or ''")).unwrap()
    };
    let shown = |s: &UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };

    // Open, it asks for the queue and the lobbies at once.
    s.run("BenillaKartFinderFrame:Show()").unwrap();
    s.tick(0.1);
    assert!(s
        .take_addon_sends()
        .iter()
        .any(|a| a.text == "KARTCRAFT\tR\tLobby"));

    // Two wait for Elwynn; two races are hosted.
    from_server(
        &mut s,
        "R\tLobby\telwynn=2\t7:elwynn:Hosty:3;9:sprint:Gnome:1",
    );
    assert_eq!(
        text(&s, "BenillaKartFinderFrameRow2Text"),
        "Elwynn Loop  |cff40ff40(2 queued)|r"
    );
    assert_eq!(
        text(&s, "BenillaKartFinderFrameRow1Text"),
        "Northshire Sprint"
    );
    assert_eq!(text(&s, "BenillaKartFinderFrameLobbies"), "Hosted races");
    assert_eq!(
        text(&s, "BenillaKartFinderFrameLobby1Text"),
        "Elwynn Loop: Hosty, 3 racers"
    );
    assert_eq!(
        text(&s, "BenillaKartFinderFrameLobby2Text"),
        "Northshire Sprint: Gnome, 1 racer"
    );
    assert!(!shown(&s, "BenillaKartFinderFrameLobby3"));

    // Join asks the server for that race.
    sent(&mut s);
    s.run("BenillaKartFinderFrameLobby1Join:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tLJ\t7"]);

    // Nobody waiting, nothing hosted.
    from_server(&mut s, "R\tLobby\t\t");
    assert_eq!(text(&s, "BenillaKartFinderFrameRow2Text"), "Elwynn Loop");
    assert!(text(&s, "BenillaKartFinderFrameLobbies").starts_with("Hosted races: none."));
    assert!(!shown(&s, "BenillaKartFinderFrameLobby1"));
}

#[test]
fn the_minimap_menu_toggles_the_race_marshals() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let ticked = |s: &mut UiScript| -> bool {
        menu(s);
        s.eval(
            "for i = 1, UIDROPDOWNMENU_MAXBUTTONS do \
             local b = getglobal('DropDownList1Button' .. i) \
             if b and b:IsVisible() and b:GetText() == 'Show Race Marshals' then \
             return getglobal(b:GetName() .. 'Check'):IsVisible() == 1 end end return false",
        )
        .unwrap()
    };
    // On by default; the entry turns the CVar off and on again.
    assert!(ticked(&mut s));
    pick(&mut s, "Show Race Marshals");
    assert_eq!(s.cvar("KartRaceMarshals").as_deref(), Some("0"));
    assert!(!ticked(&mut s));
    pick(&mut s, "Show Race Marshals");
    assert_eq!(s.cvar("KartRaceMarshals").as_deref(), Some("1"));
    assert!(ticked(&mut s));
}

#[test]
fn kartfree_steps_the_free_camera_and_kartnext_walks_the_garage() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let cvar = |s: &UiScript, name: &str| -> String {
        s.eval(&format!("return string.lower(GetCVar('{name}'))"))
            .unwrap()
    };
    // Off, fly, tripod and back to off; a word picks a camera; a wrong word changes nothing.
    assert_eq!(cvar(&s, "KartFreeCam"), "off");
    for want in ["fly", "tripod", "off"] {
        s.run("SlashCmdList['BENILLA_KARTFREE']('')").unwrap();
        assert_eq!(cvar(&s, "KartFreeCam"), want);
    }
    s.run("SlashCmdList['BENILLA_KARTFREE'](' Orbit ')")
        .unwrap();
    assert_eq!(cvar(&s, "KartFreeCam"), "orbit");
    s.run("SlashCmdList['BENILLA_KARTFREE']('sideways')")
        .unwrap();
    assert_eq!(cvar(&s, "KartFreeCam"), "orbit");
    s.run("SlashCmdList['BENILLA_KARTFREE']('')").unwrap();
    assert_eq!(cvar(&s, "KartFreeCam"), "off");

    // /kartnext walks the garage's racers from the one driven, round to the first.
    s.fire_event(
        "BENILLA_KART_GARAGE",
        vec![
            ScriptValue::Number(0.0),
            ScriptValue::Str("turn\tTurning\t0\tEasy".into()),
            ScriptValue::Str("crash\tCrash\ncortex\tCortex\ntiny\tTiny".into()),
        ],
    );
    s.run("SetCVar('KartRacer', 'cortex')").unwrap();
    s.run("SlashCmdList['BENILLA_KARTNEXT']('')").unwrap();
    assert_eq!(cvar(&s, "KartRacer"), "tiny");
    s.run("SlashCmdList['BENILLA_KARTNEXT']('')").unwrap();
    assert_eq!(cvar(&s, "KartRacer"), "crash");
}

#[test]
fn the_clean_screen_hides_every_interface_and_alt_z_brings_it_all_back() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let s = loaded();
    let state = |s: &UiScript| -> (bool, String) {
        (
            s.eval::<bool>("return UIParent:IsShown() == 1").unwrap(),
            s.eval::<String>("return GetCVar('KartHideHud')").unwrap(),
        )
    };
    assert_eq!(state(&s), (true, "0".into()));
    s.run("SlashCmdList['BENILLA_KARTCLEAN']('')").unwrap();
    assert_eq!(state(&s), (false, "1".into()));
    // The command again brings both back.
    s.run("SlashCmdList['BENILLA_KARTCLEAN']('')").unwrap();
    assert_eq!(state(&s), (true, "0".into()));
    // Hidden by the binding, shown by WoW's own Alt+Z: the kart's HUD comes back with it.
    s.run("BenillaKart_CleanScreen()").unwrap();
    assert_eq!(state(&s), (false, "1".into()));
    s.run("UIParent:Show()").unwrap();
    assert_eq!(state(&s), (true, "0".into()));
}

#[test]
fn the_track_browser_starts_a_time_trial_on_the_track_shown() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    tracks(&mut s);
    let enabled = |s: &UiScript| -> bool {
        s.eval("return BenillaKartTracksFrameTrial:IsEnabled() == 1")
            .unwrap()
    };
    s.run("BenillaKartTrackBrowser_Open()").unwrap();
    s.tick(0.0);
    // Nothing picked: nothing to race.
    assert!(!enabled(&s));
    s.run("BenillaKartTrackBrowser_Select('t2')").unwrap();
    assert!(enabled(&s));
    sent(&mut s);
    s.run("BenillaKartTracksFrameTrial:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tTB\tt2"]);
    assert!(!s
        .eval::<bool>("return BenillaKartTracksFrame:IsVisible() == 1")
        .unwrap());

    // A party course is raced inside its dungeon, from its marshal: no time trial from here.
    s.fire_event(
        "BENILLA_KART_TRACKS",
        vec![ScriptValue::Str(
            "d1\tDeadmines Run\tThe Deadmines\t36\t0\tA sprint in the Deadmines.\t0.0,0.0;10.0,10.0".into(),
        )],
    );
    s.run("BenillaKartTrackBrowser_Open() BenillaKartTrackBrowser_Select('d1')")
        .unwrap();
    assert!(!enabled(&s));
}

#[test]
fn a_party_mate_s_queue_queues_us_and_the_party_leaves_together() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    s.run(
        "BenillaKartSaid = {} local said = BenillaKart_Say \
         BenillaKart_Say = function(t) table.insert(BenillaKartSaid, t) said(t) end",
    )
    .unwrap();

    // The leader queued the party for Elwynn: we are queued, and keep our place.
    from_server(&mut s, "R\tQp\telwynn\tLeaderguy");
    assert_eq!(
        s.eval::<String>("return BenillaKartFinder.queued").unwrap(),
        "elwynn"
    );
    s.run("BenillaKartEvents_OnUpdate()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tW"]);

    // A mate left: the party is out of the queue, and we are told.
    from_server(&mut s, "R\tQx\tparty");
    assert!(s
        .eval::<bool>("return BenillaKartFinder.queued == nil")
        .unwrap());
    assert_eq!(
        s.eval::<Vec<String>>("return BenillaKartSaid").unwrap(),
        [
            "Leaderguy queued your party in the Race Finder.",
            "Your party left the Race Finder queue.",
        ]
    );
}

#[test]
fn the_guide_opens_once_on_the_first_login_and_walks_its_pages() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let shown = |s: &UiScript| -> bool {
        s.eval("return BenillaKartGuideFrame:IsVisible() == 1")
            .unwrap()
    };
    let text = |s: &UiScript, name: &str| -> String {
        s.eval(&format!("return {name}:GetText()")).unwrap()
    };
    // The first login opens it on its first page, and marks it seen.
    s.run("SetCVar('KartGuideSeen', '0')").unwrap();
    s.fire_event("PLAYER_ENTERING_WORLD", Vec::new());
    s.tick(0.0);
    assert!(shown(&s));
    assert_eq!(
        text(&s, "BenillaKartGuideFramePageTitle"),
        "Welcome to World of Kartcraft"
    );
    assert_eq!(text(&s, "BenillaKartGuideFramePage"), "1 / 5");
    assert_eq!(text(&s, "BenillaKartGuideFrameAction"), "Climb in now");
    assert!(text(&s, "BenillaKartGuideFrameBody").contains("Press J to climb into your kart"));
    assert_eq!(
        s.eval::<String>("return GetCVar('KartGuideSeen')").unwrap(),
        "1"
    );
    // Next walks the pages; the driving page names the bound keys; the last says Done and closes.
    s.run("BenillaKartGuide_Turn(1)").unwrap();
    assert_eq!(text(&s, "BenillaKartGuideFramePageTitle"), "Driving");
    assert!(text(&s, "BenillaKartGuideFrameBody").contains("hold SPACE while you steer"));
    for _ in 0..3 {
        s.run("BenillaKartGuide_Turn(1)").unwrap();
    }
    assert_eq!(text(&s, "BenillaKartGuideFramePage"), "5 / 5");
    assert_eq!(text(&s, "BenillaKartGuideFrameNext"), "Done");
    s.run("BenillaKartGuide_Turn(1)").unwrap();
    s.tick(0.0);
    assert!(!shown(&s));
    // Seen: the next login leaves it shut; /kartguide opens it again, on page one.
    s.fire_event("PLAYER_ENTERING_WORLD", Vec::new());
    s.tick(0.0);
    assert!(!shown(&s));
    s.run("SlashCmdList['BENILLA_KARTGUIDE']('')").unwrap();
    s.tick(0.0);
    assert!(shown(&s));
    assert_eq!(text(&s, "BenillaKartGuideFramePage"), "1 / 5");
    // The racing page's button opens the Race Finder in its place.
    s.run("BenillaKartGuide_Turn(1) BenillaKartGuide_Turn(1) BenillaKartGuide_Action()")
        .unwrap();
    s.tick(0.0);
    assert!(!shown(&s));
    assert!(s
        .eval::<bool>("return BenillaKartFinderFrame:IsVisible() == 1")
        .unwrap());
}

#[test]
fn the_first_drive_shows_its_tip_once() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    s.run("SetCVar('KartTipSeen', '0')").unwrap();
    let seated = |s: &mut UiScript, on: bool| {
        s.fire_event(
            "BENILLA_KART_SEATED",
            vec![ScriptValue::Number(f64::from(u8::from(on)))],
        );
        s.tick(0.0);
    };
    seated(&mut s, true);
    assert_eq!(
        s.eval::<String>("return GetCVar('KartTipSeen')").unwrap(),
        "1"
    );
    // The boost is bound as a modifier alone: named without its dash.
    s.run("GetBindingKey = function() return 'SHIFT-' end")
        .unwrap();
    assert_eq!(
        s.eval::<String>("return BenillaKartGuide_Key('KARTCRAFT_BOOST', 'x')")
            .unwrap(),
        "SHIFT"
    );
    seated(&mut s, false);
    // The second time in, nothing more: the CVar holds.
    s.run("BenillaKartGuide_FirstDrive = BenillaKartGuide_FirstDrive")
        .unwrap();
    seated(&mut s, true);
    assert_eq!(
        s.eval::<String>("return GetCVar('KartTipSeen')").unwrap(),
        "1"
    );
}

#[test]
fn the_kartcraft_menu_widens_to_a_challenge_s_long_course_name() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    // A font that measures: 7 units a character, as the game's font engine answers in the call.
    s.set_text_measurer(Box::new(super::FixedWidthFont(7.0)));
    let width = |s: &UiScript, frame: &str| -> f64 {
        s.eval(&format!("return {frame}:GetWidth()")).unwrap()
    };
    // Short names: the menu as it was.
    s.run("BenillaKartChallenge = { daily = 'd', weekly = 'w' } BenillaKartCourseNames = { d = 'Loop', w = 'Dash' } BenillaKartMenu_Refresh()")
        .unwrap();
    assert_eq!(width(&s, "BenillaKartMenuFrameEntry2"), 220.0);
    assert_eq!(width(&s, "BenillaKartMenuFrame"), 280.0);

    // A long one: every entry as wide as it needs, the window around them.
    s.run(
        "BenillaKartCourseNames.w = 'Brackwell Pumpkin Patch to Elwynn Forest Sprint Deluxe' \
         BenillaKartMenu_Refresh()",
    )
    .unwrap();
    let text: f64 = s
        .eval("return BenillaKartMenuFrameEntry3:GetTextWidth()")
        .unwrap();
    assert_eq!(
        s.eval::<String>("return BenillaKartMenuFrameEntry3:GetText()")
            .unwrap(),
        "Weekly: Brackwell Pumpkin Patch to Elwynn Forest Sprint Deluxe"
    );
    let entry = width(&s, "BenillaKartMenuFrameEntry3");
    assert!(
        entry > 220.0 && entry >= text + 32.0,
        "entry {entry} for text {text}"
    );
    assert_eq!(
        width(&s, "BenillaKartMenuFrameEntry1"),
        entry,
        "all the same width"
    );
    assert_eq!(width(&s, "BenillaKartMenuFrame"), entry + 60.0);
}

#[test]
fn courier_jobs_list_today_s_jobs_and_take_one_to_its_courier() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let text = |s: &UiScript, f: &str| -> String {
        s.eval(&format!("return {f}:GetText() or ''")).unwrap()
    };
    let enabled = |s: &UiScript, f: &str| -> bool {
        s.eval(&format!("return {f}:IsEnabled() == 1")).unwrap()
    };
    // Opening asks the server for today's jobs.
    sent(&mut s);
    s.run("BenillaKartJobs_Toggle()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tJL"]);
    from_server(&mut s, "R\tJobs\t2");
    from_server(
        &mut s,
        "R\tJob\t1\t190201\t0\tRatchet\tGadgetzan\tWharfmaster Dizzywig\tMarin Noggenfogger\t6208\t300\tArcane Crystals\t1\t-985\t-3796\t-7193\t-3793",
    );
    from_server(
        &mut s,
        "R\tJob\t2\t190907\t2\tStormwind\tGoldshire\tInnkeeper Allison\tInnkeeper Farley\t740\t120\tElwynn Lumber\t0\t-8868\t674\t-9463\t16",
    );
    // A row a job: cargo, route, distance, par, state.
    assert_eq!(text(&s, "BenillaKartJobsFrameRow1Cargo"), "Arcane Crystals");
    assert_eq!(
        text(&s, "BenillaKartJobsFrameRow1Route"),
        "Ratchet to Gadgetzan"
    );
    assert_eq!(text(&s, "BenillaKartJobsFrameRow1Distance"), "6.2k yd");
    assert_eq!(text(&s, "BenillaKartJobsFrameRow1Par"), "5:00");
    assert!(text(&s, "BenillaKartJobsFrameRow2State").contains("Delivered"));
    assert!(!s
        .eval::<bool>("return BenillaKartJobsFrameRow3:IsVisible() == 1")
        .unwrap());
    // The first is picked: who gives it, where to, and Take job takes us to its courier.
    let detail = text(&s, "BenillaKartJobsFrameText");
    assert!(
        detail.starts_with(
            "From Wharfmaster Dizzywig in Ratchet to Marin Noggenfogger in Gadgetzan."
        ),
        "{detail}"
    );
    assert!(enabled(&s, "BenillaKartJobsFrameTake"));
    assert!(!enabled(&s, "BenillaKartJobsFrameCancel"));
    // One delivered today can't be taken again.
    s.run("BenillaKartJobs_Select(2)").unwrap();
    assert!(!enabled(&s, "BenillaKartJobsFrameTake"));
    // The controller: up to the first, Cross takes it.
    s.eval::<bool>("return BenillaPad_OnButton('PADDUP') == true")
        .unwrap();
    assert!(s
        .eval::<bool>("return BenillaPad_OnButton('PAD1') == true")
        .unwrap());
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tJT\t190201"]);
    assert!(!s
        .eval::<bool>("return BenillaKartJobsFrame:IsVisible() == 1")
        .unwrap());

    // Under way: no other is taken, and Cancel job is offered.
    s.run("BenillaKartJobs_Open()").unwrap();
    sent(&mut s);
    from_server(&mut s, "R\tJobs\t2");
    from_server(
        &mut s,
        "R\tJob\t1\t190201\t1\tRatchet\tGadgetzan\tWharfmaster Dizzywig\tMarin Noggenfogger\t6208\t300\tArcane Crystals\t1\t-985\t-3796\t-7193\t-3793",
    );
    from_server(
        &mut s,
        "R\tJob\t2\t190907\t0\tStormwind\tGoldshire\tInnkeeper Allison\tInnkeeper Farley\t740\t120\tElwynn Lumber\t0\t-8868\t674\t-9463\t16",
    );
    s.run("BenillaKartJobs_Select(1)").unwrap();
    assert!(enabled(&s, "BenillaKartJobsFrameCancel"));
    s.run("BenillaKartJobs_Select(2)").unwrap();
    assert!(!enabled(&s, "BenillaKartJobsFrameTake"), "one at a time");
    // The cooldown is said.
    s.run(
        "BenillaKartSaid = {} local said = BenillaKart_Say \
         BenillaKart_Say = function(t) table.insert(BenillaKartSaid, t) said(t) end",
    )
    .unwrap();
    from_server(&mut s, "R\tJTx\t12");
    assert_eq!(
        s.eval::<Vec<String>>("return BenillaKartSaid").unwrap(),
        ["You can take another job in 12 seconds."]
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn leaving_the_kart_in_a_race_shows_its_hold_until_let_go() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let shown = |s: &UiScript| -> bool {
        s.eval("return BenillaKartLeaveFrame:IsVisible() == 1")
            .unwrap()
    };
    assert!(!shown(&s));
    s.fire_event("BENILLA_KART_LEAVE", vec![ScriptValue::Number(50.0)]);
    assert!(shown(&s));
    assert_eq!(
        s.eval::<String>("return BenillaKartLeaveFrameText:GetText()")
            .unwrap(),
        "Hold to leave the kart"
    );
    assert_eq!(
        s.eval::<f64>("return BenillaKartLeaveFrameFill:GetWidth()")
            .unwrap(),
        98.0,
        "half way"
    );
    s.fire_event("BENILLA_KART_LEAVE", vec![]);
    assert!(!shown(&s), "let go, or out");
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn a_courier_delivery_shows_its_clock_against_par_until_delivered() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let shown = |s: &UiScript| -> bool {
        s.eval("return BenillaKartDeliveryFrame:IsVisible() == 1")
            .unwrap()
    };
    let text = |s: &UiScript| -> String {
        s.eval("return BenillaKartDeliveryFrameText:GetText() or ''")
            .unwrap()
    };
    assert!(!shown(&s));
    // Taken: the town, the par, a clock from zero, green.
    from_server(&mut s, "R\tDel\tGadgetzan\t300\t0");
    assert!(shown(&s));
    let t = text(&s);
    assert!(t.starts_with("Delivery to Gadgetzan"), "{t}");
    assert!(t.contains("|cff40ff400:00|r / par 5:00"), "{t}");
    // Back from a relog six minutes in: past par, red.
    from_server(&mut s, "R\tDel\tGadgetzan\t300\t361");
    assert!(text(&s).contains("|cffff60606:01|r"), "{}", text(&s));
    // Delivered: gone.
    from_server(&mut s, "R\tDel-");
    assert!(!shown(&s));
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_navigation_window_switches_each_guide_and_colours_the_gps() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let eval_bool = |s: &UiScript, lua: &str| -> bool { s.eval(&format!("return {lua}")).unwrap() };
    // /kartnav opens it, every box ticked as every guide starts on.
    s.run("SlashCmdList['BENILLA_KARTNAV']('')").unwrap();
    assert!(eval_bool(&s, "BenillaKartNavFrame:IsVisible() == 1"));
    for (frame, cvar) in [
        ("RaceArrow", "KartArrowShow"),
        ("RaceMinimap", "KartNavRaceMinimap"),
        ("CheckpointArrows", "KartNavCheckpointArrows"),
        ("GpsArrow", "KartNavGpsArrow"),
        ("GpsChevrons", "KartNavGpsChevrons"),
        ("GpsMinimap", "KartNavGpsMinimap"),
        ("GpsMarker", "KartNavGpsMarker"),
        ("GpsDistance", "KartNavGpsDistance"),
        ("Marshals", "KartRaceMarshals"),
    ] {
        let box_ = format!("BenillaKartNavFrame{frame}");
        assert!(
            eval_bool(&s, &format!("{box_}:GetChecked() == 1")),
            "{box_} on"
        );
        s.run(&format!("{box_}:Click()")).unwrap();
        assert_eq!(
            s.cvar(cvar).as_deref(),
            Some("0"),
            "{box_} switches {cvar} off"
        );
        s.run(&format!("{box_}:Click()")).unwrap();
        assert_eq!(s.cvar(cvar).as_deref(), Some("1"));
    }
    // The colour: the stock picker opens on the GPS's own, moving it moves the CVar, and a
    // cancel puts it back; Reset is the amber it starts as.
    s.run("BenillaKartNavFrameColor:Click()").unwrap();
    assert!(eval_bool(&s, "ColorPickerFrame:IsVisible() == 1"));
    s.run("ColorPickerFrame:SetColorRGB(0, 1, 0) ColorPickerFrame.func()")
        .unwrap();
    assert_eq!(s.cvar("KartNavGpsColor").as_deref(), Some("00ff00"));
    s.run("ColorPickerFrame.cancelFunc(ColorPickerFrame.previousValues)")
        .unwrap();
    assert_eq!(s.cvar("KartNavGpsColor").as_deref(), Some("ffbf26"));
    s.run("BenillaKartNav_SetColor(0.2, 0.4, 1)").unwrap();
    assert_eq!(s.cvar("KartNavGpsColor").as_deref(), Some("3366ff"));
    s.run("BenillaKartNavFrameColorReset:Click()").unwrap();
    assert_eq!(s.cvar("KartNavGpsColor").as_deref(), Some("ffbf26"));
    // The yards on the delivery clock go with their box.
    from_server(&mut s, "R	Del	Gadgetzan	300	0");
    s.fire_event(
        "BENILLA_KART_GPS",
        vec![
            ScriptValue::Str("Gadgetzan".into()),
            ScriptValue::Number(1240.0),
        ],
    );
    s.tick(0.1);
    let clock = |s: &UiScript| -> String {
        s.eval("return BenillaKartDeliveryFrameText:GetText() or ''")
            .unwrap()
    };
    assert!(clock(&s).contains("1240 yd"), "{}", clock(&s));
    s.run("BenillaKartNavFrameGpsDistance:Click()").unwrap();
    s.tick(0.1);
    assert!(!clock(&s).contains(" yd"), "{}", clock(&s));
    // The settings window's button opens it too.
    s.run("BenillaKartNavFrame:Hide() BenillaKartSettingsFrameNavigation:Click()")
        .unwrap();
    assert!(eval_bool(&s, "BenillaKartNavFrame:IsVisible() == 1"));
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn a_marathon_delivery_is_queued_shut_counted_down_and_driven_on_the_clock() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let shown = |s: &UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };
    let text = |s: &UiScript, fs: &str| -> String {
        s.eval(&format!("return {fs}:GetText() or ''")).unwrap()
    };
    // The queue opens: the popup, its route and the time to the start.
    from_server(
        &mut s,
        "R@MarOpen@600@Ratchet;Gadgetzan;Cenarion Hold"
            .replace('@', "	")
            .as_str(),
    );
    assert!(shown(&s, "BenillaKartMarathonFrame"));
    assert_eq!(
        text(&s, "BenillaKartMarathonFrameRoute"),
        "Ratchet > Gadgetzan > Cenarion Hold"
    );
    assert!(text(&s, "BenillaKartMarathonFrameStatus").starts_with("Leaves in 10:00, 0 queued"));
    // Join: the line goes, and the server's answer says we're in.
    sent(&mut s);
    s.run("BenillaKartMarathonFrameJoin:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT@R@Mar+".replace('@', "	")]);
    from_server(&mut s, "R@MarQ@3@1".replace('@', "	").as_str());
    s.tick(0.1);
    assert!(text(&s, "BenillaKartMarathonFrameStatus").contains("3 queued"));
    assert!(text(&s, "BenillaKartMarathonFrameStatus").contains("You're in"));
    assert_eq!(text(&s, "BenillaKartMarathonFrameJoin"), "Leave queue");
    // The start: the queue shuts, and we are off in ten.
    from_server(&mut s, "R@MarShut".replace('@', "	").as_str());
    from_server(&mut s, "R@MarGo@10".replace('@', "	").as_str());
    s.tick(0.1);
    assert!(shown(&s, "BenillaKartMarathonFrame"));
    assert!(text(&s, "BenillaKartMarathonFrameStatus").starts_with("Off in"));
    // The first leg: the clock in the marathon's words, and it stays with no quest in the log.
    from_server(
        &mut s,
        "R@MDel@Gadgetzan@1800@0@1@2".replace('@', "	").as_str(),
    );
    assert!(!shown(&s, "BenillaKartMarathonFrame"));
    for _ in 0..30 {
        s.tick(0.1);
    }
    assert!(
        shown(&s, "BenillaKartDeliveryFrame"),
        "no quest log check for a marathon"
    );
    let clock = text(&s, "BenillaKartDeliveryFrameText");
    assert!(clock.starts_with("Marathon: to Gadgetzan (1/2)"), "{clock}");
    // In, then over: the clock goes.
    from_server(&mut s, "R@MarDone@2@2482000@50".replace('@', "	").as_str());
    from_server(&mut s, "R@Del-".replace('@', "	").as_str());
    from_server(&mut s, "R@MarEnd".replace('@', "	").as_str());
    assert!(!shown(&s, "BenillaKartDeliveryFrame"));
    assert!(!shown(&s, "BenillaKartMarathonFrame"));
    // /marathon's words: leave for anyone, any other a GM's order for the server.
    s.run("SlashCmdList['BENILLA_MARATHON']('  leave ')")
        .unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT@R@Mar-".replace('@', "	")]);
    s.run("SlashCmdList['BENILLA_MARATHON']('route Ratchet;Cenarion Hold;Everlook')")
        .unwrap();
    assert_eq!(
        sent(&mut s),
        ["KARTCRAFT@R@MarGM@route@Ratchet;Cenarion@Hold;Everlook".replace('@', "	")]
    );
    s.run("SlashCmdList['BENILLA_MARATHON']('open 20 kalimdor')")
        .unwrap();
    assert_eq!(
        sent(&mut s),
        ["KARTCRAFT@R@MarGM@open@20@kalimdor".replace('@', "	")]
    );
    // Someone who did not join: the queue shutting takes the popup away.
    from_server(
        &mut s,
        "R@MarOpen@60@Booty Bay;Thorium Point"
            .replace('@', "	")
            .as_str(),
    );
    assert!(shown(&s, "BenillaKartMarathonFrame"));
    from_server(&mut s, "R@MarShut".replace('@', "	").as_str());
    assert!(!shown(&s, "BenillaKartMarathonFrame"));
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_kart_s_shortcuts_show_in_the_kart_out_of_a_race_and_open_their_windows() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 3);
    tracks(&mut s);
    let shown = |s: &UiScript, frame: &str| -> bool {
        s.eval(&format!("return {frame}:IsVisible() == 1")).unwrap()
    };
    let seated = |s: &mut UiScript, on: bool| {
        s.fire_event(
            "BENILLA_KART_SEATED",
            vec![ScriptValue::Number(if on { 1.0 } else { 0.0 })],
        );
        s.tick(0.0);
    };
    let track = |s: &mut UiScript, name: &str| {
        s.fire_event(
            "BENILLA_KART_TRACKMAP",
            vec![
                ScriptValue::Str(name.into()),
                ScriptValue::Number(3.0),
                ScriptValue::Str("0,0;100,0;100,100".into()),
            ],
        );
        s.tick(0.0);
    };

    // On foot: none.
    assert!(!shown(&s, "BenillaKartShortcutsFrame"));
    // In the kart: both, at the screen's bottom, saying they move.
    seated(&mut s, true);
    assert!(shown(&s, "BenillaKartShortcutsFrame"));
    assert!(s
        .eval::<bool>("return BenillaKartShortcutsFrame:IsMovable() == 1")
        .unwrap());
    assert_eq!(
        s.eval::<String>("return BenillaKartShortcutsFrameHint:GetText()")
            .unwrap(),
        "Drag to move, right-click to hide"
    );
    assert_eq!(
        s.eval::<String>("local p = BenillaKartShortcutsFrame:GetPoint(1) return p")
            .unwrap(),
        "BOTTOM"
    );
    // A race under way puts them away; its end brings them back.
    track(&mut s, "Elwynn Loop");
    assert!(!shown(&s, "BenillaKartShortcutsFrame"));
    track(&mut s, "");
    assert!(shown(&s, "BenillaKartShortcutsFrame"));

    // Time trial opens the track browser, Race Finder the finder.
    s.run("BenillaKartShortcutsFrameTrial:Click()").unwrap();
    assert!(shown(&s, "BenillaKartTracksFrame"));
    s.run("BenillaKartTracksFrame:Hide() BenillaKartShortcutsFrameFinder:Click()")
        .unwrap();
    assert!(shown(&s, "BenillaKartFinderFrame"));

    // Out of the kart: gone.
    seated(&mut s, false);
    assert!(!shown(&s, "BenillaKartShortcutsFrame"));

    // A right-click hides it for good, said in chat; the settings' box brings it back.
    seated(&mut s, true);
    s.run("arg1 = 'RightButton' BenillaKartShortcutsFrame:GetScript('OnMouseUp')() arg1 = nil")
        .unwrap();
    assert!(
        !shown(&s, "BenillaKartShortcutsFrame"),
        "hidden by its right-click"
    );
    assert_eq!(
        s.eval::<String>("return GetCVar('KartShortcuts')").unwrap(),
        "0"
    );
    seated(&mut s, false);
    seated(&mut s, true);
    assert!(
        !shown(&s, "BenillaKartShortcutsFrame"),
        "and stays hidden in the kart"
    );
    s.run(
        "BenillaKartSettingsFrame:Show() BenillaKartSettingsFrameShortcuts:SetChecked(1)          BenillaKartSettings_Check(BenillaKartSettingsFrameShortcuts, 'KartShortcuts')",
    )
    .unwrap();
    assert!(
        shown(&s, "BenillaKartShortcutsFrame"),
        "the box brings it back"
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_controller_moves_down_the_race_finder_ticks_with_cross_and_queues_with_start() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    courses(&mut s, 12);
    let press = |s: &mut UiScript, key: &str| -> bool {
        s.eval(&format!("return BenillaPad_OnButton('{key}') == true"))
            .unwrap()
    };
    let focus = |s: &UiScript| -> String {
        s.eval("return BenillaPad.focus and BenillaPad.focus:GetName() or ''")
            .unwrap()
    };
    let picked = |s: &UiScript, key: &str| -> bool {
        s.eval(&format!("return BenillaKartFinder.picked['{key}'] == 1"))
            .unwrap()
    };
    s.run("BenillaKart_Open(BenillaKartFinderFrame)").unwrap();
    s.tick(0.0);
    sent(&mut s);

    // The cursor starts on "Any course" and goes down the courses.
    assert!(press(&mut s, "PADDUP"));
    assert_eq!(focus(&s), "BenillaKartFinderFrameAny");
    press(&mut s, "PADDDOWN");
    assert_eq!(focus(&s), "BenillaKartFinderFrameRow1");
    press(&mut s, "PADDDOWN");
    assert_eq!(focus(&s), "BenillaKartFinderFrameRow2");
    // Cross ticks the course under it, again unticks.
    press(&mut s, "PAD1");
    assert!(picked(&s, "elwynn"), "Cross ticks Elwynn Loop");
    press(&mut s, "PAD1");
    assert!(!picked(&s, "elwynn"));
    press(&mut s, "PAD1");
    // R1 a page down: the list follows the cursor, which stays on a shown row.
    press(&mut s, "PADRSHOULDER");
    let at = focus(&s);
    assert!(at.starts_with("BenillaKartFinderFrameRow"), "{at}");
    assert!(s
        .eval::<bool>("return BenillaPad.focus:IsVisible() == 1")
        .unwrap());
    // Start finds a race on the ticked course, and again leaves the queue.
    press(&mut s, "PADFORWARD");
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tQ+\telwynn"]);
    press(&mut s, "PADFORWARD");
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tQ-"]);

    // A hosted race under the courses: the cursor reaches its Join, Cross joins it.
    s.run("BenillaKartLobby_Set('', '9:elwynn:Goblin:2') BenillaKartFinder_Refresh()")
        .unwrap();
    for _ in 0..20 {
        press(&mut s, "PADDDOWN");
    }
    assert_eq!(focus(&s), "BenillaKartFinderFrameLobby1Join");
    press(&mut s, "PAD1");
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tLJ\t9"]);
    // Circle closes it.
    press(&mut s, "PAD2");
    assert!(!s
        .eval::<bool>("return BenillaKartFinderFrame:IsVisible() == 1")
        .unwrap());
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn a_controller_guide_shows_under_the_window_the_pad_drives() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    tracks(&mut s);
    let legend = |s: &mut UiScript| -> (bool, String) {
        s.eval::<String>("return BenillaPad_Owns()").unwrap();
        (
            s.eval("return BenillaPadLegendFrame:IsVisible() == 1")
                .unwrap(),
            s.eval("return BenillaPadLegendFrameText:GetText() or ''")
                .unwrap(),
        )
    };
    // No controller connected: no guide.
    s.run("SetCVar('KartPads', '') BenillaKartTrackBrowser_Open()")
        .unwrap();
    assert!(!legend(&mut s).0);
    // One connected: the window's own guide under it.
    s.run("SetCVar('KartPads', '1|Wireless Controller')")
        .unwrap();
    let (shown, text) = legend(&mut s);
    assert!(shown);
    assert!(text.contains("Cross / A: time trial"), "{text}");
    // Another window, its guide; the menu's the plain one.
    s.run("BenillaKartTracksFrame:Hide() BenillaKart_Open(BenillaKartFinderFrame)")
        .unwrap();
    assert!(legend(&mut s).1.contains("Start: find race"));
    s.run("BenillaKartFinderFrame:Hide() BenillaKartMenu_Toggle()")
        .unwrap();
    let (shown, text) = legend(&mut s);
    assert!(shown);
    assert_eq!(
        text,
        "D-pad: move    Cross / A: choose    Circle / B: close"
    );
    // No window the pad drives: no guide.
    s.run("BenillaKartMenuFrame:Hide()").unwrap();
    assert!(!legend(&mut s).0);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_controller_picks_a_track_with_the_d_pad_and_acts_with_a_face_button() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    tracks(&mut s);
    let press = |s: &mut UiScript, key: &str| -> bool {
        s.eval(&format!("return BenillaPad_OnButton('{key}') == true"))
            .unwrap()
    };
    let selected = |s: &UiScript| -> String {
        s.eval("return BenillaKartTrackBrowser.selected or ''")
            .unwrap()
    };
    let shown = |s: &UiScript| -> bool {
        s.eval("return BenillaKartTracksFrame:IsVisible() == 1")
            .unwrap()
    };
    s.run("BenillaKartTrackBrowser.selected = nil BenillaKartTrackBrowser_Open()")
        .unwrap();
    s.tick(0.0);
    assert_eq!(s.eval::<String>("return BenillaPad_Owns()").unwrap(), "ALL");

    // The D-pad picks the track itself, from the top, and stops at the list's ends.
    assert!(press(&mut s, "PADDDOWN"));
    assert_eq!(selected(&s), "t1", "the first press picks the first track");
    press(&mut s, "PADDDOWN");
    assert_eq!(selected(&s), "t2");
    press(&mut s, "PADDDOWN");
    assert_eq!(selected(&s), "t2", "the last stays picked");
    press(&mut s, "PADLSHOULDER");
    assert_eq!(selected(&s), "t1", "L1 a page up, to the top");
    assert!(shown(&s), "picking is not choosing");
    assert_eq!(
        s.eval::<String>("return BenillaKartTracksFrameTitle:GetText()")
            .unwrap(),
        "Crystal Lake Loop"
    );

    // Each action is a face button, from wherever the pick is.
    sent(&mut s);
    press(&mut s, "PAD3");
    assert_eq!(
        sent(&mut s),
        ["KARTCRAFT\tR\tMT\tt1"],
        "Square: the marshal"
    );
    assert!(!shown(&s));
    // Opening asks the server which tracks are closed.
    s.run("BenillaKartTrackBrowser_Open()").unwrap();
    sent(&mut s);
    press(&mut s, "PADDDOWN");
    press(&mut s, "PAD1");
    assert_eq!(
        sent(&mut s),
        ["KARTCRAFT\tR\tTB\tt2"],
        "Cross: the time trial"
    );
    assert!(!shown(&s));
    s.run("BenillaKartTrackBrowser_Open()").unwrap();
    sent(&mut s);
    press(&mut s, "PAD2");
    assert!(!shown(&s), "Circle closes");
    assert!(sent(&mut s).is_empty());
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn the_track_browser_takes_us_to_the_track_s_marshal() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    tracks(&mut s);
    let enabled = |s: &UiScript| -> bool {
        s.eval("return BenillaKartTracksFrameMarshal:IsEnabled() == 1")
            .unwrap()
    };
    s.run("BenillaKartTrackBrowser_Open()").unwrap();
    s.tick(0.0);
    assert!(!enabled(&s), "nothing picked");
    s.run("BenillaKartTrackBrowser_Select('t1')").unwrap();
    assert!(enabled(&s));
    sent(&mut s);
    s.run("BenillaKartTracksFrameMarshal:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tMT\tt1"]);
    assert!(!s
        .eval::<bool>("return BenillaKartTracksFrame:IsVisible() == 1")
        .unwrap());

    // Too soon after the last trip: the server says how long, and so does the chat.
    s.run(
        "BenillaKartSaid = {} local said = BenillaKart_Say \
         BenillaKart_Say = function(t) table.insert(BenillaKartSaid, t) said(t) end",
    )
    .unwrap();
    from_server(&mut s, "R\tMTx\t12");
    assert_eq!(
        s.eval::<Vec<String>>("return BenillaKartSaid").unwrap(),
        ["You can travel to a Race Marshal again in 12 seconds."]
    );

    // A party course's marshal is inside its dungeon.
    s.fire_event(
        "BENILLA_KART_TRACKS",
        vec![ScriptValue::Str(
            "d1\tDeadmines Run\tThe Deadmines\t36\t0\tA sprint.\t0.0,0.0;10.0,10.0".into(),
        )],
    );
    s.run("BenillaKartTrackBrowser_Open() BenillaKartTrackBrowser_Select('d1')")
        .unwrap();
    assert!(!enabled(&s));
}

#[test]
fn the_skin_shop_previews_buys_and_wears_through_the_server() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = loaded();
    let text = |s: &mut UiScript, name: &str| -> String {
        s.eval(&format!("return {name}:GetText() or ''")).unwrap()
    };
    let enabled = |s: &mut UiScript, name: &str| -> bool {
        s.eval(&format!("return {name}:IsEnabled() == 1")).unwrap()
    };
    let preview =
        |s: &mut UiScript| -> String { s.eval("return GetCVar('KartSkinPreview')").unwrap() };
    sent(&mut s);
    s.run("BenillaKartShop_Toggle()").unwrap();
    // Opening asks for our purse, and the first skin turns in the pane.
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tSkins"]);
    assert_eq!(preview(&mut s), "3597");
    assert_eq!(text(&mut s, "BenillaKartShopFrameSkin1Name"), "Magni");
    assert_eq!(text(&mut s, "BenillaKartShopFrameSkin10Name"), "VanCleef");
    // Ten skins: the grid's last two cells stay empty.
    assert!(!s
        .eval::<bool>("return BenillaKartShopFrameSkin11:IsVisible() == 1")
        .unwrap());
    assert_eq!(
        text(&mut s, "BenillaKartShopFrameTokens"),
        "Kartcraft Tokens: ?"
    );

    // 250 tokens: Magni (300) is out of reach, Bolvar (200) is not.
    from_server(&mut s, "R\tSkins\t250\t\t");
    assert_eq!(
        text(&mut s, "BenillaKartShopFrameTokens"),
        "Kartcraft Tokens: 250"
    );
    assert!(!enabled(&mut s, "BenillaKartShopFrameBuy"));
    s.run("BenillaKartShopFrameSkin7:Click()").unwrap();
    assert_eq!(preview(&mut s), "5566");
    assert_eq!(
        text(&mut s, "BenillaKartShopFrameSkinName"),
        "Highlord Bolvar Fordragon"
    );
    assert!(enabled(&mut s, "BenillaKartShopFrameBuy"));
    assert!(!enabled(&mut s, "BenillaKartShopFrameWear"));
    s.run("BenillaKartShopFrameBuy:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tSkinBuy\tbolvar"]);

    // Bought: Wear wears it, and again takes it off.
    from_server(&mut s, "R\tSkins\t20\t\tbolvar");
    assert_eq!(text(&mut s, "BenillaKartShopFrameSkin7Price"), "Owned");
    s.run("BenillaKartShopFrameWear:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tSkinWear\tbolvar"]);
    from_server(&mut s, "R\tSkins\t20\tbolvar\tbolvar");
    assert_eq!(text(&mut s, "BenillaKartShopFrameSkin7Price"), "Worn");
    assert_eq!(text(&mut s, "BenillaKartShopFrameWear"), "Take off");
    s.run("BenillaKartShopFrameWear:Click()").unwrap();
    assert_eq!(sent(&mut s), ["KARTCRAFT\tR\tSkinWear\t"]);

    // Closed, the pane's unit goes.
    s.run("BenillaKartShopFrame:Hide()").unwrap();
    assert_eq!(preview(&mut s), "0");
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

/// The shop's catalogue is the server's: every skin's key, display and price in
/// `BENILLA_KART_SKINS` as `Kartcraft.cpp`'s `SKINS` has them, in its order.
#[test]
fn the_skin_shop_sells_what_the_server_sells() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cpp = std::fs::read_to_string(root.join("setup/vmangos/src/Kartcraft.cpp")).unwrap();
    let xml =
        std::fs::read_to_string(root.join("crates/benilla-app/assets/ui/KartcraftFrames.xml"))
            .unwrap();
    let server: Vec<(String, u32, u32)> = cpp
        .lines()
        .filter_map(|l| {
            let l = l.trim().strip_prefix("{ \"")?.strip_suffix(" },")?;
            let (key, rest) = l.split_once("\", ")?;
            let (display, price) = rest.split_once(", ")?;
            Some((key.into(), display.parse().ok()?, price.parse().ok()?))
        })
        .collect();
    let field = |l: &str, name: &str| -> Option<String> {
        let at = l.find(&format!("{name} = "))? + name.len() + 3;
        let v = l[at..].split(',').next()?;
        Some(v.trim().trim_matches('"').to_string())
    };
    let client: Vec<(String, u32, u32)> = xml
        .lines()
        .filter(|l| l.trim_start().starts_with("{ key = "))
        .filter_map(|l| {
            Some((
                field(l, "key")?,
                field(l, "display")?.parse().ok()?,
                field(l, "price")?.parse().ok()?,
            ))
        })
        .collect();
    assert_eq!(client.len(), 10, "the shop's ten skins");
    assert_eq!(client, server);
}

/// Every entry of the Kartcraft menu has its button: two branches each adding an entry and the
/// same next button merge into one button short.
#[test]
fn every_kartcraft_menu_entry_has_a_button() {
    benilla_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let s = loaded();
    let missing: Vec<i64> = s
        .eval(
            "local out = {} for i = 1, table.getn(BENILLA_KART_MENU) do \
             if not getglobal('BenillaKartMenuFrameEntry' .. i) then table.insert(out, i) end end \
             return out",
        )
        .unwrap();
    assert!(
        missing.is_empty(),
        "menu entries without a button: {missing:?}"
    );
}
