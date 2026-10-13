//! The weapons' addon lines, `KARTCRAFT\tW\t<verb>\t<fields>`
//! (`docs/kartcraft-weapons-protocol.md`): what the server sends every racer ([`Down`]) and what a racer sends it ([`Up`]), written and
//! read. Positions are WoW world coordinates in yards (2 decimals), yaws WoW orientations in
//! radians (3), guids decimal low guids, items CTR's ids.

use ctr_kart::Ram;
use ctr_weapons::{CloudEffect, HitKind, Item};

pub const PREFIX: &str = "KARTCRAFT\tW\t";
/// A whole line stays under 200 bytes.
pub const MAX_LINE: usize = 200;

/// A racer, by low guid.
pub type Guid = u32;

/// What stands at a crate spot: a weapon crate ("?"), a fruit crate, or one Wumpa fruit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CrateKind {
    Weapon,
    Fruit,
    Wumpa,
}

impl CrateKind {
    pub fn code(self) -> char {
        match self {
            CrateKind::Weapon => 'Q',
            CrateKind::Fruit => 'F',
            CrateKind::Wumpa => 'W',
        }
    }

    pub fn from_code(c: char) -> Option<Self> {
        Some(match c {
            'Q' => CrateKind::Weapon,
            'F' => CrateKind::Fruit,
            'W' => CrateKind::Wumpa,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrateSpot {
    pub kind: CrateKind,
    /// WoW world, yards; `z` the ground.
    pub at: [f32; 3],
}

/// Where a shot leaves from and how: the shooter's kart, its facing, the missile lock-on (or the
/// warp orb's target, the server's pick), fired backwards (`b`) or dropped behind (`d`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aim {
    pub at: [f32; 3],
    pub yaw: f32,
    pub target: Guid,
    pub backward: bool,
    pub dropped: bool,
}

/// A shot as the server relays it to every racer, the shooter included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub shooter: Guid,
    /// The held id: Bomb ×3 and Missile ×3 relay as 10 and 11.
    pub item: Item,
    pub juiced: bool,
    pub shot: u16,
    pub aim: Aim,
}

/// Server → racers.
#[derive(Clone, Debug, PartialEq)]
pub enum Down {
    /// The race's crates from index `first` on (`W L`).
    Layout {
        first: usize,
        crates: Vec<CrateSpot>,
    },
    /// A crate was taken (`W X`).
    Taken { index: usize, by: Guid },
    /// A crate is back (`W U`).
    Back { index: usize },
    /// Our roulette's result (`W I`).
    Item { item: Item, shots: u8 },
    /// What the server holds for us when we took a weapon crate it gave nothing for: the two
    /// disagreed (a fire line it never had), and an empty slot or a roulette with no roll yet takes
    /// it.
    Holding { item: Item, shots: u8 },
    /// Our Wumpa total (`W F`).
    Wumpa(u8),
    /// A shot to launch (`W S`).
    Shot(Shot),
    /// A racer was hit (`W H`): every client shows it and ends the shot.
    Hit {
        victim: Guid,
        shooter: Guid,
        shot: u16,
        kind: HitKind,
        wumpa: u8,
    },
    /// The shooter pressed Fire again: its rolling bomb goes off (`W D`).
    Detonate { shooter: Guid },
    /// The shooter pressed Fire with its shield up: the shield leaves as a shield bomb (`W Z`).
    ShieldShot { shooter: Guid, shot: u16, aim: Aim },
    /// A red beaker's rain cloud over `victim` starts or starts again (`W R`), 7.68 s from now
    /// with this effect; `locked`: its held item went into the roulette, to be rolled anew (`I`).
    Cloud {
        victim: Guid,
        effect: CloudEffect,
        locked: bool,
    },
    /// `attacker` rammed `victim` with its mask, shield or turbo (`W B`): the victim's reaction
    /// shows everywhere, a shield ram spends the attacker's shield.
    Ram {
        victim: Guid,
        attacker: Guid,
        ram: Ram,
        wumpa: u8,
    },
}

/// Racer → server.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Up {
    /// Take crate `index` (`W C`).
    Claim(usize),
    /// Our shot `shot` broke crate `index` (`W K`): it touched the crate or its blast reached it.
    Break { shot: u16, index: usize },
    /// Fire the held item (`W S`): our own shot counter, 1–65535.
    Fire { shot: u16, aim: Aim },
    /// We were hit by the shooter's shot (`W H`); a self-inflicted hit names us and shot 0.
    Hit {
        shooter: Guid,
        shot: u16,
        kind: HitKind,
    },
    /// Fire again with our bomb rolling: set it off (`W D`).
    Detonate,
    /// Fire with our shield up: launch it as a shield bomb (`W Z`), a shot of our own counter.
    ShieldShot { shot: u16, aim: Aim },
    /// We lost Wumpa by no one's hit (`W P`): the server takes it and answers with our total.
    Lose(Loss),
    /// We were rammed by `attacker`'s mask, shield or turbo, and it took (`W B`).
    Ram { attacker: Guid, ram: Ram },
}

/// A ram on the wire: `M` mask, `S` shield, `T` turbo.
pub fn ram_code(ram: Ram) -> char {
    match ram {
        Ram::Mask => 'M',
        Ram::Shield => 'S',
        Ram::Turbo => 'T',
    }
}

fn ram_of(s: &str) -> Option<Ram> {
    let mut c = s.chars();
    let ram = match c.next()? {
        'M' => Ram::Mask,
        'S' => Ram::Shield,
        'T' => Ram::Turbo,
        _ => return None,
    };
    c.next().is_none().then_some(ram)
}

/// Wumpa lost by no one's hit (`RB_Player_ModifyWumpa`'s other callers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loss {
    /// A powerslide spun out: SpinFirst's 1 (`VehPhysProc.c:2868-2871`).
    SpinOut,
    /// Aku Aku put us back after a fall: the mask grab's 2 (`VehStuckProc.c:648-651`).
    MaskGrab,
}

impl Loss {
    pub fn code(self) -> char {
        match self {
            Loss::SpinOut => 'S',
            Loss::MaskGrab => 'M',
        }
    }

    pub fn from_code(c: &str) -> Option<Self> {
        Some(match c {
            "S" => Loss::SpinOut,
            "M" => Loss::MaskGrab,
            _ => return None,
        })
    }

    /// The Wumpa it costs, none while masked (`RB_Player.c:153-161`).
    pub fn wumpa(self) -> u8 {
        match self {
            Loss::SpinOut => 1,
            Loss::MaskGrab => 2,
        }
    }
}

fn flags(aim: &Aim) -> String {
    let mut f = String::new();
    if aim.backward {
        f.push('b');
    }
    if aim.dropped {
        f.push('d');
    }
    f
}

fn place(aim: &Aim) -> String {
    format!(
        "{:.2},{:.2},{:.2},{:.3}",
        aim.at[0], aim.at[1], aim.at[2], aim.yaw
    )
}

fn crate_text(c: &CrateSpot) -> String {
    format!(
        "{},{:.2},{:.2},{:.2}",
        c.kind.code(),
        c.at[0],
        c.at[1],
        c.at[2]
    )
}

impl Up {
    pub fn line(&self) -> String {
        let body = match self {
            Up::Claim(i) => format!("C\t{i}"),
            Up::Break { shot, index } => format!("K\t{shot}\t{index}"),
            Up::Fire { shot, aim } => {
                format!("S\t{shot}\t{}\t{}\t{}", place(aim), aim.target, flags(aim))
            }
            Up::Hit {
                shooter,
                shot,
                kind,
            } => format!("H\t{shooter}\t{shot}\t{}", kind.code()),
            Up::Detonate => "D".to_string(),
            Up::ShieldShot { shot, aim } => format!("Z\t{shot}\t{}", place(aim)),
            Up::Lose(loss) => format!("P\t{}", loss.code()),
            Up::Ram { attacker, ram } => format!("B\t{attacker}\t{}", ram_code(*ram)),
        };
        format!("{PREFIX}{body}")
    }

    /// Reads a racer's line, as the server (or the offline referee) does.
    pub fn parse(line: &str) -> Option<Up> {
        let f: Vec<&str> = line.strip_prefix(PREFIX)?.split('\t').collect();
        Some(match *f.first()? {
            "C" => Up::Claim(f.get(1)?.parse().ok()?),
            "K" if f.len() >= 3 => Up::Break {
                shot: shot_number(f[1])?,
                index: f[2].parse().ok()?,
            },
            "S" if f.len() >= 4 => Up::Fire {
                shot: shot_number(f[1])?,
                aim: aim(f[2], f[3], f.get(4).copied().unwrap_or(""))?,
            },
            "H" if f.len() >= 4 => Up::Hit {
                shooter: f[1].parse().ok()?,
                shot: f[2].parse().ok()?,
                kind: hit_kind(f[3])?,
            },
            "D" => Up::Detonate,
            "Z" if f.len() >= 3 => Up::ShieldShot {
                shot: shot_number(f[1])?,
                aim: aim(f[2], "0", "")?,
            },
            "P" if f.len() >= 2 => Up::Lose(Loss::from_code(f[1])?),
            "B" if f.len() >= 3 => Up::Ram {
                attacker: f[1].parse().ok()?,
                ram: ram_of(f[2])?,
            },
            _ => return None,
        })
    }
}

impl Down {
    pub fn line(&self) -> String {
        let body = match self {
            Down::Layout { first, crates } => format!(
                "L\t{first}\t{}",
                crates.iter().map(crate_text).collect::<Vec<_>>().join(";")
            ),
            Down::Taken { index, by } => format!("X\t{index}\t{by}"),
            Down::Back { index } => format!("U\t{index}"),
            Down::Item { item, shots } => format!("I\t{}\t{shots}", item.id()),
            Down::Holding { item, shots } => format!("Y\t{}\t{shots}", item.id()),
            Down::Wumpa(n) => format!("F\t{n}"),
            Down::Shot(s) => format!(
                "S\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                s.shooter,
                s.item.id(),
                u8::from(s.juiced),
                s.shot,
                place(&s.aim),
                s.aim.target,
                flags(&s.aim)
            ),
            Down::Hit {
                victim,
                shooter,
                shot,
                kind,
                wumpa,
            } => format!("H\t{victim}\t{shooter}\t{shot}\t{}\t{wumpa}", kind.code()),
            Down::Detonate { shooter } => format!("D\t{shooter}"),
            Down::ShieldShot { shooter, shot, aim } => {
                format!("Z\t{shooter}\t{shot}\t{}", place(aim))
            }
            Down::Cloud {
                victim,
                effect,
                locked,
            } => format!("R\t{victim}\t{}\t{}", effect.id(), u8::from(*locked)),
            Down::Ram {
                victim,
                attacker,
                ram,
                wumpa,
            } => format!("B\t{victim}\t{attacker}\t{}\t{wumpa}", ram_code(*ram)),
        };
        format!("{PREFIX}{body}")
    }

    /// Reads a server line (the whole addon text); `None` for anything else.
    pub fn parse(line: &str) -> Option<Down> {
        let f: Vec<&str> = line.strip_prefix(PREFIX)?.split('\t').collect();
        Some(match *f.first()? {
            "L" if f.len() >= 3 => Down::Layout {
                first: f[1].parse().ok()?,
                crates: f[2]
                    .split(';')
                    .filter(|c| !c.is_empty())
                    .map(|c| {
                        let mut p = c.split(',');
                        let kind = CrateKind::from_code(p.next()?.chars().next()?)?;
                        let mut n = || p.next()?.parse::<f32>().ok().filter(|v| v.is_finite());
                        Some(CrateSpot {
                            kind,
                            at: [n()?, n()?, n()?],
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            },
            "X" if f.len() >= 3 => Down::Taken {
                index: f[1].parse().ok()?,
                by: f[2].parse().ok()?,
            },
            "U" if f.len() >= 2 => Down::Back {
                index: f[1].parse().ok()?,
            },
            "I" if f.len() >= 3 => Down::Item {
                item: Item::from_id(f[1].parse().ok()?)?,
                shots: f[2].parse().ok()?,
            },
            "Y" if f.len() >= 3 => Down::Holding {
                item: Item::from_id(f[1].parse().ok()?)?,
                shots: f[2].parse().ok()?,
            },
            "F" if f.len() >= 2 => Down::Wumpa(f[1].parse().ok()?),
            "S" if f.len() >= 7 => Down::Shot(Shot {
                shooter: f[1].parse().ok()?,
                item: Item::from_id(f[2].parse().ok()?)?,
                juiced: f[3] == "1",
                shot: shot_number(f[4])?,
                aim: aim(f[5], f[6], f.get(7).copied().unwrap_or(""))?,
            }),
            "H" if f.len() >= 6 => Down::Hit {
                victim: f[1].parse().ok()?,
                shooter: f[2].parse().ok()?,
                shot: f[3].parse().ok()?,
                kind: hit_kind(f[4])?,
                wumpa: f[5].parse().ok()?,
            },
            "D" if f.len() >= 2 => Down::Detonate {
                shooter: f[1].parse().ok()?,
            },
            "Z" if f.len() >= 4 => Down::ShieldShot {
                shooter: f[1].parse().ok()?,
                shot: shot_number(f[2])?,
                aim: aim(f[3], "0", "")?,
            },
            "R" if f.len() >= 4 => Down::Cloud {
                victim: f[1].parse().ok()?,
                effect: CloudEffect::from_id(f[2].parse().ok()?)?,
                locked: f[3] == "1",
            },
            "B" if f.len() >= 5 => Down::Ram {
                victim: f[1].parse().ok()?,
                attacker: f[2].parse().ok()?,
                ram: ram_of(f[3])?,
                wumpa: f[4].parse().ok()?,
            },
            _ => return None,
        })
    }
}

/// The crate layout as the server sends it: as many `W L` lines as it takes, each under
/// [`MAX_LINE`] (`Kartcraft.cpp` `SendLayout`).
pub fn layout(crates: &[CrateSpot]) -> Vec<Down> {
    let mut out = Vec::new();
    let mut first = 0;
    let mut run: Vec<CrateSpot> = Vec::new();
    for (i, c) in crates.iter().enumerate() {
        let mut longer = run.clone();
        longer.push(*c);
        let line = Down::Layout {
            first,
            crates: longer,
        }
        .line();
        if !run.is_empty() && line.len() >= MAX_LINE {
            out.push(Down::Layout {
                first,
                crates: std::mem::take(&mut run),
            });
            first = i;
        }
        run.push(*c);
    }
    if !run.is_empty() {
        out.push(Down::Layout { first, crates: run });
    }
    out
}

/// At most 20 `W` lines a second from one racer; the server drops the rest
/// (`MAX_WEAPON_LINES`).
pub const LINES_PER_SECOND: usize = 20;

/// Our lines to the server, held back so no more than [`LINES_PER_SECOND`] leave in any second:
/// a claim, a shot or a hit waits rather than being dropped.
#[derive(Default)]
pub struct Pacer {
    sent: std::collections::VecDeque<f32>,
    waiting: std::collections::VecDeque<String>,
}

impl Pacer {
    pub fn push(&mut self, line: String) {
        self.waiting.push_back(line);
    }

    /// The lines that may leave at `now` (seconds).
    pub fn due(&mut self, now: f32) -> Vec<String> {
        while self.sent.front().is_some_and(|&t| now - t >= 1.0) {
            self.sent.pop_front();
        }
        let n = LINES_PER_SECOND
            .saturating_sub(self.sent.len())
            .min(self.waiting.len());
        self.sent.extend(std::iter::repeat_n(now, n));
        self.waiting.drain(..n).collect()
    }

    pub fn clear(&mut self) {
        self.waiting.clear();
    }
}

fn shot_number(s: &str) -> Option<u16> {
    s.parse::<u16>().ok().filter(|&n| n != 0)
}

fn hit_kind(s: &str) -> Option<HitKind> {
    let mut c = s.chars();
    let kind = HitKind::from_code(c.next()?)?;
    c.next().is_none().then_some(kind)
}

fn aim(place: &str, target: &str, flags: &str) -> Option<Aim> {
    let v: Vec<f32> = place
        .split(',')
        .map(|p| p.parse::<f32>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()?;
    let [x, y, z, yaw] = v[..] else {
        return None;
    };
    Some(Aim {
        at: [x, y, z],
        yaw,
        target: target.parse().ok()?,
        backward: flags.contains('b'),
        dropped: flags.contains('d'),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aim_at(backward: bool, dropped: bool) -> Aim {
        Aim {
            at: [-8913.25, -117.5, 82.75],
            yaw: 3.125,
            target: 4242,
            backward,
            dropped,
        }
    }

    #[test]
    fn racer_lines_round_trip() {
        for up in [
            Up::Claim(17),
            Up::Break {
                shot: 65535,
                index: 41,
            },
            Up::Fire {
                shot: 65535,
                aim: aim_at(true, false),
            },
            Up::Fire {
                shot: 1,
                aim: aim_at(false, true),
            },
            Up::Hit {
                shooter: 4242,
                shot: 9,
                kind: HitKind::Blasted,
            },
            Up::Hit {
                shooter: 77,
                shot: 0,
                kind: HitKind::TntOnHead,
            },
            Up::Detonate,
            Up::Ram {
                attacker: 4242,
                ram: Ram::Turbo,
            },
            Up::ShieldShot {
                shot: 3,
                aim: Aim {
                    target: 0,
                    ..aim_at(false, false)
                },
            },
            Up::Lose(Loss::SpinOut),
            Up::Lose(Loss::MaskGrab),
        ] {
            let line = up.line();
            assert!(line.len() < MAX_LINE, "{line}");
            assert_eq!(Up::parse(&line), Some(up), "{line}");
        }
        assert_eq!(Up::Lose(Loss::MaskGrab).line(), "KARTCRAFT\tW\tP\tM");
        assert_eq!(
            Up::Break { shot: 3, index: 12 }.line(),
            "KARTCRAFT\tW\tK\t3\t12"
        );
        assert_eq!(Up::parse("KARTCRAFT\tW\tK\t0\t12"), None, "shot 0");
        assert_eq!(Up::parse("KARTCRAFT\tW\tK\t3"), None);
        assert_eq!(Up::parse("KARTCRAFT\tW\tP\tX"), None);
        assert_eq!(
            Up::Fire {
                shot: 5,
                aim: aim_at(true, true)
            }
            .line(),
            "KARTCRAFT\tW\tS\t5\t-8913.25,-117.50,82.75,3.125\t4242\tbd"
        );
    }

    #[test]
    fn server_lines_round_trip() {
        for down in [
            Down::Taken { index: 3, by: 4242 },
            Down::Back { index: 3 },
            Down::Item {
                item: Item::Missile3,
                shots: 3,
            },
            Down::Holding {
                item: Item::Missile3,
                shots: 2,
            },
            Down::Wumpa(10),
            Down::Shot(Shot {
                shooter: 4242,
                item: Item::Bomb3,
                juiced: true,
                shot: 12,
                aim: aim_at(false, false),
            }),
            Down::Hit {
                victim: 7,
                shooter: 4242,
                shot: 12,
                kind: HitKind::SpinOut,
                wumpa: 4,
            },
            Down::Detonate { shooter: 9 },
            Down::Cloud {
                victim: 7,
                effect: CloudEffect::NoHop,
                locked: true,
            },
            Down::Ram {
                victim: 7,
                attacker: 9,
                ram: Ram::Shield,
                wumpa: 2,
            },
            Down::ShieldShot {
                shooter: 9,
                shot: 2,
                aim: Aim {
                    target: 0,
                    ..aim_at(false, false)
                },
            },
        ] {
            let line = down.line();
            assert!(line.len() < MAX_LINE, "{line}");
            assert_eq!(Down::parse(&line), Some(down), "{line}");
        }
    }

    #[test]
    fn the_servers_own_lines_read() {
        // As `Kartcraft.cpp` writes them: no flags leaves the last field empty.
        let s = Down::parse("KARTCRAFT\tW\tS\t12\t6\t0\t1\t-8930.00,-118.00,82.10,1.571\t0\t");
        let Some(Down::Shot(s)) = s else {
            panic!("{s:?}");
        };
        assert_eq!(
            (s.shooter, s.item, s.juiced, s.shot),
            (12, Item::Shield, false, 1)
        );
        assert!(!s.aim.backward && !s.aim.dropped && s.aim.target == 0);
        assert_eq!(
            Down::parse("KARTCRAFT\tW\tH\t12\t30\t4\tT\t0"),
            Some(Down::Hit {
                victim: 12,
                shooter: 30,
                shot: 4,
                kind: HitKind::TntOnHead,
                wumpa: 0
            })
        );
        assert!(Down::parse("KARTCRAFT\tW\tH\t12\t30\t4\tTT\t0").is_none());
        assert!(
            Down::parse("KARTCRAFT\tW\tI\t14\t1").is_none(),
            "no item 14"
        );
        assert!(Down::parse("KARTCRAFT\tR\tG\t1\t4000").is_none());
        assert_eq!(
            Down::parse("KARTCRAFT\tW\tR\t12\t1\t0"),
            Some(Down::Cloud {
                victim: 12,
                effect: CloudEffect::ItemRoll,
                locked: false
            })
        );
        assert!(
            Down::parse("KARTCRAFT\tW\tR\t12\t4\t0").is_none(),
            "no effect 4"
        );
        assert_eq!(
            Down::parse("KARTCRAFT\tW\tB\t12\t30\tM\t7"),
            Some(Down::Ram {
                victim: 12,
                attacker: 30,
                ram: Ram::Mask,
                wumpa: 7
            })
        );
        assert!(Up::parse("KARTCRAFT\tW\tB\t30\tX").is_none());
        assert!(
            Up::parse("KARTCRAFT\tW\tS\t0\t1,2,3,4\t0\t").is_none(),
            "shot 0"
        );
        assert!(Up::parse("KARTCRAFT\tW\tS\t1\t1,2,nan,4\t0\t").is_none());
    }

    #[test]
    fn the_pacer_keeps_to_twenty_lines_a_second() {
        let mut p = Pacer::default();
        for i in 0..30 {
            p.push(format!("{i}"));
        }
        assert_eq!(p.due(0.0).len(), 20);
        assert!(p.due(0.5).is_empty());
        let later = p.due(1.0);
        assert_eq!(later.first().map(String::as_str), Some("20"));
        assert_eq!(later.len(), 10);
        for i in 0..15 {
            p.push(format!("x{i}"));
        }
        assert_eq!(p.due(1.5).len(), 10, "10 of this second's 20 are left");
        assert!(p.due(1.9).is_empty());
        assert_eq!(p.due(2.0).len(), 5);
    }

    #[test]
    fn a_long_layout_splits_under_the_line_limit() {
        let crates: Vec<CrateSpot> = (0..60)
            .map(|i| CrateSpot {
                kind: [CrateKind::Weapon, CrateKind::Fruit, CrateKind::Wumpa][i % 3],
                at: [-8930.25 + i as f32, -118.5 - i as f32, 82.0],
            })
            .collect();
        let lines = layout(&crates);
        assert!(lines.len() > 1);
        let mut back = Vec::new();
        for down in &lines {
            let line = down.line();
            assert!(line.len() < MAX_LINE, "{} bytes", line.len());
            let Some(Down::Layout { first, crates }) = Down::parse(&line) else {
                panic!("{line}");
            };
            assert_eq!(first, back.len());
            back.extend(crates);
        }
        assert_eq!(back, crates);
    }
}
