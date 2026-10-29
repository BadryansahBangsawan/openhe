import React from "react";

type Kind = "alpha" | "mod" | "acc";
type Cap = [top: string, side: string, legend: string];
type Skin = { case: string; bezel: string; alpha: Cap; mod: Cap; acc: Cap };

// Colours sampled from AULA's WIN 68 HE MAX product photos.
const SKINS: Record<string, Skin> = {
  "grey-yellow": { case: "#1B1B1D", bezel: "#2B2B2E", alpha: ["#ABAAA6", "#74736F", "#F5A21A"], mod: ["#2E2E33", "#1A1A1D", "#ECECEC"], acc: ["#F6B232", "#C7861A", "#FFFFFF"] },
  "white-blue": { case: "#DCDCE1", bezel: "#C4C4CB", alpha: ["#F2EFEA", "#CFCBC4", "#4A4FC0"], mod: ["#8E8E93", "#6D6D72", "#FFFFFF"], acc: ["#4B50C4", "#363A9E", "#FFFFFF"] },
  // Legends are printed on the front of the caps, so they show only faintly.
  "black-red": { case: "#141416", bezel: "#2A2A2D", alpha: ["#1F2023", "#111214", "rgba(255,255,255,0.35)"], mod: ["#1F2023", "#111214", "rgba(255,255,255,0.35)"], acc: ["#1F2023", "#111214", "rgba(255,255,255,0.35)"] },
  gelap: { case: "#0E0F11", bezel: "#26282C", alpha: ["#26272C", "#18191C", "#E8E8EA"], mod: ["#26272C", "#18191C", "#E8E8EA"], acc: ["#26272C", "#18191C", "#E8E8EA"] },
};

type Row = [string, number, Kind?][];
const ROWS: Row[] = [
  [["Esc", 1, "acc"], ["1", 1], ["2", 1], ["3", 1], ["4", 1], ["5", 1], ["6", 1], ["7", 1], ["8", 1], ["9", 1], ["0", 1], ["-", 1], ["=", 1], ["Bksp", 2, "mod"], ["Ins", 1, "mod"]],
  [["Tab", 1.5, "mod"], ["Q", 1], ["W", 1], ["E", 1], ["R", 1], ["T", 1], ["Y", 1], ["U", 1], ["I", 1], ["O", 1], ["P", 1], ["[", 1], ["]", 1], ["\\", 1.5], ["Del", 1, "mod"]],
  [["Caps", 1.75, "mod"], ["A", 1], ["S", 1], ["D", 1], ["F", 1], ["G", 1], ["H", 1], ["J", 1], ["K", 1], ["L", 1], [";", 1], ["'", 1], ["Enter", 2.25, "acc"], ["PgUp", 1, "mod"]],
  [["LShift", 2.25, "mod"], ["Z", 1], ["X", 1], ["C", 1], ["V", 1], ["B", 1], ["N", 1], ["M", 1], [",", 1], [".", 1], ["/", 1], ["RShift", 1.75, "mod"], ["Up", 1, "acc"], ["PgDn", 1, "mod"]],
  [["LCtrl", 1.25, "mod"], ["Win", 1.25, "mod"], ["LAlt", 1.25, "mod"], ["Space", 6.25, "acc"], ["RAlt", 1, "mod"], ["Fn", 1, "mod"], ["RCtrl", 1, "mod"], ["Left", 1, "acc"], ["Down", 1, "acc"], ["Right", 1, "acc"]],
];
// WIN 60 HE MAX: 61 keys, no arrows or nav column (after the product photo).
const ROWS60: Row[] = [
  [["Esc", 1, "acc"], ["1", 1], ["2", 1], ["3", 1], ["4", 1], ["5", 1], ["6", 1], ["7", 1], ["8", 1], ["9", 1], ["0", 1], ["-", 1], ["=", 1], ["Bksp", 2, "mod"]],
  [["Tab", 1.5, "mod"], ["Q", 1], ["W", 1], ["E", 1], ["R", 1], ["T", 1], ["Y", 1], ["U", 1], ["I", 1], ["O", 1], ["P", 1], ["[", 1], ["]", 1], ["\\", 1.5]],
  [["Caps", 1.75, "mod"], ["A", 1], ["S", 1], ["D", 1], ["F", 1], ["G", 1], ["H", 1], ["J", 1], ["K", 1], ["L", 1], [";", 1], ["'", 1], ["Enter", 2.25, "acc"]],
  [["LShift", 2.25, "mod"], ["Z", 1], ["X", 1], ["C", 1], ["V", 1], ["B", 1], ["N", 1], ["M", 1], [",", 1], [".", 1], ["/", 1], ["RShift", 2.75, "mod"]],
  [["LCtrl", 1.25, "mod"], ["Win", 1.25, "mod"], ["LAlt", 1.25, "mod"], ["Space", 6.25, "acc"], ["RAlt", 1.25, "mod"], ["Super", 1.25, "mod"], ["RCtrl", 1.25, "mod"], ["Fn", 1.25, "mod"]],
];

// Custom keycaps: the user picks the top colour of three groups; the side
// wall is a darker shade and the legend is black or white, whichever reads.
function hex(c: string): [number, number, number] {
  const n = parseInt(c.replace("#", ""), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
function shade(c: string, f: number): string {
  const [r, g, b] = hex(c);
  return "#" + [r, g, b].map((v) => Math.round(v * f).toString(16).padStart(2, "0")).join("");
}
function legendFor(c: string): string {
  const [r, g, b] = hex(c);
  return 0.299 * r + 0.587 * g + 0.114 * b > 150 ? "#1A1A1C" : "#F4F4F5";
}
function customSkin(alpha: string, mod: string, acc: string): Skin {
  const cap = (c: string): Cap => [c, shade(c, 0.7), legendFor(c)];
  return { case: "#151517", bezel: "#2A2A2D", alpha: cap(alpha), mod: cap(mod), acc: cap(acc) };
}

const SHIFT: Record<string, string> = { "1": "!", "2": "@", "3": "#", "4": "$", "5": "%", "6": "^", "7": "&", "8": "*", "9": "(", "0": ")", "-": "—", "=": "+", "[": "{", "]": "}", "\\": "|", ";": ":", "'": '"', ",": "<", ".": ">", "/": "?" };
const LABEL: Record<string, string> = { Bksp: "← Backspace", Ins: "Insert", Del: "Delete", Tab: "Tab", Caps: "Caps Lock", Enter: "↵ Enter", PgUp: "PgUp", PgDn: "PgDn", LShift: "⇧ Shift", RShift: "⇧ Shift", LCtrl: "Ctrl", RCtrl: "Ctrl", Super: "Super", Win: "Win", LAlt: "Alt", RAlt: "Alt", Fn: "Fn", Esc: "Esc", Space: "", Up: "↑", Down: "↓", Left: "←", Right: "→" };
const ARROWS = new Set(["Up", "Down", "Left", "Right"]);
// Black Red: one artwork across all caps, black with thin red contour lines
// and a red swirl that rises from the bottom row to the top middle (after the
// product photo). Each cap shows its own window of it.
const SWIRL = "polygon(0% 100%, 0% 82%, 24% 80%, 40% 74%, 48% 64%, 55% 50%, 58% 36%, 52% 18%, 45% 0%, 50% 0%, 57% 16%, 61% 30%, 63% 0%, 74% 0%, 67% 30%, 70% 46%, 76% 58%, 84% 62%, 100% 60%, 100% 100%)";
function Art({ x, y, w, h, dim = false }: { x: number; y: number; w: number; h: number; dim?: boolean }) {
  const sp = Math.max(5, w / 52);
  return (
    <div style={{ position: "absolute", left: -x, top: -y, width: w, height: h, pointerEvents: "none" }}>
      <div style={{ position: "absolute", inset: 0, background: `repeating-radial-gradient(ellipse 260% 150% at 60% 46%, transparent 0 ${sp}px, rgba(196,35,48,0.9) ${sp}px ${sp + 1.2}px), #1F2023` }} />
      <div style={{ position: "absolute", inset: 0, clipPath: SWIRL, background: `repeating-radial-gradient(ellipse 120% 80% at 58% 40%, transparent 0 ${sp}px, rgba(17,18,20,0.85) ${sp}px ${sp + 1.2}px), #C42330` }} />
      {dim && <div style={{ position: "absolute", inset: 0, background: "rgba(0,0,0,0.42)" }} />}
    </div>
  );
}

export default function KeyboardSkin({ skin = "grey-yellow", model = "win68", unit = 50, states = false, selected = ["W"], marked = ["A", "S", "D"], custom = ["#D9D4C7", "#3A3F4B", "#E2725B"] }: { skin?: string; model?: "win68" | "win60"; unit?: number; states?: boolean; selected?: string[]; marked?: string[]; custom?: [string, string, string] }) {
  const s = skin === "kustom" ? customSkin(...custom) : SKINS[skin] ?? SKINS.gelap;
  const rows = model === "win60" ? ROWS60 : ROWS;
  const cols = model === "win60" ? 15 : 16;
  const g = Math.max(1, Math.round(unit / 25));
  const pad = Math.round(unit * 0.24);
  const k = unit / 50;
  const keys: React.ReactNode[] = [];
  const W = Math.round(cols * (unit + g) - g + 2 * pad);
  const H = 5 * (unit + g) - g + 2 * pad;
  const art = skin === "black-red";
  rows.forEach((row, r) => {
    let x = 0;
    row.forEach(([name, w, kind = "alpha"]) => {
      const left = Math.round(pad + x * (unit + g));
      const top = pad + r * (unit + g);
      const width = Math.round(w * (unit + g) - g);
      const [capTop, capSide, legend] = s[kind];
      const face: string = capTop;
      const wall: string = capSide;
      const sel = states && selected.includes(name);
      const mark = states && marked.includes(name);
      const arrow = ARROWS.has(name);
      keys.push(
        <div key={`${r}-${name}`} style={{ position: "absolute", left, top, width, height: unit, background: wall, borderRadius: 6 * k, overflow: art ? "hidden" : undefined, outline: sel ? "2px solid #FFB224" : mark ? "2px solid rgba(255,178,36,0.6)" : undefined, outlineOffset: sel ? 1 : undefined }}>
          {art && <Art x={left} y={top} w={W} h={H} dim />}
          <div style={{ position: "absolute", left: 4 * k, right: 4 * k, top: 2 * k, bottom: 7 * k, background: art ? undefined : face, borderRadius: 5 * k, color: legend, fontWeight: 700, padding: `${4 * k}px ${6 * k}px`, display: "flex", flexDirection: "column", justifyContent: kind === "alpha" ? "flex-start" : "center", alignItems: arrow ? "center" : "flex-start", boxShadow: "inset 0 1px 0 rgba(255,255,255,0.12)", overflow: "hidden" }}>
            {art && <Art x={left + 4 * k} y={top + 2 * k} w={W} h={H} />}
            <div style={{ position: "relative", display: "flex", flexDirection: "column", alignItems: arrow ? "center" : "flex-start" }}>
            {kind === "alpha" ? (
              SHIFT[name] ? (
                <div style={{ display: "flex", flexDirection: "column", lineHeight: 1.05, fontSize: 13 * k }}><span>{SHIFT[name]}</span><span>{name}</span></div>
              ) : (
                <span style={{ fontSize: 17 * k }}>{name}</span>
              )
            ) : (
              <span style={{ fontSize: (arrow ? 17 : 11) * k, whiteSpace: "nowrap" }}>{LABEL[name] ?? name}</span>
            )}
            </div>
          </div>
          {mark && <div style={{ position: "absolute", right: 6 * k, top: 4 * k, width: 6 * k, height: 6 * k, borderRadius: 3 * k, background: "#FFB224" }} />}
        </div>
      );
      x += w;
    });
  });
  return (
    <div style={{ position: "relative", width: W, height: H, background: s.case, border: `2px solid ${s.bezel}`, borderRadius: 10 * k, fontFamily: "Inter", boxShadow: "0 10px 30px rgba(0,0,0,0.45)" }}>
      {keys}
    </div>
  );
}
