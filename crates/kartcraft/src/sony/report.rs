//! The PlayStation pads' HID reports, as the Linux `hid-playstation` driver and SDL read and
//! write them: the DualSense's and the DualShock 4's input over USB and Bluetooth (its short
//! report until the pad is switched to its full one, then the full one), and the rumble output,
//! a Bluetooth one closed with a CRC-32 seeded with the `0xA2` output header.

/// Sony's USB vendor id.
pub const SONY: u16 = 0x054C;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    DualSense,
    DualShock4,
}

impl Model {
    /// The pad a Sony product id is: the DualSense and its Edge, the two DualShock 4s and the
    /// DualShock 4's wireless adapter.
    pub fn of(product: u16) -> Option<Self> {
        match product {
            0x0CE6 | 0x0DF2 => Some(Self::DualSense),
            0x05C4 | 0x09CC | 0x0BA0 => Some(Self::DualShock4),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::DualSense => "DualSense Wireless Controller",
            Self::DualShock4 => "DUALSHOCK 4 Wireless Controller",
        }
    }
}

/// A pad's buttons, as bits of [`PadState::buttons`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    Cross,
    Circle,
    Square,
    Triangle,
    L1,
    R1,
    L2,
    R2,
    /// Create on the DualSense, Share on the DualShock 4.
    Share,
    Options,
    L3,
    R3,
    Ps,
    Touchpad,
    Up,
    Right,
    Down,
    Left,
}

impl Button {
    pub const ALL: [Button; 18] = [
        Button::Cross,
        Button::Circle,
        Button::Square,
        Button::Triangle,
        Button::L1,
        Button::R1,
        Button::L2,
        Button::R2,
        Button::Share,
        Button::Options,
        Button::L3,
        Button::R3,
        Button::Ps,
        Button::Touchpad,
        Button::Up,
        Button::Right,
        Button::Down,
        Button::Left,
    ];

    pub fn bit(self) -> u32 {
        1 << self as u8
    }
}

/// A pad's state from one input report: the sticks -1 to 1 with up positive (left x, left y,
/// right x, right y), the analog triggers 0 to 1, and the buttons as [`Button`] bits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    pub sticks: [f32; 4],
    pub l2: f32,
    pub r2: f32,
    pub buttons: u32,
}

impl PadState {
    pub fn pressed(&self, b: Button) -> bool {
        self.buttons & b.bit() != 0
    }
}

/// A stick byte (0 left or up, 0x80 centre, 0xFF right or down) as -1 to 1.
fn stick(v: u8) -> f32 {
    ((f32::from(v) - 128.0) / 127.0).clamp(-1.0, 1.0)
}

/// The face buttons and the hat (`b0`), the shoulders and the middle buttons (`b1`), PS and the
/// touchpad click (`b2`): the same three bytes on both pads.
fn buttons(b0: u8, b1: u8, b2: u8) -> u32 {
    use Button::*;
    let mut bits = 0;
    let mut set = |on: bool, b: Button| {
        if on {
            bits |= b.bit();
        }
    };
    // The hat, clockwise from up; 8 and over is centred.
    let hat = b0 & 0x0F;
    set(matches!(hat, 7 | 0 | 1), Up);
    set(matches!(hat, 1..=3), Right);
    set(matches!(hat, 3..=5), Down);
    set(matches!(hat, 5..=7), Left);
    set(b0 & 0x10 != 0, Square);
    set(b0 & 0x20 != 0, Cross);
    set(b0 & 0x40 != 0, Circle);
    set(b0 & 0x80 != 0, Triangle);
    set(b1 & 0x01 != 0, L1);
    set(b1 & 0x02 != 0, R1);
    set(b1 & 0x04 != 0, L2);
    set(b1 & 0x08 != 0, R2);
    set(b1 & 0x10 != 0, Share);
    set(b1 & 0x20 != 0, Options);
    set(b1 & 0x40 != 0, L3);
    set(b1 & 0x80 != 0, R3);
    set(b2 & 0x01 != 0, Ps);
    set(b2 & 0x02 != 0, Touchpad);
    bits
}

/// The DualShock 4's layout, which the DualSense's short Bluetooth report shares: the sticks, the
/// three button bytes, then the triggers.
fn short_layout(d: &[u8]) -> Option<PadState> {
    let d = d.get(..9)?;
    Some(PadState {
        sticks: [stick(d[0]), -stick(d[1]), stick(d[2]), -stick(d[3])],
        buttons: buttons(d[4], d[5], d[6]),
        l2: f32::from(d[7]) / 255.0,
        r2: f32::from(d[8]) / 255.0,
    })
}

/// The DualSense's full layout: the sticks, the triggers, a sequence byte, the button bytes.
fn dualsense_layout(d: &[u8]) -> Option<PadState> {
    let d = d.get(..10)?;
    Some(PadState {
        sticks: [stick(d[0]), -stick(d[1]), stick(d[2]), -stick(d[3])],
        l2: f32::from(d[4]) / 255.0,
        r2: f32::from(d[5]) / 255.0,
        buttons: buttons(d[7], d[8], d[9]),
    })
}

/// An input report's state, or `None` for one that carries none (or is too short).
/// DualSense: USB `0x01` (64 bytes, full), Bluetooth `0x01` (short, until switched) and `0x31`
/// (full, after a sequence byte). DualShock 4: `0x01` over either, Bluetooth `0x11` (full, after
/// two bytes).
pub fn parse(model: Model, report: &[u8]) -> Option<PadState> {
    let (&id, rest) = report.split_first()?;
    match (model, id) {
        (Model::DualSense, 0x01) if report.len() >= 64 => dualsense_layout(rest),
        (Model::DualSense, 0x01) => short_layout(rest),
        (Model::DualSense, 0x31) => dualsense_layout(rest.get(1..)?),
        (Model::DualShock4, 0x01) => short_layout(rest),
        (Model::DualShock4, 0x11) => short_layout(rest.get(2..)?),
        _ => None,
    }
}

/// CRC-32 (IEEE, reflected) of `bytes`, continuing from `crc`'s complement as zlib does.
fn crc32(seed: u32, bytes: &[u8]) -> u32 {
    let mut crc = !seed;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A Bluetooth output report's CRC: over the `0xA2` header and every byte before the last four,
/// little-endian into those four.
fn close_bt(report: &mut [u8]) {
    let n = report.len() - 4;
    let crc = crc32(crc32(0, &[0xA2]), &report[..n]);
    report[n..].copy_from_slice(&crc.to_le_bytes());
}

/// The output report that sets the motors (`strong` the left, heavy one; `weak` the right) and
/// nothing else: no light bar, no LEDs. `seq` numbers the DualSense's Bluetooth reports (its low
/// four bits are sent).
pub fn rumble(model: Model, bluetooth: bool, seq: u8, strong: u8, weak: u8) -> Vec<u8> {
    match (model, bluetooth) {
        (Model::DualSense, false) => {
            let mut r = vec![0u8; 48];
            r[0] = 0x02;
            dualsense_common(&mut r[1..], strong, weak);
            r
        }
        (Model::DualSense, true) => {
            let mut r = vec![0u8; 78];
            r[0] = 0x31;
            r[1] = (seq & 0x0F) << 4;
            // `DS_OUTPUT_TAG`.
            r[2] = 0x10;
            dualsense_common(&mut r[3..], strong, weak);
            close_bt(&mut r);
            r
        }
        (Model::DualShock4, false) => {
            let mut r = vec![0u8; 32];
            r[0] = 0x05;
            // `DS4_OUTPUT_VALID_FLAG0_MOTOR`.
            r[1] = 0x01;
            r[4] = weak;
            r[5] = strong;
            r
        }
        (Model::DualShock4, true) => {
            let mut r = vec![0u8; 78];
            r[0] = 0x11;
            // `DS4_OUTPUT_HWCTL_HID | DS4_OUTPUT_HWCTL_CRC32`.
            r[1] = 0xC0;
            r[3] = 0x01;
            r[6] = weak;
            r[7] = strong;
            close_bt(&mut r);
            r
        }
    }
}

/// The DualSense's common output block (47 bytes): the motors, as compatible vibration and its
/// second form for newer firmware (`valid_flag0` bits 0 and 1, `valid_flag2` bit 2).
fn dualsense_common(c: &mut [u8], strong: u8, weak: u8) {
    c[0] = 0x03;
    c[2] = weak;
    c[3] = strong;
    c[38] = 0x04;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_is_zlib_s() {
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xCBF4_3926);
    }

    #[test]
    fn a_dualsense_usb_report_reads_its_sticks_triggers_and_buttons() {
        let mut r = [0u8; 64];
        r[0] = 0x01;
        r[1..5].copy_from_slice(&[0xFF, 0x00, 0x80, 0x80]);
        r[5] = 0xFF; // L2
        r[6] = 0x00; // R2
        r[8] = 0x20 | 0x02; // cross, hat right
        r[9] = 0x02 | 0x80; // R1, R3
        r[10] = 0x01; // PS
        let s = parse(Model::DualSense, &r).unwrap();
        assert_eq!(s.sticks[0], 1.0);
        assert_eq!(s.sticks[1], 1.0, "stick up is positive");
        assert!(s.sticks[2].abs() < 0.01);
        assert_eq!((s.l2, s.r2), (1.0, 0.0));
        for b in [
            Button::Cross,
            Button::Right,
            Button::R1,
            Button::R3,
            Button::Ps,
        ] {
            assert!(s.pressed(b), "{b:?}");
        }
        assert!(!s.pressed(Button::Up) && !s.pressed(Button::Circle));
    }

    #[test]
    fn a_dualsense_bluetooth_full_report_reads_past_its_sequence_byte() {
        let mut r = [0u8; 78];
        r[0] = 0x31;
        r[1] = 0x50;
        r[2..6].copy_from_slice(&[0x80, 0x80, 0x80, 0x80]);
        r[7] = 0x80; // R2 half
        r[9] = 0x08 | 0x40; // hat centred, circle
        let s = parse(Model::DualSense, &r).unwrap();
        assert!(s.pressed(Button::Circle));
        assert!(!s.pressed(Button::Up) && !s.pressed(Button::Left));
        assert!((s.r2 - 0.502).abs() < 0.01);
    }

    #[test]
    fn short_reports_read_the_dualshock_layout_on_both_pads() {
        // DualSense over Bluetooth before the switch, DualShock 4 over USB and its Bluetooth 0x11.
        let mut short = [0u8; 10];
        short[0] = 0x01;
        short[1..5].copy_from_slice(&[0x80, 0x80, 0x80, 0x80]);
        short[5] = 0x07 | 0x10; // hat up-left, square
        short[6] = 0x01; // L1
        short[8] = 0xFF; // L2
        for model in [Model::DualSense, Model::DualShock4] {
            let s = parse(model, &short).unwrap();
            assert!(
                s.pressed(Button::Square) && s.pressed(Button::L1),
                "{model:?}"
            );
            assert!(
                s.pressed(Button::Up) && s.pressed(Button::Left),
                "{model:?}"
            );
            assert_eq!(s.l2, 1.0);
        }
        let mut bt = [0u8; 78];
        bt[0] = 0x11;
        bt[3..12].copy_from_slice(&short[1..]);
        assert_eq!(
            parse(Model::DualShock4, &bt),
            parse(Model::DualShock4, &short)
        );
    }

    #[test]
    fn rumble_reports_put_the_motors_where_each_pad_reads_them() {
        let usb = rumble(Model::DualSense, false, 0, 200, 50);
        assert_eq!((usb.len(), usb[0], usb[1]), (48, 0x02, 0x03));
        assert_eq!((usb[3], usb[4]), (50, 200));
        let bt = rumble(Model::DualSense, true, 0x13, 200, 50);
        assert_eq!((bt.len(), bt[0], bt[1], bt[2]), (78, 0x31, 0x30, 0x10));
        assert_eq!((bt[3], bt[5], bt[6], bt[41]), (0x03, 50, 200, 0x04));
        let ds4 = rumble(Model::DualShock4, false, 0, 200, 50);
        assert_eq!((ds4[0], ds4[1], ds4[4], ds4[5]), (0x05, 0x01, 50, 200));
        let ds4bt = rumble(Model::DualShock4, true, 0, 200, 50);
        assert_eq!(
            (ds4bt[0], ds4bt[1], ds4bt[6], ds4bt[7]),
            (0x11, 0xC0, 50, 200)
        );
        // Bluetooth reports end in the CRC of the 0xA2 header and the rest.
        for r in [bt, ds4bt] {
            let mut body = vec![0xA2];
            body.extend_from_slice(&r[..74]);
            assert_eq!(r[74..], crc32(0, &body).to_le_bytes());
        }
    }
}
