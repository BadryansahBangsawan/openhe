//! A stateful WIN 68 HE MAX simulator behind the `Transport` trait.
//!
//! The keyboard state lives in a [`SimState`] shared (`Arc<Mutex<_>>`)
//! between the test and every [`SimTransport`] handle the app opens. It
//! models the firmware as far as the hardware findings go (PROTOCOL.md 4.1
//! and 7.x):
//!
//! - the real 6×21 WIN 68 matrix (68 keys, row 0 empty);
//! - per profile (0..=3): every KEY layout of every key with the factory
//!   values (finding 9) and the factory layer tables (finding 16), the
//!   advanced-function packets (TGL/MPT/END/SOCD stored per key, DKS and MT
//!   stored into their KEY layouts), PRGB lighting (finding 22 bytes), KRGB
//!   colours (uncommitted until the FE commit), macro data and MACRO_MODE
//!   (its GET reports len 0, finding 24), the Win/Mac mode (it swaps the
//!   Base codes of LAlt and LWin, finding 16) and the polling rate;
//! - lighting: only the LEN-15 PRGB GET works; a LEN-44 GET and RGBNACK get
//!   a 0xFF DeviceFail reply (findings 14, 22); unknown flag bits are kept;
//! - a profile switch, a polling change and a factory reset re-enumerate
//!   USB: the handle that wrote it is dead (no reply, then `Disconnected`)
//!   and the app has to open a new one through the connector;
//! - RM6X21 travel and calibration frames (3 reports each) and calibration
//!   start/stop.
//!
//! Faults can be injected: DeviceFail replies for chosen requests, an
//! unplug after N more writes, and connects that fail.

#![allow(dead_code)]

use openhe_app::worker::AppTransport;
use openhe_device::{Error, Keyboard, Transport};
use openhe_proto::Model;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

/// The WIN 68 HE MAX matrix read from hardware (PROTOCOL 4.1).
pub const MATRIX: [[u8; 21]; 6] = {
    let mut m = [[0u8; 21]; 6];
    let rows: [[u8; 15]; 5] = [
        [
            0x29, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2D, 0x2E, 0x2A,
            0x49,
        ],
        [
            0x2B, 0x14, 0x1A, 0x08, 0x15, 0x17, 0x1C, 0x18, 0x0C, 0x12, 0x13, 0x2F, 0x30, 0x31,
            0x4C,
        ],
        [
            0x39, 0x04, 0x16, 0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F, 0x33, 0x34, 0x00, 0x28,
            0x4B,
        ],
        [
            0xE1, 0x00, 0x1D, 0x1B, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0xE5, 0x52,
            0x4E,
        ],
        [
            0xE0, 0xE3, 0xE2, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00, 0xE6, 0x01, 0xE4, 0x50, 0x51,
            0x4F,
        ],
    ];
    let mut r = 0;
    while r < 5 {
        let mut c = 0;
        while c < 15 {
            m[r + 1][c] = rows[r][c];
            c += 1;
        }
        r += 1;
    }
    m
};

/// Every key code of the matrix, row-major.
pub fn keys() -> Vec<u8> {
    MATRIX
        .iter()
        .flatten()
        .copied()
        .filter(|&c| c != 0)
        .collect()
}

/// (row, col) of `key` in the matrix.
pub fn position(key: u8) -> (usize, usize) {
    for (r, row) in MATRIX.iter().enumerate() {
        for (c, &k) in row.iter().enumerate() {
            if k == key && k != 0 {
                return (r, c);
            }
        }
    }
    panic!("key 0x{key:02X} is not in the WIN 68 matrix")
}

pub const FN: u8 = 0x01;
pub const TRANSPARENT: u16 = 0x0001;

/// Factory Fn1 layer (finding 16); every other key is 0x0001.
pub const FACTORY_FN1: [(u8, u16); 26] = [
    (0x04, 0xF101),
    (0x16, 0xF102),
    (0x13, 0x0047),
    (0x1E, 0x003A),
    (0x1F, 0x003B),
    (0x20, 0x003C),
    (0x21, 0x003D),
    (0x22, 0x003E),
    (0x23, 0x003F),
    (0x24, 0x0040),
    (0x25, 0x0041),
    (0x26, 0x0042),
    (0x27, 0x0043),
    (0x2D, 0x0045),
    (0x29, 0x0035),
    (0x2C, 0xF100),
    (0x2F, 0x004A),
    (0x30, 0x004D),
    (0x33, 0x0046),
    (0x34, 0x0048),
    (0x39, 0xF344),
    (0x49, 0xF317),
    (0x4B, 0xF310),
    (0x4C, 0xF311),
    (0x4F, 0xF314),
    (0x50, 0xF315),
];

/// The rest of the factory Fn1 table (split for the array length).
pub const FACTORY_FN1_MORE: [(u8, u16); 3] = [(0x51, 0xF313), (0x52, 0xF312), (0xE4, 0xF002)];

/// Factory Fn2 layer (finding 16); every other key is 0x0001.
pub const FACTORY_FN2: [(u8, u16); 10] = [
    (0x1E, 0x1070),
    (0x1F, 0x106F),
    (0x20, 0x6152),
    (0x21, 0x681B),
    (0x24, 0x10B6),
    (0x25, 0x10CD),
    (0x26, 0x10B5),
    (0x27, 0x10E2),
    (0x2D, 0x10EA),
    (0x2E, 0x10E9),
];

/// The factory code of `key` on layer 0..=3 (Windows mode).
pub fn factory_layer(key: u8, layer: u8) -> u16 {
    let find = |t: &[(u8, u16)]| t.iter().find(|&&(k, _)| k == key).map(|&(_, c)| c);
    match layer {
        0 if key == FN => 0xF001,
        0 => u16::from(key),
        1 => find(&FACTORY_FN1)
            .or_else(|| find(&FACTORY_FN1_MORE))
            .unwrap_or(TRANSPARENT),
        2 => find(&FACTORY_FN2).unwrap_or(TRANSPARENT),
        _ => TRANSPARENT,
    }
}

/// Factory value of KEY `layout` for `key` (finding 9).
pub fn factory_param(key: u8, layout: u8) -> u16 {
    match layout {
        0..=3 => factory_layer(key, layout),
        4 => 2000,
        20 | 21 => 20,
        22 | 23 => 200,
        _ => 0,
    }
}

/// Every KEY layout the simulator stores.
pub const LAYOUTS: [u8; 21] = [
    0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16, 19, 20, 21, 22, 23, 25,
];

/// The PRGB reply body read from hardware (finding 22): R[0..44].
pub const FACTORY_LIGHTING: [u8; 44] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0x7F, 0x7F,
    0xFF, 0xFF, 0x00, 0x00, 0xFF, 0x7F, 0x00, 0x7F, 0xFF, 0x7F, 0x7F, 0x00, 0xFF, 0x7F, 0x7F, 0x7F,
    0xFF, 0x00, 0x00, 0x00, 0x00, 0xC1, 0x03, 0x03, 0x03, 0x02, 0x06, 0x07,
];

/// Offsets in the PRGB body.
pub mod prgb {
    pub const COLORS: usize = 5;
    pub const FLAGS: usize = 37;
    pub const BRIGHTNESS: usize = 38;
    pub const MODE: usize = 39;
    pub const SPEED: usize = 40;
    pub const SLEEP: usize = 41;
    pub const STATIC: usize = 42;
    pub const DYNAMIC: usize = 43;
}

/// One onboard profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// QueryWinMode value: 1 Windows, 0 Mac.
    pub win: u8,
    /// Polling index 0 (8 kHz) ..= 6 (125 Hz).
    pub polling: u8,
    /// (key, layout) → value.
    pub keys: BTreeMap<(u8, u8), u16>,
    /// PRGB body R[0..44].
    pub lighting: [u8; 44],
    /// Committed per-key colours.
    pub krgb: BTreeMap<u8, [u8; 3]>,
    /// Per-key colours written but not yet committed with FE.
    pub krgb_pending: BTreeMap<u8, [u8; 3]>,
    /// Function packets (without the flag), by (cmd, key).
    pub funcs: BTreeMap<(u8, u8), Vec<u8>>,
    /// Macro actions by address: (code, word).
    pub macro_data: BTreeMap<u16, (u16, u32)>,
    /// MACRO_MODE SET payloads without the flag, by key.
    pub macro_mode: BTreeMap<u8, Vec<u8>>,
}

impl Profile {
    pub fn factory() -> Profile {
        let mut keys = BTreeMap::new();
        for k in self::keys() {
            for l in LAYOUTS {
                keys.insert((k, l), factory_param(k, l));
            }
        }
        Profile {
            win: 1,
            polling: 0,
            keys,
            lighting: FACTORY_LIGHTING,
            krgb: BTreeMap::new(),
            krgb_pending: BTreeMap::new(),
            funcs: BTreeMap::new(),
            macro_data: BTreeMap::new(),
            macro_mode: BTreeMap::new(),
        }
    }

    pub fn key(&self, key: u8, layout: u8) -> u16 {
        self.keys.get(&(key, layout)).copied().unwrap_or(0)
    }

    /// The Mode (layout 8) of `key`.
    pub fn mode(&self, key: u8) -> u16 {
        self.key(key, 8)
    }
}

/// A request the simulator answers with a 0xFF DeviceFail reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reject {
    pub cmd: u8,
    /// Only SETs (flag 01, or an order with an argument).
    pub sets_only: bool,
    /// Payload bytes that must match: (index, value).
    pub at: Vec<(usize, u8)>,
    /// How many more times (u32::MAX: always).
    pub times: u32,
}

/// One frame the app wrote: (cmd, payload).
pub type Written = (u8, Vec<u8>);

#[derive(Debug)]
pub struct SimState {
    pub profile: u8,
    pub profiles: [Profile; 4],
    /// Connection generation: only a handle opened in the current one works.
    pub conn: u64,
    /// The keyboard is on the bus (false while unplugged).
    pub present: bool,
    /// How many upcoming connects fail (the keyboard is still enumerating).
    pub connect_failures: u32,
    /// Connects so far.
    pub connects: u32,
    /// USB re-enumerations so far (profile, polling, reset).
    pub reenumerations: u32,
    pub calibrating: bool,
    pub calibration_starts: u32,
    pub calibration_stops: u32,
    pub factory_resets: u32,
    pub krgb_commits: u32,
    /// A LEN-44 PRGB GET reached the keyboard (the hazard of finding 14).
    pub bad_prgb_gets: u32,
    pub rgbnack: u32,
    /// Live travel in µm by matrix position.
    pub travel: [[u16; 21]; 6],
    /// Raw calibration values by matrix position.
    pub calibration: [[u16; 21]; 6],
    /// Every frame written, in order.
    pub log: Vec<Written>,
    pub rejects: Vec<Reject>,
    /// Unplug (no reply) when this many more SETs have been written.
    pub unplug_after_sets: Option<u32>,
    /// The next write of (cmd, payload[0]) fails with `Disconnected`
    /// before reaching the keyboard (a handle that went stale), and the
    /// connection is dead; the keyboard stays on the bus.
    pub drop_before: Option<(u8, u8)>,
    /// Time every SET takes (a slow keyboard), for tests that must catch
    /// a job while it runs.
    pub set_delay: std::time::Duration,
    /// Unsolicited reports to deliver on the next read.
    pub events: VecDeque<[u8; 64]>,
}

impl SimState {
    pub fn factory() -> SimState {
        let mut calibration = [[0u16; 21]; 6];
        for (r, row) in MATRIX.iter().enumerate() {
            for (c, &k) in row.iter().enumerate() {
                if k != 0 {
                    calibration[r][c] = 2200 + (r * 21 + c) as u16;
                }
            }
        }
        SimState {
            profile: 0,
            profiles: [
                Profile::factory(),
                Profile::factory(),
                Profile::factory(),
                Profile::factory(),
            ],
            conn: 1,
            present: true,
            connect_failures: 0,
            connects: 0,
            reenumerations: 0,
            calibrating: false,
            calibration_starts: 0,
            calibration_stops: 0,
            factory_resets: 0,
            krgb_commits: 0,
            bad_prgb_gets: 0,
            rgbnack: 0,
            travel: [[0; 21]; 6],
            calibration,
            log: Vec::new(),
            rejects: Vec::new(),
            unplug_after_sets: None,
            drop_before: None,
            set_delay: std::time::Duration::ZERO,
            events: VecDeque::new(),
        }
    }

    /// The active profile.
    pub fn p(&self) -> &Profile {
        &self.profiles[usize::from(self.profile)]
    }

    pub fn p_mut(&mut self) -> &mut Profile {
        &mut self.profiles[usize::from(self.profile)]
    }

    /// Drops the current connection (USB re-enumeration): the keyboard is
    /// back on the bus at once, but every open handle is dead.
    pub fn reenumerate(&mut self) {
        self.conn += 1;
        self.reenumerations += 1;
    }

    pub fn unplug(&mut self) {
        self.present = false;
        self.conn += 1;
    }

    pub fn replug(&mut self) {
        self.present = true;
    }

    /// Writes (SETs) logged so far, for a command.
    pub fn sets(&self, cmd: u8) -> Vec<Vec<u8>> {
        self.log
            .iter()
            .filter(|(c, p)| *c == cmd && is_set(*c, p))
            .map(|(_, p)| p.clone())
            .collect()
    }

    /// Number of SETs of any kind written so far.
    pub fn set_count(&self) -> usize {
        self.log.iter().filter(|(c, p)| is_set(*c, p)).count()
    }

    /// Order frames (cmd 0x00) whose TYPE is `ty`.
    pub fn orders(&self, ty: u8) -> usize {
        self.log
            .iter()
            .filter(|(c, p)| *c == 0 && p.first() == Some(&ty))
            .count()
    }

    /// Presses `key` to `um` (live travel).
    pub fn press(&mut self, key: u8, um: u16) {
        let (r, c) = position(key);
        self.travel[r][c] = um;
    }

    fn take_reject(&mut self, cmd: u8, payload: &[u8]) -> bool {
        let set = is_set(cmd, payload);
        let hit = self.rejects.iter_mut().find(|r| {
            r.cmd == cmd
                && (!r.sets_only || set)
                && r.times > 0
                && r.at.iter().all(|&(i, v)| payload.get(i) == Some(&v))
        });
        match hit {
            Some(r) => {
                if r.times != u32::MAX {
                    r.times -= 1;
                }
                true
            }
            None => false,
        }
    }
}

/// Whether a request writes something.
pub fn is_set(cmd: u8, payload: &[u8]) -> bool {
    match cmd {
        // Orders: a SET carries an argument (LEN 4). Calibration start and
        // stop and the factory reset are LEN 3 but change state too.
        0x00 => {
            (payload.len() == 4 && payload[0] != 0x47)
                || matches!(payload.first(), Some(0x0C | 0x0D | 0x11))
        }
        0x01 | 0x2B | 0x12 => false,
        _ => payload.first() == Some(&1),
    }
}

/// A handle to the shared simulated keyboard.
#[derive(Clone)]
pub struct Sim(pub Arc<Mutex<SimState>>);

impl Default for Sim {
    fn default() -> Self {
        Sim::new()
    }
}

impl Sim {
    pub fn new() -> Sim {
        Sim(Arc::new(Mutex::new(SimState::factory())))
    }

    pub fn state(&self) -> MutexGuard<'_, SimState> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A new connection, as `HidTransport::open_first` would make.
    pub fn open(&self) -> Result<SimTransport, Error> {
        let mut s = self.state();
        if !s.present {
            return Err(Error::Unsupported("no supported keyboard was found".into()));
        }
        if s.connect_failures > 0 {
            s.connect_failures -= 1;
            return Err(Error::Unsupported("no supported keyboard was found".into()));
        }
        s.connects += 1;
        Ok(SimTransport {
            sim: self.clone(),
            conn: s.conn,
            out: VecDeque::new(),
        })
    }

    /// The connector for `Worker::spawn_with`.
    pub fn connector(&self) -> openhe_app::worker::Connector<AppTransport> {
        let sim = self.clone();
        Box::new(move || {
            sim.open()
                .map(|t| Keyboard::new(AppTransport::new(t), Model::Win68HeMax))
        })
    }

    /// A keyboard on a direct connection (for checks outside the app).
    pub fn keyboard(&self) -> Keyboard<SimTransport> {
        Keyboard::new(self.open().expect("sim connect"), Model::Win68HeMax)
    }

    /// Queues an unsolicited report (e.g. a profile change on the keyboard).
    pub fn push_event(&self, cmd: u8, body: &[u8]) {
        self.state().events.push_back(reply(cmd, body));
    }
}

/// One open connection.
pub struct SimTransport {
    sim: Sim,
    conn: u64,
    out: VecDeque<[u8; 64]>,
}

fn checksum(len: u8, cmd: u8, body: &[u8]) -> u8 {
    let last = if (1..=60).contains(&len) {
        body.get(usize::from(len) - 1).copied().unwrap_or(0)
    } else {
        0
    };
    0x35u8
        .wrapping_add(0x5C)
        .wrapping_add(len)
        .wrapping_add(cmd)
        .wrapping_add(last)
}

/// A reply report: `5C LEN CMD|80 CHK body…`.
pub fn reply(cmd: u8, body: &[u8]) -> [u8; 64] {
    let mut r = [0u8; 64];
    let n = body.len().min(60);
    r[0] = 0x5C;
    r[1] = n as u8;
    r[2] = cmd | 0x80;
    r[4..4 + n].copy_from_slice(&body[..n]);
    r[3] = checksum(n as u8, cmd | 0x80, &body[..n]);
    r
}

fn fail() -> [u8; 64] {
    let mut r = [0u8; 64];
    r[0] = 0x5C;
    r[1] = 0;
    r[2] = 0xFF;
    r
}

fn le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

impl SimTransport {
    fn alive(&self, s: &SimState) -> bool {
        s.present && s.conn == self.conn
    }

    /// Handles one request; replies go to `self.out`. Returns false when
    /// the keyboard drops off USB without replying.
    fn handle(&mut self, s: &mut SimState, cmd: u8, pl: &[u8]) {
        let prof = usize::from(s.profile);
        match cmd {
            0x01 => {
                let mut b = vec![0u8; 60];
                b[1..5].copy_from_slice(&0x1901_0001u32.to_le_bytes());
                b[8] = 16;
                b[9..25].copy_from_slice(b"A68M240193000000");
                b[25] = 16;
                b[26..36].copy_from_slice(b"App V1.1.3");
                b[42] = 16;
                b[43..54].copy_from_slice(b"Nov 14 5025");
                self.out.push_back(reply(cmd, &b));
            }
            0x00 => self.order(s, pl),
            0x2B => {
                let (ra, rb) = (usize::from(pl[1]), usize::from(pl[2]));
                let mut b = vec![0u8; 45];
                b[1] = pl[1];
                b[2..23].copy_from_slice(&MATRIX[ra]);
                b[23] = pl[2];
                b[24..45].copy_from_slice(&MATRIX[rb]);
                self.out.push_back(reply(cmd, &b));
            }
            0x23 => {
                let p = &mut s.profiles[prof];
                if pl[0] == 0 {
                    let v = p.key(pl[1], pl[2]);
                    let [lo, hi] = v.to_le_bytes();
                    let mut b = vec![0, pl[1], pl[2], lo, hi];
                    // Hardware pads with filler entries (finding 7).
                    b.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]);
                    self.out.push_back(reply(cmd, &b));
                } else {
                    for e in pl[1..].chunks_exact(4) {
                        p.keys.insert((e[0], e[1]), le(e, 2));
                    }
                    self.out
                        .push_back(reply(cmd, &[0, pl[1], pl[2], pl[3], pl[4]]));
                }
            }
            0x18 => {
                let p = &mut s.profiles[prof];
                if pl[0] == 1 {
                    if pl.len() != 44 {
                        self.out.push_back(fail());
                        return;
                    }
                    p.lighting[1..].copy_from_slice(&pl[1..]);
                    p.lighting[0] = 0;
                    self.out.push_back(reply(cmd, &p.lighting));
                } else if pl.len() == 15 && pl.iter().all(|&b| b == 0) {
                    let body = p.lighting;
                    self.out.push_back(reply(cmd, &body));
                } else {
                    // A LEN-44 (or any other) GET is rejected (finding 14).
                    s.bad_prgb_gets += 1;
                    self.out.push_back(fail());
                }
            }
            0x2A => {
                let p = &mut s.profiles[prof];
                let key = pl[1];
                if pl[0] == 1 && key == 0xFE {
                    let pending = std::mem::take(&mut p.krgb_pending);
                    p.krgb.extend(pending);
                    s.krgb_commits += 1;
                    self.out.push_back(reply(cmd, &[0, 0xFE, 0xFE, 0xFE, 0xFE]));
                    return;
                }
                if pl[0] == 1 {
                    p.krgb_pending.insert(key, [pl[2], pl[3], pl[4]]);
                }
                let c = p
                    .krgb_pending
                    .get(&key)
                    .or_else(|| p.krgb.get(&key))
                    .copied()
                    .unwrap_or([0, 0, 0]);
                self.out.push_back(reply(cmd, &[0, key, c[0], c[1], c[2]]));
            }
            0x24 => {
                // MT: hold → DKS1, tap → DKS2, delay → MTDelay.
                let p = &mut s.profiles[prof];
                let k = pl[1];
                if pl[0] == 1 {
                    p.keys.insert((k, 9), le(pl, 2));
                    p.keys.insert((k, 10), le(pl, 4));
                    p.keys.insert((k, 19), u16::from(pl[6]));
                }
                let mut b = pl.to_vec();
                b[0] = 0;
                self.out.push_back(reply(cmd, &b));
            }
            0x26 => {
                // DKS: bindings → 9..12, trps → 13..16, first → 5.
                let p = &mut s.profiles[prof];
                let k = pl[1];
                if pl[0] == 1 {
                    for i in 0..4u8 {
                        let o = 2 + 2 * usize::from(i);
                        p.keys.insert((k, 9 + i), le(pl, o));
                        p.keys
                            .insert((k, 13 + i), u16::from(pl[10 + usize::from(i)]));
                    }
                    p.keys.insert((k, 5), le(pl, 14));
                }
                let mut b = pl.to_vec();
                b[0] = 0;
                self.out.push_back(reply(cmd, &b));
            }
            0x25 | 0x27 | 0x28 => {
                let p = &mut s.profiles[prof];
                let k = pl[1];
                if pl[0] == 1 {
                    p.funcs.insert((cmd, k), pl[1..].to_vec());
                }
                let mut b = vec![0u8; 16];
                b[1] = k;
                if let Some(d) = p.funcs.get(&(cmd, k)) {
                    b[1..1 + d.len()].copy_from_slice(d);
                }
                self.out.push_back(reply(cmd, &b));
            }
            0x2C => {
                let p = &mut s.profiles[prof];
                if pl[0] == 1 {
                    for k in [pl[1], pl[2]] {
                        p.funcs.insert((cmd, k), pl[1..].to_vec());
                    }
                }
                let mut b = vec![0u8; 16];
                if let Some(d) = p.funcs.get(&(cmd, pl[1])) {
                    b[1..1 + d.len()].copy_from_slice(d);
                }
                self.out.push_back(reply(cmd, &b));
            }
            0x20 => {
                let p = &mut s.profiles[prof];
                if pl[0] == 1 {
                    let addr = le(pl, 1);
                    let n = usize::from(pl[3]);
                    for i in 0..n {
                        let o = 4 + i * 6;
                        let code = le(pl, o);
                        let word = u32::from_le_bytes([pl[o + 2], pl[o + 3], pl[o + 4], pl[o + 5]]);
                        p.macro_data.insert(addr + i as u16, (code, word));
                    }
                }
                self.out.push_back(reply(cmd, &[0, pl[1], pl[2], pl[3]]));
            }
            0x21 => {
                let p = &mut s.profiles[prof];
                let k = pl[1];
                if pl[0] == 1 {
                    p.macro_mode.insert(k, pl[1..].to_vec());
                }
                let mut b = vec![0u8; 11];
                b[1] = k;
                if let Some(d) = p.macro_mode.get(&k) {
                    b[1..1 + d.len()].copy_from_slice(d);
                    // The firmware does not report len (finding 24).
                    b[4] = 0;
                }
                self.out.push_back(reply(cmd, &b));
            }
            0x12 => self.rm6x21(s, pl),
            _ => panic!("sim: command 0x{cmd:02X} must never be sent ({pl:02X?})"),
        }
    }

    fn order(&mut self, s: &mut SimState, pl: &[u8]) {
        let t = pl[0];
        let arg = (pl.len() == 4).then(|| pl[1]);
        let prof = usize::from(s.profile);
        let v = match (t, arg) {
            (0x70, Some(a)) => {
                assert!(a <= 3, "sim: profile {a}");
                // Hardware: the keyboard re-enumerates, no reply (finding 23).
                s.profile = a;
                s.reenumerate();
                return;
            }
            (0x70, None) => s.profile,
            (0x21, None) => s.profiles[prof].win,
            (0x30 | 0x31, Some(_)) => {
                let win = u8::from(t == 0x30);
                let p = &mut s.profiles[prof];
                if p.win != win {
                    // LAlt / LWin Base codes swap with the OS mode.
                    let a = p.key(0xE2, 0);
                    let b = p.key(0xE3, 0);
                    p.keys.insert((0xE2, 0), b);
                    p.keys.insert((0xE3, 0), a);
                }
                p.win = win;
                1
            }
            (0x50, Some(a)) => {
                assert!(a <= 6, "sim: polling {a}");
                s.profiles[prof].polling = a;
                // A polling change restarts the keyboard (PROTOCOL 3.9).
                s.reenumerate();
                return;
            }
            (0x50, None) => s.profiles[prof].polling,
            (0x47, _) => {
                // FW App V1.1.3 rejects RGBNACK (finding 22).
                s.rgbnack += 1;
                self.out.push_back(fail());
                return;
            }
            (0x0C, None) => {
                s.calibrating = true;
                s.calibration_starts += 1;
                1
            }
            (0x0D, None) => {
                s.calibrating = false;
                s.calibration_stops += 1;
                1
            }
            (0x11, None) => {
                s.factory_resets += 1;
                let keep = (s.conn, s.connects, s.reenumerations);
                let log = std::mem::take(&mut s.log);
                let resets = s.factory_resets;
                let mut fresh = SimState::factory();
                fresh.conn = keep.0;
                fresh.connects = keep.1;
                fresh.reenumerations = keep.2;
                fresh.log = log;
                fresh.factory_resets = resets;
                fresh.calibration_starts = s.calibration_starts;
                fresh.calibration_stops = s.calibration_stops;
                *s = fresh;
                // The reply goes out, then the board restarts.
                self.out.push_back(reply(0, &[0, t, 1, 0xFF]));
                s.reenumerate();
                return;
            }
            _ => panic!("sim: order 0x{t:02X} {arg:?} must never be sent"),
        };
        self.out.push_back(reply(0, &[0, t, v, 0xFF]));
    }

    fn rm6x21(&mut self, s: &mut SimState, pl: &[u8]) {
        let (sub, part) = (pl[0], pl[1]);
        let src = match sub {
            2 => &s.travel,
            6 => &s.calibration,
            _ => panic!("sim: RM6X21 SUB {sub} is never read"),
        };
        let rows = if part == 1 { 0..3 } else { 3..6 };
        let mut all = vec![0x5C, 0x3C, 0x92, 0x00, 0x00, sub];
        for r in rows {
            for c in 0..21 {
                all.extend_from_slice(&src[r][c].to_le_bytes());
            }
        }
        all.resize(192, 0);
        all[3] = checksum(0x3C, 0x92, &all[4..64]);
        for chunk in all.chunks(64) {
            let mut r = [0u8; 64];
            r.copy_from_slice(chunk);
            self.out.push_back(r);
        }
    }
}

impl Transport for SimTransport {
    fn write(&mut self, report: &[u8; 64]) -> Result<(), Error> {
        let sim = self.sim.clone();
        let delay = sim.state().set_delay;
        if !delay.is_zero() && is_set(report[2], &report[4..4 + usize::from(report[1])]) {
            std::thread::sleep(delay);
        }
        let mut s = sim.state();
        if !self.alive(&s) {
            return Err(Error::Disconnected);
        }
        assert_eq!(report[0], 0x5C, "sim: bad magic");
        let cmd = report[2];
        let len = usize::from(report[1]);
        let pl = report[4..4 + len].to_vec();
        if s.drop_before == Some((cmd, pl[0])) {
            s.drop_before = None;
            s.conn += 1;
            return Err(Error::Disconnected);
        }
        assert_eq!(
            report[3],
            checksum(report[1], cmd, &pl),
            "sim: bad checksum on {cmd:02X} {pl:02X?}"
        );
        s.log.push((cmd, pl.clone()));
        let set = is_set(cmd, &pl);
        if set {
            if let Some(n) = s.unplug_after_sets.as_mut() {
                if *n == 0 {
                    s.unplug_after_sets = None;
                    s.unplug();
                    return Ok(());
                }
                *n -= 1;
            }
        }
        if s.take_reject(cmd, &pl) {
            self.out.push_back(fail());
            return Ok(());
        }
        self.handle(&mut s, cmd, &pl);
        Ok(())
    }

    fn read(&mut self, _timeout_ms: u32) -> Result<Option<[u8; 64]>, Error> {
        let sim = self.sim.clone();
        let mut s = sim.state();
        if !self.alive(&s) {
            return Err(Error::Disconnected);
        }
        if let Some(r) = self.out.pop_front() {
            return Ok(Some(r));
        }
        Ok(s.events.pop_front())
    }
}
