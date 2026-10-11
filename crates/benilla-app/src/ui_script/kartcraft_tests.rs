//! World of Kartcraft's windows (`assets/ui/KartcraftFrames.xml`) on the production load: the
//! minimap button and its menu, the Race Finder queuing and answering over the addon channel, the
//! ready check as the stock popup, the kart's race and score events reaching their frames, the
//! settings window writing its CVars, and the garage gating its classes by skill.

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
        "Return to where you were before the race?"
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
            "Leave race",
            "Kart garage...",
            "Kartcraft settings...",
            "Hide minimap button",
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
    pick(&mut s, "Hide minimap button");
    assert!(!shown(&s, "BenillaKartMinimapButton"));
    assert_eq!(s.cvar("KartMinimapButton").as_deref(), Some("0"));
    s.run("SlashCmdList['BENILLA_KARTCRAFT']('')").unwrap();
    assert!(!s
        .eval::<bool>("return BenillaKartSettingsFrameMinimapButton:GetChecked() == 1")
        .unwrap());
    s.run("BenillaKartSettingsFrameMinimapButton:Click()")
        .unwrap();
    assert!(shown(&s, "BenillaKartMinimapButton"));
    assert_eq!(s.cvar("KartMinimapButton").as_deref(), Some("1"));
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
        ("MinimapButton", "Show the minimap button"),
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
    assert!(
        s.take_cvar_changes().is_empty(),
        "showing the window wrote a CVar"
    );

    // Each control writes its CVar, which the host hears.
    s.run(
        "BenillaKartSettingsFrameArrowShow:Click() \
         BenillaKartSettingsFrameChaseCamera:Click() \
         BenillaKartSettingsFrameMusic:Click() \
         BenillaKartSettingsFrameArrowSize:SetValue(1.5) \
         BenillaKartSettingsFrameMusicVolume:SetValue(0.25)",
    )
    .unwrap();
    for (name, value) in [
        ("KartArrowShow", "0"),
        ("KartChaseCamera", "0"),
        ("KartMusic", "0"),
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
fn the_garage_opens_classes_by_skill_and_picks_a_kart() {
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

    // /garage opens it at the CVars, writing nothing: Turning ticked, the rest locked.
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
    for (i, needs) in [(2, 75), (3, 150), (4, 225)] {
        assert!(
            !eval_bool(
                &s,
                &format!("BenillaKartGarageFrameClass{i}:IsEnabled() == 1")
            ),
            "class {i} is locked at skill 1"
        );
        assert_eq!(
            text(&s, &format!("BenillaKartGarageFrameClass{i}About")),
            format!("Requires Karting ({needs})")
        );
    }
    assert!(
        s.take_cvar_changes().is_empty(),
        "showing the garage wrote a CVar"
    );

    // A locked class cannot be picked.
    s.run("BenillaKartGarage_PickClass(4)").unwrap();
    assert_eq!(s.cvar("KartClass").as_deref(), Some("turn"));

    // At 160 Balanced and Acceleration open; Speed stays locked.
    garage(&mut s, 160);
    assert_eq!(
        text(&s, "BenillaKartGarageFrameSkill"),
        "Karting skill: 160"
    );
    assert!(eval_bool(
        &s,
        "BenillaKartGarageFrameClass3:IsEnabled() == 1"
    ));
    assert!(!eval_bool(
        &s,
        "BenillaKartGarageFrameClass4:IsEnabled() == 1"
    ));
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

    // At 300 everything is open.
    garage(&mut s, 300);
    assert!((1..=4).all(|i| eval_bool(
        &s,
        &format!("BenillaKartGarageFrameClass{i}:IsEnabled() == 1")
    )));
    s.run("BenillaKartGarageFrameClass4:Click()").unwrap();
    assert_eq!(s.cvar("KartClass").as_deref(), Some("speed"));
    // Back under 225 the pick stays, and the tick falls to what the kart drives.
    garage(&mut s, 100);
    assert_eq!(s.cvar("KartClass").as_deref(), Some("speed"));
    assert!(eval_bool(
        &s,
        "BenillaKartGarageFrameClass2:GetChecked() == 1"
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
    let s = loaded();
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
        "Accelerate,Brake / Reverse,Steer Left,Steer Right,Hop / Powerslide,\
         Boost (Second Shoulder),Fire Item,Wumpa Juice,Turbo Boost,Super Engine,Aku Aku,\
         Summon / Dismiss Kart,Race Finder,Leave Race,Kartcraft Settings,Kart Garage"
    );
    s.run("RunBinding('KARTCRAFT_GARAGE')").unwrap();
    assert!(s
        .eval::<bool>("return BenillaKartGarageFrame:IsVisible() == 1")
        .unwrap());
    s.run("RunBinding('KARTCRAFT_RACEFINDER')").unwrap();
    assert!(s
        .eval::<bool>("return BenillaKartFinderFrame:IsVisible() == 1")
        .unwrap());
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
