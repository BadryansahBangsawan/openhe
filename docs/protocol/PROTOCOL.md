# AULA WIN 68 HE MAX / WIN 60 HE MAX: HID protocol and feature map

Source: static reading of AULA HUB 1.1.22 (`app.asar` extracted to `recon/asar`). Nothing was executed and nothing was sent to a device.

Confidence tags used below:

- **[C] Confirmed.** Read directly from code; the anchor is given.
- **[I] Inferred.** Reasoned from code or naming, but not explicit in it.
- **[?] Unknown.** Cannot be determined statically (see section 7).

Line numbers below (`L1234`) refer to prettified copies of the AULA HUB 1.1.22 bundle chunks (not checked in), and the minified anchors are exact strings you can `grep -F` in `recon/asar/dist/assets/*.js`.

---

## 1. Code path for this device

### 1.1 Transport

- **WebHID in the renderer, not node-hid.** [C] `dist-electron/preload.mjs` exposes no HID API. `dist-electron/main.mjs` L2466 only grants permission with `setDevicePermissionHandler((o) => o.origin.startsWith("file://") ...)`. All HID I/O goes through `navigator.hid` in the renderer.

### 1.2 Device table

The device table is in `dist/assets/index-npIkQlTa.js` (anchor `pName:["WIN 68 HE MAX"]`):

```
{pName:["WIN 68 HE PRO"],pId:6401,vId:7330,deviceType:1,usage:1,usagePage:65440,supportAcc:1,layoutClass:{...sl}}
{pName:["WIN 68 HE MAX"],pId:6401,vId:7330,deviceType:1,...same...}
{pName:["WIN 60 HE PRO"],pId:6402,...}   {pName:["WIN 60 HE MAX"],pId:6402,...}
```

- **PRO vs MAX share a PID.** [C] The app tells them apart by the **HID product string**. `addDevice` does `rs.filter(u=>u.pId===t&&u.vId===i&&u.pName.includes(r))`, where `r` is `HIDDevice.productName`. So the product string must equal `"WIN 68 HE MAX"`.
- **`supportAcc:1`** [C] means the model supports "Onboard Auto Switch", which is app-side profile switching per foreground app.
- **`layoutClass: sl`** [C] (anchor `sl={Info_Class:()=>D0(()=>import("./info-B2zKVp8F.js")`) maps each page to a chunk:

| Class | Chunk | Role |
|---|---|---|
| Info_Class | `info-B2zKVp8F.js` | **Bundled `@sparklinkplayjoy/sdk-keyboard` (V1 SDK) plus the device glue class** |
| Light_Class | `light-Cp389CIy.js` | lighting page logic |
| Trigger_Class | `trigger-VHOIH7VE.js` | actuation, RT, deadzones, calibration, live travel |
| Key_Class | `key-BkmYRml7.js` | remap, Fn layers, macro binding |
| High_Class | `high-CLmP8232.js` | DKS, MPT, MT, TGL, END, SOCD |
| Perf_Class | `perf-Dx-QLAFx.js` | polling rate, Mac mode |
| Version_Class | `version-CZpI-VxG.js` | firmware info, update link, factory reset |
| (data) | `SparkLinkV1-t4aSMPxg.js` | layouts, keycode table, switch (axis) tables |

`deviceType:1` itself does not choose the SDK. The `layoutClass` does: the glue class `ca` at the end of `info-B2zKVp8F.js` sets `this.sdkModuleName="sparkLinkV1"`.

### 1.3 Layers inside `info-B2zKVp8F.js` (prettified L4156 onward)

| Layer | Name in code | Role |
|---|---|---|
| Raw HID I/O | `ve` (L4751) | `sendReport(r){return this.device.sendReport(0,r)}`; `inputreport` listener; FIFO response queue; retries |
| HID service | `di` (L4907) | `navigator.hid.requestDevice({filters})`, open/close, reconnect |
| Command queue | `be`/`Je` (L9729) | serializes commands, strips the 4-byte response header, dispatches unsolicited reports |
| Packet builders | `Ht = {infoProtocol, keyProtocol, lightingProtocol, performanceProtocol, higherKeyProtocol, systemProtocol}` (L8504) | builds packets and parses responses |
| Controllers | `Rn` info, `jo` key, `$o` light, `Jo` performance, `qo` higher keys, `Ze` bootloader | wrap the builders |
| Facade | `on` (L10006) | public SDK methods (`setDks`, `getRtTravel`, `updateBin`, ...) |
| Glue | `ca` (L10182) | device init, profile, Win/Mac, layout discovery, axis map |

The HID filter is set in `ca.initModule` [C]:

```js
new on({usage:1, usagePage:[65440], configs:[{vendorId:t.vId, productId:t.pId, usage:1, usagePage:65440}]})
```

---

## 2. Packet framing

### 2.1 Output report (host to keyboard) [C]

Anchors: `vn=(e,t,r)=>[92,e,t,r]` and `let s=53;` in `info-B2zKVp8F.js` (L7470 to L7493).

```
byte 0   0x5C                    magic
byte 1   LEN                     payload length (bytes after the 4-byte header)
byte 2   CMD                     command (table 2.4)
byte 3   CHK                     checksum
byte 4.. PAYLOAD[LEN]
rest     0x00 padding to 64 bytes
```

- **Report ID.** `device.sendReport(0, data)` sends report ID 0, meaning no report ID, with a 64-byte body.
  - With hidapi on macOS, write `[0x00] + 64 bytes` (65-byte buffer).
  - Reads return 64 bytes with no ID prefix.
- **Checksum** [C]. This is not a CRC:

  ```
  CHK = (0x35 + 0x5C + LEN + CMD + (1 <= LEN <= 252 ? PAYLOAD[LEN-1] : 0)) & 0xFF
  ```

  Only the **last payload byte** is included. The `&0xFF` comes implicitly from storing into a `Uint8Array`.
- **Common trailer.** Many payloads end with `0xFF 0xFF`, which then become the checksum's last byte.
- **Get/Set flag** [C]. Most "per-key / object" commands begin the payload with a flag byte from `at(e)=e?0:1`, where `e` is "isGet":
  - **0x00 = GET (read)**
  - **0x01 = SET (write)**

  The flag applies to KEY, MT, TGL, DKS, MPT, END, SOCD, RS, MACRO, MACRO_MODE, PRGB, LOGORGB, KRGB, DB and DEFKEY. CMD 0x00, 0x01 and 0x12 have **no** flag; their payload starts with the sub-type.

**Multi-packet (chunked) sends** [C], via `Xs(...)`, are used only by BL_WRITE and PIC_WRITE:

- The same 4-byte header is built with LEN equal to the total payload length (for example 250).
- `[header + payload]` is split into consecutive 64-byte reports.
- Only the first report carries the header; the continuation reports are raw data.
- All chunks are sent back to back, and then one response is awaited.

### 2.2 Input report (keyboard to host)

- **Size.** 64 bytes, report ID 0.
- **Command byte** [C]. `data[2]` is the response command. The dispatcher maps it with `Ge = {128:"getCmd",163:"getKey",171:"defKey",152:"getSpecialSingleRGB",153:"getLogoRGB"}`:
  - 0x80 = 0x00|0x80
  - 0xA3 = 0x23|0x80
  - 0xAB = 0x2B|0x80
  - 0x98 = 0x18|0x80
  - 0x99 = 0x19|0x80

  So **response CMD = request CMD | 0x80**. [C for these five; I for the rest]
- **Header stripping** [C]. The queue removes the first 4 bytes before parsing: `deviceUsagePage===65456 ? data : data.slice(4)`. Our usage page is 0xFFA0, so the header is stripped. Bytes 0 to 3 are `[?, LEN, CMD|0x80, CHK]`, and byte 0's value is [?].
- **Response payload layout** after stripping 4 bytes. [I, but consistent with every parser in the SDK]

  ```
  R[0]   status / ack byte. Bootloader parsers explicitly test R[0]===0 for success.
  R[1..] echo of the request payload *without* its GET/SET flag (or without nothing, for CMD 0x00/0x01/0x12), with
         the requested value filled in.
  ```

  Example: a GET to KEY 0x23 is sent as `[00,key,layout,00,00]` and answered as `R=[st,key,layout,valLo,valHi]`. Anchor: `getDks(e){return((t)=>({dks:(t[4]<<8)|t[3]}))`.

### 2.3 Request and response handling [C]

Anchors: `MAX_RETRIES",3`, `waitForResponse`, `flushQueue`.

- **One command in flight at a time.** There is a global FIFO queue per device.
- **No matching.** Responses are matched **purely by arrival order**. The command byte is not checked, and input reports received before the command was enqueued are discarded.
  - An unsolicited event report (for example the user pressing Fn+profile) that arrives between send and reply **will be consumed as the reply**. Your Rust app should match on `CMD|0x80` and queue the rest as events.
- **Timeout and retries.** 1000 ms per attempt, up to 3 attempts when nothing arrives. The facade swaps the `timeout` and `sendTime` arguments twice, and the two swaps cancel out.
- **Failure path.** When all attempts fail, the app's queue throws and leaves the promise hanging (an app bug). Do not copy this.
- **Multiple-response commands.** `RM6X21` (live travel and calibration) uses `sendDataAndWaitMultiple(pkt, 3)`. It collects 3 input reports, concatenates them, and strips only the first 4 bytes.
- **Delays.** The SDK uses no inter-command delay in normal operation. Polling loops run every 10 ms (travel test) or 60 ms (calibration). Firmware update has explicit delays (section 5).
- **Unsolicited events** [C] (listener `this.hidService.on("inputReport",...)`). The keyboard pushes reports when changed from the keyboard itself:
  - **0x80** with a sub-type: Win/Mac mode change, or profile change (`configID`).
  - **0x98:** main lighting state, parsed like a PRGB GET.
  - **0x99:** logo lighting.
  - **0xA3 / 0xAB:** key or default-key data.

  The app reacts by reloading the layout or lighting.
- **RGBNACK** [I]. Before reading lighting, the app sends `CMD 0x00` with sub-type `0x47 (ORDER_TYPE_RGBNACK)` and arg `[1]`. This probably enables lighting-change notifications or acks. Its response `R[2]!=0` is a boolean.

### 2.4 Command byte table (enum `xt`, prettified L5263) [C]

| CMD | Name | Used for |
|---|---|---|
| 0x00 | KB2_CMD | "Order" sub-commands (table 2.5) |
| 0x01 | KB2_CMD_SYNC | device info (board ID, SN, firmware version) |
| 0x08 | KB2_BL_SIGN | bootloader unlock signature |
| 0x09 | KB2_BL_ERASE | bootloader erase |
| 0x0A | KB2_BL_REBOOT | jump to bootloader |
| 0x0B | KB2_BL_TOAPP | jump to app |
| 0x0C | KB2_BL_WRITE | write flash (chunked) |
| 0x0D | KB2_BL_READ | (unused) |
| 0x0E | KB2_BL_RCRC | read flash CRC |
| 0x12 | KB2_CMD_RM6X21 | live travel, calibration, status matrix (6×21) |
| 0x18 | KB2_CMD_PRGB | main (per-key backlight) effect |
| 0x19 | KB2_CMD_LOGORGB | logo/ambient light (not shown for WIN models) |
| 0x20 | KB2_CMD_MACRO | macro action data |
| 0x21 | KB2_CMD_MACRO_MODE | macro binding and play mode |
| 0x23 | KB2_CMD_KEY | per-key parameter by "layout" index (table 2.6) |
| 0x24 | KB2_CMD_MT | Mod-Tap / Hold-Tap |
| 0x25 | KB2_CMD_TGL | Toggle |
| 0x26 | KB2_CMD_DKS | Dynamic Keystroke |
| 0x27 | KB2_CMD_MPT | Multi-Point Trigger |
| 0x28 | KB2_CMD_END | End-point (key on release) |
| 0x29 | KB2_CMD_DB | global travel and deadzones (used only by config import) |
| 0x2A | KB2_CMD_KRGB | per-key custom RGB |
| 0x2B | KB2_CMD_DEFKEY | read default keycodes of the 6×21 matrix |
| 0x2C | KB2_CMD_SOCD | SOCD / snap-tap |
| 0x2D | KB2_CMD_RS | Rapid Shift (not exposed for this model) |
| 0x30 | KB2_CMD_PIC | screen picture start (not used by this model) |
| 0x31 | KB2_PIC_WRITE | screen picture data (not used; builder is buggy) |
| 0xFF | KB2_CMD_FAIL | error response [I] |

### 2.5 CMD 0x00 sub-types (`le` / OrderType, prettified L7522) [C]

Payload is `[TYPE, args..., 0xFF, 0xFF]`. Query and set often use the **same** TYPE, and the device tells them apart by LEN (3 = query, 4 = set with one arg) [I]. The response is parsed by `mn()` (L7683), where `R[1]` = TYPE.

| TYPE | Name | Used by UI? | Response parse |
|---|---|---|---|
| 0x01 | PROTOCOL_VERSION | no | `"${R[3]&15}.${R[2]>>4&15}.${R[2]&15}"` |
| 0x02 / 0x03 | SAVING_PARAMETER / RELOAD_PARAMETERS | no | - |
| 0x04 | CLEAR_CALIBRATION_DATA | no | - |
| 0x05 / 0x06 | OPEN_DKS / CLOSE_DKS | no | - |
| 0x07 | REVERSE | no | - |
| 0x08 / 0x09 | TURN_ON / OFF_REMAPPING | no | - |
| 0x0A / 0x0B | ENABLE / TURN_OFF_RELATIVE_TRIGGER | no | - |
| **0x0C** | START_CALIBRATION | **yes** | - |
| **0x0D** | CLOSE_CALIBRATION | **yes** | - |
| 0x0E / 0x0F | START / CLOSE_DEMONSTRATION | no | - |
| 0x10 | ERASE_MACROSTORAGE | no | - |
| **0x11** | RESTORE_FACTORY_SETTINGS | **yes** (Version page "Reset") | any reply = OK |
| 0x20 | LOCK_WIN_KEY | no (Win-lock is available as a remap target 0xF309) | - |
| **0x21** | QUERY_WIN_MODEL | **yes** | R[2]: 1 = win, 0 = mac, 255 = unsupported |
| 0x22 | QUERY_MAC_MODEL | no | R[2]: 1 = mac, 0 = win, 255 = n/a |
| 0x23 / 0x24 | QUERY_STANDARD / ADJUSTABLE_SPEED MODEL | no | - |
| 0x25 | PRECISION_STROKE | no | precision = R[2]/1000; minTravel = u16le(R[3..4])/1000; maxTravel = u16le(R[5..6])/1000 (mm) |
| 0x26 | KEYBOARD_NAME | no | UTF-8 R[2..33] |
| **0x30** | SET_WIN_MODEL | **yes** | R[2]==1 means OK |
| **0x31** | SET_MAC_MODEL | **yes** | R[2]==1 means OK |
| 0x32 / 0x33 | SET_STANDARD / ADJUSTABLE_SPEEDMODEL | no | - |
| 0x34 | TOP_DEAD_SWITCH | only in config import | R[2]!=0 |
| 0x40..0x43 | RGB1..4 | no | - |
| 0x44 | QUERY_LIGHT_FIX_RGB | no | {r:R[2],g:R[3],b:R[4]} |
| **0x47** | RGBNACK | **yes** (arg 1 before reading lighting) | R[2]!=0 |
| **0x50** | ROES (polling rate) | **yes** | R[2] = index |
| 0x60 / 0x61 | WEB / SOCD | no | - |
| **0x70** | CONFIG (onboard profile) | **yes** | R[2] = profile index 0..3 |
| 0x75 | CURRENT_AXOSOME | no | u16le(R[2..3]) |
| 0x76 | AXOSOME (switch list) | no | up to 8 × **u16 big-endian** at R[2..], 0xFFFF terminates |

### 2.6 CMD 0x23 KEY: the per-key parameter store [C]

Builder anchor: `cmdLayout(e,t)`, prettified L8072.

```
GET: payload [0x00, KEY, LAYOUT, 0x00, 0x00]          (LEN 5)
SET: payload [0x01, KEY, LAYOUT, VAL_lo, VAL_hi]      (LEN 5)
     SET can batch N×[KEY, LAYOUT, lo, hi] after the flag (cmdKey). At most 14 entries fit in 64 bytes; the app does
     not split batches.
Response: R = [st, KEY, LAYOUT, VAL_lo, VAL_hi, (KEY, LAYOUT, lo, hi)...]
          parser getFnLayoutKeyRecdata skips entries with key==0 / layout==255 / value==255
```

**KEY is the key's default keycode, not a matrix index** (see section 4).

LAYOUT index (enum `je` "KeyLayout", L7577) [C]:

| LAYOUT | Name | Meaning | Value / units |
|---|---|---|---|
| 0..3 | Fn0..Fn3 | keymap layer 0 (base), 1, 2, 3 | u16 keycode (table 4.3) |
| 4 | DB0 | **actuation point** ("single travel") | µm (mm×1000) |
| 5 | DB1 | **DKS first actuation point** | µm |
| 6 / 7 | DB2 / DB3 | (unused by UI) | µm [I] |
| 8 | Mode | `(touchMode<<4) \| advancedMode` | see below |
| 9..12 | DKS1..4 | DKS bindings (also MT: DKS1 = hold, DKS2 = tap [I]) | u16 keycode |
| 13..16 | TRPS1..4 | DKS trigger bitmask per binding | see DKS |
| 17 / 18 | MacroAddr / MacroSize | (unused by UI) | - |
| 19 | MTDelay | MT hold threshold | value×10 = ms |
| 20 | RTP | **Rapid-Trigger press sensitivity** | µm |
| 21 | RTR | **Rapid-Trigger release sensitivity** | µm |
| 22 | DP | **Top deadzone** (UI "Top Dead Zone") | µm |
| 23 | DR | **Bottom deadzone** (UI "Bottom Dead Zone") | µm |
| 24 | KR | (unused) | [?] |
| 25 | AXIS | switch type ID (table 4.2) | u16 id |
| 32 | RS | Rapid Shift data | [?] |

**Mode value** (LAYOUT 8), from `getLayoutModel`: `low = VAL & 0xFF`.

- **touchMode** = `(low>>4)&0xF`:
  - 0 = global
  - 1 = single (fixed actuation)
  - 2 = rt (Rapid Trigger)
- **advancedMode** = `low&0xF`:
  - 0 = none
  - 1 = DKS
  - 2 = MPT
  - 3 = MT
  - 4 = TGL
  - 5 = END
  - 6 = MACRO
  - 8 = SOCD
  - 9 = RS (enum from `advancedKeysSdkMap`)

App quirks:

- `deleteKey()` writes Mode=0, which clears the advanced function **and** resets the touch mode to global.
- The trigger page calls `setPerformanceMode(key,"rt",Number("DKS"))`. `NaN` coerces to 0, so changing RT/single on a key **clears its advanced bits**. Avoid this in your app.
- Float-to-µm conversion uses `1e3*mm` plus bitwise truncation, so for example 1.13 becomes 1129. **Round** instead.

---

## 3. Feature list (as exposed for WIN 68 HE MAX / WIN 60 HE MAX)

Pages for `sparkLinkV1` [C] (from `deviceRoute` plus the router):

- Lighting
- Trigger
- Pro Settings (masterSet)
- Key Mapping
- Advanced Keys
- Performance
- Version

The English strings come from the `vM={home:"Welcome to AULA Driver"...}` object in `index-npIkQlTa.js`.

### 3.1 Device info / handshake [C]

The startup sequence is in `ca.getBaseInfo` and `initDeviceLayout`.

1. **SYNC** `CMD 0x01`, payload `[01 02 03 04 FF FF]` (packet `5C 06 01 97 01 02 03 04 FF FF`). Response parse (`getCmdSync`, after the header is stripped):
   - BoardID = u32le(R[1..4])
   - KeyType = R[3]
   - KeyboardLayout = R[4]
   - u16le(R[5..6]): if < 1000 it is versionString "Vx.y.z", otherwise flash size/256
   - KeyboardRunMode = R[7] (**0 = application, non-zero = bootloader**)
   - SN = ASCII R[9..24]
   - appVersion = ASCII R[26..35] (R[26..36] if it starts with "Boot"); this is the firmware version shown in the UI
   - buildDate = ASCII R[43..53]
2. **QUERY_WIN_MODEL** (0x00 / 0x21) gives the Win/Mac state.
3. **Axis map selection.** `"WIN 68 HE MAX" -> axisMap = Gn`; `"WIN 60 HE MAX" -> Un`.
4. **DEFKEY ×3**, for matrix rows (0,1), (2,3) and (4,5) (section 4).
5. For every key, KEY GET at LAYOUT 0..3 for the current Fn layer, and AXIS GET.

### 3.2 Onboard profiles: 4 [C]

- **List.** `getProfileList` returns 4 entries, "Onboard Profile 1..4" (values 0..3).
- **Query.** `0x00 / 0x70`, payload `[70 FF FF]`; response `R[2]` = index.
- **Switch.** `[70 idx FF FF]`.
- **From the keyboard.** The device pushes 0x80 with TYPE 0x70 when the profile is changed on the keyboard; the app reloads everything.
- **Auto switch.** [C, app-side] The Electron main process monitors the foreground app (`get-active-app`) and calls `autoChangeProfile`, which sends CONFIG query and switch. This is pure host logic.

### 3.3 Trigger page (per key; any selection of keys) [C] (`trigger-VHOIH7VE.js`)

| UI control | Wire | Range / default |
|---|---|---|
| Actuation point ("Travel") | KEY LAYOUT 4 (DB0), µm | 0.1 to maxTravel (3.3/3.4 per switch), **step 0.001 mm**; default 2.0 mm. The step is 0.001 because the "Win" special-case in the code does not match "WIN". |
| Trigger mode single / RT | KEY LAYOUT 8: `(1<<4)\|adv` single, `(2<<4)\|adv` RT | - |
| RT press sensitivity | LAYOUT 20, µm | 0.005 to maxTravel, step 0.001 |
| RT release sensitivity | LAYOUT 21, µm | 0.005 to maxTravel, step 0.001 |
| "Sync press & release" | UI only (writes both 20 and 21) | - |
| Top deadzone | LAYOUT 22, µm | 0 to 1.0 mm, step 0.01; default 0.2 |
| Bottom deadzone | LAYOUT 23, µm | 0 to 1.0 mm, step 0.01; default 0.2 |
| Select switch / "Apply switch" | LAYOUT 25 SET = switch ID | IDs per table 4.2; drives maxTravel |
| Reset to default | writes actuation 2.0, DP/DR 0.2, mode single | - |
| Select All / Invert / WASD / Deselect | UI only | - |
| **Travel test (live depth)** | RM6X21 polling every **10 ms** (section 3.4) | the UI shows the max depth across keys |
| **Calibration** | `0x00/0x0C` start, poll every 60 ms, `0x00/0x0D` end | - |

On load the page reads every key's mode, DB0, RTP/RTR and DP/DR, plus AXIS.

### 3.4 Live key depth and calibration data (CMD 0x12 RM6X21) [C for framing, I for row split]

Request payload is `[SUB, PART, FF, FF]` with no flag. Each request expects **3 input reports**. They are concatenated (192 bytes) and only the first 4 bytes are stripped, so continuation reports carry no header [I].

| SUB, PART | Meaning | Data (after the strip) |
|---|---|---|
| 3,1 | per-key status | R[1]==3; then 21-byte rows until `FF FF` [?] |
| 2,1 | **travel**, matrix rows 0..2 | R[1]==2; then 3×21 u16le µm, row-major |
| 2,2 | **travel**, matrix rows 3..5 | same |
| 6,1 / 6,2 | **calibration values**, rows 0..2 / 3..5 | same format; the UI shows `value*1000` as "keyADC" |

- **Live travel readout** = `2,1` then `2,2`, giving a 6×21 matrix of mm. The UI calls this the "Travel Test".
- **Mapping matrix cells to keys.** The (row, col) is the same matrix position used by DEFKEY (section 4).
- **Packet:** `5C 04 12 A6 02 01 FF FF`.
- **Calibration flow** [C]:
  1. START_CALIBRATION.
  2. Every 60 ms read 2,1 / 2,2 / 6,1 / 6,2.
  3. A key counts as calibrated in the UI when `travel >= maxTravel - dp - dr - 0.1`.
  4. CLOSE_CALIBRATION.

  The user instruction is to press each key fully, 2 to 3 times.

### 3.5 Key mapping / Fn layers [C] (`key-BkmYRml7.js`)

- **Layers.** 4 layers: Base, Fn1, Fn2, Fn3 (LAYOUT 0..3). The layer menu is labelled "Win Base / Win Fn1.." or "Mac Base.." by OS mode. Separate Win and Mac tables are [?] (probably mode-dependent on the device).
- **Remap.** KEY SET `[01, KEY, layer, code_lo, code_hi]`. The UI sends all selected keys in one packet.
- **Read layer.** KEY GET per key `[00, KEY, layer, 0, 0]` returns the value.
- **Reset keys.** Write `value = KEY` (its own default code) on layer 0. The Fn key (code 0x01) is skipped.
- **Assignable codes.** See table 4.3: keyboard, media/consumer, mouse, profile, Win/Mac, Win-lock, WASD swap, calibrate, lighting controls.

### 3.6 Advanced keys [C] (`high-CLmP8232.js`; builders in `info` L7780 `Go`)

The UI lists DKS, MPT, MT, SOCD, TGL and END. RS exists in the SDK but is **not** offered for this model. The current function of a key comes from LAYOUT 8 low nibble.

To delete any advanced function: KEY SET LAYOUT 8 = 0.

**Version note.** `cmdDKS`/`cmdMT`/`cmdTGL`/`cmdEND` are always built with the "1.0.5" format, because the version is not passed through. SOCD uses the firmware string, and `"<1.0.7"` falls back to an old format. Version strings that do not parse as `x.y.z` compare as "equal", which means the new format.

**DKS (0x26), 4 bindings × 4 travel phases.**

- SET payload: `[01, KEY, k1 u16, k2 u16, k3 u16, k4 u16, trps1, trps2, trps3, trps4, db1 u16 (µm), db2 u16 (=3400)]` (LEN 18).
  - `db1` = first actuation 0.1 to 3.4 mm, step 0.001, default 1.4.
  - `db2` = bottom-out (fixed 3.4 mm).
- `trps` byte per binding (helper `Qa0` in `index-npIkQlTa.js`). The UI booleans are `[press@P0, hold P0→P1, press@P1, hold P1→P2, press@P2, hold P2→P3, press@P3]`, where:
  - P0 = press past actuation
  - P1 = bottom-out
  - P2 = release from bottom
  - P3 = full release

  | UI boolean | bit |
  |---|---|
  | press@P0 | 0x01 |
  | hold P0→P1 | 0x02 |
  | press@P1 | 0x04 |
  | hold P1→P2 | **0x18** |
  | press@P2 | 0x20 |
  | hold P2→P3 | 0x40 |
  | press@P3 | 0x80 |

  The decoder `Oa0` treats `(b&0x18)!=0` as the hold P1→P2 flag. The meaning of the phases is [I] from the UI ("Press / Bottom Out / Release").
- Read back: KEY GET LAYOUT 9..12 (codes), 13..16 (trps), 5 (db1).

**MPT (0x27), 3 depths.**

- SET: `[01, KEY, k1, k2, k3 (u16), d1, d2, d3 (u16 µm)]`.
- Depths are 0.1 to 3.4 mm, step 0.001, strictly increasing; default 1/2/3 mm.
- GET `[00, KEY]` gives k1 = R[2..3], k2 = R[4..5], k3 = R[6..7], d1..d3 = R[8..13] (µm). This is consistent with the status-byte model.

**MT hold/tap (0x24).**

- SET: `[01, KEY, kHold u16, kTap u16, delay/10 u8]`.
- Delay is 1 to 1000 ms (default 200), sent in 10 ms units. Which binding is hold and which is tap is [I] from the label order "Long Press, Short Press".
- Read: LAYOUT 9/10 (codes), LAYOUT 19 (×10 ms).

**TGL toggle (0x25).**

- SET: `[01, KEY, code u16, delay/10 u8]`. Delay is 1 to 1000 ms (default 200); its meaning is [?].
- GET `[00, KEY, 0, 0, 0]` gives code = R[2..3], delay = R[4]×10.

**END (0x28).** "Single key sends another key upon release".

- SET: `[01, KEY, code u16]`.
- GET `[00, KEY, 0, 0]` gives code = R[2..3] and delay = R[4..5].

**SOCD (0x2C), firmware ≥ 1.0.7 format.**

- SET: `[01, pos1, pos2, key1 u16, key2 u16, type, mode, delay u16]`, where:
  - `pos1/pos2` = the two physical keys (default codes).
  - `key1/key2` = output codes (used when type = 1).
  - `type`: 0 = bind position, 1 = bind key values.
  - `mode`: 0 = last input priority, 1 = key 1 absolute, 2 = key 2 absolute, 3 = cancel/neutral.
  - `delay`: 0 to 50 (units [?], probably ms).
- GET `[00, KEY]` returns `R = [st,pos1,pos2,k1lo,k1hi,k2lo,k2hi,type,mode,dlo,dhi]`.
- The old (<1.0.7) format is `[01,key,dks1,mode1,dks1,key,mode2]`.

**RS (0x2D)** (SDK only, hidden): `[flag, key, dks, dks, key]`.

### 3.7 Macros [C] (`key-BkmYRml7.js` `setMACROToDevice`; SDK `setMacro` L9163)

- **Storage.** The macro library (names and recorded actions) is stored **host-side only**, in the app store / localStorage (`MacroList`). Only the bound copy goes to the keyboard.
- **Binding a macro to KEY** takes three steps:
  1. **MACRO 0x20** data packets, 9 actions max per packet: `[01, addr u16, count, (code u16, word u32le)×count]`.
     - `addr` starts at **256** and advances by the number of actions.
     - `word = (flag<<24) | (delay_ms & 0xFFFFFF)`, where flag 1 = key down and flag 8 = key up.
     - `code` is from table 4.3. The browser `KeyboardEvent.code` is mapped through the table.
  2. **KEY SET LAYOUT 8** = `(0<<4)|6` (macro).
  3. **MACRO_MODE 0x21**: `[01, KEY, id u16 (=0), len (=#actions), mode, num u16 (loop count, default 1), interval u24 (ms, default 100)]`.
     - `mode`: 0 = single click (play once), 1 = repeat (click again to stop), 2 = stop immediately on release, 3 = stop on release after finishing the cycle.
- **Read.** MACRO_MODE GET `[00, KEY, ...]` gives key = R[1], id = u16(R[2..3]), len = R[4], mode = R[5], num = u16(R[6..7]), delay = u24(R[8..10]).
- **Caution** [?]: every bind writes at addr 256 with id 0, so whether two keys can hold different macros is unclear.

### 3.8 Lighting [C] (`light-Cp389CIy.js`; builder `Ho.RGB` L8179)

- **WIN models expose only "Keyboard Lighting".** The logo/ambient page is hidden (`pName.includes("WIN")`).
- **PRGB (0x18) SET payload** (LEN 44, "1.0.9" format, which the UI uses):

  ```
  [0]      01                      flag
  [1..4]   00 00 00 00
  [5..32]  7 colours × (B, G, R, 0xFF)       <- note BGR order
  [33..36] 00 00 00 00
  [37]     flags: bit0 on/off, bit1 direction(reverse), bit4 "superResponse" (UI always 1)
  [38]     brightness 0..4   (UI levels 1..5)
  [39]     mode: 0 static, 1..20 dynamic effects, 21 custom per-key
  [40]     speed 0..4        (UI 1..5; forced 0 for static/custom)
  [41]     light sleep timeout in minutes: 0(never),1,2,3,5,10,15,20,25,30,45,60,120
  [42]     staticColor: index 0..6 into the colour list, 7 = colourful/random
  [43]     dynamicColorId: 0 = colourful/random, n = colour n (1..7)
  ```

  - The SDK pads fewer than 7 colours by shortening the array, which shifts the later offsets. **Always send 7.**
  - **Off** = same packet with bit0 cleared (`closedLighting`).
  - **GET** = the same command with flag 00. The response uses the same offsets: R[5..32] colours, R[37] flags, R[38] brightness, ... R[43]. The device also pushes this as 0x98.
  - Response type: mode 0 = static, 1..20 = dynamic, other = custom.
- **Effect names** (UI value → wire mode):

  | Wire mode | Effect |
  |---|---|
  | 22 → mode 0 | Static |
  | 21 | Custom |
  | 1 | Ripple |
  | 2 | Tide |
  | 3 | Ripple Light |
  | 4 | Rotating Storm |
  | 5 | Lucky Rainbow |
  | 6 | Shining Rainbow |
  | 7 | Glitter |
  | 8 | Moving Panels |
  | 9 | Waveform |
  | 10 | Shadow Shift |
  | 11 | Flowing Water |
  | 12 | Sine Wave |
  | 13 | Blooming |
  | 14 | Mosaic |
  | 15 | Rainfall |
  | 16 | Pulse |
  | 17 | Snow Trail |
  | 18 | Plum Blossom |
  | 19 | Laser |
  | 20 | Water Ripple |

  The labels are `kLight.sparkLinkV1.mode3..mode22`.
- **Per-key custom colour, KRGB 0x2A.**
  - SET `[01, KEY, R, G, B]` (RGB order here); the response echoes `R=[st,key,R,G,B]`.
  - GET `[00, KEY]`.
  - **Commit**: `[01, FE, FE, FE, FE]` (`saveCustomLighting`). The app sends this after each batch.
  - **Clear**: write 0,0,0.
  - A batched variant `cmdKRGB` exists (N×[key,r,g,b] padded with FF) but is unused.
- **Other lighting features.**
  - Music-rhythm lighting downloads an external Windows exe (`https://app.aulacn.com/commonAssets/Aula-Music-Setup1.exe`).
  - Light sync uses `https://cdn.aulacn.com/asyncLight/狼蛛_神光同步.exe`.
  - Neither involves HID in this app.
  - Logo light (0x19) uses the same layout as PRGB without dynamicColorId. It is not shown for WIN models; whether the hardware has it is [?].

### 3.9 Performance page [C] (`perf-Dx-QLAFx.js` plus the sparkLinkV1 UI component)

- **Polling rate.** `0x00/0x50`:
  - Query `[50 FF FF]`.
  - Set `[50 idx FF FF]`, where idx 0 = 8 kHz, 1 = 4 kHz, 2 = 2 kHz, 3 = 1 kHz, 4 = 500 Hz, 5 = 250 Hz, 6 = 125 Hz.
  - The UI warns that "switching polling rate will restart the keyboard", so **expect a USB re-enumeration**.
- **Mac mode toggle.**
  - `0x00/0x31` (Mac) or `0x00/0x30` (Win). The perf page sends them **without** an arg (`[31 FF FF]`); `switchSystemMode` sends `[30|31, 01, FF, FF]`.
  - Then re-query with 0x21 and reload the layout.
- **Not exposed for this model:** Win-lock toggle, sleep timer, stability mode, and the other perf items in the i18n file.

### 3.10 Pro Settings (masterSet) [C]

- Hard-coded presets in `index-C7v5zrqA.js` (pro-player configs), applied through the same trigger writes: `{code:"KeyW",singleTravel:..,dp:..,dr:..,rtPress:..,rtRelease:..}`.
- Keys in the preset are set to RT; the others are reset to 2.0 mm, single.
- Local only.

### 3.11 Version page [C] (`version-CZpI-VxG.js`)

- Shows `appVersion` from SYNC.
- **Factory reset**: `0x00/0x11` (`5C 03 00 93 11 FF FF 00`).
- Product manual PDF (cloud).
- **Firmware update for WIN 68 HE MAX does NOT use HID in this app.** It fetches `GET /user/EXE/getFile/WIN 68 HE MAX` and opens `https://app.aulacn.com/commonAssets/<fileName>`, an external (Windows) updater. See section 5.

### 3.12 Cloud / community configs and config import [C, optional]

- A JSON config (lighting, per-key travel/RT/DP/DR, remaps, advanced keys) can be shared or downloaded through `hubapi.aulacn.com/community/...` (login required) and applied with `ca.uesConfig()`. This is just a batch of the commands above.
- The SDK also has a local AES-encrypted export/import (CryptoJS passphrase `fDPy6vvnpPsYm2T0g1bh`). This UI does not use it for this model.

### 3.13 Not applicable to this model (present elsewhere in the app)

- TFT / dot-matrix screen (`tftScreen-*`, `pointScreen-*`, `PIC` commands)
- GIF lighting
- Mouse, DPI and headset pages
- CS2 "GameSense/GSI" overlay (host-only feature in `main.mjs`)
- The sparkLinkV2 / HFD / Bytech / RLW SDKs (other models)

---

## 4. Key identification, matrix and layout

### 4.1 Key ID used in packets [C]

- **The `KEY` byte in every per-key command is the key's _default keycode_** (one byte, HID-usage-like, e.g. A = 0x04, Fn = 0x01, LCtrl = 0xE0). It is not a row/col index.
- Evidence: `initDeviceLayout` copies the DEFKEY codes into `deviceLayout[].keyValue`, and every call uses `Number(d.keyValue)`. "Reset key" writes `{key: keyValue, layout:0, value: keyValue}`.

**Reading the matrix.** DEFKEY 0x2B GET `[00, rowA, rowB]` is sent three times: (0,1), (2,3), (4,5). The response is:

```
R[1] = rowA, R[2..22] = 21 keycodes (u8) for matrix columns 0..20
R[23]= rowB, R[24..44]= 21 keycodes
```

The matrix is **6 rows × 21 columns** (the name "RM6X21" says the same). A 0 means no key at that position.

**UI layout mapping.** The UI drops zero entries and empty rows, and matches the n-th non-empty row and m-th key to the drawn layout (row, col) in order. For live travel, use the **unfiltered** (row, col) from DEFKEY. The travel/calibration arrays use the same 6×21 positions [I, strongly implied by `calibrationCallback`].

**Drawn layouts** (`SparkLinkV1-t4aSMPxg.js`):

| Model | Layout | Keys | Row sizes |
|---|---|---|---|
| WIN 68 | export `b` (const `o`) | 68 | 15 / 15 / 14 / 14 / 10 |
| WIN 60 | export `a` (const `l`) | 61 | 14 / 14 / 13 / 12 / 8 |

The legends are not stored in the app; they come from the device's default codes. A conventional 68% layout is [I], so read DEFKEY once to be certain:

```
R0: Esc 1 2 3 4 5 6 7 8 9 0 - = Backspace(2u) [extra key]
R1: Tab(1.5u) Q W E R T Y U I O P [ ] \(1.5u) [extra key]
R2: Caps(1.75u) A S D F G H J K L ; ' Enter(2.25u) [extra key]
R3: LShift(2.25u) Z X C V B N M , . / RShift(1.75u) Up [extra key]
R4: LCtrl LWin LAlt Space(6.25u) [RAlt] [Fn] [RCtrl?] Left Down Right
```

### 4.2 Switch (axis) tables [C]

`ca.getBaseInfo`: `case"WIN 68 HE MAX":this.axisMap=Gn`. Values are written with KEY LAYOUT 25.

**WIN 68 HE MAX (`Gn` = SparkLinkV1 export `e`):**

| id | label | colour | maxTravel |
|---|---|---|---|
| 0 | 咏春轴 (Wing Chun) | #00EB0C | 3.4 mm |
| 1 | 磁玉pro (Magnetic Jade Pro) | #008084 | 3.3 mm |
| 2 | 黑皇轴 (Black Emperor) | #000000 | 3.4 mm |
| 3 | 万磁王 (Magneto) | #380E58 | 3.3 mm |

**WIN 60 HE MAX (`Un` = export `c`):** 0 咏春轴 3.4, 1 黑皇轴 3.4, 2 万磁王 3.3, 3 磁玉pro 3.3.

### 4.3 Keycode / remap value table (u16) [C]

Full dump: [`keycodes.txt`](keycodes.txt) (from `SparkLinkV1` export `D`).

**Standard keys**

| Range | Meaning |
|---|---|
| 0x0004–0x00E7 | Standard HID keyboard usages (A = 0x04 … F24 = 0x73, numpad, 0xE0–0xE7 modifiers) |
| 0x0001 | Fn |
| 0x0000 | none |

**Consumer and system keys (type 0)**

| Code | Function |
|---|---|
| 0x106F / 0x1070 | screen brightness +/− |
| 0x10B5 | next track |
| 0x10B6 | previous track |
| 0x10B7 | stop |
| 0x10CD | play/pause |
| 0x10E2 | mute |
| 0x10E9 / 0x10EA | volume +/− |
| 0x1183 | media |
| 0x118A | email |
| 0x1192 | calculator |
| 0x1194 | file |
| 0x1221 | search |
| 0x1223 | www |
| 0x129F | desktop |

The consumer keys are "0x1000 + consumer usage" [I].

**Mouse (type 4)**

| Code | Function |
|---|---|
| 0x7301 / 0x7302 / 0x7303 | left / right / middle click |
| 0x7304–0x7307 | move L / R / Up / Down |
| 0x7308 / 0x7309 | wheel +/− |

**Profile and system (type 1)**

| Code | Function |
|---|---|
| 0xF001 / 0xF002 / 0xF003 | momentary Fn1 / Fn2 / Fn3 layer |
| 0xF100 | restore |
| 0xF101 | Win mode |
| 0xF102 | Mac mode |
| 0xF103 | calibrate |
| 0xF108 / 0xF109 / 0xF10A / 0xF10C | profile 1 / 2 / 3 / 4 |
| 0xF10B | WASD/arrow swap |
| 0xF309 | Win-lock |

**Main light (type 2)**

| Code | Function |
|---|---|
| 0xF317 | on/off |
| 0xF311 | direction |
| 0xF310 / 0xF32F | mode +/− |
| 0xF312 / 0xF313 | brightness +/− |
| 0xF314 / 0xF315 | speed +/− |

**Ambient light (type 3)**

| Code | Function |
|---|---|
| 0xF326 | on/off |
| 0xF327 | direction |
| 0xF325 / 0xF32C | mode +/− |
| 0xF32A / 0xF32B | brightness +/− |
| 0xF328 / 0xF329 | speed +/− |

**Hidden (no UI type)**

| Code | Function |
|---|---|
| 0xF336 | static-light change |
| 0xF339 | "snap tap on" |
| 0x6235 | "~" |

---

## 5. Firmware update (HIGH RISK: document only)

- **What this app does for WIN 68/60 HE MAX/PRO** [C]: no HID flashing. `Version_Class.onMountedFn` sends these models to `getDownloadUrl()`:
  - `GET https://hubapi.aulacn.com/user/EXE/getFile/<pName>` returns `{version, fileName}`.
  - The "Update" button opens `https://app.aulacn.com/commonAssets/<fileName>`, an external, almost certainly **Windows-only**, updater exe.
  - The in-app HID flasher (`updateBin`) is only used for other models, via `/user/BIN/binList` and `/user/BIN/getFile/{id}`.
- **SDK bootloader protocol** (`Ze` class, prettified L8636). This is present and may be what the exe does too [?]. Delays come from the code.

**Unlock signature**

- `Rt(data16, K1=[0x67452301,0xEFCDAB89,0x98BADCFE,0x10325476], K2=[0x6523C101,0x730AE556,0xF052A3CD,0xBA88A5FE], step)`.
- This is a TEA-style cipher: 16 rounds, delta 0x9E3779B9.
- It is applied to the 16-byte SN from SYNC, or to the previous signature.
- Anchors: `2654435769`, `_t=[1732584193,4023233417,2562383102,271733878]`.

**Steps**

1. `toBoot()`:
   1. SYNC.
   2. `BL_SIGN(step=2, sig=Rt(sn,..,2), sn)`. The payload is `[step, 0x10, sig16, 0x10, data16, FF×4]`, and the reply gives `signature = R[3..18]` when R[0]==0.
   3. SIGN(step 3), then `BL_ERASE(0)`.
   4. SIGN(2), SIGN(step 4), then **BL_REBOOT** `[FF FF]` (jump to bootloader).
   5. Wait 4000 ms, re-open the device, and require `KeyboardRunMode != 0`.
2. Pad the image to a multiple of 512 bytes with 0xFF.
3. SIGN(2) → SIGN(3) → `BL_ERASE(size)`. The erase payload is `[0,0,0,0, size u32le, FF]`. Wait 100 ms.
4. SIGN(2) → SIGN(6 = write).
5. Loop over **BL_WRITE** chunks:
   - Payload `[addr u32le, size u16le (244), 244 data bytes]`, LEN 250, sent as 4 raw 64-byte reports.
   - Wait for a reply; R[0]==0 means OK, and next addr = u32(R[1..4]) + u16(R[5..6]).
   - 30 ms between chunks, 1 s timeout, 3 retries.
6. **BL_RCRC(size)** `[0,0,0,0,size u32,FF,FF]` returns CRC = u16le(R[9..10]).
   - Compare it with the host CRC: CRC-16 reflected, poly 0x8408, init 0, no final xor, over 512-byte blocks (anchor `33800`).
7. SIGN(2) → SIGN(5 = to-app) → **BL_TOAPP** `[0,0,0,0,size u32, crc u16]`. Wait 4000 ms.

**Risks**

- A wrong image, wrong padding or CRC, or an interrupted erase **bricks the keyboard**. The app's own i18n warns "device may become bricked".
- Whether the bootloader re-enumerates with the same VID/PID/usage page is [?]. The code just re-runs `getDevices()` with the same filter.
- No firmware image is bundled. Recommendation: do not implement this until you have a capture of the official updater and a recovery path.

---

## 6. Server calls and cloud dependence [C]

**API base:** `https://hubapi.aulacn.com/` (axios; `Authorization: Bearer <token>` once logged in).

| Endpoint | Purpose |
|---|---|
| `/user/Client/getNewVersion`, `isNew`, `getVersionList`, `getHotVersionList`, `getLog` | app self-update and changelog |
| `/user/EXE/getFile/{pName}` | **firmware updater link for WIN 68/60 HE** |
| `/user/BIN/binList`, `/user/BIN/getFile/{id}` | HID-flashable firmware for other models |
| `/user/PDF/getFileName` | manual (served from `https://app.aulacn.com/pdf/`) |
| `/user/Log/LogList`, `/user/Log/getLogById/{id}` | (web build) logs |
| `/community/Login/*`, `/community/user/*` | account |
| `/community/keyboardConfig/*`, `/community/like/*`, `/community/favorites/*` | shared/cloud configs |

**Static downloads**

- `https://app.aulacn.com/commonAssets/…` (firmware exe, Aula-Music-Setup1.exe)
- `https://cdn.aulacn.com/asyncLight/狼蛛_神光同步.exe`

**Cloud dependence**

- **No keyboard feature depends on the cloud.** All device configuration is local HID. The cloud is only used for:
  - firmware version check and updater download
  - the manual
  - community configs
  - app updates
- Macros live host-side, in the app's local storage.

---

## 7. Open questions (need a live, read-only capture)

Suggested safe probes are GETs only: SYNC, DEFKEY, KEY GET, PRGB GET, 0x00 queries 0x21/0x50/0x70/0x25/0x01/0x26. Avoid SET, 0x0C/0x0D, 0x11 and every BL_* command until verified.

1. **Response header byte 0, and the status-byte semantics.** What is R[0] on success, and what is the 0xFF FAIL format?
2. **Do all responses use CMD|0x80?** It is confirmed only for five commands.
3. **Response layout of CMD 0x00 sub-commands.** `mn()` assumes R[1]=TYPE and R[2]=value. Verify this against a real reply.
4. **RM6X21 multi-report framing.** Do continuation reports really carry no header? Which matrix rows are covered by PART 1 vs 2? What do SUB 3 status bytes mean? Units of SUB 6 calibration data.
5. **The actual DEFKEY matrix** (key legends per (row, col)) and the code used for the Fn key, for both MAX models.
6. **Firmware precision.** What does PRECISION_STROKE report? Is 0.001 mm actuation and RT resolution honoured (the UI allows it), or is 0.01 effective?
7. **Win vs Mac tables.** Are the Fn0–Fn3 tables separate per Win/Mac mode? Why does the perf page send SET_MAC/WIN without an argument while `switchSystemMode` sends `[1]`?
8. **Macro storage.** Can several keys hold different macros, given that the app always writes at addr 256 with id 0? What is the maximum macro length?
9. **Advanced-key semantics.** MT binding order (hold vs tap). The meaning of the TGL delay. SOCD delay units. DKS phase semantics.
10. **Polling rate change.** Does it re-enumerate, and how long until the device is usable again?
11. **Firmware version string format** returned by SYNC. It affects the SOCD packet format choice.
12. **RGBNACK (0x47).** Its exact meaning, and whether it is needed before PRGB GET.
13. **Logo/side light.** Does the WIN 68 HE MAX hardware have a light that LOGORGB 0x19 controls?
14. **Persistence.** Are settings saved to flash immediately, given that SAVING_PARAMETER 0x02 is never used? KRGB needs the explicit 0xFE commit.
15. **Bootloader re-enumeration.** Does the bootloader appear with the same VID/PID/usage page? The official updater exe flow is not in this app.
16. **macOS permissions.** Whether macOS needs Input Monitoring permission to open the 0xFFA0 interface with hidapi. It probably does not, since this is a vendor usage page.
