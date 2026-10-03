// SPDX-License-Identifier: GPL-3.0-or-later

//! The Wii U GameCube controller adapter (WUP-028, and the adapters that copy it, such as
//! Mayflash's in Wii U mode), read over USB the way Dolphin's GCAdapter reads it. On Windows it
//! needs the WinUSB driver (Zadig installs it), which HIDUSBF can overclock. A thread keeps the
//! latest report and sends rumble; `ports` turns the report into what `PADRead` would return,
//! doing the origin calibration the console's SI and PAD library would.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nusb::MaybeFuture;
use nusb::transfer::{Buffer, In, Interrupt, Out};
use ssbm_sdk::PadStatus;

const VENDOR: u16 = 0x057e;
const PRODUCT: u16 = 0x0337;
/// A report: this byte, then nine per port.
const REPORT: u8 = 0x21;
const REPORT_LEN: usize = 37;
/// Starts the reports.
const START: u8 = 0x13;
/// Sets the four motors.
const RUMBLE: u8 = 0x11;

// PADStatus buttons.
const X: u16 = 0x0400;
const Y: u16 = 0x0800;
const START_BUTTON: u16 = 0x1000;

pub struct Adapter {
    shared: Mutex<Shared>,
}

#[derive(Default)]
struct Shared {
    /// The latest report, while the adapter is open.
    report: Option<[u8; REPORT_LEN]>,
    ports: [Port; 4],
    /// The motors the game wants on, and those the adapter was last told.
    rumble: [bool; 4],
    sent: [bool; 4],
}

#[derive(Clone, Copy, Default)]
struct Port {
    connected: bool,
    wireless: bool,
    /// Stick x and y, C-stick x and y, L and R, as the controller reported them when plugged in.
    origin: [u8; 6],
    /// Since when X, Y and Start alone have been held.
    combo: Option<Instant>,
}

/// Starts reading the adapter, now and whenever it's plugged in later.
pub fn start() -> Arc<Adapter> {
    let adapter = Arc::new(Adapter {
        shared: Mutex::default(),
    });
    let a = adapter.clone();
    std::thread::Builder::new()
        .name("gc-adapter".into())
        .spawn(move || a.run())
        .expect("the adapter thread");
    adapter
}

impl Adapter {
    fn run(&self) {
        let mut said = String::new();
        loop {
            let found = nusb::list_devices()
                .wait()
                .ok()
                .and_then(|mut d| d.find(|d| d.vendor_id() == VENDOR && d.product_id() == PRODUCT));
            if let Some(info) = found {
                let Err(e) = self.read(&info);
                let was_open = {
                    let mut s = self.shared.lock().unwrap();
                    s.sent = [false; 4];
                    s.report.take().is_some()
                };
                if was_open {
                    eprintln!("window: GameCube adapter disconnected ({e})");
                    said.clear();
                } else {
                    // Said once, not every second.
                    let msg = format!(
                        "window: found a GameCube adapter but can't read it ({e}); it needs the \
                         WinUSB driver (Zadig) on Windows, and no other program (Dolphin) can have it open"
                    );
                    if msg != said {
                        eprintln!("{msg}");
                        said = msg;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    /// Reads reports until the adapter goes away.
    fn read(&self, info: &nusb::DeviceInfo) -> Result<Infallible, String> {
        let device = info.open().wait().map_err(|e| e.to_string())?;
        let intf = device
            .claim_interface(0)
            .wait()
            .map_err(|e| e.to_string())?;
        let mut out = intf
            .endpoint::<Interrupt, Out>(0x02)
            .map_err(|e| e.to_string())?;
        let mut send = |bytes: &[u8]| {
            let mut b = Buffer::new(bytes.len());
            b.extend_from_slice(bytes);
            out.transfer_blocking(b, Duration::from_millis(100))
                .status
                .map_err(|e| e.to_string())
        };
        send(&[START])?;
        let mut input = intf
            .endpoint::<Interrupt, In>(0x81)
            .map_err(|e| e.to_string())?;
        let len = input.max_packet_size();
        for _ in 0..4 {
            input.submit(Buffer::new(len));
        }
        let mut first = true;
        loop {
            let Some(c) = input.wait_next_complete(Duration::from_secs(1)) else {
                continue;
            };
            c.status.map_err(|e| e.to_string())?;
            let data = &c.buffer[..c.actual_len];
            let rumble = {
                let mut s = self.shared.lock().unwrap();
                if data.len() == REPORT_LEN && data[0] == REPORT {
                    s.report = Some(data.try_into().unwrap());
                    if first {
                        eprintln!("window: GameCube adapter connected");
                        first = false;
                    }
                }
                // WaveBirds have no motor.
                let mut want = s.rumble;
                for (w, p) in want.iter_mut().zip(&s.ports) {
                    *w &= p.connected && !p.wireless;
                }
                (want != s.sent).then(|| {
                    s.sent = want;
                    want
                })
            };
            input.submit(c.buffer);
            if let Some(m) = rumble {
                send(&[RUMBLE, m[0] as u8, m[1] as u8, m[2] as u8, m[3] as u8])?;
            }
        }
    }

    /// The four ports as `PADRead` would report them; `None` where no controller is plugged in.
    pub fn ports(&self) -> [Option<PadStatus>; 4] {
        let mut s = self.shared.lock().unwrap();
        let Some(report) = s.report else {
            s.ports = Default::default();
            return [None; 4];
        };
        let mut out = [None; 4];
        for (i, port) in s.ports.iter_mut().enumerate() {
            let d = &report[1 + 9 * i..][..9];
            let kind = d[0] >> 4;
            let raw = [d[3], d[4], d[5], d[6], d[7], d[8]];
            // 1 wired, 2 wireless. A WaveBird that isn't on yet reports its sticks at 0.
            if kind == 0 || raw[..4] == [0; 4] {
                port.connected = false;
                continue;
            }
            let button = buttons(d[1], d[2]);
            if !port.connected {
                *port = Port {
                    connected: true,
                    wireless: kind == 2,
                    origin: raw,
                    combo: None,
                };
            }
            // X, Y and Start held alone for three seconds take the origin again, as on the console.
            if button == X | Y | START_BUTTON {
                match port.combo {
                    None => port.combo = Some(Instant::now()),
                    Some(t) if t.elapsed() >= Duration::from_secs(3) => {
                        port.origin = raw;
                        port.combo = None;
                    }
                    Some(_) => {}
                }
            } else {
                port.combo = None;
            }
            out[i] = Some(status(button, raw, port.origin));
        }
        out
    }

    /// Turns the motors on or off, for ports with a controller that has one.
    pub fn rumble(&self, on: [bool; 4]) {
        self.shared.lock().unwrap().rumble = on;
    }

    /// Stops the motors before the program exits, waiting briefly for the adapter to hear it.
    pub fn stop(&self) {
        self.rumble([false; 4]);
        let until = Instant::now() + Duration::from_millis(100);
        while Instant::now() < until {
            let s = self.shared.lock().unwrap();
            if s.report.is_none() || s.sent == [false; 4] {
                return;
            }
            drop(s);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// The adapter's two button bytes as `PADStatus` buttons.
fn buttons(b1: u8, b2: u8) -> u16 {
    const BITS: [(u8, u8, u16); 12] = [
        (0, 0x01, 0x0100), // A
        (0, 0x02, 0x0200), // B
        (0, 0x04, X),
        (0, 0x08, Y),
        (0, 0x10, 0x0001), // left
        (0, 0x20, 0x0002), // right
        (0, 0x40, 0x0004), // down
        (0, 0x80, 0x0008), // up
        (1, 0x01, START_BUTTON),
        (1, 0x02, 0x0010), // Z
        (1, 0x04, 0x0020), // R
        (1, 0x08, 0x0040), // L
    ];
    let mut out = 0;
    for (byte, bit, b) in BITS {
        if [b1, b2][byte as usize] & bit != 0 {
            out |= b;
        }
    }
    out
}

/// What the PAD library makes of a controller's raw values (`SPEC2_MakeStatus` in analog mode 3,
/// the default), given its origin (`UpdateOrigin`).
fn status(button: u16, raw: [u8; 6], origin: [u8; 6]) -> PadStatus {
    let centered = |v: u8| v.wrapping_sub(128) as i8;
    let mut org = origin.map(centered);
    // The X patch, for controllers that report a bad stick x origin.
    if org[0] > 0x40 {
        org[0] = 0;
    }
    PadStatus {
        connected: true,
        button,
        stick_x: clamp_s8(centered(raw[0]), org[0]),
        stick_y: clamp_s8(centered(raw[1]), org[1]),
        substick_x: clamp_s8(centered(raw[2]), org[2]),
        substick_y: clamp_s8(centered(raw[3]), org[3]),
        trigger_l: raw[4].max(origin[4]) - origin[4],
        trigger_r: raw[5].max(origin[5]) - origin[5],
        analog_a: 0,
        analog_b: 0,
    }
}

/// `ClampS8`: the value less the origin, kept in range.
fn clamp_s8(v: i8, org: i8) -> i8 {
    let v = if org > 0 {
        v.max(i8::MIN + org)
    } else if org < 0 {
        v.min(i8::MAX + org)
    } else {
        v
    };
    v.wrapping_sub(org)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_calibration() {
        // At rest at the origin: centered.
        let origin = [0x7f, 0x80, 0x82, 0x7e, 0x20, 0x00];
        let p = status(0, origin, origin);
        assert_eq!(
            (p.stick_x, p.stick_y, p.substick_x, p.substick_y),
            (0, 0, 0, 0)
        );
        assert_eq!((p.trigger_l, p.trigger_r), (0, 0));
        // Full left from an origin of -1: clamped to -127 rather than wrapping.
        let p = status(0, [0x00, 0xff, 0x82, 0x7e, 0x10, 0xff], origin);
        assert_eq!((p.stick_x, p.stick_y), (-127, 127));
        // Triggers below their origin read 0.
        assert_eq!((p.trigger_l, p.trigger_r), (0, 0xff));
        // The X patch: a stick x origin past 0x40 counts as 0.
        let p = status(
            0,
            [0xd0, 0x80, 0x80, 0x80, 0, 0],
            [0xd0, 0x80, 0x80, 0x80, 0, 0],
        );
        assert_eq!(p.stick_x, 0x50);
    }

    #[test]
    fn adapter_buttons() {
        assert_eq!(buttons(0x01, 0x00), 0x0100);
        assert_eq!(buttons(0x0c, 0x01), X | Y | START_BUTTON);
        assert_eq!(buttons(0x80, 0x0e), 0x0008 | 0x0010 | 0x0020 | 0x0040);
    }
}
