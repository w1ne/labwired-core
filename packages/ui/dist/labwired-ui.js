import { jsxs as p, jsx as n, Fragment as Lr } from "react/jsx-runtime";
import Ue, { useMemo as gt, useCallback as C, useRef as D, useEffect as L, useState as z, useReducer as qr, memo as zt } from "react";
function Xr({ active: t, color: e = "#ff3333", size: r = 20, label: i, style: o }) {
  const l = Qr(e, 0.6);
  return /* @__PURE__ */ p("div", { style: { display: "inline-flex", flexDirection: "column", alignItems: "center", gap: 4, ...o }, children: [
    /* @__PURE__ */ p("svg", { width: r, height: r, viewBox: "0 0 20 20", children: [
      /* @__PURE__ */ n("defs", { children: t && /* @__PURE__ */ p("radialGradient", { id: `led-glow-${e}`, children: [
        /* @__PURE__ */ n("stop", { offset: "0%", stopColor: e, stopOpacity: "0.8" }),
        /* @__PURE__ */ n("stop", { offset: "100%", stopColor: e, stopOpacity: "0" })
      ] }) }),
      t && /* @__PURE__ */ n("circle", { cx: "10", cy: "10", r: "10", fill: `url(#led-glow-${e})`, opacity: "0.5" }),
      /* @__PURE__ */ n(
        "circle",
        {
          cx: "10",
          cy: "10",
          r: "7",
          fill: t ? e : l,
          stroke: "var(--lw-black, #000)",
          strokeWidth: "1.5"
        }
      ),
      t && /* @__PURE__ */ n("circle", { cx: "8", cy: "8", r: "2", fill: "rgba(255,255,255,0.4)" })
    ] }),
    i && /* @__PURE__ */ n("span", { style: {
      fontFamily: "var(--lw-font-mono, monospace)",
      fontSize: "0.65rem",
      color: "var(--lw-gray, #444)"
    }, children: i })
  ] });
}
function Qr(t, e) {
  const r = parseInt(t.replace("#", ""), 16), i = Math.max(0, (r >> 16 & 255) * (1 - e)), o = Math.max(0, (r >> 8 & 255) * (1 - e)), l = Math.max(0, (r & 255) * (1 - e));
  return `rgb(${Math.round(i)}, ${Math.round(o)}, ${Math.round(l)})`;
}
function Rn({
  boardName: t,
  chipId: e,
  boardIo: r,
  boardIoStates: i,
  onButtonToggle: o,
  width: l = 600,
  height: c = 400,
  style: a
}) {
  const s = gt(() => {
    const m = /* @__PURE__ */ new Map();
    for (const W of i)
      m.set(W.id, W.active);
    return m;
  }, [i]), f = gt(() => {
    if (r.length === 0) return [];
    const m = l / 2, W = c / 2, R = Math.min(l, c) * 0.32;
    return r.map((k, I) => {
      const T = I / r.length * 2 * Math.PI - Math.PI / 2;
      return {
        id: k.id,
        kind: k.kind,
        label: k.id,
        sublabel: `${k.peripheral.toUpperCase()}[${k.pin}]`,
        x: m + R * Math.cos(T),
        y: W + R * Math.sin(T),
        active: s.get(k.id) ?? null
      };
    });
  }, [r, i, l, c, s]), d = l / 2, u = c / 2, y = 180, x = 100;
  return /* @__PURE__ */ n("div", { style: {
    background: "var(--lw-bg, #fff)",
    border: "var(--lw-border, 2px solid #000)",
    borderRadius: "var(--lw-radius, 12px)",
    boxShadow: "var(--lw-shadow, 4px 4px 0px #000)",
    overflow: "hidden",
    ...a
  }, children: /* @__PURE__ */ p(
    "svg",
    {
      width: l,
      height: c,
      viewBox: `0 0 ${l} ${c}`,
      style: { display: "block" },
      children: [
        /* @__PURE__ */ n("defs", { children: /* @__PURE__ */ n("style", { children: `
            .mcu-block { fill: #1e1e28; stroke: #000; stroke-width: 2; rx: 10; }
            .mcu-label { fill: #fff; font-family: 'Outfit', sans-serif; font-size: 16px; font-weight: 700; }
            .mcu-sub { fill: #888; font-family: 'JetBrains Mono', monospace; font-size: 11px; }
            .wire { stroke: #ccc; stroke-width: 2; stroke-dasharray: 6 3; }
            .node-card { fill: #fff; stroke: #000; stroke-width: 2; rx: 8; }
            .node-label { fill: #000; font-family: 'Outfit', sans-serif; font-size: 12px; font-weight: 700; }
            .node-sub { fill: #888; font-family: 'JetBrains Mono', monospace; font-size: 10px; }
            .pill-on { fill: #27c93f; }
            .pill-off { fill: #888; }
            .pill-unknown { fill: #444; }
            .pill-text { fill: #fff; font-family: 'Outfit', sans-serif; font-size: 9px; font-weight: 700; }
          ` }) }),
        f.map((m) => /* @__PURE__ */ n(
          "line",
          {
            x1: d,
            y1: u,
            x2: m.x,
            y2: m.y,
            className: "wire"
          },
          `wire-${m.id}`
        )),
        /* @__PURE__ */ p("g", { transform: `translate(${d - y / 2}, ${u - x / 2})`, children: [
          /* @__PURE__ */ n("rect", { width: y, height: x, className: "mcu-block" }),
          /* @__PURE__ */ n("text", { x: y / 2, y: 40, textAnchor: "middle", className: "mcu-label", children: e }),
          /* @__PURE__ */ n("text", { x: y / 2, y: 62, textAnchor: "middle", className: "mcu-sub", children: t })
        ] }),
        f.map((m) => {
          const k = Zr(m.kind, m.active);
          return /* @__PURE__ */ p(
            "g",
            {
              transform: `translate(${m.x - 140 / 2}, ${m.y - 72 / 2})`,
              style: { cursor: m.kind === "button" ? "pointer" : "default" },
              onMouseDown: m.kind === "button" ? () => o == null ? void 0 : o(m.id, !0) : void 0,
              onMouseUp: m.kind === "button" ? () => o == null ? void 0 : o(m.id, !1) : void 0,
              onMouseLeave: m.kind === "button" && m.active ? () => o == null ? void 0 : o(m.id, !1) : void 0,
              children: [
                /* @__PURE__ */ n("rect", { width: 140, height: 72, className: "node-card" }),
                /* @__PURE__ */ n("text", { x: 10, y: 20, className: "node-label", children: m.label }),
                /* @__PURE__ */ n("text", { x: 10, y: 36, className: "node-sub", children: m.sublabel }),
                /* @__PURE__ */ n(
                  "rect",
                  {
                    x: 70,
                    y: 48,
                    width: 60,
                    height: 16,
                    rx: 8,
                    className: `pill-${k.css}`
                  }
                ),
                /* @__PURE__ */ n(
                  "text",
                  {
                    x: 100,
                    y: 59,
                    textAnchor: "middle",
                    className: "pill-text",
                    children: k.label
                  }
                ),
                m.kind === "led" && /* @__PURE__ */ n("foreignObject", { x: 10, y: 44, width: 24, height: 24, children: /* @__PURE__ */ n(Xr, { active: m.active === !0, size: 18 }) })
              ]
            },
            m.id
          );
        }),
        f.length === 0 && /* @__PURE__ */ n(
          "text",
          {
            x: d,
            y: u + 80,
            textAnchor: "middle",
            className: "mcu-sub",
            children: "No board IO configured"
          }
        )
      ]
    }
  ) });
}
function Zr(t, e) {
  return e === null ? { label: "N/A", css: "unknown" } : t === "button" ? e ? { label: "PRESSED", css: "on" } : { label: "RELEASED", css: "off" } : e ? { label: "ON", css: "on" } : { label: "OFF", css: "off" };
}
function zn({ id: t, pressed: e, onToggle: r, label: i, size: o = 28, style: l }) {
  const c = C(() => r(t, !0), [t, r]), a = C(() => r(t, !1), [t, r]);
  return /* @__PURE__ */ p("div", { style: { display: "inline-flex", alignItems: "center", gap: 8, ...l }, children: [
    /* @__PURE__ */ p(
      "svg",
      {
        width: o,
        height: o,
        viewBox: "0 0 28 28",
        style: { cursor: "pointer" },
        onMouseDown: c,
        onMouseUp: a,
        onMouseLeave: () => e && r(t, !1),
        children: [
          /* @__PURE__ */ n(
            "rect",
            {
              x: "2",
              y: "2",
              width: "24",
              height: "24",
              rx: "4",
              fill: e ? "var(--lw-gray, #444)" : "var(--lw-bg-alt, #f8f9fa)",
              stroke: "var(--lw-black, #000)",
              strokeWidth: "2"
            }
          ),
          /* @__PURE__ */ n(
            "rect",
            {
              x: "6",
              y: "6",
              width: "16",
              height: "16",
              rx: "2",
              fill: e ? "var(--lw-black, #000)" : "var(--lw-gray-light, #888)",
              stroke: "var(--lw-black, #000)",
              strokeWidth: "1"
            }
          )
        ]
      }
    ),
    i && /* @__PURE__ */ n("span", { style: {
      fontFamily: "var(--lw-font-mono, monospace)",
      fontSize: "0.7rem",
      color: "var(--lw-gray, #444)"
    }, children: i })
  ] });
}
function Ln({
  running: t,
  onPlay: e,
  onPause: r,
  onStep: i,
  onReset: o,
  pc: l,
  cycles: c,
  style: a
}) {
  return /* @__PURE__ */ p("div", { style: {
    display: "flex",
    alignItems: "center",
    gap: "0.75rem",
    padding: "0.5rem 1rem",
    background: "var(--lw-bg, #fff)",
    border: "var(--lw-border, 2px solid #000)",
    borderRadius: "var(--lw-radius-sm, 8px)",
    boxShadow: "var(--lw-shadow, 4px 4px 0px #000)",
    fontFamily: "var(--lw-font-mono, monospace)",
    fontSize: "0.8rem",
    ...a
  }, children: [
    t ? /* @__PURE__ */ n(it, { onClick: r, title: "Pause", children: /* @__PURE__ */ n(ti, {}) }) : /* @__PURE__ */ n(it, { onClick: e, title: "Run", children: /* @__PURE__ */ n(ei, {}) }),
    /* @__PURE__ */ n(it, { onClick: i, title: "Step", disabled: t, children: /* @__PURE__ */ n(ri, {}) }),
    /* @__PURE__ */ n(it, { onClick: o, title: "Reset", children: /* @__PURE__ */ n(ii, {}) }),
    (l !== void 0 || c !== void 0) && /* @__PURE__ */ p("div", { style: {
      marginLeft: "auto",
      display: "flex",
      gap: "1rem",
      color: "var(--lw-gray, #444)",
      fontSize: "0.75rem"
    }, children: [
      l !== void 0 && /* @__PURE__ */ p("span", { children: [
        "PC: 0x",
        l.toString(16).toUpperCase().padStart(8, "0")
      ] }),
      c !== void 0 && /* @__PURE__ */ p("span", { children: [
        "Cycles: ",
        c.toLocaleString()
      ] })
    ] })
  ] });
}
function it({ onClick: t, title: e, disabled: r, children: i }) {
  return /* @__PURE__ */ n(
    "button",
    {
      onClick: t,
      title: e,
      disabled: r,
      style: {
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: 32,
        height: 32,
        padding: 0,
        background: r ? "var(--lw-bg-alt, #f8f9fa)" : "var(--lw-black, #000)",
        color: r ? "var(--lw-gray, #444)" : "var(--lw-bg, #fff)",
        border: "var(--lw-border, 2px solid #000)",
        borderRadius: 6,
        cursor: r ? "not-allowed" : "pointer",
        boxShadow: r ? "none" : "2px 2px 0px #000",
        textTransform: "none",
        letterSpacing: "normal",
        fontSize: "0.8rem"
      },
      children: i
    }
  );
}
function ei() {
  return /* @__PURE__ */ n("svg", { width: "14", height: "14", viewBox: "0 0 14 14", fill: "currentColor", children: /* @__PURE__ */ n("polygon", { points: "2,0 14,7 2,14" }) });
}
function ti() {
  return /* @__PURE__ */ p("svg", { width: "14", height: "14", viewBox: "0 0 14 14", fill: "currentColor", children: [
    /* @__PURE__ */ n("rect", { x: "1", y: "0", width: "4", height: "14" }),
    /* @__PURE__ */ n("rect", { x: "9", y: "0", width: "4", height: "14" })
  ] });
}
function ri() {
  return /* @__PURE__ */ p("svg", { width: "14", height: "14", viewBox: "0 0 14 14", fill: "currentColor", children: [
    /* @__PURE__ */ n("polygon", { points: "0,0 8,7 0,14" }),
    /* @__PURE__ */ n("rect", { x: "10", y: "0", width: "3", height: "14" })
  ] });
}
function ii() {
  return /* @__PURE__ */ n("svg", { width: "14", height: "14", viewBox: "0 0 14 14", fill: "currentColor", children: /* @__PURE__ */ n(
    "path",
    {
      d: "M7 1a6 6 0 1 0 6 6h-2a4 4 0 1 1-4-4V1L3 4l4 3V5a2 2 0 1 0 2 2h2A4 4 0 0 0 7 1z",
      transform: "translate(0, 1)"
    }
  ) });
}
function Bn({ registers: t, style: e }) {
  const r = gt(() => Array.from(t.entries()), [t]);
  return /* @__PURE__ */ p("div", { style: {
    background: "var(--lw-dark-bg, #1e1e28)",
    border: "var(--lw-border, 2px solid #000)",
    borderRadius: "var(--lw-radius-sm, 8px)",
    padding: "0.75rem",
    fontFamily: "var(--lw-font-mono, monospace)",
    fontSize: "0.75rem",
    overflow: "auto",
    ...e
  }, children: [
    /* @__PURE__ */ n("div", { style: {
      fontFamily: "var(--lw-font-heading, sans-serif)",
      fontWeight: 700,
      fontSize: "0.7rem",
      textTransform: "uppercase",
      letterSpacing: "0.05em",
      color: "var(--lw-gray-light, #888)",
      marginBottom: "0.5rem"
    }, children: "Registers" }),
    /* @__PURE__ */ n("div", { style: {
      display: "grid",
      gridTemplateColumns: "repeat(2, 1fr)",
      gap: "2px 1rem"
    }, children: r.map(([i, o]) => /* @__PURE__ */ p("div", { style: { display: "flex", justifyContent: "space-between" }, children: [
      /* @__PURE__ */ n("span", { style: { color: "var(--lw-cyan, #0ff)" }, children: i }),
      /* @__PURE__ */ p("span", { style: { color: "var(--lw-dark-text, #d4d4d4)" }, children: [
        "0x",
        o.toString(16).toUpperCase().padStart(8, "0")
      ] })
    ] }, i)) })
  ] });
}
function Hn({
  data: t,
  baseAddress: e,
  bytesPerRow: r = 16,
  style: i
}) {
  const o = gt(() => {
    const l = [];
    for (let c = 0; c < t.length; c += r) {
      const a = t.slice(c, c + r), s = Array.from(a).map((d) => d.toString(16).toUpperCase().padStart(2, "0")), f = Array.from(a).map((d) => d >= 32 && d < 127 ? String.fromCharCode(d) : ".").join("");
      l.push({ addr: e + c, hex: s, ascii: f });
    }
    return l;
  }, [t, e, r]);
  return /* @__PURE__ */ p("div", { style: {
    background: "var(--lw-dark-bg, #1e1e28)",
    border: "var(--lw-border, 2px solid #000)",
    borderRadius: "var(--lw-radius-sm, 8px)",
    padding: "0.75rem",
    fontFamily: "var(--lw-font-mono, monospace)",
    fontSize: "0.7rem",
    overflow: "auto",
    maxHeight: 200,
    ...i
  }, children: [
    /* @__PURE__ */ n("div", { style: {
      fontFamily: "var(--lw-font-heading, sans-serif)",
      fontWeight: 700,
      fontSize: "0.7rem",
      textTransform: "uppercase",
      letterSpacing: "0.05em",
      color: "var(--lw-gray-light, #888)",
      marginBottom: "0.5rem"
    }, children: "Memory" }),
    /* @__PURE__ */ n("table", { style: { borderCollapse: "collapse", width: "100%" }, children: /* @__PURE__ */ n("tbody", { children: o.map((l) => /* @__PURE__ */ p("tr", { children: [
      /* @__PURE__ */ p("td", { style: { color: "#569cd6", paddingRight: "1rem", whiteSpace: "nowrap" }, children: [
        "0x",
        l.addr.toString(16).toUpperCase().padStart(8, "0")
      ] }),
      /* @__PURE__ */ n("td", { style: { color: "var(--lw-dark-text, #d4d4d4)", whiteSpace: "nowrap" }, children: l.hex.join(" ") }),
      /* @__PURE__ */ n("td", { style: { color: "var(--lw-green, #27c93f)", paddingLeft: "1rem", whiteSpace: "nowrap" }, children: l.ascii })
    ] }, l.addr)) }) })
  ] });
}
function Gn({ entries: t, maxEntries: e = 50, style: r }) {
  const i = D(null), o = t.slice(-e);
  return L(() => {
    i.current && (i.current.scrollTop = i.current.scrollHeight);
  }, [t.length]), /* @__PURE__ */ p(
    "div",
    {
      ref: i,
      style: {
        background: "var(--lw-dark-bg, #1e1e28)",
        border: "var(--lw-border, 2px solid #000)",
        borderRadius: "var(--lw-radius-sm, 8px)",
        padding: "0.75rem",
        fontFamily: "var(--lw-font-mono, monospace)",
        fontSize: "0.7rem",
        overflow: "auto",
        maxHeight: 200,
        ...r
      },
      children: [
        /* @__PURE__ */ n("div", { style: {
          fontFamily: "var(--lw-font-heading, sans-serif)",
          fontWeight: 700,
          fontSize: "0.7rem",
          textTransform: "uppercase",
          letterSpacing: "0.05em",
          color: "var(--lw-gray-light, #888)",
          marginBottom: "0.5rem"
        }, children: "Instruction Trace" }),
        o.map((l, c) => /* @__PURE__ */ p("div", { style: { display: "flex", gap: "1rem" }, children: [
          /* @__PURE__ */ p("span", { style: { color: "#569cd6", minWidth: 80 }, children: [
            "0x",
            l.pc.toString(16).toUpperCase().padStart(8, "0")
          ] }),
          /* @__PURE__ */ n("span", { style: { color: "var(--lw-cyan, #0ff)" }, children: l.disassembly })
        ] }, c))
      ]
    }
  );
}
function jn({ output: t, onClear: e, onSend: r, style: i }) {
  const o = D(null), [l, c] = z(""), a = C(() => {
    !l || !r || (r(l), c(""));
  }, [l, r]);
  return L(() => {
    o.current && (o.current.scrollTop = o.current.scrollHeight);
  }, [t]), /* @__PURE__ */ p("div", { style: {
    background: "var(--lw-dark-bg, #1e1e28)",
    border: "var(--lw-border, 2px solid #000)",
    borderRadius: "var(--lw-radius-sm, 8px)",
    display: "flex",
    flexDirection: "column",
    overflow: "hidden",
    ...i
  }, children: [
    /* @__PURE__ */ p("div", { style: {
      display: "flex",
      justifyContent: "space-between",
      alignItems: "center",
      padding: "0.5rem 0.75rem",
      borderBottom: "1px solid var(--lw-dark-border, #333)"
    }, children: [
      /* @__PURE__ */ n("span", { style: {
        fontFamily: "var(--lw-font-heading, sans-serif)",
        fontWeight: 700,
        fontSize: "0.7rem",
        textTransform: "uppercase",
        letterSpacing: "0.05em",
        color: "var(--lw-gray-light, #888)"
      }, children: "Serial Monitor" }),
      e && /* @__PURE__ */ n(
        "button",
        {
          onClick: e,
          style: {
            background: "transparent",
            border: "none",
            color: "var(--lw-gray-light, #888)",
            cursor: "pointer",
            fontFamily: "var(--lw-font-mono, monospace)",
            fontSize: "0.65rem",
            padding: "2px 6px",
            boxShadow: "none",
            textTransform: "none"
          },
          children: "Clear"
        }
      )
    ] }),
    /* @__PURE__ */ n(
      "pre",
      {
        ref: o,
        style: {
          margin: 0,
          padding: "0.75rem",
          fontFamily: "var(--lw-font-mono, monospace)",
          fontSize: "0.75rem",
          color: "var(--lw-green, #27c93f)",
          lineHeight: 1.5,
          overflow: "auto",
          flex: 1,
          minHeight: 80,
          whiteSpace: "pre-wrap",
          wordBreak: "break-all"
        },
        children: t || /* @__PURE__ */ n("span", { style: { color: "var(--lw-gray-light, #888)", fontStyle: "italic" }, children: "No output yet..." })
      }
    ),
    r && /* @__PURE__ */ p("div", { style: {
      display: "flex",
      gap: "4px",
      padding: "4px 8px",
      borderTop: "1px solid var(--lw-dark-border, #333)"
    }, children: [
      /* @__PURE__ */ n(
        "input",
        {
          type: "text",
          value: l,
          onChange: (s) => c(s.target.value),
          onKeyDown: (s) => {
            s.key === "Enter" && a();
          },
          placeholder: "Type to send...",
          style: {
            flex: 1,
            fontFamily: "var(--lw-font-mono, monospace)",
            fontSize: "0.75rem",
            padding: "4px 8px",
            border: "1px solid rgba(255,255,255,0.15)",
            borderRadius: "3px",
            background: "rgba(255,255,255,0.05)",
            color: "#fff",
            outline: "none"
          }
        }
      ),
      /* @__PURE__ */ n(
        "button",
        {
          onClick: a,
          style: {
            fontFamily: "var(--lw-font-mono, monospace)",
            fontSize: "0.65rem",
            padding: "4px 10px",
            border: "1px solid var(--lw-pink, #e83e8c)",
            borderRadius: "3px",
            background: "var(--lw-pink, #e83e8c)",
            color: "#fff",
            cursor: "pointer",
            boxShadow: "none",
            textTransform: "none"
          },
          children: "Send"
        }
      )
    ] })
  ] });
}
class et {
  constructor(e) {
    this._cycles = 0, this.sim = e;
  }
  /** Initialize from YAML config + firmware ELF bytes. */
  static async fromConfig(e, r) {
    const i = e.WasmSimulator.new_from_config(
      r.systemYaml,
      r.chipYaml,
      r.firmware
    );
    return new et(i);
  }
  /** Initialize with legacy hardcoded board (backward compat). */
  static async fromFirmware(e, r) {
    const i = new e.WasmSimulator(r);
    return new et(i);
  }
  get totalCycles() {
    return this._cycles;
  }
  stepBatch(e) {
    const r = this.sim.step_batch(e);
    return this._cycles += r, r;
  }
  stepSingle() {
    this.sim.step_single(), this._cycles += 1;
  }
  getPC() {
    return this.sim.get_pc();
  }
  getRegister(e) {
    return this.sim.get_register(e);
  }
  getRegisterNames() {
    return this.sim.get_register_names();
  }
  getDisassembly() {
    return this.sim.get_disassembly();
  }
  readMemory(e, r) {
    return this.sim.read_memory(e, r);
  }
  getBoardIoConfig() {
    return this.sim.get_board_io_config();
  }
  getBoardIoStates() {
    return this.sim.get_board_io_states();
  }
  setBoardIoInput(e, r) {
    this.sim.set_board_io_input(e, r);
  }
  drainUartOutput() {
    return this.sim.drain_uart_output();
  }
  feedUartInput(e) {
    this.sim.feed_uart_input(new TextEncoder().encode(e));
  }
  setAdcValue(e, r) {
    this.sim.set_adc_value(e, r);
  }
  getAnalogStates() {
    return this.sim.get_board_io_analog_states() ?? [];
  }
  getPeripheralSnapshot(e) {
    return this.sim.get_peripheral_snapshot(e);
  }
  getPeripheralList() {
    return this.sim.get_peripheral_list();
  }
  /** Legacy: hardcoded LED state for backward compat. */
  getLedState() {
    return this.sim.get_led_state();
  }
  dispose() {
    this.sim.free();
  }
}
function Un(t) {
  const [e, r] = z(null), [i, o] = z(!0), [l, c] = z(null), a = D(null);
  return L(() => {
    let s = !1;
    async function f() {
      o(!0), c(null);
      try {
        let d;
        if (t.config)
          d = await et.fromConfig(t.wasmModule, t.config);
        else if (t.firmware)
          d = await et.fromFirmware(t.wasmModule, t.firmware);
        else
          throw new Error("Either config or firmware must be provided");
        s ? d.dispose() : (a.current = d, r(d), o(!1));
      } catch (d) {
        s || (c(d instanceof Error ? d.message : String(d)), o(!1));
      }
    }
    return f(), () => {
      s = !0, a.current && (a.current.dispose(), a.current = null);
    };
  }, [t.wasmModule, t.config, t.firmware]), { bridge: e, loading: i, error: l };
}
const ni = {
  pc: 0,
  cycles: 0,
  boardIoStates: [],
  uartOutput: "",
  disassembly: ""
};
function Vn(t) {
  const { bridge: e, running: r, cyclesPerFrame: i = 5e3 } = t, [o, l] = z(ni), c = D(""), a = D(0), s = C(
    (u) => {
      const y = u.drainUartOutput();
      if (y.length > 0) {
        const x = new TextDecoder();
        c.current += x.decode(y);
      }
      l({
        pc: u.getPC(),
        cycles: u.totalCycles,
        boardIoStates: u.getBoardIoStates(),
        uartOutput: c.current,
        disassembly: u.getDisassembly()
      });
    },
    []
  );
  L(() => {
    if (!e || !r) return;
    function u() {
      if (e) {
        try {
          e.stepBatch(i);
        } catch {
          return;
        }
        s(e), a.current = requestAnimationFrame(u);
      }
    }
    return a.current = requestAnimationFrame(u), () => {
      a.current && cancelAnimationFrame(a.current);
    };
  }, [e, r, i, s]), L(() => {
    e && !r && s(e);
  }, [e, r, s]);
  const f = C(() => {
    e && (e.stepSingle(), s(e));
  }, [e, s]), d = C(() => {
    c.current = "", l((u) => ({ ...u, uartOutput: "" }));
  }, []);
  return { state: o, stepOnce: f, clearUart: d };
}
const ne = 200, Ge = 280, oi = 20, li = 40;
function kt() {
  const t = [], e = [
    { prefix: "PA", count: 16, side: "left" },
    { prefix: "PB", count: 16, side: "right" }
  ];
  for (const r of e)
    for (let i = 0; i < Math.min(r.count, 12); i++)
      t.push({
        id: `${r.prefix}${i}`,
        x: r.side === "left" ? 0 : ne,
        y: li + i * oi,
        side: r.side,
        label: `${r.prefix}${i}`
      });
  return t.push({ id: "VCC", x: ne / 2 - 20, y: Ge, side: "bottom", label: "VCC" }), t.push({ id: "GND", x: ne / 2 + 20, y: Ge, side: "bottom", label: "GND" }), t;
}
const Bt = {
  type: "mcu",
  label: "MCU",
  category: "mcu",
  width: ne,
  height: Ge,
  pins: kt(),
  defaultAttrs: {},
  render: (t, e) => /* @__PURE__ */ p("g", { children: [
    /* @__PURE__ */ n(
      "rect",
      {
        width: ne,
        height: Ge,
        rx: 8,
        fill: "#1e1e28",
        stroke: e != null && e.selected ? "#e83e8c" : "#000",
        strokeWidth: e != null && e.selected ? 3 : 2
      }
    ),
    /* @__PURE__ */ n(
      "text",
      {
        x: ne / 2,
        y: 20,
        textAnchor: "middle",
        fill: "#fff",
        fontFamily: "'Outfit', sans-serif",
        fontSize: 14,
        fontWeight: 700,
        children: "STM32"
      }
    ),
    kt().filter((r) => r.side === "left").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: 8,
        y: r.y + 4,
        fill: "#888",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 8,
        children: r.label
      },
      r.id
    )),
    kt().filter((r) => r.side === "right").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: ne - 8,
        y: r.y + 4,
        textAnchor: "end",
        fill: "#888",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 8,
        children: r.label
      },
      r.id
    )),
    /* @__PURE__ */ n(
      "text",
      {
        x: ne / 2 - 20,
        y: Ge - 8,
        textAnchor: "middle",
        fill: "#ff3333",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 8,
        children: "VCC"
      }
    ),
    /* @__PURE__ */ n(
      "text",
      {
        x: ne / 2 + 20,
        y: Ge - 8,
        textAnchor: "middle",
        fill: "#888",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 8,
        children: "GND"
      }
    )
  ] })
}, X = 60, ae = 80, Ht = {
  type: "led",
  label: "LED",
  category: "output",
  width: X,
  height: ae,
  pins: [
    { id: "A", x: X / 2, y: 0, side: "top", label: "A" },
    { id: "C", x: X / 2, y: ae, side: "bottom", label: "C" }
  ],
  defaultAttrs: { color: "red" },
  boardIoKind: "led",
  attrFields: [
    {
      key: "color",
      label: "Color",
      type: "select",
      options: ["red", "green", "blue", "yellow", "white"]
    }
  ],
  render: (t, e) => {
    const r = t.color || "red", o = {
      red: "#ff3333",
      green: "#27c93f",
      blue: "#3399ff",
      yellow: "#ffcc00",
      white: "#ffffff"
    }[r] || r, l = e != null && e.active ? o : ci(o, 0.6), c = e == null ? void 0 : e.selected;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 6,
          y: 16,
          width: X - 12,
          height: ae - 32,
          rx: 6,
          fill: "#f8f9fa",
          stroke: c ? "#e83e8c" : "#000",
          strokeWidth: c ? 2.5 : 1.5
        }
      ),
      (e == null ? void 0 : e.active) && /* @__PURE__ */ n("circle", { cx: X / 2, cy: ae / 2, r: 22, fill: o, opacity: 0.3 }),
      /* @__PURE__ */ n(
        "circle",
        {
          cx: X / 2,
          cy: ae / 2,
          r: 14,
          fill: l,
          stroke: "#000",
          strokeWidth: 1
        }
      ),
      (e == null ? void 0 : e.active) && /* @__PURE__ */ n("circle", { cx: X / 2 - 4, cy: ae / 2 - 4, r: 4, fill: "rgba(255,255,255,0.5)" }),
      /* @__PURE__ */ n(
        "text",
        {
          x: X / 2,
          y: 12,
          textAnchor: "middle",
          fill: "#888",
          fontFamily: "monospace",
          fontSize: 8,
          children: "A"
        }
      ),
      /* @__PURE__ */ n(
        "text",
        {
          x: X / 2,
          y: ae - 4,
          textAnchor: "middle",
          fill: "#888",
          fontFamily: "monospace",
          fontSize: 8,
          children: "C"
        }
      ),
      (e == null ? void 0 : e.analogValue) !== void 0 && /* @__PURE__ */ p(
        "text",
        {
          x: X / 2,
          y: ae + 12,
          textAnchor: "middle",
          fill: "#27c93f",
          fontFamily: "monospace",
          fontSize: 8,
          children: [
            Math.round(e.analogValue / 40.95),
            "%"
          ]
        }
      )
    ] });
  }
};
function ci(t, e) {
  const r = parseInt(t.replace("#", ""), 16), i = Math.max(0, (r >> 16 & 255) * (1 - e)), o = Math.max(0, (r >> 8 & 255) * (1 - e)), l = Math.max(0, (r & 255) * (1 - e));
  return `rgb(${Math.round(i)},${Math.round(o)},${Math.round(l)})`;
}
const De = 64, se = 64, Gt = {
  type: "button",
  label: "Push Button",
  category: "input",
  width: De,
  height: se,
  pins: [
    { id: "1", x: 0, y: se / 2, side: "left", label: "1" },
    { id: "2", x: De, y: se / 2, side: "right", label: "2" }
  ],
  defaultAttrs: {},
  boardIoKind: "button",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = e == null ? void 0 : e.active;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 3,
          y: 3,
          width: De - 6,
          height: se - 6,
          rx: 8,
          fill: "#f8f9fa",
          stroke: r ? "#e83e8c" : "#000",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n(
        "rect",
        {
          x: 14,
          y: 14,
          width: De - 28,
          height: se - 28,
          rx: 6,
          fill: i ? "#333" : "#888",
          stroke: "#000",
          strokeWidth: 1
        }
      ),
      /* @__PURE__ */ n("text", { x: 8, y: se / 2 + 4, fill: "#888", fontFamily: "monospace", fontSize: 8, children: "1" }),
      /* @__PURE__ */ n("text", { x: De - 8, y: se / 2 + 4, textAnchor: "end", fill: "#888", fontFamily: "monospace", fontSize: 8, children: "2" }),
      i && /* @__PURE__ */ n(
        "text",
        {
          x: De / 2,
          y: se + 12,
          textAnchor: "middle",
          fill: "#27c93f",
          fontFamily: "monospace",
          fontSize: 8,
          children: "ON"
        }
      )
    ] });
  }
}, Ve = 80, O = 32, jt = {
  type: "resistor",
  label: "Resistor",
  category: "passive",
  width: Ve,
  height: O,
  pins: [
    { id: "1", x: 0, y: O / 2, side: "left", label: "1" },
    { id: "2", x: Ve, y: O / 2, side: "right", label: "2" }
  ],
  defaultAttrs: { value: "220" },
  attrFields: [
    { key: "value", label: "Resistance (Ω)", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = t.value || "220";
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n("line", { x1: 0, y1: O / 2, x2: 14, y2: O / 2, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: Ve - 14, y1: O / 2, x2: Ve, y2: O / 2, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n(
        "polyline",
        {
          points: `14,${O / 2} 20,${O / 2 - 8} 26,${O / 2 + 8} 32,${O / 2 - 8} 38,${O / 2 + 8} 44,${O / 2 - 8} 50,${O / 2 + 8} 56,${O / 2 - 8} 62,${O / 2 + 8} 66,${O / 2}`,
          fill: "none",
          stroke: r ? "#e83e8c" : "#000",
          strokeWidth: r ? 2.5 : 2,
          strokeLinejoin: "round"
        }
      ),
      /* @__PURE__ */ p(
        "text",
        {
          x: Ve / 2,
          y: O / 2 - 12,
          textAnchor: "middle",
          fill: "#444",
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: 9,
          children: [
            i,
            "Ω"
          ]
        }
      )
    ] });
  }
}, H = 72, Q = 72, Ut = {
  type: "potentiometer",
  label: "Potentiometer",
  category: "input",
  width: H,
  height: Q,
  pins: [
    { id: "1", x: 0, y: Q - 12, side: "left", label: "1" },
    { id: "W", x: H / 2, y: 0, side: "top", label: "W" },
    { id: "2", x: H, y: Q - 12, side: "right", label: "2" }
  ],
  defaultAttrs: { value: "10K" },
  boardIoKind: "adc_input",
  attrFields: [
    { key: "value", label: "Resistance", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = t.value || "10K";
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "circle",
        {
          cx: H / 2,
          cy: Q / 2 + 4,
          r: 28,
          fill: "#f8f9fa",
          stroke: r ? "#e83e8c" : "#000",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n(
        "line",
        {
          x1: H / 2,
          y1: 10,
          x2: H / 2,
          y2: Q / 2 + 4,
          stroke: "#e83e8c",
          strokeWidth: 2.5
        }
      ),
      /* @__PURE__ */ n(
        "polygon",
        {
          points: `${H / 2},10 ${H / 2 - 5},20 ${H / 2 + 5},20`,
          fill: "#e83e8c"
        }
      ),
      /* @__PURE__ */ n("text", { x: 6, y: Q, fill: "#888", fontFamily: "monospace", fontSize: 8, children: "1" }),
      /* @__PURE__ */ n("text", { x: H / 2, y: Q, textAnchor: "middle", fill: "#888", fontFamily: "monospace", fontSize: 8, children: "W" }),
      /* @__PURE__ */ n("text", { x: H - 6, y: Q, textAnchor: "end", fill: "#888", fontFamily: "monospace", fontSize: 8, children: "2" }),
      /* @__PURE__ */ n(
        "text",
        {
          x: H / 2,
          y: Q / 2 + 10,
          textAnchor: "middle",
          fill: "#444",
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: 9,
          children: i
        }
      ),
      (e == null ? void 0 : e.analogValue) !== void 0 && /* @__PURE__ */ n(
        "text",
        {
          x: H / 2,
          y: Q + 14,
          textAnchor: "middle",
          fill: "#3399ff",
          fontFamily: "monospace",
          fontSize: 9,
          children: e.analogValue
        }
      )
    ] });
  }
}, Z = 64, we = 80, Vt = {
  type: "rgb-led",
  label: "RGB LED",
  category: "output",
  width: Z,
  height: we,
  pins: [
    { id: "R", x: 10, y: 0, side: "top", label: "R" },
    { id: "G", x: Z / 2, y: 0, side: "top", label: "G" },
    { id: "B", x: Z - 10, y: 0, side: "top", label: "B" },
    { id: "GND", x: Z / 2, y: we, side: "bottom", label: "GND" }
  ],
  defaultAttrs: {},
  boardIoKind: "led",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = e == null ? void 0 : e.active;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 6,
          y: 16,
          width: Z - 12,
          height: we - 32,
          rx: 6,
          fill: "#f8f9fa",
          stroke: r ? "#e83e8c" : "#000",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("circle", { cx: 16, cy: we / 2, r: 10, fill: i ? "#ff3333" : "#661111", stroke: "#000", strokeWidth: 0.5 }),
      /* @__PURE__ */ n("circle", { cx: Z / 2, cy: we / 2, r: 10, fill: i ? "#27c93f" : "#0d4d16", stroke: "#000", strokeWidth: 0.5 }),
      /* @__PURE__ */ n("circle", { cx: Z - 16, cy: we / 2, r: 10, fill: i ? "#3399ff" : "#0d2d4d", stroke: "#000", strokeWidth: 0.5 }),
      /* @__PURE__ */ n("text", { x: 10, y: 12, textAnchor: "middle", fill: "#ff3333", fontFamily: "monospace", fontSize: 7, children: "R" }),
      /* @__PURE__ */ n("text", { x: Z / 2, y: 12, textAnchor: "middle", fill: "#27c93f", fontFamily: "monospace", fontSize: 7, children: "G" }),
      /* @__PURE__ */ n("text", { x: Z - 10, y: 12, textAnchor: "middle", fill: "#3399ff", fontFamily: "monospace", fontSize: 7, children: "B" }),
      /* @__PURE__ */ n("text", { x: Z / 2, y: we - 4, textAnchor: "middle", fill: "#888", fontFamily: "monospace", fontSize: 7, children: "GND" })
    ] });
  }
}, de = 80, nt = 110, Kt = {
  type: "seven-segment",
  label: "7-Segment",
  category: "display",
  width: de,
  height: nt,
  pins: [
    { id: "A", x: 0, y: 14, side: "left", label: "A" },
    { id: "B", x: 0, y: 30, side: "left", label: "B" },
    { id: "C", x: 0, y: 46, side: "left", label: "C" },
    { id: "D", x: 0, y: 62, side: "left", label: "D" },
    { id: "E", x: de, y: 14, side: "right", label: "E" },
    { id: "F", x: de, y: 30, side: "right", label: "F" },
    { id: "G", x: de, y: 46, side: "right", label: "G" },
    { id: "DP", x: de, y: 62, side: "right", label: "DP" },
    { id: "COM", x: de / 2, y: nt, side: "bottom", label: "COM" }
  ],
  defaultAttrs: { color: "red" },
  boardIoKind: "spi_device",
  attrFields: [
    { key: "color", label: "Color", type: "select", options: ["red", "green", "blue", "yellow"] }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = t.color || "red", o = { red: "#ff3333", green: "#27c93f", blue: "#3399ff", yellow: "#ffcc00" }[i] || "#ff3333", l = "#2a1a1a", c = 20, a = 14, s = 32, f = 5, d = 28;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 3,
          y: 3,
          width: de - 6,
          height: nt - 6,
          rx: 5,
          fill: "#1a1a1a",
          stroke: r ? "#e83e8c" : "#333",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("rect", { x: c, y: a, width: s, height: f, rx: 1, fill: l }),
      /* @__PURE__ */ n("rect", { x: c + s - f, y: a, width: f, height: d, rx: 1, fill: l }),
      /* @__PURE__ */ n("rect", { x: c + s - f, y: a + d, width: f, height: d, rx: 1, fill: l }),
      /* @__PURE__ */ n("rect", { x: c, y: a + d * 2 - f, width: s, height: f, rx: 1, fill: l }),
      /* @__PURE__ */ n("rect", { x: c, y: a + d, width: f, height: d, rx: 1, fill: l }),
      /* @__PURE__ */ n("rect", { x: c, y: a, width: f, height: d, rx: 1, fill: l }),
      /* @__PURE__ */ n("rect", { x: c, y: a + d - f / 2, width: s, height: f, rx: 1, fill: l }),
      /* @__PURE__ */ n("circle", { cx: c + s + 8, cy: a + d * 2 - f, r: 3, fill: l }),
      /* @__PURE__ */ n(
        "text",
        {
          x: de / 2,
          y: nt - 10,
          textAnchor: "middle",
          fill: o,
          fontFamily: "monospace",
          fontSize: 8,
          children: "7-SEG"
        }
      )
    ] });
  }
}, ee = 220, ot = 110, Yt = {
  type: "lcd1602",
  label: "LCD 16x2",
  category: "display",
  width: ee,
  height: ot,
  pins: [
    { id: "VSS", x: 0, y: 16, side: "left", label: "VSS" },
    { id: "VDD", x: 0, y: 32, side: "left", label: "VDD" },
    { id: "V0", x: 0, y: 48, side: "left", label: "V0" },
    { id: "RS", x: 0, y: 64, side: "left", label: "RS" },
    { id: "RW", x: 0, y: 80, side: "left", label: "RW" },
    { id: "E", x: 0, y: 96, side: "left", label: "E" },
    { id: "D4", x: ee, y: 16, side: "right", label: "D4" },
    { id: "D5", x: ee, y: 32, side: "right", label: "D5" },
    { id: "D6", x: ee, y: 48, side: "right", label: "D6" },
    { id: "D7", x: ee, y: 64, side: "right", label: "D7" },
    { id: "BLA", x: ee, y: 80, side: "right", label: "BLA" },
    { id: "BLK", x: ee, y: 96, side: "right", label: "BLK" }
  ],
  defaultAttrs: { text: "Hello World!" },
  boardIoKind: "i2c_device",
  attrFields: [
    { key: "text", label: "Display Text", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = (e == null ? void 0 : e.displayText) || t.text || "Hello World!", o = i.slice(0, 16).padEnd(16), l = i.slice(16, 32).padEnd(16);
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 0,
          y: 0,
          width: ee,
          height: ot,
          rx: 5,
          fill: "#1c6b3c",
          stroke: r ? "#e83e8c" : "#0d4d1e",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n(
        "rect",
        {
          x: 28,
          y: 14,
          width: ee - 56,
          height: ot - 28,
          rx: 3,
          fill: "#2a5a1a",
          stroke: "#1a3a0a",
          strokeWidth: 1
        }
      ),
      /* @__PURE__ */ n(
        "rect",
        {
          x: 34,
          y: 20,
          width: ee - 68,
          height: ot - 40,
          rx: 2,
          fill: "#5cb85c"
        }
      ),
      /* @__PURE__ */ n(
        "text",
        {
          x: 40,
          y: 44,
          fill: "#1a3a0a",
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: 13,
          letterSpacing: 2,
          children: o
        }
      ),
      /* @__PURE__ */ n(
        "text",
        {
          x: 40,
          y: 68,
          fill: "#1a3a0a",
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: 13,
          letterSpacing: 2,
          children: l
        }
      )
    ] });
  }
}, B = 60, _e = 60, Jt = {
  type: "buzzer",
  label: "Buzzer",
  category: "output",
  width: B,
  height: _e,
  pins: [
    { id: "+", x: B / 2 - 10, y: _e, side: "bottom", label: "+" },
    { id: "-", x: B / 2 + 10, y: _e, side: "bottom", label: "-" }
  ],
  defaultAttrs: {},
  boardIoKind: "pwm_output",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = e == null ? void 0 : e.active;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "circle",
        {
          cx: B / 2,
          cy: B / 2,
          r: 26,
          fill: "#222",
          stroke: r ? "#e83e8c" : "#444",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n(
        "circle",
        {
          cx: B / 2,
          cy: B / 2,
          r: 8,
          fill: i ? "#ffcc00" : "#555"
        }
      ),
      /* @__PURE__ */ n("text", { x: B / 2 - 10, y: _e - 2, textAnchor: "middle", fill: "#ff3333", fontFamily: "monospace", fontSize: 8, children: "+" }),
      /* @__PURE__ */ n("text", { x: B / 2 + 10, y: _e - 2, textAnchor: "middle", fill: "#888", fontFamily: "monospace", fontSize: 8, children: "-" }),
      i && /* @__PURE__ */ p(Lr, { children: [
        /* @__PURE__ */ n("path", { d: `M${B / 2 + 16},${B / 2 - 6} q6,-6 0,-12`, fill: "none", stroke: "#ffcc00", strokeWidth: 1.5, opacity: 0.6 }),
        /* @__PURE__ */ n("path", { d: `M${B / 2 + 22},${B / 2 - 3} q8,-8 0,-16`, fill: "none", stroke: "#ffcc00", strokeWidth: 1.5, opacity: 0.4 })
      ] }),
      (e == null ? void 0 : e.frequency) !== void 0 && /* @__PURE__ */ p(
        "text",
        {
          x: B / 2,
          y: _e + 12,
          textAnchor: "middle",
          fill: "#ffcc00",
          fontFamily: "monospace",
          fontSize: 8,
          children: [
            e.frequency,
            "Hz"
          ]
        }
      )
    ] });
  }
}, Ct = 100, Ke = 68, qt = {
  type: "servo",
  label: "Servo Motor",
  category: "output",
  width: Ct,
  height: Ke,
  pins: [
    { id: "SIG", x: 0, y: 16, side: "left", label: "SIG" },
    { id: "VCC", x: 0, y: 34, side: "left", label: "VCC" },
    { id: "GND", x: 0, y: 52, side: "left", label: "GND" }
  ],
  defaultAttrs: { angle: "90" },
  boardIoKind: "pwm_output",
  attrFields: [
    { key: "angle", label: "Angle (0-180)", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = (e == null ? void 0 : e.angle) ?? parseInt(t.angle || "90", 10), o = (i - 90) * Math.PI / 180, l = Ct - 22, c = Ke / 2, a = 20, s = l + Math.cos(o) * a, f = c - Math.sin(o) * a;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 12,
          y: 6,
          width: Ct - 34,
          height: Ke - 12,
          rx: 6,
          fill: "#2a4a8a",
          stroke: r ? "#e83e8c" : "#1a2a4a",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("rect", { x: 6, y: 20, width: 10, height: 8, rx: 2, fill: "#2a4a8a", stroke: "#1a2a4a", strokeWidth: 0.5 }),
      /* @__PURE__ */ n("rect", { x: 6, y: Ke - 28, width: 10, height: 8, rx: 2, fill: "#2a4a8a", stroke: "#1a2a4a", strokeWidth: 0.5 }),
      /* @__PURE__ */ n("circle", { cx: l, cy: c, r: 12, fill: "#ddd", stroke: "#888", strokeWidth: 1.5 }),
      /* @__PURE__ */ n("line", { x1: l, y1: c, x2: s, y2: f, stroke: "#333", strokeWidth: 4, strokeLinecap: "round" }),
      /* @__PURE__ */ n("circle", { cx: s, cy: f, r: 3, fill: "#333" }),
      /* @__PURE__ */ n("text", { x: 16, y: 22, fill: "#ffcc00", fontFamily: "monospace", fontSize: 7, children: "SIG" }),
      /* @__PURE__ */ n("text", { x: 16, y: 40, fill: "#ff3333", fontFamily: "monospace", fontSize: 7, children: "VCC" }),
      /* @__PURE__ */ n("text", { x: 16, y: 58, fill: "#888", fontFamily: "monospace", fontSize: 7, children: "GND" }),
      /* @__PURE__ */ p(
        "text",
        {
          x: l,
          y: Ke + 12,
          textAnchor: "middle",
          fill: "#569cd6",
          fontFamily: "monospace",
          fontSize: 9,
          children: [
            i,
            "°"
          ]
        }
      )
    ] });
  }
}, Ee = 120, Ye = 36, Xt = {
  type: "neopixel",
  label: "NeoPixel Strip",
  category: "output",
  width: Ee,
  height: Ye,
  pins: [
    { id: "DIN", x: 0, y: Ye / 2, side: "left", label: "DIN" },
    { id: "VCC", x: Ee / 2 - 14, y: 0, side: "top", label: "VCC" },
    { id: "GND", x: Ee / 2 + 14, y: 0, side: "top", label: "GND" },
    { id: "DOUT", x: Ee, y: Ye / 2, side: "right", label: "DOUT" }
  ],
  defaultAttrs: { count: "8" },
  boardIoKind: "spi_device",
  attrFields: [
    { key: "count", label: "LED Count", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = Math.min(parseInt(t.count || "8", 10), 8), o = ["#ff3333", "#27c93f", "#3399ff", "#ffcc00", "#e83e8c", "#00cccc", "#ff6633", "#9966ff"], l = (Ee - 16) / i;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 0,
          y: 3,
          width: Ee,
          height: Ye - 6,
          rx: 4,
          fill: "#1a1a1a",
          stroke: r ? "#e83e8c" : "#333",
          strokeWidth: r ? 2.5 : 1
        }
      ),
      Array.from({ length: i }, (c, a) => /* @__PURE__ */ n(
        "rect",
        {
          x: 8 + a * l,
          y: 8,
          width: l - 3,
          height: Ye - 16,
          rx: 2,
          fill: e != null && e.active ? o[a % o.length] : "#333",
          opacity: e != null && e.active ? 0.9 : 0.4
        },
        a
      ))
    ] });
  }
}, ve = 52, Se = 28, Qt = {
  type: "slide-switch",
  label: "Slide Switch",
  category: "input",
  width: ve,
  height: Se,
  pins: [
    { id: "1", x: 8, y: Se, side: "bottom", label: "1" },
    { id: "COM", x: ve / 2, y: Se, side: "bottom", label: "COM" },
    { id: "2", x: ve - 8, y: Se, side: "bottom", label: "2" }
  ],
  defaultAttrs: { position: "left" },
  boardIoKind: "button",
  attrFields: [
    { key: "position", label: "Position", type: "select", options: ["left", "right"] }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, o = (t.position === "right" ? "right" : "left") === "left" ? 14 : ve - 14;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 2,
          y: 2,
          width: ve - 4,
          height: Se - 4,
          rx: 4,
          fill: "#ddd",
          stroke: r ? "#e83e8c" : "#888",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("rect", { x: 10, y: 8, width: ve - 20, height: 8, rx: 4, fill: "#aaa" }),
      /* @__PURE__ */ n(
        "rect",
        {
          x: o - 6,
          y: 6,
          width: 12,
          height: 12,
          rx: 3,
          fill: "#444",
          stroke: "#222",
          strokeWidth: 0.5
        }
      ),
      /* @__PURE__ */ n("text", { x: 8, y: Se + 10, textAnchor: "middle", fill: "#888", fontFamily: "monospace", fontSize: 6, children: "1" }),
      /* @__PURE__ */ n("text", { x: ve - 8, y: Se + 10, textAnchor: "middle", fill: "#888", fontFamily: "monospace", fontSize: 6, children: "2" })
    ] });
  }
}, Pt = 64, ke = 40, Zt = {
  type: "dip-switch",
  label: "DIP Switch",
  category: "input",
  width: Pt,
  height: ke,
  pins: [
    { id: "1", x: 8, y: ke, side: "bottom", label: "1" },
    { id: "2", x: 24, y: ke, side: "bottom", label: "2" },
    { id: "3", x: 40, y: ke, side: "bottom", label: "3" },
    { id: "4", x: 56, y: ke, side: "bottom", label: "4" },
    { id: "C1", x: 8, y: 0, side: "top", label: "C1" },
    { id: "C2", x: 24, y: 0, side: "top", label: "C2" },
    { id: "C3", x: 40, y: 0, side: "top", label: "C3" },
    { id: "C4", x: 56, y: 0, side: "top", label: "C4" }
  ],
  defaultAttrs: {},
  boardIoKind: "button",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 0,
          y: 3,
          width: Pt,
          height: ke - 6,
          rx: 3,
          fill: "#cc2222",
          stroke: r ? "#e83e8c" : "#881111",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      [8, 24, 40, 56].map((i, o) => /* @__PURE__ */ p("g", { children: [
        /* @__PURE__ */ n("rect", { x: i - 4, y: 8, width: 8, height: 22, rx: 1, fill: "#fff", opacity: 0.2 }),
        /* @__PURE__ */ n("rect", { x: i - 3.5, y: 8, width: 7, height: 11, rx: 1, fill: "#eee" })
      ] }, o)),
      /* @__PURE__ */ n("text", { x: Pt / 2, y: ke + 10, textAnchor: "middle", fill: "#888", fontFamily: "monospace", fontSize: 6, children: "DIP-4" })
    ] });
  }
}, pe = 64, Oe = 64, er = {
  type: "rotary-encoder",
  label: "Rotary Encoder",
  category: "input",
  width: pe,
  height: Oe,
  pins: [
    { id: "CLK", x: 0, y: 14, side: "left", label: "CLK" },
    { id: "DT", x: 0, y: 32, side: "left", label: "DT" },
    { id: "SW", x: 0, y: 50, side: "left", label: "SW" },
    { id: "VCC", x: pe, y: 22, side: "right", label: "VCC" },
    { id: "GND", x: pe, y: 42, side: "right", label: "GND" }
  ],
  defaultAttrs: {},
  boardIoKind: "button",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 3,
          y: 3,
          width: pe - 6,
          height: Oe - 6,
          rx: 6,
          fill: "#1a3a6a",
          stroke: r ? "#e83e8c" : "#0d2040",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("circle", { cx: pe / 2, cy: Oe / 2, r: 20, fill: "#888", stroke: "#555", strokeWidth: 1.5 }),
      /* @__PURE__ */ n("circle", { cx: pe / 2, cy: Oe / 2, r: 14, fill: "#aaa", stroke: "#888", strokeWidth: 1 }),
      /* @__PURE__ */ n(
        "line",
        {
          x1: pe / 2,
          y1: Oe / 2 - 14,
          x2: pe / 2,
          y2: Oe / 2 - 6,
          stroke: "#333",
          strokeWidth: 2.5,
          strokeLinecap: "round"
        }
      ),
      /* @__PURE__ */ n("text", { x: 10, y: 18, fill: "#569cd6", fontFamily: "monospace", fontSize: 6, children: "CLK" }),
      /* @__PURE__ */ n("text", { x: 10, y: 36, fill: "#569cd6", fontFamily: "monospace", fontSize: 6, children: "DT" }),
      /* @__PURE__ */ n("text", { x: 10, y: 54, fill: "#569cd6", fontFamily: "monospace", fontSize: 6, children: "SW" })
    ] });
  }
}, Te = 88, tr = 108, rr = {
  type: "keypad",
  label: "4x4 Keypad",
  category: "input",
  width: Te,
  height: tr,
  pins: [
    { id: "R1", x: 0, y: 16, side: "left", label: "R1" },
    { id: "R2", x: 0, y: 38, side: "left", label: "R2" },
    { id: "R3", x: 0, y: 60, side: "left", label: "R3" },
    { id: "R4", x: 0, y: 82, side: "left", label: "R4" },
    { id: "C1", x: Te, y: 16, side: "right", label: "C1" },
    { id: "C2", x: Te, y: 38, side: "right", label: "C2" },
    { id: "C3", x: Te, y: 60, side: "right", label: "C3" },
    { id: "C4", x: Te, y: 82, side: "right", label: "C4" }
  ],
  defaultAttrs: {},
  boardIoKind: "button",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = [
      ["1", "2", "3", "A"],
      ["4", "5", "6", "B"],
      ["7", "8", "9", "C"],
      ["*", "0", "#", "D"]
    ], o = 16, l = 16, c = 10, a = 10, s = 3;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 3,
          y: 3,
          width: Te - 6,
          height: tr - 6,
          rx: 6,
          fill: "#f0f0f0",
          stroke: r ? "#e83e8c" : "#888",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      i.map(
        (f, d) => f.map((u, y) => /* @__PURE__ */ p("g", { children: [
          /* @__PURE__ */ n(
            "rect",
            {
              x: c + y * (o + s),
              y: a + d * (l + s + 4),
              width: o,
              height: l,
              rx: 3,
              fill: "#ddd",
              stroke: "#999",
              strokeWidth: 0.5
            }
          ),
          /* @__PURE__ */ n(
            "text",
            {
              x: c + y * (o + s) + o / 2,
              y: a + d * (l + s + 4) + l / 2 + 4,
              textAnchor: "middle",
              fill: "#333",
              fontFamily: "monospace",
              fontSize: 9,
              children: u
            }
          )
        ] }, `${d}-${y}`))
      )
    ] });
  }
}, fe = 44, Je = 60, ir = {
  type: "dht22",
  label: "DHT22 Sensor",
  category: "sensor",
  width: fe,
  height: Je,
  pins: [
    { id: "VCC", x: 6, y: Je, side: "bottom", label: "VCC" },
    { id: "DATA", x: fe / 2, y: Je, side: "bottom", label: "DATA" },
    { id: "GND", x: fe - 6, y: Je, side: "bottom", label: "GND" }
  ],
  defaultAttrs: { temperature: "25", humidity: "50" },
  boardIoKind: "button",
  attrFields: [
    { key: "temperature", label: "Temp (°C)", type: "text" },
    { key: "humidity", label: "Humidity (%)", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = t.temperature || "25", o = t.humidity || "50";
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 2,
          y: 2,
          width: fe - 4,
          height: Je - 8,
          rx: 4,
          fill: "#f8f8f8",
          stroke: r ? "#e83e8c" : "#ccc",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      [14, 20, 26, 32].map((l) => /* @__PURE__ */ n("line", { x1: 10, y1: l, x2: fe - 10, y2: l, stroke: "#ddd", strokeWidth: 0.5 }, l)),
      /* @__PURE__ */ n(
        "text",
        {
          x: fe / 2,
          y: 12,
          textAnchor: "middle",
          fill: "#333",
          fontFamily: "monospace",
          fontSize: 7,
          fontWeight: 700,
          children: "DHT22"
        }
      ),
      /* @__PURE__ */ p(
        "text",
        {
          x: fe / 2,
          y: 38,
          textAnchor: "middle",
          fill: "#666",
          fontFamily: "monospace",
          fontSize: 8,
          children: [
            i,
            "°C"
          ]
        }
      ),
      /* @__PURE__ */ p(
        "text",
        {
          x: fe / 2,
          y: 48,
          textAnchor: "middle",
          fill: "#569cd6",
          fontFamily: "monospace",
          fontSize: 7,
          children: [
            o,
            "%"
          ]
        }
      )
    ] });
  }
}, Fe = 56, ue = 56, nr = {
  type: "pir-sensor",
  label: "PIR Sensor",
  category: "sensor",
  width: Fe,
  height: ue,
  pins: [
    { id: "VCC", x: 0, y: 16, side: "left", label: "VCC" },
    { id: "OUT", x: 0, y: ue / 2, side: "left", label: "OUT" },
    { id: "GND", x: 0, y: ue - 16, side: "left", label: "GND" }
  ],
  defaultAttrs: {},
  boardIoKind: "button",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = e == null ? void 0 : e.active;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "circle",
        {
          cx: Fe / 2,
          cy: ue / 2,
          r: 24,
          fill: "#1a6a3a",
          stroke: r ? "#e83e8c" : "#0d4d1e",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n(
        "circle",
        {
          cx: Fe / 2,
          cy: ue / 2,
          r: 16,
          fill: i ? "rgba(255,204,0,0.3)" : "#f8f8f8",
          stroke: "#ccc",
          strokeWidth: 1
        }
      ),
      /* @__PURE__ */ n("circle", { cx: Fe / 2, cy: ue / 2, r: 6, fill: "#ddd", stroke: "#bbb", strokeWidth: 0.5 }),
      i && /* @__PURE__ */ n(
        "circle",
        {
          cx: Fe / 2,
          cy: ue / 2,
          r: 22,
          fill: "none",
          stroke: "#ffcc00",
          strokeWidth: 1.5,
          strokeDasharray: "4,4",
          opacity: 0.6
        }
      ),
      /* @__PURE__ */ n("text", { x: Fe / 2, y: ue + 10, textAnchor: "middle", fill: "#888", fontFamily: "monospace", fontSize: 6, children: "PIR" })
    ] });
  }
}, Re = 88, ze = 52, or = {
  type: "ultrasonic",
  label: "HC-SR04",
  category: "sensor",
  width: Re,
  height: ze,
  pins: [
    { id: "VCC", x: 14, y: ze, side: "bottom", label: "VCC" },
    { id: "TRIG", x: 32, y: ze, side: "bottom", label: "TRIG" },
    { id: "ECHO", x: 56, y: ze, side: "bottom", label: "ECHO" },
    { id: "GND", x: 74, y: ze, side: "bottom", label: "GND" }
  ],
  defaultAttrs: { distance: "100" },
  boardIoKind: "button",
  attrFields: [
    { key: "distance", label: "Distance (cm)", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = t.distance || "100";
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 3,
          y: 3,
          width: Re - 6,
          height: ze - 8,
          rx: 4,
          fill: "#1a6aaa",
          stroke: r ? "#e83e8c" : "#0d4060",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("circle", { cx: 24, cy: 22, r: 14, fill: "#ccc", stroke: "#999", strokeWidth: 1 }),
      /* @__PURE__ */ n("circle", { cx: 24, cy: 22, r: 8, fill: "#ddd" }),
      /* @__PURE__ */ n("circle", { cx: Re - 24, cy: 22, r: 14, fill: "#ccc", stroke: "#999", strokeWidth: 1 }),
      /* @__PURE__ */ n("circle", { cx: Re - 24, cy: 22, r: 8, fill: "#ddd" }),
      /* @__PURE__ */ n("rect", { x: Re / 2 - 5, y: 8, width: 10, height: 5, rx: 1, fill: "#888" }),
      /* @__PURE__ */ p(
        "text",
        {
          x: Re / 2,
          y: 38,
          textAnchor: "middle",
          fill: "#fff",
          fontFamily: "monospace",
          fontSize: 8,
          children: [
            i,
            "cm"
          ]
        }
      )
    ] });
  }
}, Y = 40, te = 40, lr = {
  type: "ldr",
  label: "Photoresistor",
  category: "sensor",
  width: Y,
  height: te,
  pins: [
    { id: "1", x: 0, y: te / 2, side: "left", label: "1" },
    { id: "2", x: Y, y: te / 2, side: "right", label: "2" }
  ],
  defaultAttrs: { value: "10K" },
  boardIoKind: "adc_input",
  attrFields: [
    { key: "value", label: "Resistance", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n("line", { x1: 0, y1: te / 2, x2: 8, y2: te / 2, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: Y - 8, y1: te / 2, x2: Y, y2: te / 2, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n(
        "circle",
        {
          cx: Y / 2,
          cy: te / 2,
          r: 14,
          fill: "#8B4513",
          stroke: r ? "#e83e8c" : "#5C3317",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n(
        "path",
        {
          d: `M${Y / 2 - 6},${te / 2 - 4} l4,8 l4,-8 l4,8`,
          fill: "none",
          stroke: "#daa520",
          strokeWidth: 1.2
        }
      ),
      /* @__PURE__ */ n("line", { x1: Y / 2 - 10, y1: 6, x2: Y / 2 - 4, y2: 10, stroke: "#ffcc00", strokeWidth: 1 }),
      /* @__PURE__ */ n("line", { x1: Y / 2 + 2, y1: 3, x2: Y / 2 + 5, y2: 9, stroke: "#ffcc00", strokeWidth: 1 }),
      (e == null ? void 0 : e.analogValue) !== void 0 && /* @__PURE__ */ n(
        "text",
        {
          x: Y / 2,
          y: te + 12,
          textAnchor: "middle",
          fill: "#3399ff",
          fontFamily: "monospace",
          fontSize: 8,
          children: e.analogValue
        }
      )
    ] });
  }
}, Le = 140, J = 84, cr = {
  type: "oled-ssd1306",
  label: "OLED 128x64",
  category: "display",
  width: Le,
  height: J,
  pins: [
    { id: "GND", x: 22, y: J, side: "bottom", label: "GND" },
    { id: "VCC", x: 50, y: J, side: "bottom", label: "VCC" },
    { id: "SCL", x: 78, y: J, side: "bottom", label: "SCL" },
    { id: "SDA", x: 106, y: J, side: "bottom", label: "SDA" }
  ],
  defaultAttrs: {},
  boardIoKind: "i2c_device",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = e == null ? void 0 : e.displayText;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 0,
          y: 0,
          width: Le,
          height: J,
          rx: 4,
          fill: "#1a2a4a",
          stroke: r ? "#e83e8c" : "#0d1a30",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n(
        "rect",
        {
          x: 8,
          y: 6,
          width: Le - 16,
          height: J - 24,
          rx: 2,
          fill: "#000",
          stroke: "#222",
          strokeWidth: 0.5
        }
      ),
      i ? /* @__PURE__ */ n(
        "text",
        {
          x: Le / 2,
          y: 28,
          textAnchor: "middle",
          fill: "#00aaff",
          fontFamily: "monospace",
          fontSize: 10,
          children: i.slice(0, 20)
        }
      ) : /* @__PURE__ */ p(Lr, { children: [
        /* @__PURE__ */ n(
          "text",
          {
            x: Le / 2,
            y: 28,
            textAnchor: "middle",
            fill: "#00aaff",
            fontFamily: "monospace",
            fontSize: 10,
            children: "128x64"
          }
        ),
        /* @__PURE__ */ n(
          "text",
          {
            x: Le / 2,
            y: 44,
            textAnchor: "middle",
            fill: "#00aaff",
            fontFamily: "monospace",
            fontSize: 8,
            children: "OLED"
          }
        )
      ] }),
      /* @__PURE__ */ n("text", { x: 22, y: J - 4, textAnchor: "middle", fill: "#888", fontFamily: "monospace", fontSize: 6, children: "GND" }),
      /* @__PURE__ */ n("text", { x: 50, y: J - 4, textAnchor: "middle", fill: "#ff3333", fontFamily: "monospace", fontSize: 6, children: "VCC" }),
      /* @__PURE__ */ n("text", { x: 78, y: J - 4, textAnchor: "middle", fill: "#569cd6", fontFamily: "monospace", fontSize: 6, children: "SCL" }),
      /* @__PURE__ */ n("text", { x: 106, y: J - 4, textAnchor: "middle", fill: "#569cd6", fontFamily: "monospace", fontSize: 6, children: "SDA" })
    ] });
  }
}, lt = 88, At = 88, It = 8, $t = 8, ar = {
  type: "led-matrix",
  label: "8x8 LED Matrix",
  category: "display",
  width: lt,
  height: At,
  pins: [
    ...Array.from({ length: It }, (t, e) => ({
      id: `R${e + 1}`,
      x: 0,
      y: 10 + e * 9,
      side: "left",
      label: `R${e + 1}`
    })),
    ...Array.from({ length: $t }, (t, e) => ({
      id: `C${e + 1}`,
      x: lt,
      y: 10 + e * 9,
      side: "right",
      label: `C${e + 1}`
    }))
  ],
  defaultAttrs: { color: "red" },
  boardIoKind: "spi_device",
  attrFields: [
    { key: "color", label: "LED Color", type: "select", options: ["red", "green", "blue"] }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = t.color || "red", o = { red: "#661111", green: "#0d4d16", blue: "#0d2d4d" }[i] || "#661111", l = 3.5, c = 14, a = 8, s = (lt - 2 * c) / ($t - 1), f = (At - 2 * a) / (It - 1);
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 3,
          y: 3,
          width: lt - 6,
          height: At - 6,
          rx: 4,
          fill: "#1a1a1a",
          stroke: r ? "#e83e8c" : "#333",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      Array.from(
        { length: It },
        (d, u) => Array.from({ length: $t }, (y, x) => /* @__PURE__ */ n(
          "circle",
          {
            cx: c + x * s,
            cy: a + u * f,
            r: l,
            fill: o,
            opacity: 0.6
          },
          `${u}-${x}`
        ))
      )
    ] });
  }
}, re = 44, U = 32, sr = {
  type: "capacitor",
  label: "Capacitor",
  category: "passive",
  width: re,
  height: U,
  pins: [
    { id: "1", x: 0, y: U / 2, side: "left", label: "+" },
    { id: "2", x: re, y: U / 2, side: "right", label: "-" }
  ],
  defaultAttrs: { value: "100nF" },
  attrFields: [
    { key: "value", label: "Capacitance", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = t.value || "100nF", o = r ? "#e83e8c" : "#000", l = r ? 2.5 : 2;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n("line", { x1: 0, y1: U / 2, x2: re / 2 - 4, y2: U / 2, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: re / 2 + 4, y1: U / 2, x2: re, y2: U / 2, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n(
        "line",
        {
          x1: re / 2 - 4,
          y1: U / 2 - 10,
          x2: re / 2 - 4,
          y2: U / 2 + 10,
          stroke: o,
          strokeWidth: l
        }
      ),
      /* @__PURE__ */ n(
        "line",
        {
          x1: re / 2 + 4,
          y1: U / 2 - 10,
          x2: re / 2 + 4,
          y2: U / 2 + 10,
          stroke: o,
          strokeWidth: l
        }
      ),
      /* @__PURE__ */ n(
        "text",
        {
          x: re / 2,
          y: U / 2 - 12,
          textAnchor: "middle",
          fill: "#444",
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: 8,
          children: i
        }
      )
    ] });
  }
}, ct = 64, at = 32, dr = {
  type: "diode",
  label: "Diode",
  category: "passive",
  width: ct,
  height: at,
  pins: [
    { id: "A", x: 0, y: at / 2, side: "left", label: "A" },
    { id: "C", x: ct, y: at / 2, side: "right", label: "C" }
  ],
  defaultAttrs: { type: "1N4148" },
  attrFields: [
    { key: "type", label: "Part Number", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = r ? "#e83e8c" : "#000", o = r ? 2.5 : 2, l = ct / 2, c = at / 2;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n("line", { x1: 0, y1: c, x2: l - 10, y2: c, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: l + 10, y1: c, x2: ct, y2: c, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n(
        "polygon",
        {
          points: `${l - 10},${c - 9} ${l - 10},${c + 9} ${l + 8},${c}`,
          fill: "none",
          stroke: i,
          strokeWidth: o,
          strokeLinejoin: "round"
        }
      ),
      /* @__PURE__ */ n(
        "line",
        {
          x1: l + 8,
          y1: c - 9,
          x2: l + 8,
          y2: c + 9,
          stroke: i,
          strokeWidth: o
        }
      ),
      /* @__PURE__ */ n("rect", { x: l + 8, y: c - 9, width: 4, height: 18, fill: "#444", opacity: 0.3 }),
      /* @__PURE__ */ n(
        "text",
        {
          x: l,
          y: c - 11,
          textAnchor: "middle",
          fill: "#444",
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: 7,
          children: t.type || "1N4148"
        }
      )
    ] });
  }
}, he = 56, Ce = 64, pr = {
  type: "transistor",
  label: "Transistor",
  category: "passive",
  width: he,
  height: Ce,
  pins: [
    { id: "B", x: 0, y: Ce / 2, side: "left", label: "B" },
    { id: "C", x: he, y: 10, side: "right", label: "C" },
    { id: "E", x: he, y: Ce - 10, side: "right", label: "E" }
  ],
  defaultAttrs: { type: "NPN", part: "2N2222" },
  attrFields: [
    { key: "type", label: "Type", type: "select", options: ["NPN", "PNP"] },
    { key: "part", label: "Part Number", type: "text" }
  ],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected, i = t.type !== "PNP", o = he / 2, l = Ce / 2;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "circle",
        {
          cx: o,
          cy: l,
          r: 22,
          fill: "#f8f9fa",
          stroke: r ? "#e83e8c" : "#000",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("line", { x1: 0, y1: l, x2: o - 8, y2: l, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: o - 8, y1: l - 10, x2: o - 8, y2: l + 10, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: o - 8, y1: l - 6, x2: o + 10, y2: l - 16, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: o + 10, y1: l - 16, x2: he, y2: 10, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: o - 8, y1: l + 6, x2: o + 10, y2: l + 16, stroke: "#444", strokeWidth: 2 }),
      /* @__PURE__ */ n("line", { x1: o + 10, y1: l + 16, x2: he, y2: Ce - 10, stroke: "#444", strokeWidth: 2 }),
      i ? /* @__PURE__ */ n(
        "polygon",
        {
          points: `${o + 10},${l + 16} ${o + 4},${l + 10} ${o + 6},${l + 18}`,
          fill: "#444"
        }
      ) : /* @__PURE__ */ n(
        "polygon",
        {
          points: `${o - 6},${l - 8} ${o + 2},${l - 12} ${o},${l - 2}`,
          fill: "#444"
        }
      ),
      /* @__PURE__ */ n("text", { x: 6, y: l - 6, fill: "#888", fontFamily: "monospace", fontSize: 7, children: "B" }),
      /* @__PURE__ */ n("text", { x: he - 6, y: 16, textAnchor: "end", fill: "#888", fontFamily: "monospace", fontSize: 7, children: "C" }),
      /* @__PURE__ */ n("text", { x: he - 6, y: Ce - 6, textAnchor: "end", fill: "#888", fontFamily: "monospace", fontSize: 7, children: "E" }),
      /* @__PURE__ */ n(
        "text",
        {
          x: o,
          y: Ce + 10,
          textAnchor: "middle",
          fill: "#444",
          fontFamily: "monospace",
          fontSize: 7,
          children: t.part || "2N2222"
        }
      )
    ] });
  }
}, Pe = 80, st = 140, dt = 16, fr = ["QB", "QC", "QD", "QE", "QF", "QG", "QH", "GND"], ur = ["VCC", "QA", "SER", "OE", "RCLK", "SRCLK", "SRCLR", "QH'"], hr = {
  type: "74hc595",
  label: "74HC595",
  category: "ic",
  width: Pe,
  height: st,
  pins: [
    ...fr.map((t, e) => ({
      id: `L${e + 1}`,
      x: 0,
      y: 14 + e * dt,
      side: "left",
      label: t
    })),
    ...ur.map((t, e) => ({
      id: `R${e + 1}`,
      x: Pe,
      y: 14 + e * dt,
      side: "right",
      label: t
    }))
  ],
  defaultAttrs: {},
  boardIoKind: "spi_device",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 8,
          y: 3,
          width: Pe - 16,
          height: st - 6,
          rx: 3,
          fill: "#333",
          stroke: r ? "#e83e8c" : "#111",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("circle", { cx: Pe / 2, cy: 8, r: 5, fill: "none", stroke: "#555", strokeWidth: 1 }),
      /* @__PURE__ */ n("circle", { cx: 16, cy: 16, r: 2, fill: "#888" }),
      fr.map((i, o) => /* @__PURE__ */ n(
        "text",
        {
          x: 14,
          y: 18 + o * dt,
          fill: "#888",
          fontFamily: "monospace",
          fontSize: 6,
          children: i
        },
        `l${o}`
      )),
      ur.map((i, o) => /* @__PURE__ */ n(
        "text",
        {
          x: Pe - 14,
          y: 18 + o * dt,
          textAnchor: "end",
          fill: "#888",
          fontFamily: "monospace",
          fontSize: 6,
          children: i
        },
        `r${o}`
      )),
      /* @__PURE__ */ n(
        "text",
        {
          x: Pe / 2,
          y: st / 2 + 2,
          textAnchor: "middle",
          fill: "#aaa",
          fontFamily: "monospace",
          fontSize: 7,
          transform: `rotate(-90, ${Pe / 2}, ${st / 2})`,
          children: "74HC595"
        }
      )
    ] });
  }
}, Ae = 80, pt = 140, ft = 16, yr = ["EN1,2", "IN1", "OUT1", "GND", "GND", "OUT2", "IN2", "VS"], mr = ["VSS", "IN4", "OUT4", "GND", "GND", "OUT3", "IN3", "EN3,4"], gr = {
  type: "l293d",
  label: "L293D",
  category: "ic",
  width: Ae,
  height: pt,
  pins: [
    ...yr.map((t, e) => ({
      id: `L${e + 1}`,
      x: 0,
      y: 14 + e * ft,
      side: "left",
      label: t
    })),
    ...mr.map((t, e) => ({
      id: `R${e + 1}`,
      x: Ae,
      y: 14 + e * ft,
      side: "right",
      label: t
    }))
  ],
  defaultAttrs: {},
  boardIoKind: "pwm_output",
  attrFields: [],
  render: (t, e) => {
    const r = e == null ? void 0 : e.selected;
    return /* @__PURE__ */ p("g", { children: [
      /* @__PURE__ */ n(
        "rect",
        {
          x: 8,
          y: 3,
          width: Ae - 16,
          height: pt - 6,
          rx: 3,
          fill: "#333",
          stroke: r ? "#e83e8c" : "#111",
          strokeWidth: r ? 2.5 : 1.5
        }
      ),
      /* @__PURE__ */ n("circle", { cx: Ae / 2, cy: 8, r: 5, fill: "none", stroke: "#555", strokeWidth: 1 }),
      /* @__PURE__ */ n("circle", { cx: 16, cy: 16, r: 2, fill: "#888" }),
      yr.map((i, o) => /* @__PURE__ */ n(
        "text",
        {
          x: 14,
          y: 18 + o * ft,
          fill: "#888",
          fontFamily: "monospace",
          fontSize: 5.5,
          children: i
        },
        `l${o}`
      )),
      mr.map((i, o) => /* @__PURE__ */ n(
        "text",
        {
          x: Ae - 14,
          y: 18 + o * ft,
          textAnchor: "end",
          fill: "#888",
          fontFamily: "monospace",
          fontSize: 5.5,
          children: i
        },
        `r${o}`
      )),
      /* @__PURE__ */ n(
        "text",
        {
          x: Ae / 2,
          y: pt / 2 + 2,
          textAnchor: "middle",
          fill: "#aaa",
          fontFamily: "monospace",
          fontSize: 7,
          transform: `rotate(-90, ${Ae / 2}, ${pt / 2})`,
          children: "L293D"
        }
      )
    ] });
  }
}, Ie = 220, Rt = 300, qe = 18, Xe = 40;
function ai() {
  const t = [];
  for (let e = 0; e <= 13; e++)
    t.push({
      id: `D${e}`,
      x: Ie,
      y: Xe + e * qe,
      side: "right",
      label: `D${e}`
    });
  for (let e = 0; e <= 5; e++)
    t.push({
      id: `A${e}`,
      x: 0,
      y: Xe + e * qe,
      side: "left",
      label: `A${e}`
    });
  return t.push({ id: "5V", x: 0, y: Xe + 7 * qe, side: "left", label: "5V" }), t.push({ id: "3V3", x: 0, y: Xe + 8 * qe, side: "left", label: "3.3V" }), t.push({ id: "GND", x: Ie / 2, y: Rt, side: "bottom", label: "GND" }), t.push({ id: "VIN", x: 0, y: Xe + 9 * qe, side: "left", label: "VIN" }), t;
}
const Mt = ai(), xr = {
  type: "arduino-uno",
  label: "Arduino Uno",
  category: "mcu",
  width: Ie,
  height: Rt,
  pins: Mt,
  defaultAttrs: {},
  render: (t, e) => /* @__PURE__ */ p("g", { children: [
    /* @__PURE__ */ n(
      "rect",
      {
        width: Ie,
        height: Rt,
        rx: 8,
        fill: "#00687c",
        stroke: e != null && e.selected ? "#e83e8c" : "#005c6e",
        strokeWidth: e != null && e.selected ? 3 : 2
      }
    ),
    /* @__PURE__ */ n(
      "text",
      {
        x: Ie / 2,
        y: 20,
        textAnchor: "middle",
        fill: "#fff",
        fontFamily: "'Outfit', sans-serif",
        fontSize: 13,
        fontWeight: 700,
        children: "Arduino Uno"
      }
    ),
    /* @__PURE__ */ n(
      "text",
      {
        x: Ie / 2,
        y: 32,
        textAnchor: "middle",
        fill: "rgba(255,255,255,0.5)",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 8,
        children: "ATmega328P"
      }
    ),
    Mt.filter((r) => r.side === "right").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: Ie - 8,
        y: r.y + 4,
        textAnchor: "end",
        fill: "#aad",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 7,
        children: r.label
      },
      r.id
    )),
    Mt.filter((r) => r.side === "left").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: 8,
        y: r.y + 4,
        fill: "#aad",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 7,
        children: r.label
      },
      r.id
    ))
  ] })
}, oe = 180, xt = 360, br = 16, wr = 40;
function si() {
  const t = [], e = [0, 1, 2, 3, 4, 5, 12, 13, 14, 15, 16, 17, 18, 19];
  for (let i = 0; i < e.length; i++)
    t.push({
      id: `GPIO${e[i]}`,
      x: 0,
      y: wr + i * br,
      side: "left",
      label: `GP${e[i]}`
    });
  const r = [21, 22, 23, 25, 26, 27, 32, 33, 34, 35, 36, 39];
  for (let i = 0; i < r.length; i++)
    t.push({
      id: `GPIO${r[i]}`,
      x: oe,
      y: wr + i * br,
      side: "right",
      label: `GP${r[i]}`
    });
  return t.push({ id: "3V3", x: oe / 2 - 20, y: xt, side: "bottom", label: "3.3V" }), t.push({ id: "GND", x: oe / 2 + 20, y: xt, side: "bottom", label: "GND" }), t;
}
const Wt = si(), vr = {
  type: "esp32",
  label: "ESP32",
  category: "mcu",
  width: oe,
  height: xt,
  pins: Wt,
  defaultAttrs: {},
  render: (t, e) => /* @__PURE__ */ p("g", { children: [
    /* @__PURE__ */ n(
      "rect",
      {
        width: oe,
        height: xt,
        rx: 6,
        fill: "#1e1e28",
        stroke: e != null && e.selected ? "#e83e8c" : "#333",
        strokeWidth: e != null && e.selected ? 3 : 2
      }
    ),
    /* @__PURE__ */ n("rect", { x: oe / 2 - 15, y: 0, width: 30, height: 12, rx: 2, fill: "#444" }),
    /* @__PURE__ */ n(
      "text",
      {
        x: oe / 2,
        y: 24,
        textAnchor: "middle",
        fill: "#fff",
        fontFamily: "'Outfit', sans-serif",
        fontSize: 12,
        fontWeight: 700,
        children: "ESP32"
      }
    ),
    /* @__PURE__ */ n(
      "text",
      {
        x: oe / 2,
        y: 34,
        textAnchor: "middle",
        fill: "rgba(255,255,255,0.5)",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 7,
        children: "ESP32-WROOM-32"
      }
    ),
    Wt.filter((r) => r.side === "left").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: 8,
        y: r.y + 3,
        fill: "#888",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 6,
        children: r.label
      },
      r.id
    )),
    Wt.filter((r) => r.side === "right").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: oe - 8,
        y: r.y + 3,
        textAnchor: "end",
        fill: "#888",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 6,
        children: r.label
      },
      r.id
    ))
  ] })
}, le = 160, bt = 340, Sr = 16, kr = 35;
function di() {
  const t = [];
  for (let e = 0; e <= 15; e++)
    t.push({
      id: `GP${e}`,
      x: 0,
      y: kr + e * Sr,
      side: "left",
      label: `GP${e}`
    });
  for (let e = 16; e <= 28; e++)
    t.push({
      id: `GP${e}`,
      x: le,
      y: kr + (e - 16) * Sr,
      side: "right",
      label: `GP${e}`
    });
  return t.push({ id: "3V3", x: le / 2 - 15, y: bt, side: "bottom", label: "3V3" }), t.push({ id: "GND", x: le / 2 + 15, y: bt, side: "bottom", label: "GND" }), t;
}
const Nt = di(), Cr = {
  type: "rpi-pico",
  label: "RPi Pico",
  category: "mcu",
  width: le,
  height: bt,
  pins: Nt,
  defaultAttrs: {},
  render: (t, e) => /* @__PURE__ */ p("g", { children: [
    /* @__PURE__ */ n(
      "rect",
      {
        width: le,
        height: bt,
        rx: 6,
        fill: "#2d8040",
        stroke: e != null && e.selected ? "#e83e8c" : "#1a5c2a",
        strokeWidth: e != null && e.selected ? 3 : 2
      }
    ),
    /* @__PURE__ */ n("rect", { x: le / 2 - 10, y: -4, width: 20, height: 8, rx: 2, fill: "#888" }),
    /* @__PURE__ */ n(
      "text",
      {
        x: le / 2,
        y: 20,
        textAnchor: "middle",
        fill: "#fff",
        fontFamily: "'Outfit', sans-serif",
        fontSize: 11,
        fontWeight: 700,
        children: "RPi Pico"
      }
    ),
    /* @__PURE__ */ n(
      "text",
      {
        x: le / 2,
        y: 30,
        textAnchor: "middle",
        fill: "rgba(255,255,255,0.5)",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 7,
        children: "RP2040"
      }
    ),
    Nt.filter((r) => r.side === "left").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: 6,
        y: r.y + 3,
        fill: "#cfc",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 6,
        children: r.label
      },
      r.id
    )),
    Nt.filter((r) => r.side === "right").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: le - 6,
        y: r.y + 3,
        textAnchor: "end",
        fill: "#cfc",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 6,
        children: r.label
      },
      r.id
    ))
  ] })
}, ye = 180, wt = 320, Pr = 16, Ar = 40;
function pi() {
  const t = [];
  for (let e = 0; e <= 15; e++)
    t.push({
      id: `P0.${String(e).padStart(2, "0")}`,
      x: 0,
      y: Ar + e * Pr,
      side: "left",
      label: `P0.${String(e).padStart(2, "0")}`
    });
  for (let e = 16; e <= 31; e++)
    t.push({
      id: `P0.${e}`,
      x: ye,
      y: Ar + (e - 16) * Pr,
      side: "right",
      label: `P0.${e}`
    });
  return t.push({ id: "VDD", x: ye / 2 - 15, y: wt, side: "bottom", label: "VDD" }), t.push({ id: "GND", x: ye / 2 + 15, y: wt, side: "bottom", label: "GND" }), t;
}
const Dt = pi(), Ir = {
  type: "nrf52840-dk",
  label: "nRF52840 DK",
  category: "mcu",
  width: ye,
  height: wt,
  pins: Dt,
  defaultAttrs: {},
  render: (t, e) => /* @__PURE__ */ p("g", { children: [
    /* @__PURE__ */ n(
      "rect",
      {
        width: ye,
        height: wt,
        rx: 6,
        fill: "#1e2848",
        stroke: e != null && e.selected ? "#e83e8c" : "#2a3a6e",
        strokeWidth: e != null && e.selected ? 3 : 2
      }
    ),
    /* @__PURE__ */ n(
      "text",
      {
        x: ye / 2,
        y: 22,
        textAnchor: "middle",
        fill: "#fff",
        fontFamily: "'Outfit', sans-serif",
        fontSize: 11,
        fontWeight: 700,
        children: "nRF52840 DK"
      }
    ),
    /* @__PURE__ */ n(
      "text",
      {
        x: ye / 2,
        y: 34,
        textAnchor: "middle",
        fill: "rgba(255,255,255,0.5)",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 7,
        children: "Nordic Semi"
      }
    ),
    Dt.filter((r) => r.side === "left").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: 6,
        y: r.y + 3,
        fill: "#88a",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 5.5,
        children: r.label
      },
      r.id
    )),
    Dt.filter((r) => r.side === "right").map((r) => /* @__PURE__ */ n(
      "text",
      {
        x: ye - 6,
        y: r.y + 3,
        textAnchor: "end",
        fill: "#88a",
        fontFamily: "'JetBrains Mono', monospace",
        fontSize: 5.5,
        children: r.label
      },
      r.id
    ))
  ] })
}, me = /* @__PURE__ */ new Map([
  // MCUs
  [Bt.type, Bt],
  [xr.type, xr],
  [vr.type, vr],
  [Cr.type, Cr],
  [Ir.type, Ir],
  // Output
  [Ht.type, Ht],
  [Vt.type, Vt],
  [Jt.type, Jt],
  [qt.type, qt],
  [Xt.type, Xt],
  // Input
  [Gt.type, Gt],
  [Ut.type, Ut],
  [Qt.type, Qt],
  [Zt.type, Zt],
  [er.type, er],
  [rr.type, rr],
  // Sensors
  [ir.type, ir],
  [nr.type, nr],
  [or.type, or],
  [lr.type, lr],
  // Displays
  [Kt.type, Kt],
  [Yt.type, Yt],
  [cr.type, cr],
  [ar.type, ar],
  // Passives
  [jt.type, jt],
  [sr.type, sr],
  [dr.type, dr],
  [pr.type, pr],
  // ICs
  [hr.type, hr],
  [gr.type, gr]
]);
function fi() {
  const t = {};
  for (const e of me.values()) {
    if (e.category === "mcu") continue;
    const r = e.category;
    t[r] || (t[r] = []), t[r].push(e);
  }
  return t;
}
const ut = 20;
function $r(t) {
  switch (t) {
    case "left":
      return { x: -1, y: 0 };
    case "right":
      return { x: 1, y: 0 };
    case "top":
      return { x: 0, y: -1 };
    case "bottom":
      return { x: 0, y: 1 };
  }
}
function Mr(t, e, r, i) {
  const o = $r(e), l = $r(i), c = { x: t.x + o.x * ut, y: t.y + o.y * ut }, a = { x: r.x + l.x * ut, y: r.y + l.y * ut }, s = o.x !== 0, f = l.x !== 0;
  if (s && f) {
    const d = (c.x + a.x) / 2;
    return [
      c,
      { x: d, y: c.y },
      { x: d, y: a.y },
      a
    ];
  }
  if (!s && !f) {
    const d = (c.y + a.y) / 2;
    return [
      c,
      { x: c.x, y: d },
      { x: a.x, y: d },
      a
    ];
  }
  return s && !f ? [
    c,
    { x: a.x, y: c.y },
    a
  ] : [
    c,
    { x: c.x, y: a.y },
    a
  ];
}
function _t(t, e, r) {
  const i = t.find((W) => W.id === e);
  if (!i) return null;
  const o = me.get(i.type);
  if (!o) return null;
  const l = o.pins.find((W) => W.id === r);
  if (!l) return null;
  const c = o.width / 2, a = o.height / 2, s = l.x - c, f = l.y - a, d = (i.rotate || 0) * Math.PI / 180, u = Math.cos(d), y = Math.sin(d), x = s * u - f * y, m = s * y + f * u;
  return { x: i.x + c + x, y: i.y + a + m };
}
function Et(t, e, r) {
  const i = t.find((f) => f.id === e);
  if (!i) return "right";
  const o = me.get(i.type);
  if (!o) return "right";
  const l = o.pins.find((f) => f.id === r);
  if (!l) return "right";
  const c = ["top", "right", "bottom", "left"], a = c.indexOf(l.side), s = Math.round((i.rotate || 0) % 360 / 90);
  return c[(a + s) % 4];
}
function Wr(t) {
  return t.map((e) => `${e.x},${e.y}`).join(" ");
}
function ui({ wires: t, parts: e, wireFrom: r, cursorPos: i, onDeleteWire: o }) {
  return /* @__PURE__ */ p("g", { className: "wire-layer", children: [
    t.map((l, c) => {
      const a = _t(e, l.from.part, l.from.pin), s = _t(e, l.to.part, l.to.pin);
      if (!a || !s) return null;
      let f = l.waypoints;
      if (!f || f.length === 0) {
        const y = Et(e, l.from.part, l.from.pin), x = Et(e, l.to.part, l.to.pin);
        f = Mr(a, y, s, x);
      }
      const d = [a, ...f, s], u = Wr(d);
      return /* @__PURE__ */ p("g", { children: [
        /* @__PURE__ */ n(
          "polyline",
          {
            points: u,
            fill: "none",
            stroke: l.color,
            strokeWidth: 2.5,
            strokeLinecap: "round",
            strokeLinejoin: "round"
          }
        ),
        /* @__PURE__ */ n(
          "polyline",
          {
            points: u,
            fill: "none",
            stroke: "transparent",
            strokeWidth: 12,
            style: { cursor: "pointer" },
            onClick: (y) => {
              y.stopPropagation(), o == null || o(c);
            }
          }
        )
      ] }, c);
    }),
    r && i && (() => {
      const l = _t(e, r.part, r.pin);
      if (!l) return null;
      const c = Et(e, r.part, r.pin), a = Mr(l, c, i, "left"), s = [l, ...a, i];
      return /* @__PURE__ */ n(
        "polyline",
        {
          points: Wr(s),
          fill: "none",
          stroke: "#e83e8c",
          strokeWidth: 2,
          strokeDasharray: "6,4",
          strokeLinejoin: "round",
          pointerEvents: "none"
        }
      );
    })()
  ] });
}
const V = 10;
function ht(t) {
  return Math.round(t / V) * V;
}
function Kn({
  state: t,
  boardIoStates: e,
  onMovePart: r,
  onSelect: i,
  onSelectRect: o,
  onStartWire: l,
  onCompleteWire: c,
  onCancelWire: a,
  onDeleteWire: s,
  onDropPart: f
}) {
  const d = D(null), [u, y] = z({ x: -100, y: -50, w: 1200, h: 800 }), [x, m] = z(null), [W, R] = z(null), [k, I] = z(null), [T, q] = z(null), [v, E] = z(null), $ = C(
    (h, g) => {
      const P = d.current;
      if (!P) return { x: 0, y: 0 };
      const A = P.getBoundingClientRect(), S = u.w / A.width, w = u.h / A.height;
      return {
        x: u.x + (h - A.left) * S,
        y: u.y + (g - A.top) * w
      };
    },
    [u]
  ), xe = C(
    (h) => {
      if (h.button === 0 && (h.target.tagName === "svg" || h.target.classList.contains("editor-grid"))) {
        if (t.wireInProgress) {
          a();
          return;
        }
        const g = $(h.clientX, h.clientY);
        h.shiftKey ? E({ x1: g.x, y1: g.y, x2: g.x, y2: g.y }) : (R({ startClientX: h.clientX, startClientY: h.clientY, startVB: { ...u } }), i(null));
      }
    },
    [u, $, i, a, t.wireInProgress]
  ), F = C(
    (h) => {
      const g = $(h.clientX, h.clientY);
      if (I(g), v) {
        E({ ...v, x2: g.x, y2: g.y });
        return;
      }
      if (W) {
        const P = d.current;
        if (!P) return;
        const A = P.getBoundingClientRect(), S = W.startVB.w / A.width, w = W.startVB.h / A.height;
        y({
          ...W.startVB,
          x: W.startVB.x - (h.clientX - W.startClientX) * S,
          y: W.startVB.y - (h.clientY - W.startClientY) * w
        });
        return;
      }
      if (x) {
        const P = ht(g.x - x.offsetX), A = ht(g.y - x.offsetY);
        !x.moved && (Math.abs(g.x - x.startX) > 3 || Math.abs(g.y - x.startY) > 3) && m({ ...x, moved: !0 }), r(x.partId, P, A);
      }
    },
    [$, W, x, v, r]
  ), N = C(
    (h) => {
      if (v) {
        const g = Math.min(v.x1, v.x2), P = Math.max(v.x1, v.x2), A = Math.min(v.y1, v.y2), S = Math.max(v.y1, v.y2), w = t.diagram.parts.filter((_) => {
          const j = me.get(_.type);
          if (!j) return !1;
          const K = _.x + j.width / 2, Lt = _.y + j.height / 2;
          return K >= g && K <= P && Lt >= A && Lt <= S;
        }).map((_) => _.id);
        o == null || o(w), E(null);
        return;
      }
      x && !x.moved && i(x.partId, h.shiftKey), m(null), R(null);
    },
    [x, v, i, o, t.diagram.parts]
  ), ce = C(
    (h) => {
      h.preventDefault();
      const g = h.deltaY > 0 ? 1.1 : 0.9, P = $(h.clientX, h.clientY), A = Math.min(Math.max(u.w * g, 200), 6e3), S = Math.min(Math.max(u.h * g, 150), 4500), w = A / u.w;
      y({
        x: P.x - (P.x - u.x) * w,
        y: P.y - (P.y - u.y) * w,
        w: A,
        h: S
      });
    },
    [$, u]
  ), We = C(
    (h, g) => {
      if (h.stopPropagation(), t.wireInProgress) return;
      const P = $(h.clientX, h.clientY);
      m({
        partId: g.id,
        offsetX: P.x - g.x,
        offsetY: P.y - g.y,
        startX: P.x,
        startY: P.y,
        moved: !1
      });
    },
    [$, t.wireInProgress]
  ), Ne = C(
    (h, g, P) => {
      h.stopPropagation();
      const A = { part: g, pin: P };
      t.wireInProgress ? c(A) : l(A);
    },
    [t.wireInProgress, l, c]
  ), be = C((h) => {
    h.preventDefault(), h.dataTransfer.dropEffect = "copy";
  }, []), b = C(
    (h) => {
      h.preventDefault();
      const g = h.dataTransfer.getData("application/x-component-type");
      if (!g || !f) return;
      const P = $(h.clientX, h.clientY), A = me.get(g);
      f(g, ht(P.x - ((A == null ? void 0 : A.width) ?? 40) / 2), ht(P.y - ((A == null ? void 0 : A.height) ?? 40) / 2));
    },
    [$, f]
  );
  L(() => {
    const h = (g) => {
      g.target.tagName !== "INPUT" && g.key === "Escape" && a();
    };
    return window.addEventListener("keydown", h), () => window.removeEventListener("keydown", h);
  }, [a]);
  const M = v ? {
    x: Math.min(v.x1, v.x2),
    y: Math.min(v.y1, v.y2),
    w: Math.abs(v.x2 - v.x1),
    h: Math.abs(v.y2 - v.y1)
  } : null;
  return /* @__PURE__ */ p(
    "svg",
    {
      ref: d,
      className: "editor-canvas",
      viewBox: `${u.x} ${u.y} ${u.w} ${u.h}`,
      style: { width: "100%", height: "100%", background: "#1a1a2e", cursor: W ? "grabbing" : "default" },
      onMouseDown: xe,
      onMouseMove: F,
      onMouseUp: N,
      onMouseLeave: () => {
        m(null), R(null), E(null);
      },
      onWheel: ce,
      onDragOver: be,
      onDrop: b,
      children: [
        /* @__PURE__ */ p("defs", { children: [
          /* @__PURE__ */ n("pattern", { id: "editor-grid-sm", width: V, height: V, patternUnits: "userSpaceOnUse", children: /* @__PURE__ */ n("circle", { cx: V / 2, cy: V / 2, r: 0.5, fill: "rgba(255,255,255,0.06)" }) }),
          /* @__PURE__ */ p("pattern", { id: "editor-grid-lg", width: V * 10, height: V * 10, patternUnits: "userSpaceOnUse", children: [
            /* @__PURE__ */ n("rect", { width: V * 10, height: V * 10, fill: "url(#editor-grid-sm)" }),
            /* @__PURE__ */ n("circle", { cx: V * 5, cy: V * 5, r: 1, fill: "rgba(255,255,255,0.12)" })
          ] })
        ] }),
        /* @__PURE__ */ n(
          "rect",
          {
            className: "editor-grid",
            x: u.x - 1e3,
            y: u.y - 1e3,
            width: u.w + 2e3,
            height: u.h + 2e3,
            fill: "url(#editor-grid-lg)"
          }
        ),
        /* @__PURE__ */ n(
          ui,
          {
            wires: t.diagram.wires,
            parts: t.diagram.parts,
            wireFrom: t.wireInProgress,
            cursorPos: k,
            onDeleteWire: s
          }
        ),
        t.diagram.parts.map((h) => {
          const g = me.get(h.type);
          if (!g) return null;
          const P = t.selectedIds.has(h.id), A = e == null ? void 0 : e[h.id], S = {
            selected: P,
            active: (A == null ? void 0 : A.active) ?? !1,
            ...A
          };
          return /* @__PURE__ */ p(
            "g",
            {
              transform: `translate(${h.x}, ${h.y}) rotate(${h.rotate}, ${g.width / 2}, ${g.height / 2})`,
              style: { cursor: (x == null ? void 0 : x.partId) === h.id ? "grabbing" : "grab" },
              onMouseDown: (w) => We(w, h),
              children: [
                g.render(h.attrs, S),
                g.pins.map((w) => {
                  const _ = (T == null ? void 0 : T.partId) === h.id && (T == null ? void 0 : T.pinId) === w.id, j = t.wireInProgress !== null;
                  return /* @__PURE__ */ n(
                    "circle",
                    {
                      cx: w.x,
                      cy: w.y,
                      r: _ ? 6 : 4,
                      fill: j ? "#27c93f" : "#e83e8c",
                      stroke: "#fff",
                      strokeWidth: 1,
                      opacity: _ || j ? 0.9 : 0.5,
                      style: { cursor: "crosshair" },
                      onMouseDown: (K) => Ne(K, h.id, w.id),
                      onMouseEnter: () => q({ partId: h.id, pinId: w.id }),
                      onMouseLeave: () => q(null)
                    },
                    w.id
                  );
                })
              ]
            },
            h.id
          );
        }),
        M && /* @__PURE__ */ n(
          "rect",
          {
            x: M.x,
            y: M.y,
            width: M.w,
            height: M.h,
            fill: "rgba(86,156,214,0.15)",
            stroke: "#569cd6",
            strokeWidth: 1,
            strokeDasharray: "4,4",
            pointerEvents: "none"
          }
        )
      ]
    }
  );
}
const hi = {
  output: "Output",
  input: "Input",
  passive: "Passive",
  sensor: "Sensors",
  display: "Displays",
  ic: "ICs"
}, yi = ["output", "input", "sensor", "display", "passive", "ic"];
function Yn({ onAddPart: t }) {
  const e = fi(), r = (i, o) => {
    i.dataTransfer.setData("application/x-component-type", o), i.dataTransfer.effectAllowed = "copy";
  };
  return /* @__PURE__ */ p("div", { className: "editor-palette", children: [
    /* @__PURE__ */ n("h3", { className: "palette-title", children: "Components" }),
    yi.filter((i) => e[i]).map((i) => [i, e[i]]).map(([i, o]) => /* @__PURE__ */ p("div", { className: "palette-group", children: [
      /* @__PURE__ */ n("div", { className: "palette-category", children: hi[i] || i }),
      o.map((l) => /* @__PURE__ */ p(
        "div",
        {
          className: "palette-item",
          draggable: !0,
          onDragStart: (c) => r(c, l.type),
          onClick: () => t == null ? void 0 : t(l.type),
          title: `Drag or click to add ${l.label}`,
          children: [
            /* @__PURE__ */ n(
              "svg",
              {
                width: 32,
                height: 32,
                viewBox: `0 0 ${l.width} ${l.height}`,
                style: { flexShrink: 0 },
                children: l.render(l.defaultAttrs)
              }
            ),
            /* @__PURE__ */ n("span", { className: "palette-label", children: l.label })
          ]
        },
        l.type
      ))
    ] }, i))
  ] });
}
function Jn({ parts: t, onUpdateAttrs: e, onDelete: r, onRotate: i }) {
  if (t.length === 0)
    return /* @__PURE__ */ n("div", { className: "editor-property-panel", children: /* @__PURE__ */ n("div", { className: "panel-empty", children: "Select a component to edit its properties" }) });
  if (t.length > 1)
    return /* @__PURE__ */ p("div", { className: "editor-property-panel", children: [
      /* @__PURE__ */ p("h3", { className: "panel-title", children: [
        t.length,
        " selected"
      ] }),
      /* @__PURE__ */ n("div", { className: "panel-actions", children: /* @__PURE__ */ n("button", { className: "panel-btn panel-btn-danger", onClick: r, title: "Delete selected", children: "Delete All" }) })
    ] });
  const o = t[0], l = me.get(o.type);
  return l ? /* @__PURE__ */ p("div", { className: "editor-property-panel", children: [
    /* @__PURE__ */ n("h3", { className: "panel-title", children: l.label }),
    /* @__PURE__ */ p("div", { className: "panel-id", children: [
      "ID: ",
      o.id
    ] }),
    /* @__PURE__ */ p("div", { className: "panel-section", children: [
      /* @__PURE__ */ p("div", { className: "panel-row", children: [
        /* @__PURE__ */ n("label", { children: "X" }),
        /* @__PURE__ */ n(
          "input",
          {
            type: "number",
            value: o.x,
            readOnly: !0,
            className: "panel-input panel-input-sm"
          }
        )
      ] }),
      /* @__PURE__ */ p("div", { className: "panel-row", children: [
        /* @__PURE__ */ n("label", { children: "Y" }),
        /* @__PURE__ */ n(
          "input",
          {
            type: "number",
            value: o.y,
            readOnly: !0,
            className: "panel-input panel-input-sm"
          }
        )
      ] }),
      /* @__PURE__ */ p("div", { className: "panel-row", children: [
        /* @__PURE__ */ n("label", { children: "Rotation" }),
        /* @__PURE__ */ p("span", { className: "panel-value", children: [
          o.rotate,
          "°"
        ] })
      ] })
    ] }),
    l.attrFields && l.attrFields.length > 0 && /* @__PURE__ */ p("div", { className: "panel-section", children: [
      /* @__PURE__ */ n("div", { className: "panel-section-title", children: "Attributes" }),
      l.attrFields.map((c) => /* @__PURE__ */ p("div", { className: "panel-row", children: [
        /* @__PURE__ */ n("label", { children: c.label }),
        c.type === "select" && c.options ? /* @__PURE__ */ n(
          "select",
          {
            className: "panel-input",
            value: o.attrs[c.key] || "",
            onChange: (a) => e(o.id, { [c.key]: a.target.value }),
            children: c.options.map((a) => /* @__PURE__ */ n("option", { value: a, children: a }, a))
          }
        ) : /* @__PURE__ */ n(
          "input",
          {
            type: "text",
            className: "panel-input",
            value: o.attrs[c.key] || "",
            onChange: (a) => e(o.id, { [c.key]: a.target.value })
          }
        )
      ] }, c.key))
    ] }),
    /* @__PURE__ */ p("div", { className: "panel-actions", children: [
      /* @__PURE__ */ n("button", { className: "panel-btn", onClick: () => i(o.id), title: "Rotate 90°", children: "↻ Rotate" }),
      /* @__PURE__ */ n("button", { className: "panel-btn panel-btn-danger", onClick: r, title: "Delete component", children: "Delete" })
    ] })
  ] }) : null;
}
function mi(t = "stm32f103") {
  return { version: 1, board: t, parts: [], wires: [] };
}
const Nr = ["#e83e8c", "#27c93f", "#569cd6", "#ffcc00", "#ff6633", "#00cccc"];
let Dr = 0;
function gi() {
  const t = Nr[Dr % Nr.length];
  return Dr++, t;
}
const He = /* @__PURE__ */ new Set();
function Be(t) {
  return {
    ...t,
    undoStack: [...t.undoStack.slice(-49), structuredClone(t.diagram)],
    redoStack: []
  };
}
function xi(t, e) {
  switch (e.type) {
    case "ADD_PART": {
      const r = Be(t);
      return {
        ...r,
        diagram: { ...r.diagram, parts: [...r.diagram.parts, e.part] },
        selectedIds: /* @__PURE__ */ new Set([e.part.id])
      };
    }
    case "MOVE_PART":
      return {
        ...t,
        diagram: {
          ...t.diagram,
          parts: t.diagram.parts.map(
            (r) => r.id === e.id ? { ...r, x: e.x, y: e.y } : r
          )
        }
      };
    case "ROTATE_PART": {
      const r = Be(t);
      return {
        ...r,
        diagram: {
          ...r.diagram,
          parts: r.diagram.parts.map(
            (i) => i.id === e.id ? { ...i, rotate: (i.rotate + 90) % 360 } : i
          )
        }
      };
    }
    case "DELETE_SELECTED": {
      if (t.selectedIds.size === 0) return t;
      const r = Be(t), i = r.selectedIds;
      return {
        ...r,
        diagram: {
          ...r.diagram,
          parts: r.diagram.parts.filter((o) => !i.has(o.id)),
          wires: r.diagram.wires.filter((o) => !i.has(o.from.part) && !i.has(o.to.part))
        },
        selectedIds: He
      };
    }
    case "UPDATE_ATTRS": {
      const r = Be(t);
      return {
        ...r,
        diagram: {
          ...r.diagram,
          parts: r.diagram.parts.map(
            (i) => i.id === e.id ? { ...i, attrs: { ...i.attrs, ...e.attrs } } : i
          )
        }
      };
    }
    case "START_WIRE":
      return { ...t, wireInProgress: e.endpoint };
    case "COMPLETE_WIRE": {
      if (!t.wireInProgress) return t;
      if (t.wireInProgress.part === e.endpoint.part && t.wireInProgress.pin === e.endpoint.pin)
        return { ...t, wireInProgress: null };
      const r = Be(t), i = {
        from: r.wireInProgress,
        to: e.endpoint,
        color: e.color
      };
      return {
        ...r,
        diagram: { ...r.diagram, wires: [...r.diagram.wires, i] },
        wireInProgress: null
      };
    }
    case "CANCEL_WIRE":
      return { ...t, wireInProgress: null };
    case "DELETE_WIRE": {
      const r = Be(t);
      return {
        ...r,
        diagram: {
          ...r.diagram,
          wires: r.diagram.wires.filter((i, o) => o !== e.index)
        }
      };
    }
    case "SELECT": {
      if (e.id === null)
        return { ...t, selectedIds: He };
      if (e.add) {
        const r = new Set(t.selectedIds);
        return r.has(e.id) ? r.delete(e.id) : r.add(e.id), { ...t, selectedIds: r };
      }
      return { ...t, selectedIds: /* @__PURE__ */ new Set([e.id]) };
    }
    case "SELECT_RECT":
      return { ...t, selectedIds: new Set(e.ids) };
    case "LOAD_DIAGRAM":
      return {
        ...t,
        diagram: e.diagram,
        selectedIds: He,
        wireInProgress: null,
        undoStack: [],
        redoStack: []
      };
    case "UNDO": {
      if (t.undoStack.length === 0) return t;
      const r = t.undoStack[t.undoStack.length - 1];
      return {
        ...t,
        diagram: r,
        undoStack: t.undoStack.slice(0, -1),
        redoStack: [...t.redoStack, structuredClone(t.diagram)],
        selectedIds: He,
        wireInProgress: null
      };
    }
    case "REDO": {
      if (t.redoStack.length === 0) return t;
      const r = t.redoStack[t.redoStack.length - 1];
      return {
        ...t,
        diagram: r,
        undoStack: [...t.undoStack, structuredClone(t.diagram)],
        redoStack: t.redoStack.slice(0, -1),
        selectedIds: He,
        wireInProgress: null
      };
    }
    default:
      return t;
  }
}
function qn(t) {
  const [e, r] = qr(xi, {
    diagram: t ?? mi(),
    selectedIds: He,
    wireInProgress: null,
    undoStack: [],
    redoStack: []
  }), i = C(
    (k) => r({ type: "ADD_PART", part: k }),
    []
  ), o = C(
    (k, I, T) => r({ type: "MOVE_PART", id: k, x: I, y: T }),
    []
  ), l = C(
    (k) => r({ type: "ROTATE_PART", id: k }),
    []
  ), c = C(() => r({ type: "DELETE_SELECTED" }), []), a = C(
    (k, I) => r({ type: "UPDATE_ATTRS", id: k, attrs: I }),
    []
  ), s = C(
    (k, I) => r({ type: "SELECT", id: k, add: I }),
    []
  ), f = C(
    (k) => r({ type: "SELECT_RECT", ids: k }),
    []
  ), d = C(
    (k) => r({ type: "START_WIRE", endpoint: k }),
    []
  ), u = C(
    (k) => r({ type: "COMPLETE_WIRE", endpoint: k, color: gi() }),
    []
  ), y = C(() => r({ type: "CANCEL_WIRE" }), []), x = C(
    (k) => r({ type: "DELETE_WIRE", index: k }),
    []
  ), m = C(
    (k) => r({ type: "LOAD_DIAGRAM", diagram: k }),
    []
  ), W = C(() => r({ type: "UNDO" }), []), R = C(() => r({ type: "REDO" }), []);
  return {
    state: e,
    addPart: i,
    movePart: o,
    rotatePart: l,
    deleteSelected: c,
    updateAttrs: a,
    select: s,
    selectRect: f,
    startWire: d,
    completeWire: u,
    cancelWire: y,
    deleteWire: x,
    loadDiagram: m,
    undo: W,
    redo: R
  };
}
const _r = {
  PA0: { gpio: { peripheral: "gpioa", pin: 0 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 0 },
    { type: "timer", peripheral: "tim2", channel: 1 }
  ] },
  PA1: { gpio: { peripheral: "gpioa", pin: 1 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 1 },
    { type: "timer", peripheral: "tim2", channel: 2 }
  ] },
  PA2: { gpio: { peripheral: "gpioa", pin: 2 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 2 },
    { type: "uart", peripheral: "uart2", role: "tx" },
    { type: "timer", peripheral: "tim2", channel: 3 }
  ] },
  PA3: { gpio: { peripheral: "gpioa", pin: 3 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 3 },
    { type: "uart", peripheral: "uart2", role: "rx" },
    { type: "timer", peripheral: "tim2", channel: 4 }
  ] },
  PA4: { gpio: { peripheral: "gpioa", pin: 4 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 4 },
    { type: "spi", peripheral: "spi1", role: "nss" }
  ] },
  PA5: { gpio: { peripheral: "gpioa", pin: 5 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 5 },
    { type: "spi", peripheral: "spi1", role: "sck" }
  ] },
  PA6: { gpio: { peripheral: "gpioa", pin: 6 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 6 },
    { type: "spi", peripheral: "spi1", role: "miso" },
    { type: "timer", peripheral: "tim3", channel: 1 }
  ] },
  PA7: { gpio: { peripheral: "gpioa", pin: 7 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 7 },
    { type: "spi", peripheral: "spi1", role: "mosi" },
    { type: "timer", peripheral: "tim3", channel: 2 }
  ] },
  PA8: { gpio: { peripheral: "gpioa", pin: 8 }, functions: [
    { type: "timer", peripheral: "tim1", channel: 1 }
  ] },
  PA9: { gpio: { peripheral: "gpioa", pin: 9 }, functions: [
    { type: "uart", peripheral: "uart1", role: "tx" },
    { type: "timer", peripheral: "tim1", channel: 2 }
  ] },
  PA10: { gpio: { peripheral: "gpioa", pin: 10 }, functions: [
    { type: "uart", peripheral: "uart1", role: "rx" },
    { type: "timer", peripheral: "tim1", channel: 3 }
  ] },
  PA11: { gpio: { peripheral: "gpioa", pin: 11 }, functions: [
    { type: "timer", peripheral: "tim1", channel: 4 }
  ] },
  PA12: { gpio: { peripheral: "gpioa", pin: 12 }, functions: [] },
  PA13: { gpio: { peripheral: "gpioa", pin: 13 }, functions: [] },
  PA14: { gpio: { peripheral: "gpioa", pin: 14 }, functions: [] },
  PA15: { gpio: { peripheral: "gpioa", pin: 15 }, functions: [
    { type: "timer", peripheral: "tim2", channel: 1 },
    { type: "spi", peripheral: "spi1", role: "nss" }
  ] },
  PB0: { gpio: { peripheral: "gpiob", pin: 0 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 8 },
    { type: "timer", peripheral: "tim3", channel: 3 }
  ] },
  PB1: { gpio: { peripheral: "gpiob", pin: 1 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 9 },
    { type: "timer", peripheral: "tim3", channel: 4 }
  ] },
  PB3: { gpio: { peripheral: "gpiob", pin: 3 }, functions: [
    { type: "spi", peripheral: "spi1", role: "sck" },
    { type: "timer", peripheral: "tim2", channel: 2 }
  ] },
  PB4: { gpio: { peripheral: "gpiob", pin: 4 }, functions: [
    { type: "spi", peripheral: "spi1", role: "miso" },
    { type: "timer", peripheral: "tim3", channel: 1 }
  ] },
  PB5: { gpio: { peripheral: "gpiob", pin: 5 }, functions: [
    { type: "spi", peripheral: "spi1", role: "mosi" }
  ] },
  PB6: { gpio: { peripheral: "gpiob", pin: 6 }, functions: [
    { type: "i2c", peripheral: "i2c1", role: "scl" },
    { type: "timer", peripheral: "tim4", channel: 1 }
  ] },
  PB7: { gpio: { peripheral: "gpiob", pin: 7 }, functions: [
    { type: "i2c", peripheral: "i2c1", role: "sda" },
    { type: "timer", peripheral: "tim4", channel: 2 }
  ] },
  PB8: { gpio: { peripheral: "gpiob", pin: 8 }, functions: [
    { type: "i2c", peripheral: "i2c1", role: "scl" },
    { type: "timer", peripheral: "tim4", channel: 3 }
  ] },
  PB9: { gpio: { peripheral: "gpiob", pin: 9 }, functions: [
    { type: "i2c", peripheral: "i2c1", role: "sda" },
    { type: "timer", peripheral: "tim4", channel: 4 }
  ] },
  PB10: { gpio: { peripheral: "gpiob", pin: 10 }, functions: [
    { type: "i2c", peripheral: "i2c2", role: "scl" },
    { type: "uart", peripheral: "uart3", role: "tx" }
  ] },
  PB11: { gpio: { peripheral: "gpiob", pin: 11 }, functions: [
    { type: "i2c", peripheral: "i2c2", role: "sda" },
    { type: "uart", peripheral: "uart3", role: "rx" }
  ] },
  PB12: { gpio: { peripheral: "gpiob", pin: 12 }, functions: [
    { type: "spi", peripheral: "spi2", role: "nss" }
  ] },
  PB13: { gpio: { peripheral: "gpiob", pin: 13 }, functions: [
    { type: "spi", peripheral: "spi2", role: "sck" }
  ] },
  PB14: { gpio: { peripheral: "gpiob", pin: 14 }, functions: [
    { type: "spi", peripheral: "spi2", role: "miso" }
  ] },
  PB15: { gpio: { peripheral: "gpiob", pin: 15 }, functions: [
    { type: "spi", peripheral: "spi2", role: "mosi" }
  ] },
  PC0: { gpio: { peripheral: "gpioc", pin: 0 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 10 }
  ] },
  PC1: { gpio: { peripheral: "gpioc", pin: 1 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 11 }
  ] },
  PC2: { gpio: { peripheral: "gpioc", pin: 2 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 12 }
  ] },
  PC3: { gpio: { peripheral: "gpioc", pin: 3 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 13 }
  ] },
  PC4: { gpio: { peripheral: "gpioc", pin: 4 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 14 }
  ] },
  PC5: { gpio: { peripheral: "gpioc", pin: 5 }, functions: [
    { type: "adc", peripheral: "adc1", channel: 15 }
  ] },
  PC13: { gpio: { peripheral: "gpioc", pin: 13 }, functions: [] },
  PC14: { gpio: { peripheral: "gpioc", pin: 14 }, functions: [] },
  PC15: { gpio: { peripheral: "gpioc", pin: 15 }, functions: [] }
}, bi = {
  stm32f103: _r,
  stm32f401: _r
  // Similar enough for now
};
function wi(t, e) {
  const r = bi[t];
  return r ? r[e.toUpperCase()] ?? null : null;
}
function vi(t, e, r) {
  const i = wi(t, e);
  return i ? i.functions.find((o) => o.type === r) ?? null : null;
}
const Si = {
  stm32f103: `
name: "stm32f103c8"
arch: "arm"
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x40010800
    size: "1KB"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x40010C00
    size: "1KB"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x40011000
    size: "1KB"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    irq: 37
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 38
  - id: "i2c1"
    type: "i2c"
    base_address: 0x40005400
    size: "1KB"
    irq: 31
  - id: "afio"
    type: "afio"
    base_address: 0x40010000
    size: "1KB"
  - id: "exti"
    type: "exti"
    base_address: 0x40010400
    size: "1KB"
  - id: "dma1"
    type: "dma"
    base_address: 0x40020000
    size: "1KB"
  - id: "adc1"
    type: "adc"
    base_address: 0x40012400
    size: "1KB"
    irq: 18
`,
  stm32f401: `
name: "stm32f401re"
arch: "arm"
flash:
  base: 0x08000000
  size: "512KB"
ram:
  base: 0x20000000
  size: "96KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40023800
    size: "1KB"
    config:
      profile: "stm32f4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x40020000
    size: "1KB"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x40020400
    size: "1KB"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x40020800
    size: "1KB"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 38
`
};
function ki(t) {
  const e = t.match(/^P([A-C])(\d+)$/i);
  if (e) return { peripheral: `gpio${e[1].toLowerCase()}`, pin: parseInt(e[2], 10) };
  const r = t.match(/^([DA])(\d+)$/i);
  if (r) return { peripheral: r[1].toLowerCase() === "d" ? "gpiod" : "gpioa", pin: parseInt(r[2], 10) };
  const i = t.match(/^(?:GPIO|GP)(\d+)$/i);
  if (i) return { peripheral: "gpio0", pin: parseInt(i[1], 10) };
  const o = t.match(/^P(\d+)\.(\d+)$/);
  return o ? { peripheral: `gpio${o[1]}`, pin: parseInt(o[2], 10) } : null;
}
function Xn(t) {
  const e = Si[t.board];
  if (!e)
    throw new Error(`Unknown board: ${t.board}`);
  const r = [];
  for (const o of t.wires) {
    let l = null, c = null;
    if (o.from.part === "mcu")
      l = o.from, c = o.to;
    else if (o.to.part === "mcu")
      l = o.to, c = o.from;
    else
      continue;
    const a = t.parts.find((m) => m.id === c.part);
    if (!a) continue;
    const s = me.get(a.type);
    if (!(s != null && s.boardIoKind)) continue;
    const f = ki(l.pin);
    if (!f) continue;
    const d = s.boardIoKind, u = d === "button" || d === "adc_input" ? "input" : "output", y = d;
    let x = f.peripheral;
    if (d === "adc_input") {
      const m = vi(t.board, l.pin, "adc");
      m && (x = m.peripheral);
    }
    r.push(`  - id: "${a.id}"
    kind: "${y}"
    peripheral: "${x}"
    pin: ${f.pin}
    signal: "${u}"
    active_high: true`);
  }
  return { systemYaml: `name: "playground-board"
chip: "inline"
board_io:
${r.length > 0 ? r.join(`
`) : "  []"}
`, chipYaml: e };
}
function Er(t, e) {
  (e == null || e > t.length) && (e = t.length);
  for (var r = 0, i = Array(e); r < e; r++) i[r] = t[r];
  return i;
}
function Ci(t) {
  if (Array.isArray(t)) return t;
}
function Pi(t, e, r) {
  return (e = Di(e)) in t ? Object.defineProperty(t, e, {
    value: r,
    enumerable: !0,
    configurable: !0,
    writable: !0
  }) : t[e] = r, t;
}
function Ai(t, e) {
  var r = t == null ? null : typeof Symbol < "u" && t[Symbol.iterator] || t["@@iterator"];
  if (r != null) {
    var i, o, l, c, a = [], s = !0, f = !1;
    try {
      if (l = (r = r.call(t)).next, e !== 0) for (; !(s = (i = l.call(r)).done) && (a.push(i.value), a.length !== e); s = !0) ;
    } catch (d) {
      f = !0, o = d;
    } finally {
      try {
        if (!s && r.return != null && (c = r.return(), Object(c) !== c)) return;
      } finally {
        if (f) throw o;
      }
    }
    return a;
  }
}
function Ii() {
  throw new TypeError(`Invalid attempt to destructure non-iterable instance.
In order to be iterable, non-array objects must have a [Symbol.iterator]() method.`);
}
function Or(t, e) {
  var r = Object.keys(t);
  if (Object.getOwnPropertySymbols) {
    var i = Object.getOwnPropertySymbols(t);
    e && (i = i.filter(function(o) {
      return Object.getOwnPropertyDescriptor(t, o).enumerable;
    })), r.push.apply(r, i);
  }
  return r;
}
function Tr(t) {
  for (var e = 1; e < arguments.length; e++) {
    var r = arguments[e] != null ? arguments[e] : {};
    e % 2 ? Or(Object(r), !0).forEach(function(i) {
      Pi(t, i, r[i]);
    }) : Object.getOwnPropertyDescriptors ? Object.defineProperties(t, Object.getOwnPropertyDescriptors(r)) : Or(Object(r)).forEach(function(i) {
      Object.defineProperty(t, i, Object.getOwnPropertyDescriptor(r, i));
    });
  }
  return t;
}
function $i(t, e) {
  if (t == null) return {};
  var r, i, o = Mi(t, e);
  if (Object.getOwnPropertySymbols) {
    var l = Object.getOwnPropertySymbols(t);
    for (i = 0; i < l.length; i++) r = l[i], e.indexOf(r) === -1 && {}.propertyIsEnumerable.call(t, r) && (o[r] = t[r]);
  }
  return o;
}
function Mi(t, e) {
  if (t == null) return {};
  var r = {};
  for (var i in t) if ({}.hasOwnProperty.call(t, i)) {
    if (e.indexOf(i) !== -1) continue;
    r[i] = t[i];
  }
  return r;
}
function Wi(t, e) {
  return Ci(t) || Ai(t, e) || _i(t, e) || Ii();
}
function Ni(t, e) {
  if (typeof t != "object" || !t) return t;
  var r = t[Symbol.toPrimitive];
  if (r !== void 0) {
    var i = r.call(t, e);
    if (typeof i != "object") return i;
    throw new TypeError("@@toPrimitive must return a primitive value.");
  }
  return (e === "string" ? String : Number)(t);
}
function Di(t) {
  var e = Ni(t, "string");
  return typeof e == "symbol" ? e : e + "";
}
function _i(t, e) {
  if (t) {
    if (typeof t == "string") return Er(t, e);
    var r = {}.toString.call(t).slice(8, -1);
    return r === "Object" && t.constructor && (r = t.constructor.name), r === "Map" || r === "Set" ? Array.from(t) : r === "Arguments" || /^(?:Ui|I)nt(?:8|16|32)(?:Clamped)?Array$/.test(r) ? Er(t, e) : void 0;
  }
}
function Ei(t, e, r) {
  return e in t ? Object.defineProperty(t, e, {
    value: r,
    enumerable: !0,
    configurable: !0,
    writable: !0
  }) : t[e] = r, t;
}
function Fr(t, e) {
  var r = Object.keys(t);
  if (Object.getOwnPropertySymbols) {
    var i = Object.getOwnPropertySymbols(t);
    e && (i = i.filter(function(o) {
      return Object.getOwnPropertyDescriptor(t, o).enumerable;
    })), r.push.apply(r, i);
  }
  return r;
}
function Rr(t) {
  for (var e = 1; e < arguments.length; e++) {
    var r = arguments[e] != null ? arguments[e] : {};
    e % 2 ? Fr(Object(r), !0).forEach(function(i) {
      Ei(t, i, r[i]);
    }) : Object.getOwnPropertyDescriptors ? Object.defineProperties(t, Object.getOwnPropertyDescriptors(r)) : Fr(Object(r)).forEach(function(i) {
      Object.defineProperty(t, i, Object.getOwnPropertyDescriptor(r, i));
    });
  }
  return t;
}
function Oi() {
  for (var t = arguments.length, e = new Array(t), r = 0; r < t; r++)
    e[r] = arguments[r];
  return function(i) {
    return e.reduceRight(function(o, l) {
      return l(o);
    }, i);
  };
}
function Qe(t) {
  return function e() {
    for (var r = this, i = arguments.length, o = new Array(i), l = 0; l < i; l++)
      o[l] = arguments[l];
    return o.length >= t.length ? t.apply(this, o) : function() {
      for (var c = arguments.length, a = new Array(c), s = 0; s < c; s++)
        a[s] = arguments[s];
      return e.apply(r, [].concat(o, a));
    };
  };
}
function vt(t) {
  return {}.toString.call(t).includes("Object");
}
function Ti(t) {
  return !Object.keys(t).length;
}
function tt(t) {
  return typeof t == "function";
}
function Fi(t, e) {
  return Object.prototype.hasOwnProperty.call(t, e);
}
function Ri(t, e) {
  return vt(e) || ge("changeType"), Object.keys(e).some(function(r) {
    return !Fi(t, r);
  }) && ge("changeField"), e;
}
function zi(t) {
  tt(t) || ge("selectorType");
}
function Li(t) {
  tt(t) || vt(t) || ge("handlerType"), vt(t) && Object.values(t).some(function(e) {
    return !tt(e);
  }) && ge("handlersType");
}
function Bi(t) {
  t || ge("initialIsRequired"), vt(t) || ge("initialType"), Ti(t) && ge("initialContent");
}
function Hi(t, e) {
  throw new Error(t[e] || t.default);
}
var Gi = {
  initialIsRequired: "initial state is required",
  initialType: "initial state should be an object",
  initialContent: "initial state shouldn't be an empty object",
  handlerType: "handler should be an object or a function",
  handlersType: "all handlers should be a functions",
  selectorType: "selector should be a function",
  changeType: "provided value of changes should be an object",
  changeField: 'it seams you want to change a field in the state which is not specified in the "initial" state',
  default: "an unknown error accured in `state-local` package"
}, ge = Qe(Hi)(Gi), yt = {
  changes: Ri,
  selector: zi,
  handler: Li,
  initial: Bi
};
function ji(t) {
  var e = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
  yt.initial(t), yt.handler(e);
  var r = {
    current: t
  }, i = Qe(Ki)(r, e), o = Qe(Vi)(r), l = Qe(yt.changes)(t), c = Qe(Ui)(r);
  function a() {
    var f = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : function(d) {
      return d;
    };
    return yt.selector(f), f(r.current);
  }
  function s(f) {
    Oi(i, o, l, c)(f);
  }
  return [a, s];
}
function Ui(t, e) {
  return tt(e) ? e(t.current) : e;
}
function Vi(t, e) {
  return t.current = Rr(Rr({}, t.current), e), e;
}
function Ki(t, e, r) {
  return tt(e) ? e(t.current) : Object.keys(r).forEach(function(i) {
    var o;
    return (o = e[i]) === null || o === void 0 ? void 0 : o.call(e, t.current[i]);
  }), r;
}
var Yi = {
  create: ji
}, Ji = {
  paths: {
    vs: "https://cdn.jsdelivr.net/npm/monaco-editor@0.55.1/min/vs"
  }
};
function qi(t) {
  return function e() {
    for (var r = this, i = arguments.length, o = new Array(i), l = 0; l < i; l++)
      o[l] = arguments[l];
    return o.length >= t.length ? t.apply(this, o) : function() {
      for (var c = arguments.length, a = new Array(c), s = 0; s < c; s++)
        a[s] = arguments[s];
      return e.apply(r, [].concat(o, a));
    };
  };
}
function Xi(t) {
  return {}.toString.call(t).includes("Object");
}
function Qi(t) {
  return t || zr("configIsRequired"), Xi(t) || zr("configType"), t.urls ? (Zi(), {
    paths: {
      vs: t.urls.monacoBase
    }
  }) : t;
}
function Zi() {
  console.warn(Br.deprecation);
}
function en(t, e) {
  throw new Error(t[e] || t.default);
}
var Br = {
  configIsRequired: "the configuration object is required",
  configType: "the configuration object should be an object",
  default: "an unknown error accured in `@monaco-editor/loader` package",
  deprecation: `Deprecation warning!
    You are using deprecated way of configuration.

    Instead of using
      monaco.config({ urls: { monacoBase: '...' } })
    use
      monaco.config({ paths: { vs: '...' } })

    For more please check the link https://github.com/suren-atoyan/monaco-loader#config
  `
}, zr = qi(en)(Br), tn = {
  config: Qi
}, rn = function() {
  for (var e = arguments.length, r = new Array(e), i = 0; i < e; i++)
    r[i] = arguments[i];
  return function(o) {
    return r.reduceRight(function(l, c) {
      return c(l);
    }, o);
  };
};
function Hr(t, e) {
  return Object.keys(e).forEach(function(r) {
    e[r] instanceof Object && t[r] && Object.assign(e[r], Hr(t[r], e[r]));
  }), Tr(Tr({}, t), e);
}
var nn = {
  type: "cancelation",
  msg: "operation is manually canceled"
};
function Ot(t) {
  var e = !1, r = new Promise(function(i, o) {
    t.then(function(l) {
      return e ? o(nn) : i(l);
    }), t.catch(o);
  });
  return r.cancel = function() {
    return e = !0;
  }, r;
}
var on = ["monaco"], ln = Yi.create({
  config: Ji,
  isInitialized: !1,
  resolve: null,
  reject: null,
  monaco: null
}), Gr = Wi(ln, 2), rt = Gr[0], St = Gr[1];
function cn(t) {
  var e = tn.config(t), r = e.monaco, i = $i(e, on);
  St(function(o) {
    return {
      config: Hr(o.config, i),
      monaco: r
    };
  });
}
function an() {
  var t = rt(function(e) {
    var r = e.monaco, i = e.isInitialized, o = e.resolve;
    return {
      monaco: r,
      isInitialized: i,
      resolve: o
    };
  });
  if (!t.isInitialized) {
    if (St({
      isInitialized: !0
    }), t.monaco)
      return t.resolve(t.monaco), Ot(Tt);
    if (window.monaco && window.monaco.editor)
      return jr(window.monaco), t.resolve(window.monaco), Ot(Tt);
    rn(sn, pn)(fn);
  }
  return Ot(Tt);
}
function sn(t) {
  return document.body.appendChild(t);
}
function dn(t) {
  var e = document.createElement("script");
  return t && (e.src = t), e;
}
function pn(t) {
  var e = rt(function(i) {
    var o = i.config, l = i.reject;
    return {
      config: o,
      reject: l
    };
  }), r = dn("".concat(e.config.paths.vs, "/loader.js"));
  return r.onload = function() {
    return t();
  }, r.onerror = e.reject, r;
}
function fn() {
  var t = rt(function(r) {
    var i = r.config, o = r.resolve, l = r.reject;
    return {
      config: i,
      resolve: o,
      reject: l
    };
  }), e = window.require;
  e.config(t.config), e(["vs/editor/editor.main"], function(r) {
    var i = r.m || r;
    jr(i), t.resolve(i);
  }, function(r) {
    t.reject(r);
  });
}
function jr(t) {
  rt().monaco || St({
    monaco: t
  });
}
function un() {
  return rt(function(t) {
    var e = t.monaco;
    return e;
  });
}
var Tt = new Promise(function(t, e) {
  return St({
    resolve: t,
    reject: e
  });
}), Ur = {
  config: cn,
  init: an,
  __getMonacoInstance: un
}, hn = { wrapper: { display: "flex", position: "relative", textAlign: "initial" }, fullWidth: { width: "100%" }, hide: { display: "none" } }, Ft = hn, yn = { container: { display: "flex", height: "100%", width: "100%", justifyContent: "center", alignItems: "center" } }, mn = yn;
function gn({ children: t }) {
  return Ue.createElement("div", { style: mn.container }, t);
}
var xn = gn, bn = xn;
function wn({ width: t, height: e, isEditorReady: r, loading: i, _ref: o, className: l, wrapperProps: c }) {
  return Ue.createElement("section", { style: { ...Ft.wrapper, width: t, height: e }, ...c }, !r && Ue.createElement(bn, null, i), Ue.createElement("div", { ref: o, style: { ...Ft.fullWidth, ...!r && Ft.hide }, className: l }));
}
var vn = wn, Vr = zt(vn);
function Sn(t) {
  L(t, []);
}
var Kr = Sn;
function kn(t, e, r = !0) {
  let i = D(!0);
  L(i.current || !r ? () => {
    i.current = !1;
  } : t, e);
}
var G = kn;
function Ze() {
}
function je(t, e, r, i) {
  return Cn(t, i) || Pn(t, e, r, i);
}
function Cn(t, e) {
  return t.editor.getModel(Yr(t, e));
}
function Pn(t, e, r, i) {
  return t.editor.createModel(e, r, i ? Yr(t, i) : void 0);
}
function Yr(t, e) {
  return t.Uri.parse(e);
}
function An({ original: t, modified: e, language: r, originalLanguage: i, modifiedLanguage: o, originalModelPath: l, modifiedModelPath: c, keepCurrentOriginalModel: a = !1, keepCurrentModifiedModel: s = !1, theme: f = "light", loading: d = "Loading...", options: u = {}, height: y = "100%", width: x = "100%", className: m, wrapperProps: W = {}, beforeMount: R = Ze, onMount: k = Ze }) {
  let [I, T] = z(!1), [q, v] = z(!0), E = D(null), $ = D(null), xe = D(null), F = D(k), N = D(R), ce = D(!1);
  Kr(() => {
    let b = Ur.init();
    return b.then((M) => ($.current = M) && v(!1)).catch((M) => (M == null ? void 0 : M.type) !== "cancelation" && console.error("Monaco initialization: error:", M)), () => E.current ? be() : b.cancel();
  }), G(() => {
    if (E.current && $.current) {
      let b = E.current.getOriginalEditor(), M = je($.current, t || "", i || r || "text", l || "");
      M !== b.getModel() && b.setModel(M);
    }
  }, [l], I), G(() => {
    if (E.current && $.current) {
      let b = E.current.getModifiedEditor(), M = je($.current, e || "", o || r || "text", c || "");
      M !== b.getModel() && b.setModel(M);
    }
  }, [c], I), G(() => {
    let b = E.current.getModifiedEditor();
    b.getOption($.current.editor.EditorOption.readOnly) ? b.setValue(e || "") : e !== b.getValue() && (b.executeEdits("", [{ range: b.getModel().getFullModelRange(), text: e || "", forceMoveMarkers: !0 }]), b.pushUndoStop());
  }, [e], I), G(() => {
    var b, M;
    (M = (b = E.current) == null ? void 0 : b.getModel()) == null || M.original.setValue(t || "");
  }, [t], I), G(() => {
    let { original: b, modified: M } = E.current.getModel();
    $.current.editor.setModelLanguage(b, i || r || "text"), $.current.editor.setModelLanguage(M, o || r || "text");
  }, [r, i, o], I), G(() => {
    var b;
    (b = $.current) == null || b.editor.setTheme(f);
  }, [f], I), G(() => {
    var b;
    (b = E.current) == null || b.updateOptions(u);
  }, [u], I);
  let We = C(() => {
    var h;
    if (!$.current) return;
    N.current($.current);
    let b = je($.current, t || "", i || r || "text", l || ""), M = je($.current, e || "", o || r || "text", c || "");
    (h = E.current) == null || h.setModel({ original: b, modified: M });
  }, [r, e, o, t, i, l, c]), Ne = C(() => {
    var b;
    !ce.current && xe.current && (E.current = $.current.editor.createDiffEditor(xe.current, { automaticLayout: !0, ...u }), We(), (b = $.current) == null || b.editor.setTheme(f), T(!0), ce.current = !0);
  }, [u, f, We]);
  L(() => {
    I && F.current(E.current, $.current);
  }, [I]), L(() => {
    !q && !I && Ne();
  }, [q, I, Ne]);
  function be() {
    var M, h, g, P;
    let b = (M = E.current) == null ? void 0 : M.getModel();
    a || ((h = b == null ? void 0 : b.original) == null || h.dispose()), s || ((g = b == null ? void 0 : b.modified) == null || g.dispose()), (P = E.current) == null || P.dispose();
  }
  return Ue.createElement(Vr, { width: x, height: y, isEditorReady: I, loading: d, _ref: xe, className: m, wrapperProps: W });
}
var In = An;
zt(In);
function $n(t) {
  let e = D();
  return L(() => {
    e.current = t;
  }, [t]), e.current;
}
var Mn = $n, mt = /* @__PURE__ */ new Map();
function Wn({ defaultValue: t, defaultLanguage: e, defaultPath: r, value: i, language: o, path: l, theme: c = "light", line: a, loading: s = "Loading...", options: f = {}, overrideServices: d = {}, saveViewState: u = !0, keepCurrentModel: y = !1, width: x = "100%", height: m = "100%", className: W, wrapperProps: R = {}, beforeMount: k = Ze, onMount: I = Ze, onChange: T, onValidate: q = Ze }) {
  let [v, E] = z(!1), [$, xe] = z(!0), F = D(null), N = D(null), ce = D(null), We = D(I), Ne = D(k), be = D(), b = D(i), M = Mn(l), h = D(!1), g = D(!1);
  Kr(() => {
    let S = Ur.init();
    return S.then((w) => (F.current = w) && xe(!1)).catch((w) => (w == null ? void 0 : w.type) !== "cancelation" && console.error("Monaco initialization: error:", w)), () => N.current ? A() : S.cancel();
  }), G(() => {
    var w, _, j, K;
    let S = je(F.current, t || i || "", e || o || "", l || r || "");
    S !== ((w = N.current) == null ? void 0 : w.getModel()) && (u && mt.set(M, (_ = N.current) == null ? void 0 : _.saveViewState()), (j = N.current) == null || j.setModel(S), u && ((K = N.current) == null || K.restoreViewState(mt.get(l))));
  }, [l], v), G(() => {
    var S;
    (S = N.current) == null || S.updateOptions(f);
  }, [f], v), G(() => {
    !N.current || i === void 0 || (N.current.getOption(F.current.editor.EditorOption.readOnly) ? N.current.setValue(i) : i !== N.current.getValue() && (g.current = !0, N.current.executeEdits("", [{ range: N.current.getModel().getFullModelRange(), text: i, forceMoveMarkers: !0 }]), N.current.pushUndoStop(), g.current = !1));
  }, [i], v), G(() => {
    var w, _;
    let S = (w = N.current) == null ? void 0 : w.getModel();
    S && o && ((_ = F.current) == null || _.editor.setModelLanguage(S, o));
  }, [o], v), G(() => {
    var S;
    a !== void 0 && ((S = N.current) == null || S.revealLine(a));
  }, [a], v), G(() => {
    var S;
    (S = F.current) == null || S.editor.setTheme(c);
  }, [c], v);
  let P = C(() => {
    var S;
    if (!(!ce.current || !F.current) && !h.current) {
      Ne.current(F.current);
      let w = l || r, _ = je(F.current, i || t || "", e || o || "", w || "");
      N.current = (S = F.current) == null ? void 0 : S.editor.create(ce.current, { model: _, automaticLayout: !0, ...f }, d), u && N.current.restoreViewState(mt.get(w)), F.current.editor.setTheme(c), a !== void 0 && N.current.revealLine(a), E(!0), h.current = !0;
    }
  }, [t, e, r, i, o, l, f, d, u, c, a]);
  L(() => {
    v && We.current(N.current, F.current);
  }, [v]), L(() => {
    !$ && !v && P();
  }, [$, v, P]), b.current = i, L(() => {
    var S, w;
    v && T && ((S = be.current) == null || S.dispose(), be.current = (w = N.current) == null ? void 0 : w.onDidChangeModelContent((_) => {
      g.current || T(N.current.getValue(), _);
    }));
  }, [v, T]), L(() => {
    if (v) {
      let S = F.current.editor.onDidChangeMarkers((w) => {
        var j;
        let _ = (j = N.current.getModel()) == null ? void 0 : j.uri;
        if (_ && w.find((K) => K.path === _.path)) {
          let K = F.current.editor.getModelMarkers({ resource: _ });
          q == null || q(K);
        }
      });
      return () => {
        S == null || S.dispose();
      };
    }
    return () => {
    };
  }, [v, q]);
  function A() {
    var S, w;
    (S = be.current) == null || S.dispose(), y ? u && mt.set(l, N.current.saveViewState()) : (w = N.current.getModel()) == null || w.dispose(), N.current.dispose();
  }
  return Ue.createElement(Vr, { width: x, height: m, isEditorReady: v, loading: s, _ref: ce, className: W, wrapperProps: R });
}
var Nn = Wn, Dn = zt(Nn), _n = Dn;
function Qn({
  source: t,
  language: e = "c",
  onChange: r,
  errors: i = [],
  readOnly: o = !1
}) {
  const l = D(null), c = D(null), a = C((f, d) => {
    l.current = f, c.current = d, d.languages.register({ id: "c" }), d.languages.register({ id: "cpp" });
    const u = [
      "void",
      "int",
      "char",
      "float",
      "double",
      "long",
      "unsigned",
      "signed",
      "const",
      "static",
      "volatile",
      "extern",
      "return",
      "if",
      "else",
      "for",
      "while",
      "do",
      "switch",
      "case",
      "break",
      "continue",
      "default",
      "struct",
      "typedef",
      "enum",
      "sizeof",
      "include",
      "define",
      "ifdef",
      "ifndef",
      "endif",
      "pragma"
    ], y = [
      "setup",
      "loop",
      "pinMode",
      "digitalWrite",
      "digitalRead",
      "analogRead",
      "analogWrite",
      "delay",
      "delayMicroseconds",
      "millis",
      "micros",
      "Serial",
      "Wire",
      "SPI",
      "HIGH",
      "LOW",
      "INPUT",
      "OUTPUT",
      "INPUT_PULLUP",
      "LED_BUILTIN",
      "A0",
      "A1",
      "A2",
      "A3",
      "A4",
      "A5",
      "HAL_GPIO_WritePin",
      "HAL_GPIO_ReadPin",
      "HAL_GPIO_Init",
      "HAL_UART_Transmit",
      "HAL_UART_Receive",
      "HAL_Delay",
      "HAL_GetTick",
      "GPIO_PIN_SET",
      "GPIO_PIN_RESET",
      "GPIOA",
      "GPIOB",
      "GPIOC"
    ];
    d.languages.registerCompletionItemProvider("c", {
      provideCompletionItems: (x, m) => {
        const W = x.getWordUntilPosition(m), R = {
          startLineNumber: m.lineNumber,
          endLineNumber: m.lineNumber,
          startColumn: W.startColumn,
          endColumn: W.endColumn
        };
        return { suggestions: [
          ...u.map((I) => ({
            label: I,
            kind: d.languages.CompletionItemKind.Keyword,
            insertText: I,
            range: R
          })),
          ...y.map((I) => ({
            label: I,
            kind: d.languages.CompletionItemKind.Function,
            insertText: I,
            range: R
          }))
        ] };
      }
    }), d.editor.defineTheme("labwired-dark", {
      base: "vs-dark",
      inherit: !0,
      rules: [
        { token: "keyword", foreground: "569cd6", fontStyle: "bold" },
        { token: "type", foreground: "4ec9b0" },
        { token: "string", foreground: "ce9178" },
        { token: "comment", foreground: "6a9955" },
        { token: "number", foreground: "b5cea8" }
      ],
      colors: {
        "editor.background": "#1a1a2e",
        "editor.foreground": "#d4d4d4",
        "editorLineNumber.foreground": "#858585",
        "editor.selectionBackground": "#264f78",
        "editor.lineHighlightBackground": "#2a2a4a"
      }
    }), d.editor.setTheme("labwired-dark");
  }, []), s = D([]);
  if (i !== s.current && c.current && l.current) {
    s.current = i;
    const f = l.current.getModel();
    if (f) {
      const d = c.current, u = i.map((y) => ({
        severity: y.severity === "error" ? d.MarkerSeverity.Error : d.MarkerSeverity.Warning,
        startLineNumber: y.line,
        startColumn: y.column,
        endLineNumber: y.line,
        endColumn: y.column + 100,
        message: y.message
      }));
      d.editor.setModelMarkers(f, "compile", u);
    }
  }
  return /* @__PURE__ */ n("div", { className: "code-editor-container", style: { width: "100%", height: "100%" }, children: /* @__PURE__ */ n(
    _n,
    {
      defaultLanguage: e,
      value: t,
      onChange: (f) => r(f ?? ""),
      onMount: a,
      options: {
        readOnly: o,
        fontSize: 14,
        fontFamily: "'JetBrains Mono', 'Fira Code', 'Consolas', monospace",
        minimap: { enabled: !1 },
        scrollBeyondLastLine: !1,
        lineNumbers: "on",
        renderWhitespace: "selection",
        tabSize: 2,
        automaticLayout: !0,
        suggestOnTriggerCharacters: !0,
        quickSuggestions: !0,
        wordWrap: "off",
        folding: !0,
        bracketPairColorization: { enabled: !0 }
      }
    }
  ) });
}
async function Zn(t) {
  const { source: e } = t, r = En(e);
  if (r.length > 0)
    return { success: !1, errors: r, output: "Compilation failed with syntax errors." };
  const i = ["http://localhost:3001/api/compile", "/api/compile"];
  for (const o of i)
    try {
      const l = await fetch(o, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(t)
      });
      if (l.ok) {
        const c = await l.json();
        return c.success ? { success: !0, elf: Uint8Array.from(atob(c.elf), (s) => s.charCodeAt(0)), errors: [], output: c.output ?? "Compilation successful." } : {
          success: !1,
          errors: c.errors ?? [],
          output: c.output ?? "Compilation failed."
        };
      }
    } catch {
      continue;
    }
  return {
    success: !1,
    errors: [],
    output: `No compile server available. Use a pre-built demo firmware instead.
To set up the compile server, see the documentation.`
  };
}
function En(t) {
  const e = [], r = t.split(`
`);
  let i = 0;
  for (let o = 0; o < r.length; o++) {
    for (const l of r[o])
      l === "{" && i++, l === "}" && i--;
    if (i < 0) {
      e.push({ line: o + 1, column: 1, message: "Unexpected closing brace", severity: "error" });
      break;
    }
  }
  return i > 0 && e.push({ line: r.length, column: 1, message: `Missing ${i} closing brace(s)`, severity: "error" }), e;
}
const eo = [
  {
    name: "Blink",
    language: "arduino",
    source: `// Blink — the classic Arduino sketch
// Toggles LED on pin 5 (PA5 on STM32F103)

void setup() {
  pinMode(5, OUTPUT);
}

void loop() {
  digitalWrite(5, HIGH);
  delay(500);
  digitalWrite(5, LOW);
  delay(500);
}
`
  },
  {
    name: "Button + LED",
    language: "arduino",
    source: `// Read a button and control an LED
// Button on pin 32+13 = PC13, LED on pin 5 = PA5

#define BUTTON_PIN 45  // PC13 (32 + 13)
#define LED_PIN    5   // PA5

void setup() {
  pinMode(LED_PIN, OUTPUT);
  pinMode(BUTTON_PIN, INPUT_PULLUP);
}

void loop() {
  int state = digitalRead(BUTTON_PIN);
  if (state == LOW) {
    digitalWrite(LED_PIN, HIGH);
  } else {
    digitalWrite(LED_PIN, LOW);
  }
}
`
  },
  {
    name: "Serial Hello",
    language: "arduino",
    source: `// Serial communication example
// Prints "Hello LabWired!" and echoes received characters

void setup() {
  Serial_begin(115200);
  Serial_println("Hello LabWired!");
  Serial_println("Type something...");
}

void loop() {
  if (Serial_available()) {
    int ch = Serial_read();
    char buf[2] = { (char)ch, 0 };
    Serial_print("Echo: ");
    Serial_println(buf);
  }
  delay(10);
}
`
  },
  {
    name: "Analog Read",
    language: "arduino",
    source: `// Read analog value from a potentiometer
// Potentiometer on PA0 (pin 0), LED on PA5 (pin 5)

void setup() {
  pinMode(5, OUTPUT);
  Serial_begin(115200);
}

void loop() {
  int val = analogRead(0);
  Serial_print("ADC: ");
  Serial_println_int(val);

  // Turn LED on if value > half
  if (val > 2048) {
    digitalWrite(5, HIGH);
  } else {
    digitalWrite(5, LOW);
  }
  delay(200);
}
`
  },
  {
    name: "LED Fade",
    language: "arduino",
    source: `// Simulate LED fade using rapid toggling
// LED on PA5 (pin 5)

void setup() {
  pinMode(5, OUTPUT);
}

void loop() {
  // Ramp up
  for (int i = 0; i < 100; i++) {
    digitalWrite(5, HIGH);
    delayMicroseconds(i * 10);
    digitalWrite(5, LOW);
    delayMicroseconds((100 - i) * 10);
  }
  // Ramp down
  for (int i = 100; i > 0; i--) {
    digitalWrite(5, HIGH);
    delayMicroseconds(i * 10);
    digitalWrite(5, LOW);
    delayMicroseconds((100 - i) * 10);
  }
}
`
  }
];
let $e = null, ie = null, Me = null;
function On() {
  return $e || ($e = new AudioContext()), $e;
}
function to(t, e = 0.15) {
  const r = On();
  ie || (ie = r.createOscillator(), Me = r.createGain(), ie.type = "square", ie.connect(Me), Me.connect(r.destination), ie.start()), ie.frequency.setValueAtTime(t, r.currentTime), Me.gain.setValueAtTime(e, r.currentTime);
}
function ro() {
  ie && (ie.stop(), ie.disconnect(), ie = null), Me && (Me.disconnect(), Me = null);
}
function io() {
  ($e == null ? void 0 : $e.state) === "suspended" && $e.resume();
}
async function Jr(t, e) {
  const r = JSON.stringify({ d: t, s: e }), i = new TextEncoder().encode(r);
  if (typeof CompressionStream < "u") {
    const o = new CompressionStream("deflate"), l = o.writable.getWriter();
    l.write(i), l.close();
    const c = [], a = o.readable.getReader();
    for (; ; ) {
      const { done: d, value: u } = await a.read();
      if (d) break;
      c.push(u);
    }
    const s = new Uint8Array(c.reduce((d, u) => d + u.length, 0));
    let f = 0;
    for (const d of c)
      s.set(d, f), f += d.length;
    return "z" + btoa(String.fromCharCode(...s));
  }
  return "r" + btoa(r);
}
async function no(t) {
  if (!t || t.length < 2) return null;
  try {
    const e = t[0], r = t.slice(1);
    if (e === "z" && typeof DecompressionStream < "u") {
      const i = Uint8Array.from(atob(r), (y) => y.charCodeAt(0)), o = new DecompressionStream("deflate"), l = o.writable.getWriter();
      l.write(i), l.close();
      const c = [], a = o.readable.getReader();
      for (; ; ) {
        const { done: y, value: x } = await a.read();
        if (y) break;
        c.push(x);
      }
      const s = new Uint8Array(c.reduce((y, x) => y + x.length, 0));
      let f = 0;
      for (const y of c)
        s.set(y, f), f += y.length;
      const d = new TextDecoder().decode(s), u = JSON.parse(d);
      return { diagram: u.d, source: u.s ?? "" };
    }
    if (e === "r") {
      const i = atob(r), o = JSON.parse(i);
      return { diagram: o.d, source: o.s ?? "" };
    }
  } catch {
  }
  return null;
}
function oo() {
  return new URLSearchParams(window.location.search).get("embed") === "true";
}
async function lo(t, e) {
  const r = await Jr(t, e), i = new URL(window.location.href);
  return i.hash = r, i.searchParams.delete("embed"), i.toString();
}
async function co(t, e) {
  const r = await Jr(t, e), i = new URL(window.location.href);
  return i.hash = r, i.searchParams.set("embed", "true"), i.toString();
}
export {
  Rn as BoardCanvas,
  me as COMPONENT_REGISTRY,
  Qn as CodeEditor,
  Yn as ComponentPalette,
  eo as EXAMPLE_SKETCHES,
  Kn as EditorCanvas,
  Gn as InstructionTrace,
  Xr as Led,
  Hn as MemoryInspector,
  Jn as PropertyPanel,
  zn as PushButton,
  Bn as RegisterGrid,
  jn as SerialMonitor,
  Ln as SimControls,
  et as SimulatorBridge,
  ui as WireLayer,
  Zn as compileCode,
  mi as createEmptyDiagram,
  no as decodeProject,
  Xn as diagramToConfig,
  Jr as encodeProject,
  vi as findPinFunction,
  co as generateEmbedUrl,
  lo as generateShareUrl,
  fi as getComponentsByCategory,
  wi as getPinMapping,
  oo as isEmbedMode,
  gi as nextWireColor,
  io as resumeAudio,
  Mr as routeWire,
  to as startTone,
  ro as stopTone,
  qn as useEditorState,
  Vn as useSimulationLoop,
  Un as useSimulator
};
