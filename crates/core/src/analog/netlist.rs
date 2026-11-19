// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Parser for the SPICE subset the in-core analog engine solves.
//!
//! The subset is deliberately small, because everything in it has to stay
//! deterministic in the browser with no dependencies. Unsupported elements
//! and circuit constructs such as AC/DC sweeps are not approximated or
//! silently dropped: they are a hard error that names the line and points at
//! the ngspice adapter, which supports a much larger SPICE subset.
//!
//! Model libraries (`.include <file>`, `.lib <file> <section>`) are read by
//! [`parse_netlist_with_files`], which the co-simulation adapter uses with
//! the manifest's directory; [`parse_netlist`] has no files and refuses them.
//!
//! A line starting with `+` continues the previous line, with comment lines
//! between them skipped, so a vendor model card split over many lines reads
//! as one card. A `.subckt <name> <port> ...` … `.ends` definition built from
//! the elements below is flattened at each `X` call that names it, with
//! ngspice's names (`R.X1.R1`, node `X1.mid`). Nested definitions are
//! refused.
//!
//! `.param <name>=<value> ...` and `{expression}` values follow ngspice's
//! numparam: global `.param` lines resolve in dependency order, a `.subckt`
//! line may declare `params:` defaults that a call overrides with
//! `<name>=<value>`, a body may hold its own `.param` lines, and a body sees
//! the parameters of the call that placed it. Every `{...}` is replaced by
//! its number before the line is parsed; the expression subset is listed in
//! `numparam.rs`. A `.model` inside a body that uses that body's parameters
//! is refused, because ngspice gives each instance its own card.
//!
//! ## Elements
//!
//! | Element | Line |
//! |---|---|
//! | Resistor | `R<name> n1 n2 <value> [beta=<K> [tnom=<°C>] [temp=<°C>]] [tc1=<1/°C> [tc2=<1/°C²>] [tnom=<°C>] [temp=<°C>]` |
//! | Capacitor | `C<name> n1 n2 <value> [ic=<v>]` |
//! | Inductor | `L<name> n1 n2 <value> [ic=<i>]` |
//! | Mutual inductance | `K<name> L<name1> L<name2> <k>` |
//! | Voltage source | `V<name> n+ n- <source>` |
//! | Current source | `I<name> n+ n- <source>` |
//! | Switch | `S<name> n1 n2 <ctrl> ron=<r> roff=<r>` or `S<name> n1 n2 nc+ nc- vt=<v> [vh=<v>] [invert=0\|1] ron=<r> roff=<r>` |
//! | Voltage switch | `S<name> n1 n2 nc+ nc- vt=<v> [vh=<v>] [invert=0|1] ron=<r> roff=<r>` |
//! | SPICE voltage switch | `S<name> n+ n- nc+ nc- <model> [on\|off]` with `.model <model> SW(VT= VH= RON= ROFF=)` |
//! | SPICE current switch | `W<name> n+ n- <element> <model> [on\|off]` with `.model <model> CSW(IT= IH= RON= ROFF=)` |
//! | Diode | `D<name> n+ n- <model> [<area>] [pj=<perimeter>] [off]` |
//! | BJT | `Q<name> nc nb ne [ns] <model> [temp=<°C>] [off]` |
//! | MOSFET | `M<name> nd ng ns nb <model> [w=<m>] [l=<m>] [ad=<m²>] [as=<m²>] [pd=<m>] [ps=<m>] [nrd=<squares>] [nrs=<squares>] [off]` |
//! | JFET | `J<name> nd ng ns <model> [temp=<°C>] [off]` |
//! | VCVS | `E<name> n+ n- nc+ nc- <gain>` |
//! | VCCS | `G<name> n+ n- nc+ nc- <transconductance>` |
//! | CCCS | `F<name> n+ n- <vsource> <gain>` |
//! | CCVS | `H<name> n+ n- <vsource> <transresistance>` |
//! | Behavioural source | `B<name> n+ n- V=<expr>` or `B<name> n+ n- I=<expr>` |
//! | Behavioural VCVS / VCCS | `E<name> n+ n- VALUE={<expr>}` / `G<name> n+ n- VALUE={<expr>}` |
//! | Polynomial E / G / F / H | `E<name> n+ n- POLY(<n>) <n node pairs> <p0> <p1> ...`, `F<name> n+ n- POLY(<n>) <n elements> <p0> ...` |
//! | Table E / G | `E<name> n+ n- TABLE {<expr>} = (x1, y1) (x2, y2) ...` |
//! | Laplace E / G | `E<name> n+ n- LAPLACE {<expr>} = {<H(s)>}` or `E<name> n+ n- nc+ nc- LAPLACE=<H(s)>` |
//! | Op-amp / comparator | `X<name> in+ in- v+ v- out <model>` |
//!
//! A semiconductor's bare `off` flag is SPICE's: the operating point holds
//! that device cut off until the rest of the circuit converges, then releases
//! it. It picks one state of a latch or other bistable circuit and changes
//! nothing where the operating point is unique. Instance `ic=` on a diode
//! (`ic=<vd>`), BJT (`ic=<vbe>,<vce>`) or JFET (`ic=<vds>,<vgs>`) is SPICE's:
//! under `uic` the junction charge history starts from those voltages, as in
//! ngspice's `MODEINITTRAN`, and without `uic` it is ignored. MOSFET
//! `ic=<vds>[,<vgs>[,<vbs>]]` seeds the bulk-junction, overlap and Meyer
//! charges the same way; a value left out comes from the terminal nodes.
//!
//! `.tran ... uic` skips the operating point, as SPICE does: every node starts
//! at 0 V unless `.ic` sets it, a capacitor starts at its `ic=` (else the
//! voltage across its nodes), and an inductor at its `ic=` (else 0 A).
//!
//! `.temp <°C>` is the default `TEMP` of every `D`, `Q`, `J` and `M` instance
//! and of every `beta`/`tc1`/`tc2` resistor that has no `temp=` of its own, so
//! each takes the temperature laws below exactly as an explicit `temp=` would.
//! Only one value is accepted: ngspice reruns the analysis once per listed
//! temperature. `.options temp=` is the same value spelled as an option; giving
//! both is refused. `.options tnom=<°C>` is ngspice's `CKTnomTemp`: the `TNOM`
//! of every `D`, `NPN`/`PNP`, `NMOS`/`PMOS` and `NJF`/`PJF` card that does not
//! set its own, and the `tnom` of every `tc1`/`tc2` resistor without one. A
//! deck with `tnom` but no `.temp` runs at ngspice's default 27 °C. Built-in
//! parts keep `TNOM` 27 °C, where their parameters were fitted, and an NTC
//! `beta` keeps its 25 °C datasheet nominal. Without `.temp` or `tnom`, an
//! instance without `temp=` keeps the historical fixed 300 K thermal voltage
//! and nominal parameters.
//!
//! The four dependent sources are SPICE's linear ones, plus `E`/`G`
//! `VALUE={<expr>}`, which is a behavioural source, and SPICE2's
//! `POLY(n)` form of all four, which lowers to one (see below), and the
//! `E`/`G` `TABLE {<expr>} = (x1, y1) ...` lookup, which also does (its
//! corner rounding is in [`super::behavioural`]), and the `E`/`G` `LAPLACE`
//! transfer function (below). `TABLE` on `F`/`H` (fatal in ngspice) is
//! refused by name. `E` and `H` each add a
//! branch current, numbered after every voltage source and inductor; `F` and
//! `H` read the branch current of the element they name, which must be one that
//! carries one (a `V`, `L`, `E` or `H`).
//!
//! A behavioural source (`B`, or `E`/`G` with `VALUE=`) sets a voltage or a
//! current to an expression of `V(node)`, `V(node1, node2)`, `I(<element>)`
//! and `time`; the grammar and functions are ngspice's, listed in
//! [`super::behavioural`]. A voltage form adds a branch current after every
//! `H`, so `I(B1)` and `I(E1)` read it. `{...}` groups in the expression may
//! use `.param` and subcircuit parameters; a `.subckt` body may hold
//! behavioural sources, whose `V(...)` and `I(...)` names are flattened like
//! every other node and element name.
//!
//! `E|G<name> n+ n- LAPLACE {<expr>} = {<H(s)>}` (PSpice) and
//! `E|G<name> n+ n- nc+ nc- LAPLACE=<H(s)>` (LTspice, input `V(nc+, nc-)`)
//! apply a rational transfer function to the input. `H(s)` holds numbers,
//! `s`, `pi`, `{...}` parameters, `+ - * /`, parentheses and integer powers
//! ([`Transfer::parse`]). It lowers to elements the solver already has, in
//! controllable canonical form after the frequency scaling `s = w0·p` (`w0`
//! the geometric mean of the pole magnitudes): one internal node
//! `<name>#s<k>` and one `1/w0` F capacitor to ground per pole, linear `G`
//! sources for the companion matrix, a behavioural current that feeds the
//! input into the last state, and a behavioural `E`/`G` output
//! `Σ c_k·V(<name>#s<k>) + d·input`. The operating point is therefore
//! `H(0)·input` and the transient is the exact state-space response under the
//! chosen integration rule. Refused: a numerator of higher order than the
//! denominator (it differentiates), a pole at `s = 0` (no operating point),
//! and anything not rational in `s` (`exp(-s*T)` delays, node voltages).
//! ngspice-42 has no `LAPLACE` keyword; its XSPICE `s_xfer` is the reference.
//!
//! `E|G<name> n+ n- POLY(n) nc1+ nc1- ... ncn+ ncn- p0 p1 ...` and
//! `F|H<name> n+ n- POLY(n) <elem1> ... <elemn> p0 p1 ...` are SPICE2
//! polynomial sources. A node pair may be written `(a,b)`. The polynomial
//! becomes the behavioural source `p0 + p1·x1 + ... + pn·xn + pn+1·x1² +
//! pn+2·x1·x2 + ...` over the controlling voltages or branch currents, with
//! the higher-degree terms in SPICE2's order. As in ngspice-42, a single
//! coefficient is the constant `p0`; SPICE2 read it as `p1`.
//!
//! A `K` element couples two inductors with signed mutual inductance
//! `M = k·sqrt(L1·L2)`. Each inductor's terminal order defines its dot
//! convention: positive `k` gives aiding voltage for currents entering both
//! first-listed terminals. Coupling lines may precede their inductors.
//!
//! `X` is SPICE's subcircuit call. An `X` line that names a `.subckt` in the
//! deck is flattened into that body. Otherwise it is accepted only when its
//! model names an `OPAMP` or `COMP` card
//! (built in, or declared with `.model`); the pin order is the one vendor
//! op-amp and comparator models use (`IN+ IN- VCC VEE OUT`). Any other `X` line
//! calls a subcircuit the deck does not define and is refused with the pointer to ngspice. The
//! element is lowered, at parse time, into the primitives above; see
//! [`OpAmpModel`] and [`ComparatorModel`] for exactly what it becomes.
//!
//! `S<name> n+ n- nc+ nc- <model> [on|off]` and `W<name> n+ n- <element>
//! <model> [on|off]` are SPICE's voltage- and current-controlled switches.
//! The `SW` card takes `VT`, `VH`, `RON`, `ROFF` and the `CSW` card `IT`,
//! `IH`, `RON`, `ROFF`, with ngspice's defaults (0, 0, 1 Ω, 1e12 Ω). The
//! switch closes when the signed control voltage `V(nc+) − V(nc-)`, or the
//! signed branch current of `<element>` (a `V`, `L`, `E` or `H`; ngspice
//! takes only a `V`), rises above threshold + hysteresis, opens below
//! threshold − hysteresis, and keeps its state in between; `on` sets the
//! state the operating point starts from. The state is read from the last
//! accepted solution: the operating point repeats until no switch moves, and
//! in a transient run an edge takes effect one step after the control
//! crosses. Two differences from ngspice-42 are deliberate. Negative
//! hysteresis is refused. `W ... on` holds closed inside the band, as `S`
//! does there; ngspice's `cswload.c` reads the band state from the previous
//! time point, which an operating point has not set, and reports it open.
//! Any other card parameter (LTspice's `LSER`, `VSER`, `ILIMIT`) is an error.
//! The `S` forms with `ron=`/`roff=` on the element line are this engine's
//! own and take no model.
//!
//! `T<name> n1+ n1- n2+ n2- Z0=<ohms> TD=<seconds>` is SPICE's lossless
//! transmission line; `F=<Hz> [NL=<wavelengths>]` gives the delay as
//! `NL / F` instead (`NL` defaults to 0.25), as in ngspice. The line is the
//! method of characteristics: each port sees `Z0` in series with the wave the
//! other port sent `TD` earlier, read from the stored solution by linear
//! interpolation, and the operating point sees two wires. Only the
//! differential mode exists, as in SPICE: no current flows between the ports'
//! reference nodes. When a step is longer than `TD` the delayed wave falls
//! inside the step and is interpolated to the step's own end point, so the
//! line stays stable but the edge is resolved no finer than the step. When
//! the delay is not a whole number of steps, interpolation widens an edge by
//! up to one step on each pass down the line; the plateaus are exact. `ic=`
//! and ngspice's `rel=`/`abs=` breakpoint controls are refused by name.
//!
//! `<source>` is `[dc] <value>`, `SIN(vo va freq [td [theta]])`,
//! `SFFM(vo va fc mdi fs)`, `EXP(v1 v2 td1 tau1 td2 tau2)`,
//! `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])`, or
//! `PWL(t1 v1 ... tn vn) [r=repeat_time] [td=delay]`, spelled as in SPICE. A
//! source that carries a function is driven by the clock, so it cannot also be
//! a routed input — [`super::adapter`] rejects a manifest that wires one.
//!
//! A resistor can use either an NTC `beta` law or the instance coefficients
//! `tc1`/`tc2`, not both. The polynomial form is
//! `R = Rnom · (1 + TC1·ΔT + TC2·ΔT²)`; `tnom` and `temp` default to 27 °C for
//! that form, or to `.options tnom=` and `.temp`. The temperature is imposed externally; self-heating is not
//! modelled.
//!
//! ## Models
//!
//! `.model <name> D|NPN|PNP|NMOS|PMOS|NJF|PJF (<param>=<value> ...)` declares a model
//! card; parameters may also be written without the parentheses, and a card may
//! be declared after the elements that use it. Parameters this engine does not
//! model (except diode forward/reverse knee currents `IK`/`IKF`/`IKR`, diode
//! depletion capacitance `CJO`/`VJ`/`M`/`FC`, diffusion charge `TT` and
//! soft-recovery parameter `VP`, diode sidewall saturation current
//! `JSW`/`ISW` with emission `NS` and perimeter `PJ`, diode sidewall depletion
//! capacitance `CJSW`/`CJP` with `PHP`/`MJSW`/`FCS`, diode transit-time temperature
//! coefficients `TTT1`/`TTT2`, diode grading-coefficient temperature
//! coefficients `TM1`/`TM2`, forward/reverse BJT transit-time temperature
//! coefficients `TTF1`/`TTF2`/`TTR1`/`TTR2`, JFET terminal
//! resistances `RD`/`RS`, the Parker–Skellern channel fit `B`, and
//! gate-junction depletion capacitance `CGS`/`CGD`/`PB`/`FC`,
//! MOSFET terminal resistances `RD`/`RS` or `RSH` times instance `NRD`/`NRS`,
//! level-1 channel charge from `TOX`, gate overlap capacitance
//! `CGSO`/`CGDO`/`CGBO`, BJT base-junction depletion capacitance
//! `CJE`/`VJE`/`MJE` and `CJC`/`VJC`/`MJC` with shared `FC`,
//! including the `XCJC` split between the internal and external base,
//! MOSFET bulk junction depletion capacitance `CBD`/`CBS` with shared
//! `PB`/`MJ`/`FC`, or junction geometry `CJ`/`CJSW`/`MJ`/`MJSW` with instance
//! `AD`/`AS`/`PD`/`PS`, semiconductor temperature coefficients beyond the diode,
//! BJT, and JFET laws below,
//! and most terminal resistances, …) are **accepted and ignored rather than
//! rejected**. A diode instance's `TEMP` and model `TNOM`/`EG`/`XTI` adjust
//! its thermal voltage and saturation currents; `TCV`/`TBV1` adjusts the
//! breakdown voltage; `TRS1`/`TRS2` adjust series resistance, `TTT1`/`TTT2`
//! adjust transit time, and `TM1`/`TM2` adjust the depletion grading
//! coefficient; `CJO` and `VJ` follow the diode junction-capacitance
//! temperature law. Diode `JTUN`·area + `JTUNSW`·`PJ` add the trap-assisted
//! tunnel current `−JT·(exp(−V/(NTUN·Vt)) − 1)` (`NTUN` default 30); with
//! `TEMP`, `JT` follows the saturation-current law with `XTITUN` (default 3)
//! and `KEG·EG` (`KEG` default 1). Diode `TLEV` is accepted and ignored:
//! ngspice-42 gives identical results for 0, 1 and 2. A BJT
//! instance's `TEMP` uses model `TNOM`/`EG`/
//! `XTI`/`XTB` to adjust thermal voltage, transport and recombination
//! saturation currents, and forward/reverse beta. BJT `CJE`/`VJE`/`MJE` and
//! `CJC`/`VJC`/`MJC` follow the default silicon temperature law; `XCJC` assigns
//! the given fraction of `CJC` to the internal base and the remainder to the
//! external base when `RB` creates both nodes. `TMJE1`/`2`
//! and `TMJC1`/`2` adjust the grading factors. `TLEVC=1` selects the linear
//! `CTE`/`CTC` and `TVJE`/`TVJC` law. With `TEMP`, `TRB1`/`TRB2`, `TRBM1`/`TRBM2`,
//! `TIRB1`/`TIRB2`, `TRC1`/`TRC2`, and `TRE1`/`TRE2` adjust base,
//! minimum-base, base-current knee, collector, and emitter resistances using
//! `ΔT = TEMP − TNOM`; `TRM1`/`TRM2` are accepted aliases for `TRBM1`/`TRBM2`.
//! `TTF1`/`TTF2` and `TTR1`/`TTR2` apply the same polynomial to forward and
//! reverse transit times. `TITF1`/`TITF2` apply it to `ITF`, which shapes the
//! forward `XTF` transit-charge current adjustment. Without instance `TEMP`,
//! these parameters retain their nominal values.
//! A JFET instance `TEMP` adjusts level-1 `VTO` by `VTOTC` when supplied, or
//! otherwise by `-TCV`; it adjusts `BETA` by `1.01^(BETATCE·ΔT)` when
//! `BETATCE` is supplied, or otherwise by `(T/TNOM)^BEX` using absolute
//! temperatures. The same `TEMP` scales gate-junction `IS` by the `EG`/`XTI`
//! saturation-current law (defaults 1.11 eV and 3), moves `PB` and `CGS`/`CGD`
//! by the diode depletion law with grading 0.5, and evaluates the gate diodes
//! at the instance thermal voltage. Without instance `TEMP`, the historical
//! channel and gate parameters are retained.
//! OPAMP and COMP `IBIAS`/`IOS` model the
//! average and signed difference of DC currents at the two input pins; `RIN`
//! models fixed differential resistance between them, and `RINCM` models an
//! equal fixed resistance from each input to `V−`; `CMRR` converts common-mode
//! input voltage relative to the supply midpoint into input-referred
//! differential voltage;
//! `CINCM` models common-mode capacitance from each input to `V−`, `CIND`
//! models differential capacitance between the inputs, `IQ` models a
//! constant quiescent current from `V+` to `V−`, OPAMP `IOLIM` clamps the
//! output current symmetrically, and OPAMP `SR` caps the gain-node capacitor
//! current for large-signal slew. BJT
//! high-injection rolloff from `IKF`/`IKR` with optional `NKF` (default
//! exponent 0.5), base recombination from `ISE`/`NE`/`ISC`/`NC`, and transit-time
//! charge from `TF`/`TR`,
//! including the forward `XTF`/`VTF`/`ITF` adjustment, plus ohmic terminal
//! resistances `RB`/`RC`/`RE` are modelled. `RCO` enables Kull's
//! quasi-saturation model of the epitaxial collector, with `VO`, `GAMMA` and
//! the epitaxial charge `QCO`, between the collector contact behind `RC` and
//! the intrinsic collector; `QUASIMOD=1` applies its `CN`/`D`/`VG`
//! temperature laws with `TEMP`. `RBM`/`IRB` make the base
//! spreading resistance fall with conduction base current using the
//! ngspice level-1 interpolation; `TRB*`/`TRBM*`/`TIRB*` apply its temperature
//! coefficients. JFET
//! channel current uses the Parker–Skellern fit `B` (default 1), with explicit
//! instance `TEMP` laws for level-1 `VTO` and `BETA`; terminal
//! resistances `RD`/`RS` create series resistors to its intrinsic
//! drain/source nodes. MOSFET `RD`/`RS` and `RSH` multiplied by instance
//! `NRD`/`NRS` do the same; level-1 `GAMMA`/`PHI` body effect is also modelled;
//! lateral diffusion `LD` shortens the channel to `L − 2·LD` for `KP·W/L`,
//! `TOX` channel charge and `CGBO`, and a card with `TOX` but no `KP` takes
//! ngspice's process value `KP = UO·1e-4·εox/TOX` (`UO`/`U0`, default 600);
//! with `TOX`, substrate doping `NSUB` derives every omitted `PHI`, `GAMMA`
//! and `VTO` as ngspice level 1 does, using gate type `TPG` and surface-state
//! density `NSS` for the flat-band voltage;
//! MOSFET `IS` is used for the two bulk junction currents, or `JS` times
//! instance `AD`/`AS` when `JS`, `AD` and `AS` are all non-zero. `CGSO` and
//! `CGDO` are capacitance per unit width (F/m), and `CGBO` is capacitance per
//! unit length (F/m); each is multiplied by instance `W` or `L` and integrated
//! as linear overlap charge. `CBD` and `CBS` integrate nonlinear bulk junction
//! charge with the shared `PB`, `MJ`, and `FC` parameters; each explicitly
//! supplied total capacitance takes precedence over `CJ` times its area.
//! Otherwise `CJ` scales by `AD`/`AS`, and `CJSW` scales independently by
//! `PD`/`PS`, using `MJSW` for sidewall grading. `TOX`, when present, enables
//! intrinsic channel charge: Meyer by default, or the engine-specific
//! Ward–Dutton terminal-charge model with `CHARGEMOD=WARD_DUTTON`. See
//! [`super::device`] for what is and is not modelled.
//!
//! An element may also name one of seven built-in cards and skip `.model`
//! entirely: `D` (1N4148-class, `IS=2.52n N=1.752`), `NPN`, `PNP` (β = 100),
//! `NMOS`, `PMOS` (`VTO=±1 V`, `KP=20u`, `LAMBDA=0.02`, bulk `IS=1e-14`),
//! `NJF` and `PJF` (level-1 defaults). Those are *this engine's* convenience
//! values and are not ngspice's parameter defaults; a
//! parameter omitted from a real `.model` line gets ngspice's default, so a
//! deck written out in full means the same thing to both engines.
//!
//! Five more built-in cards name part families, for the same reason: an
//! imported board says `1N4733A` or `LM358`, not a parameter list. They are
//! `1N4733A` (5.1 V zener), `SMAJ5.0A` (5 V TVS), `IDEAL_OPAMP`, `LM358` and
//! `LM393`; [`builtin_model`] gives their values and where each came from.
//! They are *-like* cards — the datasheet's headline numbers, not the vendor's
//! macro model.
//!
//! A diode card may carry `TNOM` (measurement temperature, default 27 °C),
//! `EG` (activation energy, default 1.11 eV), and `XTI` (saturation-current
//! temperature exponent, default 3). On a diode instance, `TEMP=<°C>` applies
//! ngspice's temperature law to the junction thermal voltage and to `IS` and
//! `ISR` (using `N` and `NR`, respectively). Without `TEMP`, the historical
//! fixed 300 K diode behavior is retained. `TCV` (also `TBV1`) applies a
//! linear coefficient to `BV` when `TEMP` is present:
//! `BV(T) = BV · (1 + TCV · (TEMP − TNOM))`. `RS` uses the polynomial
//! `RS(T) = RS · (1 + TRS1·ΔT + TRS2·ΔT²)`, where `TRS` aliases `TRS1` and
//! `ΔT = TEMP − TNOM`. `TTT1`/`TTT2` apply the SPICE polynomial to diode
//! transit time and `TM1`/`TM2` apply it to the depletion grading coefficient.
//!
//! A BJT model uses `TNOM` (or `TREF`, default 27 °C), `EG` (default 1.11 eV),
//! `XTI` (default 3), and `XTB` (default 0). On a BJT instance,
//! `TEMP=<°C>` applies ngspice's level-1 temperature laws to thermal voltage,
//! transport `IS`, base-recombination `ISE`/`ISC`, and forward/reverse beta
//! `BF`/`BR`. Without instance `TEMP`, the historical fixed-300 K behavior is
//! retained. `TRB*`/`TRM*` (also `TRBM*`)/`TIRB*`/`TRC*`/`TRE*` adjust
//! terminal resistances, and `TTF*`/`TTR*` adjust forward/reverse transit time,
//! all with `ΔT = TEMP − TNOM`. BJT junction depletion charge follows ngspice's
//! default silicon temperature law, or its `TLEVC=1` linear coefficient form.
//! The optional fourth node `ns` is the substrate (ground when omitted); it is
//! read as a node unless it names a `.model`, as ngspice does. Its junction
//! runs to the internal collector (`SUBS=1`, NPN default) or the internal base
//! (`SUBS=-1`, PNP default) with `ISS`/`NS` current and `CJS`/`VJS`/`MJS`
//! (also `CCS`/`CSUB`, `PS`, `MS`) depletion charge with no `FC`, following
//! `TMJS1`/`TMJS2`, `CTS`/`TVJS` and `TNS1`/`TNS2` with `TEMP`.
//! Self-heating is not modeled.
//!
//! A diode card may carry `IK`/`IKF` (forward knee current) and `IKR` (reverse
//! knee current) to reduce the Shockley current under high injection. It may
//! also carry `BV` (reverse breakdown voltage), `IBV` (the current
//! at `BV`, default 1 mA) and `NBV` (breakdown emission coefficient, default
//! `N`); with `BV` set the diode is solved with ngspice's three-region level-1
//! equation, and with it unset the diode is exactly what it was before
//! breakdown existed. `ISR`/`NR` add its optional recombination-current branch;
//! `CJO`, `VJ`, `M`, and `FC` add depletion charge to transient steps; `TT`
//! adds quasi-static diffusion charge, or `TT` with `VP > 0` selects the
//! first-order soft reverse-recovery model. BJT `TF`/`TR`
//! add transit-time charge based on the forward/reverse transport current;
//! `XTF`/`VTF`/`ITF` adjust the forward transit time with bias; `PTF`
//! delays the forward transport current by Weil's excess-phase filter with
//! `td = PTF·π/180·TF` in transient steps, as ngspice does;
//! `RB`/`RC`/`RE` add series resistance between the external terminals and
//! probeable internal junction nodes when nonzero. JFET
//! `CGS`/`CGD`/`PB`/`FC` add gate-junction depletion charge; fixed level-1
//! grading coefficient `M=0.5` is used. MOSFET `CGSO`/`CGDO`/`CGBO` add linear
//! gate overlap charge based on instance `W`/`L`; `TOX` enables intrinsic
//! channel charge (Meyer by default, or engine-specific Ward–Dutton with
//! `CHARGEMOD=WARD_DUTTON`). `.model <name> OPAMP(...)` and
//! `.model <name> COMP(...)` are this engine's own card types, not ngspice's.
//!
//! ## Directives and comments
//!
//! Plus `*` comments, `;`/`$` trailing comments, `.end`, `.ic V(node)=<v>` and
//! `.options`/`.print`/`.tran`/`.save`/`.control`…`.endc`, which are accepted
//! and ignored so that one deck can be handed to both this engine and ngspice.
//! `+` continuation lines and `.subckt`…`.ends` blocks are rewritten before
//! any of this, as described at the top.
//!
//! Note on the first line: a classic SPICE deck treats line 1 as a title and
//! ignores it. This parser does not, because silently dropping an element line
//! is the worst failure mode a netlist parser has. Put a `*` on your comment.

use std::collections::BTreeMap;
use std::fmt;

use super::behavioural::{
    BehaviouralKind, BehaviouralSource, Control, ControlName, Expression, Transfer,
};
use std::path::{Path, PathBuf};

/// A node reference: `None` is ground, `Some(i)` indexes [`Circuit::node_name`].
pub type NodeRef = Option<usize>;

/// Everything that can go wrong between a netlist string and a solved step.
///
/// Every parse variant carries the 1-based line number and the line's text, so
/// a manifest can report which line of which file the user has to fix.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalogError {
    /// A line is in the subset's vocabulary but malformed.
    Parse {
        /// 1-based line number in the netlist.
        line: usize,
        /// The offending line, trimmed.
        text: String,
        /// What was wrong with it.
        message: String,
    },
    /// A line names an element this engine does not model at all.
    Unsupported {
        /// 1-based line number in the netlist.
        line: usize,
        /// The offending line, trimmed.
        text: String,
    },
    /// The circuit is bigger than the in-core dense solver accepts.
    TooLarge {
        /// Nodes + branch currents the circuit needs.
        unknowns: usize,
        /// The ceiling.
        max: usize,
    },
    /// The MNA matrix has no unique solution (a floating node, a shorted
    /// voltage source, a zero-valued resistor).
    Singular {
        /// The unknown at which elimination found no pivot.
        row: usize,
    },
    /// Newton–Raphson ran out of iterations on one step of a nonlinear
    /// circuit.
    ///
    /// This is the variant that exists so a hard circuit reports *where* it
    /// gave up instead of quietly filling the trace with `NaN`: the caller
    /// gets the step, the simulated time, and the unknown that was still
    /// moving when the budget ran out.
    NoConvergence {
        /// 0-based internal step index since the solver was built, or `None`
        /// for the DC operating point.
        step: Option<u64>,
        /// Simulated time at the end of the step, in seconds.
        time: f64,
        /// Iterations attempted before giving up.
        iterations: u32,
        /// Name of the unknown with the largest residual movement.
        unknown: String,
        /// How much that unknown still moved on the last iteration.
        delta: f64,
    },
    /// A behavioural source's expression left its domain (`sqrt` of a
    /// negative number, a division by zero) at the point being solved.
    Evaluation {
        /// The behavioural element.
        element: String,
        /// Simulated time of the step, in seconds; 0 at the operating point.
        time: f64,
        /// What went wrong.
        message: String,
    },
    /// The manifest `config` block is not a usable analog configuration.
    Config(String),
}

impl AnalogError {
    /// 1-based netlist line this error is about, when it is about a line.
    pub fn line(&self) -> Option<usize> {
        match self {
            Self::Parse { line, .. } | Self::Unsupported { line, .. } => Some(*line),
            _ => None,
        }
    }

    /// The netlist line's text, when this error is about a line.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Parse { text, .. } | Self::Unsupported { text, .. } => Some(text),
            _ => None,
        }
    }
}

impl fmt::Display for AnalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse {
                line,
                text,
                message,
            } => write!(f, "netlist line {line}: {message} (in `{text}`)"),
            // The exact wording the design calls for: it has to tell the user
            // which adapter does support the line they wrote.
            Self::Unsupported { text, .. } => write!(
                f,
                "element `{text}` needs ngspice; use `adapter: external_process` \
                 with `tools/cosim/labwired_ngspice.py`"
            ),
            Self::TooLarge { unknowns, max } => write!(
                f,
                "circuit needs {unknowns} unknowns (nodes + branch currents), over the \
                 in-core limit of {max}; use `adapter: external_process` with \
                 `tools/cosim/labwired_ngspice.py`"
            ),
            Self::Singular { row } => write!(
                f,
                "circuit matrix is singular at unknown {row}: check for a floating node, \
                 a zero-ohm resistor or two voltage sources in parallel"
            ),
            Self::NoConvergence {
                step,
                time,
                iterations,
                unknown,
                delta,
            } => {
                let where_ = match step {
                    Some(step) => format!("step {step} (t = {time:e} s)"),
                    None => "the DC operating point".to_string(),
                };
                write!(
                    f,
                    "Newton iteration did not converge at {where_} after {iterations} \
                     iterations; `{unknown}` was still moving by {delta:e} per iteration. \
                     Try a smaller `substeps` interval, or use `adapter: external_process` \
                     with `tools/cosim/labwired_ngspice.py`"
                )
            }
            Self::Evaluation {
                element,
                time,
                message,
            } => write!(
                f,
                "behavioural source `{element}` at t = {time:e} s: {message}"
            ),
            Self::Config(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for AnalogError {}

/// Beta-model NTC parameters attached to an `R` element.
///
/// Temperature is an external environment input. Self-heating and thermal
/// inertia are not modelled; at a fixed temperature the part is an ordinary
/// resistor with `R = R25 · exp(B · (1/T − 1/T25))`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NtcModel {
    /// Material beta value, in kelvin.
    pub beta: f64,
    /// The Celsius temperature at which the resistor's `ohms` value applies.
    pub nominal_temperature_celsius: f64,
    /// Current external temperature input, in Celsius.
    pub temperature_celsius: f64,
}

/// Polynomial temperature coefficients on a resistor instance.
///
/// The resistor value is `Rnom · (1 + TC1·ΔT + TC2·ΔT²)`, where `ΔT` is the
/// instance temperature minus the parameter measurement temperature. This is
/// an externally specified temperature; the resistor does not self-heat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResistorTemperatureModel {
    /// First-order coefficient, per °C.
    pub tc1: f64,
    /// Second-order coefficient, per °C².
    pub tc2: f64,
    /// Parameter measurement temperature, in °C.
    pub nominal_temperature_celsius: f64,
    /// Instance temperature, in °C.
    pub temperature_celsius: f64,
}

impl ResistorTemperatureModel {
    /// Resistance at `temperature_celsius`, in ohms.
    pub fn resistance_ohms(self, nominal_ohms: f64) -> f64 {
        let delta = self.temperature_celsius - self.nominal_temperature_celsius;
        nominal_ohms * (1.0 + self.tc1 * delta + self.tc2 * delta * delta)
    }
}

impl NtcModel {
    /// Resistance at `temperature_celsius`, in ohms.
    pub fn resistance_ohms(self, r25_ohms: f64) -> f64 {
        let t = self.temperature_celsius + 273.15;
        let t25 = self.nominal_temperature_celsius + 273.15;
        r25_ohms * libm::exp(self.beta * (1.0 / t - 1.0 / t25))
    }
}

/// `R<name> n1 n2 <ohms>` with optional NTC or `tc1=`/`tc2=` temperature data.
#[derive(Debug, Clone, PartialEq)]
pub struct Resistor {
    /// Element name as written, e.g. `R1`.
    pub name: String,
    /// First terminal.
    pub a: NodeRef,
    /// Second terminal.
    pub b: NodeRef,
    /// Fixed resistance, or nominal resistance for either temperature model.
    pub ohms: f64,
    /// Optional external-temperature Beta model.
    pub ntc: Option<NtcModel>,
    /// Optional first- and second-order temperature coefficient model.
    pub temperature_coefficients: Option<ResistorTemperatureModel>,
}

impl Resistor {
    /// Resistance at the model's current external temperature, in ohms.
    pub fn resistance_ohms(&self) -> f64 {
        if let Some(ntc) = self.ntc {
            ntc.resistance_ohms(self.ohms)
        } else if let Some(model) = self.temperature_coefficients {
            model.resistance_ohms(self.ohms)
        } else {
            self.ohms
        }
    }
}

/// `C<name> n1 n2 <farads> [ic=<volts>]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Capacitor {
    /// Element name as written.
    pub name: String,
    /// Positive terminal (the one `ic=` is measured at, relative to `b`).
    pub a: NodeRef,
    /// Negative terminal.
    pub b: NodeRef,
    /// Capacitance in farads.
    pub farads: f64,
    /// Initial voltage across the element, overriding the operating point.
    pub ic: Option<f64>,
}

/// `L<name> n1 n2 <henries> [ic=<amps>]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Inductor {
    /// Element name as written.
    pub name: String,
    /// Terminal the branch current flows into.
    pub a: NodeRef,
    /// Terminal the branch current flows out of.
    pub b: NodeRef,
    /// Inductance in henries.
    pub henries: f64,
    /// Initial current `a` → `b`, overriding the operating point.
    pub ic: Option<f64>,
}

/// `K<name> L<name1> L<name2> <k>` — a pair of magnetically coupled
/// inductors. `mutual_henries` is signed, so a negative coupling coefficient
/// reverses the dot convention.
#[derive(Debug, Clone, PartialEq)]
pub struct MutualInductor {
    /// Element name as written.
    pub name: String,
    /// Index of the first coupled inductor in [`Circuit::inductors`].
    pub first: usize,
    /// Index of the second coupled inductor in [`Circuit::inductors`].
    pub second: usize,
    /// Signed mutual inductance in henries: `k·sqrt(L1·L2)`.
    pub mutual_henries: f64,
}

/// How an independent source's value depends on time.
///
/// The operating point is solved at `t = 0` with [`Waveform::at`] evaluated
/// there, which is what SPICE's `.tran` does when it is not given `uic`, so a
/// deck starts from the same state in both engines.
#[derive(Debug, Clone, PartialEq)]
pub enum Waveform {
    /// A constant. This is the only kind a routed input may drive.
    Dc(f64),
    /// `SIN(vo va freq [td [theta]])`:
    /// `vo + va·exp(−(t−td)·theta)·sin(2π·freq·(t−td))` after `td`, `vo`
    /// before it.
    Sin {
        /// Offset, volts or amps.
        offset: f64,
        /// Peak amplitude.
        amplitude: f64,
        /// Frequency in hertz.
        frequency: f64,
        /// Delay before the sine starts, seconds.
        delay: f64,
        /// Exponential damping factor, 1/s.
        theta: f64,
    },
    /// `SFFM(vo va fc mdi fs)`: `vo + va·sin(2π·fc·t + mdi·sin(2π·fs·t))`,
    /// with carrier frequency `fc`, modulating frequency `fs`, and modulation
    /// index `mdi`.
    Sffm {
        /// Offset, volts or amps.
        offset: f64,
        /// Peak amplitude.
        amplitude: f64,
        /// Carrier frequency in hertz.
        carrier_frequency: f64,
        /// Modulation index.
        modulation_index: f64,
        /// Modulating frequency in hertz.
        signal_frequency: f64,
    },
    /// `EXP(v1 v2 td1 tau1 td2 tau2)` — exponential rise toward `v2` after
    /// `td1`, then a second exponential term returns toward `v1` after `td2`.
    Exp {
        /// Initial value before the rise delay.
        v1: f64,
        /// Rise target.
        v2: f64,
        /// Rise delay, seconds.
        td1: f64,
        /// Rise time constant, seconds.
        tau1: f64,
        /// Fall delay, seconds.
        td2: f64,
        /// Fall time constant, seconds.
        tau2: f64,
    },
    /// `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])` — a trapezoidal pulse train.
    Pulse {
        /// Initial (and resting) value.
        v1: f64,
        /// Pulsed value.
        v2: f64,
        /// Delay before the first edge, seconds.
        delay: f64,
        /// Rise time, seconds.
        rise: f64,
        /// Fall time, seconds.
        fall: f64,
        /// Time held at `v2`, seconds, not counting the edges.
        width: f64,
        /// Period, seconds.
        period: f64,
    },
    /// `PWL(t1 v1 ... tn vn) [r=repeat_time] [td=delay]` — piecewise-linear
    /// interpolation, optionally repeating the suffix that starts at a time
    /// point in the sequence.
    Pwl {
        /// Ordered `(time, value)` pairs. Values before the first point and
        /// after the last point hold the nearest endpoint.
        points: Vec<(f64, f64)>,
        /// Time-point suffix to repeat, if any.
        repeat_time: Option<f64>,
        /// Delay applied to the entire time sequence.
        delay: f64,
    },
}

impl Waveform {
    /// Value at `t` seconds.
    ///
    /// Deterministic: `sin`, `exp` and `floor` come from `libm` rather than
    /// from the platform's libm, for the reason [`super::device`] gives.
    pub fn at(&self, t: f64) -> f64 {
        match self {
            Self::Dc(value) => *value,
            Self::Sin {
                offset,
                amplitude,
                frequency,
                delay,
                theta,
            } => {
                if t <= *delay {
                    *offset
                } else {
                    let elapsed = t - *delay;
                    let envelope = if *theta == 0.0 {
                        1.0
                    } else {
                        libm::exp(-elapsed * *theta)
                    };
                    *offset
                        + *amplitude
                            * envelope
                            * libm::sin(2.0 * core::f64::consts::PI * *frequency * elapsed)
                }
            }
            Self::Sffm {
                offset,
                amplitude,
                carrier_frequency,
                modulation_index,
                signal_frequency,
            } => {
                let modulation = *modulation_index
                    * libm::sin(2.0 * core::f64::consts::PI * *signal_frequency * t);
                *offset
                    + *amplitude
                        * libm::sin(
                            2.0 * core::f64::consts::PI * *carrier_frequency * t + modulation,
                        )
            }
            Self::Exp {
                v1,
                v2,
                td1,
                tau1,
                td2,
                tau2,
            } => {
                if t < *td1 {
                    return *v1;
                }
                let rise = (*v2 - *v1) * (1.0 - libm::exp(-(t - *td1) / *tau1));
                if t < *td2 {
                    *v1 + rise
                } else {
                    let fall = (*v1 - *v2) * (1.0 - libm::exp(-(t - *td2) / *tau2));
                    *v1 + rise + fall
                }
            }
            Self::Pulse {
                v1,
                v2,
                delay,
                rise,
                fall,
                width,
                period,
            } => {
                if t < *delay {
                    return *v1;
                }
                let mut phase = t - *delay;
                if *period > 0.0 && phase >= *period {
                    phase -= libm::floor(phase / *period) * *period;
                }
                if *rise > 0.0 && phase < *rise {
                    *v1 + (*v2 - *v1) * (phase / *rise)
                } else if phase < *rise + *width {
                    *v2
                } else if *fall > 0.0 && phase < *rise + *width + *fall {
                    *v2 + (*v1 - *v2) * ((phase - *rise - *width) / *fall)
                } else {
                    *v1
                }
            }
            Self::Pwl {
                points,
                repeat_time,
                delay,
            } => {
                let Some(first) = points.first().copied() else {
                    return 0.0;
                };
                let Some(last) = points.last().copied() else {
                    return 0.0;
                };
                if t < *delay {
                    return first.1;
                }
                let mut elapsed = t - *delay;
                if let Some(repeat_time) = repeat_time {
                    let period = last.0 - *repeat_time;
                    if elapsed >= *repeat_time && period > 0.0 {
                        let after_repeat = elapsed - *repeat_time;
                        let cycles = libm::floor(after_repeat / period);
                        elapsed = *repeat_time + (after_repeat - cycles * period);
                    }
                }
                if elapsed <= first.0 {
                    return first.1;
                }
                if elapsed >= last.0 {
                    return last.1;
                }
                let right = points.partition_point(|(time, _)| *time < elapsed);
                let (left_time, left_value) = points[right - 1];
                let (right_time, right_value) = points[right];
                left_value
                    + (right_value - left_value) * (elapsed - left_time) / (right_time - left_time)
            }
        }
    }

    /// True when a routed input may drive this source.
    pub fn is_constant(&self) -> bool {
        matches!(self, Self::Dc(_))
    }
}

/// `V<name> n+ n- <source>`.
#[derive(Debug, Clone, PartialEq)]
pub struct VoltageSource {
    /// Element name as written, e.g. `Vgpio`.
    pub name: String,
    /// Positive terminal.
    pub p: NodeRef,
    /// Negative terminal.
    pub n: NodeRef,
    /// Value at `t = 0`; routed inputs replace it once time runs.
    pub dc: f64,
    /// Time dependence. `Dc` sources are the ones a routed input may drive.
    pub wave: Waveform,
}

/// `I<name> n+ n- <source>`. Positive current flows from `p` through the
/// source to `n`, as in SPICE.
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentSource {
    /// Element name as written.
    pub name: String,
    /// Positive terminal.
    pub p: NodeRef,
    /// Negative terminal.
    pub n: NodeRef,
    /// Value at `t = 0`; routed inputs replace it once time runs.
    pub dc: f64,
    /// Time dependence. `Dc` sources are the ones a routed input may drive.
    pub wave: Waveform,
}

/// Which way round a three-terminal device's junctions face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// NPN or N-channel: the device conducts for positive controlling
    /// voltages, and its sign factor is `+1`.
    N,
    /// PNP or P-channel: every terminal voltage and current is mirrored, and
    /// its sign factor is `−1`.
    P,
}

impl Polarity {
    /// `+1.0` for [`Polarity::N`], `−1.0` for [`Polarity::P`] — the factor
    /// that turns circuit coordinates into device coordinates.
    pub fn sign(self) -> f64 {
        match self {
            Self::N => 1.0,
            Self::P => -1.0,
        }
    }
}

/// Resolved parameters of a `.model <name> D(...)` card.
///
/// Defaults are ngspice's, so a `.model` line that omits a parameter means the
/// same thing to both engines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiodeModel {
    /// Saturation current `IS`, amps.
    pub is: f64,
    /// Emission coefficient `N`.
    pub n: f64,
    /// Forward high-injection knee current `IK` (also called `IKF`), amps.
    /// Zero disables the correction, matching ngspice's default.
    pub ik: f64,
    /// Reverse high-injection knee current `IKR`, amps. Zero disables the
    /// correction, matching ngspice's default.
    pub ikr: f64,
    /// Recombination saturation current `ISR`, amps. Zero means the optional
    /// ngspice recombination branch is disabled, matching an omitted `ISR`.
    pub isr: f64,
    /// Recombination emission coefficient `NR`, default 2 in ngspice 47.
    pub nr: f64,
    /// Ohmic series resistance `RS`. Non-zero adds one internal node.
    pub rs: f64,
    /// Forward transit time `TT`, seconds. Zero disables diffusion charge,
    /// matching ngspice's default.
    pub tt: f64,
    /// First and second order temperature coefficients for `TT`, per °C and
    /// per °C². They apply when an instance `TEMP` is present.
    pub ttt1: f64,
    pub ttt2: f64,
    /// First and second order temperature coefficients for the depletion
    /// grading coefficient `M`, per °C and per °C². They apply with instance
    /// `TEMP`.
    pub tm1: f64,
    pub tm2: f64,
    /// Soft reverse-recovery coefficient `VP`. Positive `VP` with positive
    /// `TT` replaces quasi-static diffusion charge with a first-order charge
    /// state; zero keeps `Qd = TT·Id`.
    pub vp: f64,
    /// Reverse breakdown voltage `BV`, volts, positive. `None` is a diode
    /// with no breakdown at all, solved exactly as before `BV` existed.
    pub bv: Option<f64>,
    /// Current at the breakdown knee `IBV`, amps. ngspice-47's default, 1 mA.
    pub ibv: f64,
    /// Breakdown emission coefficient `NBV`. `None` means "same as `N`",
    /// which is ngspice's default.
    pub nbv: Option<f64>,
    /// Zero-bias junction capacitance `CJO`, farads. Zero disables depletion
    /// charge, matching ngspice's default.
    pub cjo: f64,
    /// Junction potential `VJ`, volts, default 1 V.
    pub vj: f64,
    /// Junction grading coefficient `M`, default 0.5.
    pub m: f64,
    /// Forward-bias depletion-capacitance transition coefficient `FC`, default
    /// 0.5.
    pub fc: f64,
    /// Activation energy `EG` for the saturation-current temperature law, eV.
    pub eg: f64,
    /// Saturation-current temperature exponent `XTI`.
    pub xti: f64,
    /// Parameter measurement temperature `TNOM`, in Celsius.
    pub tnom_celsius: f64,
    /// Breakdown-voltage temperature coefficient `TCV`/`TBV1`, per Celsius
    /// degree. Applies when an instance `TEMP` is present.
    pub tcv: f64,
    /// First-order series-resistance temperature coefficient `TRS1`/`TRS`,
    /// per Celsius degree. Applies when an instance `TEMP` is present.
    pub trs1: f64,
    /// Second-order series-resistance temperature coefficient `TRS2`, per
    /// Celsius degree squared. Applies when an instance `TEMP` is present.
    pub trs2: f64,
    /// Sidewall saturation current `JSW`/`ISW`, amps per unit of perimeter
    /// `PJ`. Zero (the default) disables the sidewall junction. On a bound
    /// instance it is already multiplied by `PJ`; see [`Self::with_perimeter`].
    pub jsw: f64,
    /// Sidewall emission coefficient `NS`. `None` merges the sidewall current
    /// into the bottom junction and uses `N`, as ngspice-42 does; the
    /// temperature law still uses ngspice's default `NS = 1` then.
    pub ns: Option<f64>,
    /// Default junction perimeter `PJ` for instances that do not give `pj=`.
    /// ngspice's default is 0, so `JSW` alone adds no current.
    pub pj: f64,
    /// Zero-bias sidewall depletion capacitance `CJSW`/`CJP`, farads per unit
    /// of perimeter `PJ`. Zero (the default) disables sidewall charge. On a
    /// bound instance it is already multiplied by `PJ`.
    pub cjsw: f64,
    /// Sidewall junction potential `PHP`, volts, default 1 V.
    pub php: f64,
    /// Sidewall grading coefficient `MJSW`, default 0.33.
    pub mjsw: f64,
    /// Forward-bias sidewall capacitance transition coefficient `FCS`,
    /// default 0.5.
    pub fcs: f64,
    /// Depletion temperature law selector `TLEVC`. 0 (the default) is
    /// ngspice's silicon-bandgap law; 1 is the linear `CTA`/`CTP`/`TPB`/`TPHP`
    /// law. Both apply only with an instance `TEMP`.
    pub tlevc: u8,
    /// `TLEVC=1` linear temperature coefficient of `CJO` (`CTA`/`CTC`), per
    /// Celsius degree.
    pub cta: f64,
    /// `TLEVC=1` linear temperature coefficient of `CJSW` (`CTP`), per
    /// Celsius degree.
    pub ctp: f64,
    /// `TLEVC=1` linear temperature coefficient of `VJ` (`TPB`/`TVJ`), volts
    /// per Celsius degree; `VJ` falls by `TPB·ΔT`.
    pub tpb: f64,
    /// `TLEVC=1` linear temperature coefficient of `PHP` (`TPHP`), volts per
    /// Celsius degree; `PHP` falls by `TPHP·ΔT`.
    pub tphp: f64,
    /// Bottom tunnel saturation current `JTUN`, amps per unit area. Zero (the
    /// default) disables trap-assisted tunnelling. On a bound instance it is
    /// already multiplied by `AREA`.
    pub jtun: f64,
    /// Sidewall tunnel saturation current `JTUNSW`, amps per unit of perimeter
    /// `PJ`. On a bound instance it is already multiplied by `PJ`.
    pub jtunsw: f64,
    /// Tunnel emission coefficient `NTUN`, default 30.
    pub ntun: f64,
    /// Tunnel saturation-current temperature exponent `XTITUN`, default 3.
    pub xtitun: f64,
    /// Fraction `KEG` of `EG` in the tunnel temperature law, default 1.
    pub keg: f64,
}

impl DiodeModel {
    /// Fold the junction perimeter into the card: `JSW` and `CJSW` become the
    /// instance's total sidewall saturation current `JSW·PJ` and capacitance
    /// `CJSW·PJ`, as ngspice's `diosetup.c`/`diotemp.c` do. The instance
    /// `pj=` wins over the model `PJ`. `AREA` does not touch the sidewall.
    pub fn with_perimeter(self, instance_pj: Option<f64>) -> Self {
        let pj = instance_pj.unwrap_or(self.pj);
        Self {
            jsw: self.jsw * pj,
            cjsw: self.cjsw * pj,
            jtunsw: self.jtunsw * pj,
            pj,
            ..self
        }
    }

    /// The card one instance with SPICE `AREA` sees: the currents `IS`,
    /// `ISR`, `IK`, `IKR` and the capacitance `CJO` scale with the junction
    /// area and `RS` divides by it, as ngspice's `dioload.c` and `diotemp.c`
    /// do. `IBV` stays as written: ngspice-45 fits the breakdown knee to the
    /// unscaled `IBV`, so a larger area moves `BV` slightly. Unlike the
    /// instance multiplier `m`, GMIN is untouched. `area == 1` returns the
    /// card bit-identical.
    pub fn with_area(self, area: f64) -> Self {
        Self {
            is: self.is * area,
            isr: self.isr * area,
            ik: self.ik * area,
            ikr: self.ikr * area,
            cjo: self.cjo * area,
            jtun: self.jtun * area,
            rs: self.rs / area,
            ..self
        }
    }

    /// Series resistance at the instance temperature, in ohms.
    ///
    /// With no instance temperature, retain the engine's historical fixed
    /// 300 K behavior and nominal resistance. With `TEMP`, apply ngspice's
    /// quadratic `TRS1`/`TRS2` law measured from `TNOM`.
    pub fn series_resistance_ohms(self, temperature_celsius: Option<f64>) -> f64 {
        let Some(temperature_celsius) = temperature_celsius else {
            return self.rs;
        };
        let delta = temperature_celsius - self.tnom_celsius;
        self.rs * (1.0 + self.trs1 * delta + self.trs2 * delta * delta)
    }

    /// Transit time at the instance temperature, in seconds.
    ///
    /// With no instance temperature, retain the engine's historical fixed
    /// behavior. With `TEMP`, apply ngspice's quadratic `TTT1`/`TTT2` law
    /// measured from `TNOM`.
    pub fn transit_time_seconds(self, temperature_celsius: Option<f64>) -> f64 {
        let Some(temperature_celsius) = temperature_celsius else {
            return self.tt;
        };
        let delta = temperature_celsius - self.tnom_celsius;
        self.tt * (1.0 + self.ttt1 * delta + self.ttt2 * delta * delta)
    }

    /// Depletion grading coefficient `M(T)` using the SPICE `TM1`/`TM2` law.
    pub fn grading_coefficient_at_temperature(self, temperature_celsius: Option<f64>) -> f64 {
        let Some(temperature_celsius) = temperature_celsius else {
            return self.m;
        };
        let delta = temperature_celsius - self.tnom_celsius;
        self.m * (1.0 + self.tm1 * delta + self.tm2 * delta * delta)
    }
}

impl Default for DiodeModel {
    fn default() -> Self {
        Self {
            is: 1e-14,
            n: 1.0,
            ik: 0.0,
            ikr: 0.0,
            isr: 0.0,
            nr: 2.0,
            rs: 0.0,
            tt: 0.0,
            ttt1: 0.0,
            ttt2: 0.0,
            tm1: 0.0,
            tm2: 0.0,
            vp: 0.0,
            bv: None,
            ibv: 1e-3,
            nbv: None,
            cjo: 0.0,
            vj: 1.0,
            m: 0.5,
            fc: 0.5,
            eg: 1.11,
            xti: 3.0,
            tnom_celsius: 27.0,
            tcv: 0.0,
            trs1: 0.0,
            trs2: 0.0,
            jsw: 0.0,
            ns: None,
            pj: 0.0,
            cjsw: 0.0,
            php: 1.0,
            mjsw: 0.33,
            fcs: 0.5,
            tlevc: 0,
            cta: 0.0,
            ctp: 0.0,
            tpb: 0.0,
            tphp: 0.0,
            jtun: 0.0,
            jtunsw: 0.0,
            ntun: 30.0,
            xtitun: 3.0,
            keg: 1.0,
        }
    }
}

/// How a comparator's output stage drives its pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparatorOutput {
    /// Open collector / open drain: pulls `out` down to `v-` through `ROUT`
    /// when `in+ < in-`, and lets it float otherwise. Needs a pull-up. This is
    /// the LM393/LM339 output.
    OpenCollector,
    /// Push-pull: drives `out` to either rail through `ROUT`.
    PushPull,
}

/// Resolved parameters of a `.model <name> OPAMP(...)` card.
///
/// The element is a voltage-feedback op-amp macro, lowered into primitives:
///
/// ```text
///   gain node p:  G  = AOL/Rg · (v(in+) − v(in−) + VOS)  into p
///                 Rg = 1 MΩ   and   Cg = AOL/(2π·GBW·Rg)   from p to ground
///                 optional SR limits the Cg current to ±(Cg·SR)
///   clamps:       D  p → (v+ − DROP_HI)      D  (v− + DROP_LO) → p
///   output:       Thevenin v(p) behind ROUT, i.e.  G = 1/ROUT into out,
///                 R = ROUT from out to ground; optional IOLIM clamps the
///                 current once |v(p) − v(out)| / ROUT reaches the limit
/// ```
///
/// So the DC gain is `AOL`, the single pole sits at `GBW/AOL`, the unity-gain
/// frequency is `GBW`, and the output cannot leave `[v− + DROP_LO, v+ −
/// DROP_HI]` by more than one clamp-diode drop (a few mV — the clamp diode's
/// emission coefficient is 0.02, so its knee is 50× sharper than silicon's).
/// The clamps are real diodes solved with SPICE's `pnjlim`, which is what lets
/// Newton walk a saturated output back into the linear region. The rails come
/// from the `v+`/`v−` pins, so the same card works at ±15 V and at 3.3 V.
///
/// `IBIAS` is the average signed DC current into `v−`, and `IOS` is the signed
/// difference `I(+) − I(−)`. The input currents are `IBIAS ± IOS/2`; negative
/// values source current from `v−` into the inputs. This is a constant-current
/// approximation, with no input bias temperature dependence.
///
/// `RIN` is a fixed differential resistance between the inputs. Zero disables
/// it; a positive value draws `(v(in+) − v(in−))/RIN` from the non-inverting
/// pin and returns it to the inverting pin. `RINCM` is one fixed resistance
/// from each input to `v−`; zero disables those paths. Neither has temperature
/// dependence.
///
/// `CMRR`, when present, converts common-mode input voltage relative to the
/// supply midpoint into input-referred differential voltage using
/// `10^(−CMRR/20)`. Omission gives ideal common-mode rejection.
///
/// `IOLIM` is an optional, symmetric output-current limit. Below the limit the
/// output follows the normal Thevenin source and `ROUT`; above it, the output
/// current is held at `±IOLIM`. Omission leaves the output stage unlimited.
///
/// `SR` (volts/second) optionally caps the gain-node capacitor current to
/// `Cg · SR`, where `Cg = AOL/(2π·GBW·Rg)`. A large input step therefore
/// cannot move the gain node faster than the selected slew rate. The limit
/// acts on transient capacitor current only; it preserves settled DC gain.
/// Omission preserves the linear one-pole response.
///
/// `CINCM` is the common-mode capacitance from each input to `v−`; `CIND` is
/// the differential capacitance between the inputs. Both are fixed linear
/// capacitances with no voltage or temperature dependence. `IQ` is a fixed
/// quiescent current from `v+` to `v−`, independent of signal and output load.
///
/// What it does not model: signal- or load-dependent supply current. The
/// output stage's current returns through ground, not through the supply pins.
/// Clamp current reaches `v+`/`v−`; input bias current reaches `v−`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpAmpModel {
    /// DC open-loop gain `AOL`, V/V.
    pub aol: f64,
    /// Gain-bandwidth product `GBW`, Hz.
    pub gbw: f64,
    /// Output resistance `ROUT`, ohms.
    pub rout: f64,
    /// Input offset voltage `VOS`, volts, added to `v(in+) − v(in−)`.
    pub vos: f64,
    /// Average signed DC bias current, amps; positive current flows from the
    /// input pins into `v−`.
    pub ibias: f64,
    /// Signed input offset current `I(+) − I(−)`, amps.
    pub ios: f64,
    /// Differential resistance between the inputs, ohms; zero disables it.
    pub rin: f64,
    /// Common-mode resistance from each input to `v−`, ohms; zero disables it.
    pub rin_cm: f64,
    /// Common-mode input capacitance from each input to `v−`, farads.
    pub cin_cm: f64,
    /// Differential capacitance between the two inputs, farads.
    pub cin_diff: f64,
    /// Quiescent current drawn from `v+` to `v−`, amps.
    pub iq: f64,
    /// Common-mode rejection ratio in dB. `None` means ideal rejection.
    pub cmrr: Option<f64>,
    /// Symmetric output current limit, amps. `None` leaves the output stage
    /// unlimited, preserving the ordinary Thevenin `ROUT` behavior.
    pub ilim: Option<f64>,
    /// Large-signal output slew rate `SR`, volts per second. `None` leaves the
    /// gain-node capacitor current unlimited, preserving the one-pole response.
    pub sr: Option<f64>,
    /// `DROP_HI`: how far below `v+` the output saturates, volts.
    pub drop_hi: f64,
    /// `DROP_LO`: how far above `v−` the output saturates, volts.
    pub drop_lo: f64,
}

impl Default for OpAmpModel {
    fn default() -> Self {
        Self {
            aol: 1e5,
            gbw: 1e6,
            rout: 100.0,
            vos: 0.0,
            ibias: 0.0,
            ios: 0.0,
            rin: 0.0,
            rin_cm: 0.0,
            cin_cm: 0.0,
            cin_diff: 0.0,
            iq: 0.0,
            cmrr: None,
            ilim: None,
            sr: None,
            drop_hi: 0.0,
            drop_lo: 0.0,
        }
    }
}

/// Resolved parameters of a `.model <name> COMP(...)` card.
///
/// A comparator is an op-amp with no compensation, a decision node of fixed
/// 1 V span, and an output stage:
///
/// ```text
///   decision node q (referenced to v−):
///       G  = AOL/Rq · (v(in−) − v(in+) − VOS + VHYS·(v(q) − v(v−) − ½))  into q
///       Rq = 1 MΩ,  Cq = AOL/(2π·GBW·Rq),  diode-clamped to [v−, v− + 1 V]
///   OUTPUT=OC:  an N-channel switch from out to v−, gate q, VTO = 0.5 V,
///               on-resistance ROUT at full drive
///   OUTPUT=PP:  a second node p = −10⁴·(v(q) − v(v−) − ½), clamped to the
///               rails, driving out through ROUT like the op-amp's output
/// ```
///
/// `VHYS` is internal positive feedback: the input thresholds sit at
/// `−VOS ± VHYS/2`, a window `VHYS` volts wide, independent of the supply.
/// `GBW` here sets the response time: the decision node slews at
/// `2π·GBW·overdrive` volts per second across its 1 V span.
///
/// `IBIAS` is the average signed DC current into `v−`, and `IOS` is the signed
/// difference `I(+) − I(−)`. The input currents are `IBIAS ± IOS/2`; negative
/// values source current from `v−` into the inputs. This constant-current
/// approximation has no temperature dependence. `CINCM` is the common-mode
/// capacitance from each input to `v−`; `CIND` is the differential capacitance
/// between the inputs. Both are fixed linear capacitances. `RIN` is a fixed
/// differential resistance between the inputs; zero disables it. `RINCM` is
/// one fixed resistance from each input to `v−`; zero disables those paths.
/// `IQ` is a fixed quiescent current from `v+` to `v−`, independent of signal
/// and output load. `CMRR`, when present, converts common-mode input voltage
/// relative to the supply midpoint into input-referred differential voltage
/// using `10^(−CMRR/20)`. Omission gives ideal common-mode rejection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComparatorModel {
    /// DC gain `AOL`, V/V.
    pub aol: f64,
    /// Gain-bandwidth product `GBW`, Hz.
    pub gbw: f64,
    /// Output on-resistance (open collector) or output resistance (push-pull),
    /// ohms.
    pub rout: f64,
    /// Input offset voltage `VOS`, volts.
    pub vos: f64,
    /// Average signed DC bias current, amps; positive current flows from the
    /// input pins into `v−`.
    pub ibias: f64,
    /// Signed input offset current `I(+) − I(−)`, amps.
    pub ios: f64,
    /// Differential resistance between the inputs, ohms; zero disables it.
    pub rin: f64,
    /// Common-mode resistance from each input to `v−`, ohms; zero disables it.
    pub rin_cm: f64,
    /// Common-mode input capacitance from each input to `v−`, farads.
    pub cin_cm: f64,
    /// Differential capacitance between the two inputs, farads.
    pub cin_diff: f64,
    /// Quiescent current drawn from `v+` to `v−`, amps.
    pub iq: f64,
    /// Common-mode rejection ratio in dB. `None` means ideal rejection.
    pub cmrr: Option<f64>,
    /// Hysteresis window `VHYS`, volts. Zero is none.
    pub vhys: f64,
    /// `OUTPUT=OC` (default) or `OUTPUT=PP`.
    pub output: ComparatorOutput,
}

impl Default for ComparatorModel {
    fn default() -> Self {
        Self {
            aol: 2e5,
            gbw: 10e6,
            rout: 60.0,
            vos: 0.0,
            ibias: 0.0,
            ios: 0.0,
            rin: 0.0,
            rin_cm: 0.0,
            cin_cm: 0.0,
            cin_diff: 0.0,
            iq: 0.0,
            cmrr: None,
            vhys: 0.0,
            output: ComparatorOutput::OpenCollector,
        }
    }
}

/// Which regulator a `.model <name> LDO|LDOADJ|BUCK(...)` card describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegulatorKind {
    /// `LDO`: a fixed linear regulator, pins `in out gnd`. The output is
    /// referenced to the `gnd` pin.
    Linear,
    /// `LDOADJ`: an adjustable three-terminal linear regulator (LM317), pins
    /// `in out adj`. It holds `v(out) − v(adj)` at `VREF` and drives `IADJ`
    /// out of the `adj` pin; the external divider sets the output.
    LinearAdjustable,
    /// `BUCK`: an averaged step-down converter, pins `in out gnd` (fixed,
    /// `VOUT` set) or `in out gnd fb` (adjustable, `VREF` set).
    Buck,
}

/// Resolved parameters of a regulator card: `LDO`, `LDOADJ` or `BUCK`.
///
/// Every regulator lowers into the same core, written about a reference pin
/// `ref` (the `gnd` pin, or the `adj` pin of an `LDOADJ`) and one internal
/// set-point node `a` that holds the output the regulator is trying to make,
/// `v(out) − v(ref)`:
///
/// ```text
///   set point a:  Norton drive to VSET through Ra = 1 MΩ
///                 (VSET = VOUT, or VREF for LDOADJ; plus LINE·(v(in) − v(ref)
///                 − VSET − VDO) of line regulation). An adjustable BUCK
///                 instead drives a with an error amplifier:
///                 AOL/Ra · (VREF − (v(fb) − v(ref))).
///   dropout:      D  a → hd,  hd = max(DMAX·(v(in) − v(ref)) − VDO, 0)
///                 (the max: 1 kΩ and a clamp to ground, buffered by an E)
///   current limit D  a → a + K·(ILIM − I(out)),  K = 100 V/A
///   output:       E  oi = v(ref) + v(a),  then ROUT from oi to out
///   input:        LDO/LDOADJ: F draws the output current from in into ref
///                 (a pass element: I(in) = I(out));
///                 BUCK: a power-balance sink draws
///                 (v(out) − v(ref)) · I(out) / (EFF · max(v(in) − v(ref), 0.1 V))
///                 from in into ref.
///   quiescent:    IQ from in to ref (IADJ from in out of the adj pin), scaled
///                 by the set point, so it is IQ while regulating and zero with
///                 no input.
/// ```
///
/// So `v(out) = min(VSET, DMAX·v(in) − VDO) − ROUT·I(out)` while the output
/// current is under `ILIM`, and the current is held at `ILIM` past it. The
/// dropout clamps are the op-amp macro's clamp diode ([`MACRO_CLAMP_DIODE`]),
/// the current-limit clamp a softer one ([`REGULATOR_LIMIT_DIODE`]): in
/// dropout the output sits one clamp drop (≈ 8 mV at the set-point node's
/// 3 µA) above `v(in) − VDO`.
///
/// What it does not model: dropout that grows with load current, ripple and
/// switching (the buck is averaged: no inductor current, no loop dynamics),
/// the LM317's minimum load current, thermal shutdown, and reverse current
/// from the output into the input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegulatorModel {
    /// Topology.
    pub kind: RegulatorKind,
    /// Fixed output voltage `VOUT`, volts. Zero on an adjustable part.
    pub vout: f64,
    /// Reference `VREF`, volts: `v(out) − v(adj)` of an `LDOADJ`, `v(fb)` of
    /// an adjustable `BUCK`.
    pub vref: f64,
    /// Dropout / headroom `VDO`, volts: the least `v(in) − v(out)` (for a
    /// buck, `DMAX·v(in) − v(out)`) the part regulates at.
    pub vdo: f64,
    /// Maximum duty cycle `DMAX` of a buck, 0..=1. 1 for a linear part.
    pub dmax: f64,
    /// Output current limit `ILIM`, amps.
    pub ilim: f64,
    /// Quiescent current `IQ` (`IADJ` on an `LDOADJ`), amps.
    pub iq: f64,
    /// Output resistance `ROUT`, ohms: the load regulation, ΔV/ΔI.
    pub rout: f64,
    /// Line regulation `LINE`, V/V: ΔVout/ΔVin while regulating.
    pub line: f64,
    /// Conversion efficiency `EFF` of a buck, 0..=1.
    pub eff: f64,
    /// Error-amplifier gain `AOL` of an adjustable buck, V/V.
    pub aol: f64,
}

impl RegulatorModel {
    /// The card's defaults for `kind`: a working part, not a datasheet.
    pub fn defaults(kind: RegulatorKind) -> Self {
        let common = Self {
            kind,
            vout: 0.0,
            vref: 0.0,
            vdo: 0.0,
            dmax: 1.0,
            ilim: 1.0,
            iq: 0.0,
            rout: 0.01,
            line: 0.0,
            eff: 1.0,
            aol: 1e5,
        };
        match kind {
            RegulatorKind::Linear => Self {
                vout: 3.3,
                vdo: 0.3,
                ..common
            },
            RegulatorKind::LinearAdjustable => Self {
                vref: 1.25,
                vdo: 1.5,
                ilim: 1.5,
                iq: 50e-6,
                ..common
            },
            RegulatorKind::Buck => Self {
                eff: 0.9,
                ilim: 3.0,
                ..common
            },
        }
    }

    /// True for a buck whose output is set by a divider into `fb`.
    pub fn is_adjustable_buck(&self) -> bool {
        self.kind == RegulatorKind::Buck && self.vref > 0.0
    }

    /// The set point a fixed part (or an `LDOADJ`) holds `v(out) − v(ref)` at.
    pub fn setpoint(&self) -> f64 {
        match self.kind {
            RegulatorKind::LinearAdjustable => self.vref,
            RegulatorKind::Linear | RegulatorKind::Buck => self.vout,
        }
    }
}

/// Gain of a regulator's current-limit sense, volts per amp.
///
/// The limit clamp compares `K·(ILIM − I(out))` against the clamp diode's
/// knee, so it conducts a few milliamps past `ILIM` at 100 V/A (see
/// [`REGULATOR_LIMIT_DIODE`]). Sensing the output voltage across `ROUT`
/// instead would put the knee at `(clamp drop)/ROUT`, which on a 10 mΩ part
/// is amperes.
pub const REGULATOR_LIMIT_GAIN: f64 = 100.0;

/// The current-limit clamp's diode: a plain `IS=1e-12 N=1` junction.
///
/// Softer than [`MACRO_CLAMP_DIODE`] on purpose. The limit loop runs through
/// the output current, whose slope against the set point is `K/ROUT` (10⁴ on
/// a 10 mΩ part); with the macro clamp's 50× sharper knee on top, the loop is
/// too stiff for SPICE's timestep control (ngspice stops with "timestep too
/// small" at the knee). At N = 1 the knee is ~0.4 V at the set point's
/// microamps — 4 mA of limit error at K = 100 V/A.
pub const REGULATOR_LIMIT_DIODE: DiodeModel = DiodeModel {
    is: 1e-12,
    n: 1.0,
    ik: 0.0,
    ikr: 0.0,
    isr: 0.0,
    nr: 2.0,
    rs: 0.0,
    tt: 0.0,
    ttt1: 0.0,
    ttt2: 0.0,
    tm1: 0.0,
    tm2: 0.0,
    vp: 0.0,
    bv: None,
    ibv: 1e-3,
    nbv: None,
    cjo: 0.0,
    vj: 1.0,
    m: 0.5,
    fc: 0.5,
    eg: 1.11,
    xti: 3.0,
    tnom_celsius: 27.0,
    tcv: 0.0,
    trs1: 0.0,
    trs2: 0.0,
    jsw: 0.0,
    ns: None,
    pj: 0.0,
    cjsw: 0.0,
    php: 1.0,
    mjsw: 0.33,
    fcs: 0.5,
    tlevc: 0,
    cta: 0.0,
    ctp: 0.0,
    tpb: 0.0,
    tphp: 0.0,
    jtun: 0.0,
    jtunsw: 0.0,
    ntun: 30.0,
    xtitun: 3.0,
    keg: 1.0,
};

/// Floor on the input voltage a buck's power-balance sink divides by, volts.
///
/// Below it the converter delivers almost nothing (its output is clamped to
/// `DMAX·v(in) − VDO`), and dividing a vanishing output power by a vanishing
/// input voltage is 0/0; the floor turns that into "a vanishing input current".
pub const POWER_SINK_VMIN: f64 = 0.1;

/// The averaged buck's input: a current
/// `(v(op) − v(on)) · I / (eff · max(v(p) − v(n), POWER_SINK_VMIN))` flows from
/// `p` through the element to `n`, where `I` is the output current, i.e. minus
/// the branch current of `control` (an `E` sourcing the output).
///
/// Only a `BUCK` lowering creates one; there is no netlist letter for it.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerSink {
    /// Element name (`<X name>#pin`).
    pub name: String,
    /// Input terminal the current leaves the circuit at.
    pub p: NodeRef,
    /// Terminal the current returns at (the regulator's `gnd` pin).
    pub n: NodeRef,
    /// Positive output terminal the delivered power is measured at.
    pub op: NodeRef,
    /// Negative output terminal.
    pub on: NodeRef,
    /// The `E` element sourcing the output current.
    pub control: String,
    /// That element's branch-current index.
    pub control_branch: usize,
    /// Conversion efficiency, 0 < eff <= 1.
    pub eff: f64,
}

/// Resolved parameters of a `.model <name> NPN|PNP(...)` card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BjtModel {
    /// NPN or PNP.
    pub polarity: Polarity,
    /// Transport saturation current `IS`, amps.
    pub is: f64,
    /// Base-emitter and base-collector saturation currents `IBE`/`IBC`,
    /// amps. As in ngspice-42, they replace `IS` in the two junction
    /// currents only when both are given; `IS` then sets only the Newton
    /// limiter's critical voltage. See [`BjtModel::junction_saturation_currents`].
    pub ibe: Option<f64>,
    pub ibc: Option<f64>,
    /// Forward current gain `BF`.
    pub bf: f64,
    /// Reverse current gain `BR`.
    pub br: f64,
    /// Forward emission coefficient `NF`.
    pub nf: f64,
    /// Reverse emission coefficient `NR`.
    pub nr: f64,
    /// Forward Early voltage `VAF`, volts. Infinity disables forward Early
    /// effect, matching ngspice's default.
    pub vaf: f64,
    /// Reverse Early voltage `VAR`, volts. Infinity disables reverse Early
    /// effect, matching ngspice's default.
    pub var: f64,
    /// Forward beta rolloff knee current `IKF`, amps. Infinity disables
    /// forward high-injection rolloff, matching ngspice's default.
    pub ikf: f64,
    /// Reverse beta rolloff knee current `IKR`, amps. Infinity disables
    /// reverse high-injection rolloff, matching ngspice's default.
    pub ikr: f64,
    /// Optional high-current beta rolloff exponent `NKF` (`NK`). `None`
    /// retains ngspice's default square-root form (exponent 0.5).
    pub nkf: Option<f64>,
    /// Base-emitter recombination saturation current `ISE`, amps. Zero
    /// disables this current, matching ngspice's default.
    pub ise: f64,
    /// Base-emitter recombination emission coefficient `NE`.
    pub ne: f64,
    /// Base-collector recombination saturation current `ISC`, amps. Zero
    /// disables this current, matching ngspice's default.
    pub isc: f64,
    /// Base-collector recombination emission coefficient `NC`.
    pub nc: f64,
    /// Parameter measurement temperature `TNOM`, in Celsius.
    pub tnom_celsius: f64,
    /// Energy gap `EG` used by the saturation-current temperature law, in eV.
    pub eg: f64,
    /// Saturation-current temperature exponent `XTI`.
    pub xti: f64,
    /// Forward/reverse beta temperature exponent `XTB`.
    pub xtb: f64,
    /// Zero-bias base-emitter depletion capacitance `CJE`, farads.
    pub cje: f64,
    /// Base-emitter junction potential `VJE`, volts.
    pub vje: f64,
    /// Base-emitter junction grading coefficient `MJE`.
    pub mje: f64,
    /// First and second order temperature coefficients for `MJE`, per °C and
    /// per °C².
    pub tmje1: f64,
    pub tmje2: f64,
    /// Zero-bias base-collector depletion capacitance `CJC`, farads.
    pub cjc: f64,
    /// Fraction of base-collector depletion capacitance attached to the
    /// internal base node. The remainder is attached to the external base.
    /// The ngspice default is 1; without `RB`, both portions share one node.
    pub xcjc: f64,
    /// Base-collector junction potential `VJC`, volts.
    pub vjc: f64,
    /// Base-collector junction grading coefficient `MJC`.
    pub mjc: f64,
    /// First and second order temperature coefficients for `MJC`, per °C and
    /// per °C².
    pub tmjc1: f64,
    pub tmjc2: f64,
    /// BJT junction-capacitance temperature equation selector (`TLEVC`): 0
    /// uses the silicon bandgap law, 1 uses the linear coefficient form.
    pub tlevc: u8,
    /// Linear temperature coefficients used by `TLEVC=1`, per °C.
    pub cte: f64,
    pub ctc: f64,
    pub tvje: f64,
    pub tvjc: f64,
    /// Forward-bias depletion-capacitance coefficient `FC`.
    pub fc: f64,
    /// Ideal forward transit time `TF`, seconds. Zero disables forward
    /// diffusion charge, matching ngspice's default.
    pub tf: f64,
    /// First and second order temperature coefficients for `TF`, per °C and
    /// per °C².
    pub ttf1: f64,
    pub ttf2: f64,
    /// Forward transit-time bias coefficient `XTF`. Zero disables the
    /// `VBC`/current-dependent adjustment, matching ngspice's default.
    pub xtf: f64,
    /// Voltage scale `VTF` for the forward transit-time `VBC` dependence,
    /// volts: the factor is `exp(VBC/(1.44·VTF))`, as in SPICE2 and ngspice.
    /// Infinity disables that voltage dependence, matching ngspice's default.
    pub vtf: f64,
    /// High-current scale `ITF` for the forward transit-time adjustment,
    /// amps. Zero disables current suppression, matching ngspice's default.
    pub itf: f64,
    /// Excess phase `PTF` at `1/(2π·TF)`, degrees. Zero disables Weil's
    /// second-order delay of the forward transport current, matching
    /// ngspice's default. The delay `PTF·π/180·TF` uses the card `TF`, before
    /// `TTF1`/`TTF2`, as ngspice's `bjttemp.c` does.
    pub ptf: f64,
    /// Ideal reverse transit time `TR`, seconds. Zero disables reverse
    /// diffusion charge, matching ngspice's default.
    pub tr: f64,
    /// First and second order temperature coefficients for `TR`, per °C and
    /// per °C².
    pub ttr1: f64,
    pub ttr2: f64,
    /// Zero-bias base terminal resistance `RB`, ohms. Zero disables the
    /// internal base node, matching ngspice's default.
    pub rb: f64,
    /// First and second order temperature coefficients for `RB`, per °C and
    /// per °C².
    pub trb1: f64,
    pub trb2: f64,
    /// Minimum high-current base resistance `RBM`, ohms. `None` means RBM=RB,
    /// ngspice's default, so the base resistance stays fixed.
    pub rbm: Option<f64>,
    /// First and second order temperature coefficients for `RBM`, per °C and
    /// per °C².
    pub trbm1: f64,
    pub trbm2: f64,
    /// Base current where resistance falls halfway from RB to RBM (`IRB`),
    /// amps. Infinity disables the current-dependent transition, ngspice's
    /// default.
    pub irb: f64,
    /// First and second order temperature coefficients for `IRB`, per °C and
    /// per °C².
    pub tirb1: f64,
    pub tirb2: f64,
    /// Ohmic collector terminal resistance `RC`, ohms. Zero disables the
    /// internal collector node, matching ngspice's default.
    pub rc: f64,
    /// First and second order temperature coefficients for `RC`, per °C and
    /// per °C².
    pub trc1: f64,
    pub trc2: f64,
    /// Ohmic emitter terminal resistance `RE`, ohms. Zero disables the
    /// internal emitter node, matching ngspice's default.
    pub re: f64,
    /// First and second order temperature coefficients for `RE`, per °C and
    /// per °C².
    pub tre1: f64,
    pub tre2: f64,
    /// Zero-bias substrate junction capacitance `CJS` (`CCS`, `CSUB`), farads.
    /// Zero, ngspice's default, leaves only the substrate current.
    pub cjs: f64,
    /// Substrate junction potential `VJS` (`PS`), volts.
    pub vjs: f64,
    /// Substrate junction grading coefficient `MJS` (`MS`). ngspice limits
    /// the temperature-adjusted value to 0.999.
    pub mjs: f64,
    /// First and second order temperature coefficients for `MJS`, per °C and
    /// per °C².
    pub tmjs1: f64,
    pub tmjs2: f64,
    /// `TLEVC=1` linear temperature coefficients for `CJS` and `VJS`, per °C.
    pub cts: f64,
    pub tvjs: f64,
    /// Substrate junction saturation current `ISS`, amps. Zero, ngspice's
    /// default, leaves only GMIN across the substrate junction.
    pub iss: f64,
    /// Substrate junction emission coefficient `NS`.
    pub ns: f64,
    /// First and second order temperature coefficients for `NS`, per °C and
    /// per °C².
    pub tns1: f64,
    pub tns2: f64,
    /// Temperature law `TLEV` for saturation currents and beta: 0 (the
    /// `XTB` power law), 1 (`1 + XTB·ΔT`) or 3 (`IS^(1 + TIS1·ΔT + TIS2·ΔT²)`).
    pub tlev: u8,
    /// First and second order temperature coefficients, per °C and per °C²,
    /// for `BF`, `BR`, `IKF`, `IKR`, `NF`, `NR`, `NE`, `NC`, `VAF`, `VAR` and
    /// `ITF`, as ngspice-42 `bjttemp.c` applies them. `TBF1`/`TBF2` (and
    /// `TBR1`/`TBR2`) replace the `XTB` beta factor when either is given.
    pub tbf1: f64,
    pub tbf2: f64,
    pub tbr1: f64,
    pub tbr2: f64,
    pub tikf1: f64,
    pub tikf2: f64,
    pub tikr1: f64,
    pub tikr2: f64,
    pub tnf1: f64,
    pub tnf2: f64,
    pub tnr1: f64,
    pub tnr2: f64,
    pub tne1: f64,
    pub tne2: f64,
    pub tnc1: f64,
    pub tnc2: f64,
    pub tvaf1: f64,
    pub tvaf2: f64,
    pub tvar1: f64,
    pub tvar2: f64,
    pub titf1: f64,
    pub titf2: f64,
    pub tbf_given: bool,
    pub tbr_given: bool,
    /// `TLEV=3` exponent coefficients for `IS`, `ISE`, `ISC` and `ISS`.
    pub tis1: f64,
    pub tis2: f64,
    pub tise1: f64,
    pub tise2: f64,
    pub tisc1: f64,
    pub tisc2: f64,
    pub tiss1: f64,
    pub tiss2: f64,
    /// Substrate geometry `SUBS`: `Some(1)` vertical (the junction sits on
    /// the collector), `Some(-1)` lateral (on the base). `None` is ngspice's
    /// default: vertical for NPN, lateral for PNP.
    pub subs: Option<i8>,
    /// Epitaxial collector resistance `RCO`, ohms. `Some` enables Kull's
    /// quasi-saturation model between the collector contact and the intrinsic
    /// collector, as in ngspice, which also raises a supplied value below
    /// 0.01 Ω to 0.01 Ω. `None`, the default, leaves the collector ohmic.
    pub rco: Option<f64>,
    /// Epitaxial drift saturation voltage `VO`, volts.
    pub vo: f64,
    /// Epitaxial doping factor `GAMMA`.
    pub gamma: f64,
    /// Epitaxial charge factor `QCO`, coulombs.
    pub qco: f64,
    /// `QUASIMOD`: 1 applies the `CN`/`D`/`VG` temperature laws to
    /// `RCO`/`VO`/`GAMMA` with an instance `TEMP`; 0 leaves them fixed.
    pub quasimod: u8,
    /// Energy gap `VG` for the `QUASIMOD=1` `GAMMA` law, eV.
    pub vg: f64,
    /// Temperature exponents `CN` of `RCO` and `D` of `VO` for `QUASIMOD=1`.
    /// `None` takes ngspice's polarity defaults: 2.42 and 0.87 for NPN,
    /// 2.2 and 0.52 for PNP.
    pub cn: Option<f64>,
    pub d: Option<f64>,
}

impl BjtModel {
    /// The card one instance with SPICE `AREA` sees: `IS`, `ISE`, `ISC`,
    /// `IKF`, `IKR`, `IRB`, `ITF`, `CJE` and `CJC` scale with the emitter
    /// area and `RB`, `RBM`, `RC`, `RE` divide by it, as ngspice's
    /// `bjtload.c` does when `AREAB`/`AREAC` default to `AREA`. Unlike the
    /// instance multiplier `m`, GMIN is untouched. The quasi-saturation
    /// parameters `RCO`/`VO`/`GAMMA`/`QCO` are not area-scaled, as in
    /// ngspice. `area == 1` returns the card bit-identical.
    pub fn with_area(self, area: f64) -> Self {
        Self {
            is: self.is * area,
            ibe: self.ibe.map(|ibe| ibe * area),
            ibc: self.ibc.map(|ibc| ibc * area),
            ise: self.ise * area,
            isc: self.isc * area,
            ikf: self.ikf * area,
            ikr: self.ikr * area,
            irb: self.irb * area,
            itf: self.itf * area,
            cje: self.cje * area,
            cjc: self.cjc * area,
            cjs: self.cjs * area,
            iss: self.iss * area,
            rb: self.rb / area,
            rbm: self.rbm.map(|rbm| rbm / area),
            rc: self.rc / area,
            re: self.re / area,
            ..self
        }
    }

    fn defaults(polarity: Polarity) -> Self {
        Self {
            polarity,
            is: 1e-16,
            ibe: None,
            ibc: None,
            bf: 100.0,
            br: 1.0,
            nf: 1.0,
            nr: 1.0,
            vaf: f64::INFINITY,
            var: f64::INFINITY,
            ikf: f64::INFINITY,
            ikr: f64::INFINITY,
            nkf: None,
            ise: 0.0,
            ne: 1.5,
            isc: 0.0,
            nc: 2.0,
            tnom_celsius: 27.0,
            eg: 1.11,
            xti: 3.0,
            xtb: 0.0,
            cje: 0.0,
            vje: 0.75,
            mje: 0.33,
            tmje1: 0.0,
            tmje2: 0.0,
            cjc: 0.0,
            vjc: 0.75,
            mjc: 0.33,
            tmjc1: 0.0,
            tmjc2: 0.0,
            xcjc: 1.0,
            tlevc: 0,
            cte: 0.0,
            ctc: 0.0,
            tvje: 0.0,
            tvjc: 0.0,
            fc: 0.5,
            tf: 0.0,
            ttf1: 0.0,
            ttf2: 0.0,
            xtf: 0.0,
            vtf: f64::INFINITY,
            itf: 0.0,
            ptf: 0.0,
            tr: 0.0,
            ttr1: 0.0,
            ttr2: 0.0,
            rb: 0.0,
            trb1: 0.0,
            trb2: 0.0,
            rbm: None,
            trbm1: 0.0,
            trbm2: 0.0,
            irb: f64::INFINITY,
            tirb1: 0.0,
            tirb2: 0.0,
            rc: 0.0,
            trc1: 0.0,
            trc2: 0.0,
            re: 0.0,
            tre1: 0.0,
            tre2: 0.0,
            cjs: 0.0,
            vjs: 0.75,
            mjs: 0.0,
            tmjs1: 0.0,
            tmjs2: 0.0,
            cts: 0.0,
            tvjs: 0.0,
            iss: 0.0,
            ns: 1.0,
            tns1: 0.0,
            tns2: 0.0,
            tlev: 0,
            tbf1: 0.0,
            tbf2: 0.0,
            tbr1: 0.0,
            tbr2: 0.0,
            tikf1: 0.0,
            tikf2: 0.0,
            tikr1: 0.0,
            tikr2: 0.0,
            tnf1: 0.0,
            tnf2: 0.0,
            tnr1: 0.0,
            tnr2: 0.0,
            tne1: 0.0,
            tne2: 0.0,
            tnc1: 0.0,
            tnc2: 0.0,
            tvaf1: 0.0,
            tvaf2: 0.0,
            tvar1: 0.0,
            tvar2: 0.0,
            titf1: 0.0,
            titf2: 0.0,
            tis1: 0.0,
            tis2: 0.0,
            tise1: 0.0,
            tise2: 0.0,
            tisc1: 0.0,
            tisc2: 0.0,
            tiss1: 0.0,
            tiss2: 0.0,
            tbf_given: false,
            tbr_given: false,
            subs: None,
            rco: None,
            vo: 10.0,
            gamma: 1e-11,
            qco: 0.0,
            quasimod: 0,
            vg: 1.206,
            cn: None,
            d: None,
        }
    }

    /// Saturation currents `(B-E, B-C)` of the two junction diodes. ngspice-42
    /// `bjttemp.c` uses `IBE`/`IBC` only when both are given, and `IS` for
    /// both otherwise. The transport current is their difference, so the
    /// forward, reverse, base and transit currents all follow them.
    pub fn junction_saturation_currents(&self) -> (f64, f64) {
        match (self.ibe, self.ibc) {
            (Some(ibe), Some(ibc)) => (ibe, ibc),
            _ => (self.is, self.is),
        }
    }

    /// Whether the substrate junction sits on the collector (vertical) rather
    /// than the base (lateral), per `SUBS` or ngspice's polarity default.
    pub fn substrate_is_vertical(&self) -> bool {
        match self.subs {
            Some(subs) => subs > 0,
            None => self.polarity == Polarity::N,
        }
    }
}

/// Intrinsic MOSFET channel charge formulation selected by the model card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MosChannelChargeModel {
    /// ngspice-compatible piecewise Meyer capacitance model.
    Meyer,
    /// Engine-specific Ward–Dutton inversion-charge partition.
    WardDutton,
}

/// Resolved parameters of a `.model <name> NMOS|PMOS(...)` card, SPICE
/// level 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MosModel {
    /// N-channel or P-channel.
    pub polarity: Polarity,
    /// Zero-bias threshold `VTO`, volts — **negative for a normal PMOS**, as
    /// in ngspice. When the card omits `VTO` but supplies `TOX` and `NSUB`,
    /// the resolved instance uses ngspice's process value instead (see
    /// [`Self::nsub`]).
    pub vto: f64,
    /// Whether the model card explicitly supplied `VTO`.
    pub vto_given: bool,
    /// Transconductance parameter `KP`, A/V². When the card omits `KP` but
    /// supplies `TOX`, the resolved instance uses ngspice's process value
    /// `UO·1e-4·εox/TOX` instead (see [`Self::kp_given`]).
    pub kp: f64,
    /// Whether the model card explicitly supplied `KP`.
    pub kp_given: bool,
    /// Surface mobility `UO`, cm²/V·s (default 600). Used only to derive `KP`
    /// from `TOX` when `KP` is absent, as ngspice level 1 does.
    pub uo: f64,
    /// Lateral diffusion `LD`, metres. The channel is `L − 2·LD` long for
    /// drain current, intrinsic oxide charge and `CGBO` (default 0).
    pub ld: f64,
    /// Channel-length modulation `LAMBDA`, 1/V.
    pub lambda: f64,
    /// Ohmic drain terminal resistance `RD`, ohms. Zero disables the internal
    /// drain node, matching ngspice's default.
    pub rd: f64,
    /// Whether the model card explicitly supplied `RD`. When present, this
    /// value takes precedence over `RSH`, including when it is zero.
    pub rd_given: bool,
    /// Ohmic source terminal resistance `RS`, ohms. Zero disables the internal
    /// source node, matching ngspice's default.
    pub rs: f64,
    /// Whether the model card explicitly supplied `RS`. When present, this
    /// value takes precedence over `RSH`, including when it is zero.
    pub rs_given: bool,
    /// Drain/source diffusion sheet resistance `RSH`, ohms per square. When
    /// the corresponding direct `RD` or `RS` is zero, the instance's `NRD` or
    /// `NRS` multiplies this value.
    pub rsh: f64,
    /// Bulk threshold parameter `GAMMA`, sqrt(V).
    pub gamma: f64,
    /// Whether the model card explicitly supplied `GAMMA`.
    pub gamma_given: bool,
    /// Surface potential `PHI`, volts.
    pub phi: f64,
    /// Whether the model card explicitly supplied `PHI`.
    pub phi_given: bool,
    /// Substrate doping `NSUB`, cm⁻³. With `TOX`, ngspice level 1 derives
    /// every one of `PHI`, `GAMMA` and `VTO` the card omits from it; without
    /// `TOX` it is ignored, as in ngspice.
    pub nsub: Option<f64>,
    /// Fast surface-state density `NSS`, cm⁻². Shifts the process-derived
    /// flat-band voltage by `−NSS·q/C'ox` (default 0).
    pub nss: f64,
    /// Gate material type `TPG`: `+1` opposite to the substrate (default),
    /// `−1` same as the substrate, `0` aluminium. Used only for the
    /// process-derived `VTO`.
    pub tpg: f64,
    /// Bulk junction saturation current `IS`, amps.
    pub is: f64,
    /// Bulk junction saturation current density `JS`, A/m². When positive
    /// and the instance supplies both `AD` and `AS`, ngspice level 1 uses
    /// `JS·AD` and `JS·AS` for the drain and source junctions instead of
    /// `IS` (default 0).
    pub js: f64,
    /// Zero-bias bulk-drain junction capacitance `CBD`, farads.
    pub cbd: f64,
    /// Whether `CBD` was explicitly supplied; an explicit zero also overrides
    /// the geometry-derived bottom capacitance `CJ·AD`.
    pub cbd_given: bool,
    /// Zero-bias bulk-source junction capacitance `CBS`, farads.
    pub cbs: f64,
    /// Whether `CBS` was explicitly supplied; an explicit zero also overrides
    /// the geometry-derived bottom capacitance `CJ·AS`.
    pub cbs_given: bool,
    /// Zero-bias bottom junction capacitance density `CJ`, F/m².
    pub cj: f64,
    /// Zero-bias sidewall junction capacitance density `CJSW`, F/m.
    pub cjsw: f64,
    /// Bulk junction potential `PB`, volts.
    pub pb: f64,
    /// Bulk junction grading coefficient `MJ`.
    pub mj: f64,
    /// Bulk junction sidewall grading coefficient `MJSW`.
    pub mjsw: f64,
    /// Forward-bias depletion-capacitance coefficient `FC`.
    pub fc: f64,
    /// Default channel width `W`, metres (the element line may override it).
    pub w: f64,
    /// Default channel length `L`, metres (the element line may override it).
    pub l: f64,
    /// Gate-source overlap capacitance per unit width `CGSO`, F/m.
    pub cgso: f64,
    /// Gate-drain overlap capacitance per unit width `CGDO`, F/m.
    pub cgdo: f64,
    /// Gate-bulk overlap capacitance per unit length `CGBO`, F/m.
    pub cgbo: f64,
    /// Gate oxide thickness `TOX`, metres. When present, enables intrinsic
    /// channel charge; absent means no intrinsic channel capacitance, matching
    /// ngspice's level-1 behavior.
    pub tox: Option<f64>,
    /// Intrinsic channel charge formulation. `WardDutton` is an engine
    /// extension selected by `CHARGEMOD=WARD_DUTTON`; `Meyer` is the default.
    pub channel_charge_model: MosChannelChargeModel,
    /// Parameter measurement temperature `TNOM`, in Celsius. Only used when
    /// an instance supplies `TEMP`.
    pub tnom_celsius: f64,
}

/// Resolved parameters of a `.model <name> NJF|PJF(...)` card, SPICE
/// level 1 with the Parker–Skellern `B` fitting parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JfetModel {
    /// N-channel or P-channel.
    pub polarity: Polarity,
    /// Pinch-off voltage `VTO`, volts (negative for NJF, positive for PJF).
    pub vto: f64,
    /// Transconductance parameter `BETA`, A/V².
    pub beta: f64,
    /// Channel-length modulation `LAMBDA`, 1/V.
    pub lambda: f64,
    /// Parker–Skellern fitting parameter B for the level-1 channel equation.
    pub b: f64,
    /// Ohmic drain terminal resistance `RD`, ohms. Zero disables the internal
    /// drain node, matching ngspice's default.
    pub rd: f64,
    /// Ohmic source terminal resistance `RS`, ohms. Zero disables the internal
    /// source node, matching ngspice's default.
    pub rs: f64,
    /// Gate-junction saturation current `IS`, amps.
    pub is: f64,
    /// Gate-junction emission coefficient `N`, shared by the gate-source and
    /// gate-drain diodes. Default 1, as in ngspice.
    pub n: f64,
    /// Zero-bias gate-source depletion capacitance `CGS`, farads.
    pub cgs: f64,
    /// Zero-bias gate-drain depletion capacitance `CGD`, farads.
    pub cgd: f64,
    /// Gate-junction potential `PB`, volts.
    pub pb: f64,
    /// Forward-bias depletion-capacitance coefficient `FC`.
    pub fc: f64,
    /// Parameter measurement temperature `TNOM`, in Celsius.
    pub tnom_celsius: f64,
    /// Alternative threshold temperature coefficient. When supplied, even as
    /// zero, this takes precedence over `TCV`.
    pub vtotc: Option<f64>,
    /// Threshold voltage temperature coefficient used when `VTOTC` is absent.
    pub tcv: f64,
    /// Alternative beta temperature coefficient. When supplied, even as
    /// zero, this takes precedence over `BEX`.
    pub betatce: Option<f64>,
    /// Mobility temperature exponent used when `BETATCE` is absent.
    pub bex: f64,
    /// Gate-junction activation energy `EG`, eV, for the `IS` temperature law.
    pub eg: f64,
    /// Gate-junction saturation-current temperature exponent `XTI`.
    pub xti: f64,
}

impl JfetModel {
    /// The card one instance with SPICE `AREA` sees: `BETA`, `IS`, `CGS` and
    /// `CGD` scale with the gate area and `RD`, `RS` divide by it, as
    /// ngspice's `jfetload.c` does. Unlike the instance multiplier `m`, GMIN
    /// is untouched. `area == 1` returns the card bit-identical.
    pub fn with_area(self, area: f64) -> Self {
        Self {
            beta: self.beta * area,
            is: self.is * area,
            cgs: self.cgs * area,
            cgd: self.cgd * area,
            rd: self.rd / area,
            rs: self.rs / area,
            ..self
        }
    }

    fn defaults(polarity: Polarity) -> Self {
        Self {
            polarity,
            vto: if polarity == Polarity::N { -2.0 } else { 2.0 },
            beta: 1e-4,
            lambda: 0.0,
            b: 1.0,
            rd: 0.0,
            rs: 0.0,
            is: 1e-14,
            n: 1.0,
            cgs: 0.0,
            cgd: 0.0,
            pb: 1.0,
            fc: 0.5,
            tnom_celsius: 27.0,
            vtotc: None,
            tcv: 0.0,
            betatce: None,
            bex: 0.0,
            eg: 1.11,
            xti: 3.0,
        }
    }
}

impl MosModel {
    fn defaults(polarity: Polarity) -> Self {
        Self {
            polarity,
            vto: 0.0,
            vto_given: false,
            kp: 2e-5,
            kp_given: false,
            uo: 600.0,
            ld: 0.0,
            lambda: 0.0,
            rd: 0.0,
            rd_given: false,
            rs: 0.0,
            rs_given: false,
            rsh: 0.0,
            gamma: 0.0,
            gamma_given: false,
            phi: 0.6,
            phi_given: false,
            nsub: None,
            nss: 0.0,
            tpg: 1.0,
            is: 1e-14,
            js: 0.0,
            cbd: 0.0,
            cbd_given: false,
            cbs: 0.0,
            cbs_given: false,
            cj: 0.0,
            cjsw: 0.0,
            pb: 0.8,
            mj: 0.5,
            mjsw: 0.5,
            fc: 0.5,
            w: 1e-4,
            l: 1e-4,
            cgso: 0.0,
            cgdo: 0.0,
            cgbo: 0.0,
            tox: None,
            channel_charge_model: MosChannelChargeModel::Meyer,
            tnom_celsius: 27.0,
        }
    }
}

/// One `.model` card, before it is attached to an element.
#[allow(clippy::large_enum_variant)] // Keep cards Copy; model cards are small, short-lived parser values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModelCard {
    /// A `D` card.
    Diode(DiodeModel),
    /// An `NPN` or `PNP` card.
    Bjt(BjtModel),
    /// An `NMOS` or `PMOS` card.
    Mos(MosModel),
    /// An `NJF` or `PJF` card.
    Jfet(JfetModel),
    /// An `OPAMP` card (this engine's type, not ngspice's).
    OpAmp(OpAmpModel),
    /// A `COMP` card (this engine's type, not ngspice's).
    Comparator(ComparatorModel),
    /// An `LDO`, `LDOADJ` or `BUCK` card (this engine's types).
    Regulator(RegulatorModel),
    /// An `SW` or `CSW` card.
    Switch(SwitchModel),
}

impl ModelCard {
    /// The card's SPICE type keyword, for diagnostics.
    fn kind(&self) -> &'static str {
        match self {
            Self::OpAmp(_) => "OPAMP",
            Self::Comparator(_) => "COMP",
            Self::Regulator(model) => match model.kind {
                RegulatorKind::Linear => "LDO",
                RegulatorKind::LinearAdjustable => "LDOADJ",
                RegulatorKind::Buck => "BUCK",
            },
            Self::Diode(_) => "D",
            Self::Switch(model) => {
                if model.current {
                    "CSW"
                } else {
                    "SW"
                }
            }
            Self::Bjt(model) => match model.polarity {
                Polarity::N => "NPN",
                Polarity::P => "PNP",
            },
            Self::Mos(model) => match model.polarity {
                Polarity::N => "NMOS",
                Polarity::P => "PMOS",
            },
            Self::Jfet(model) => match model.polarity {
                Polarity::N => "NJF",
                Polarity::P => "PJF",
            },
        }
    }
}

/// The five model names an element may use with no `.model` line of its own.
///
/// These are this engine's convenience values, **not** ngspice's parameter
/// defaults: `NMOS` here is a working enhancement FET, whereas ngspice's
/// level-1 defaults (`VTO = 0`) describe a device that is never off. Write a
/// `.model` line when the deck has to mean the same thing to both engines.
fn builtin_model(name: &str) -> Option<ModelCard> {
    match name.to_ascii_uppercase().as_str() {
        // 1N4148-class small-signal silicon switching diode.
        "D" => Some(ModelCard::Diode(DiodeModel {
            is: 2.52e-9,
            n: 1.752,
            rs: 0.0,
            ..DiodeModel::default()
        })),
        // 1N4733A-like 5.1 V, 1 W zener. The datasheet's headline point is
        // VZ = 5.1 V at IZT = 49 mA with ZZT ≤ 7 Ω. Level-1 breakdown puts the
        // junction at BV when IBV flows, and RS adds IZT·RS on top, so
        // BV = 4.8 V with RS = 6 Ω lands VZ(49 mA) at 5.09 V; the dynamic
        // impedance there is RS + N·Vt/IZT = 6.6 Ω. The price of one RS for
        // both directions is a soft forward knee: 1.9 V at 200 mA, where the
        // datasheet says ≤ 1.2 V.
        "1N4733A" => Some(ModelCard::Diode(DiodeModel {
            is: 1e-11,
            n: 1.2,
            ik: 0.0,
            ikr: 0.0,
            isr: 0.0,
            nr: 2.0,
            rs: 6.0,
            tt: 0.0,
            ttt1: 0.0,
            ttt2: 0.0,
            tm1: 0.0,
            tm2: 0.0,
            vp: 0.0,
            bv: Some(4.8),
            ibv: 49e-3,
            nbv: None,
            cjo: 0.0,
            vj: 1.0,
            m: 0.5,
            fc: 0.5,
            eg: 1.11,
            xti: 3.0,
            tnom_celsius: 27.0,
            tcv: 0.0,
            trs1: 0.0,
            trs2: 0.0,
            jsw: 0.0,
            ns: None,
            pj: 0.0,
            cjsw: 0.0,
            php: 1.0,
            mjsw: 0.33,
            fcs: 0.5,
            tlevc: 0,
            cta: 0.0,
            ctp: 0.0,
            tpb: 0.0,
            tphp: 0.0,
            jtun: 0.0,
            jtunsw: 0.0,
            ntun: 30.0,
            xtitun: 3.0,
            keg: 1.0,
        })),
        // SMAJ5.0A-like 400 W unidirectional TVS. Datasheet: VBR 6.40–7.07 V
        // at IT = 10 mA, VC = 9.2 V at IPP = 43.5 A. BV sits mid-window at
        // 6.7 V; RS = 52.5 mΩ is what puts the junction plus RS·IPP at 9.2 V
        // for 43.5 A.
        "SMAJ5.0A" => Some(ModelCard::Diode(DiodeModel {
            is: 1e-12,
            n: 1.0,
            ik: 0.0,
            ikr: 0.0,
            isr: 0.0,
            nr: 2.0,
            rs: 0.0525,
            tt: 0.0,
            ttt1: 0.0,
            ttt2: 0.0,
            tm1: 0.0,
            tm2: 0.0,
            vp: 0.0,
            bv: Some(6.7),
            ibv: 10e-3,
            nbv: None,
            cjo: 0.0,
            vj: 1.0,
            m: 0.5,
            fc: 0.5,
            eg: 1.11,
            xti: 3.0,
            tnom_celsius: 27.0,
            tcv: 0.0,
            trs1: 0.0,
            trs2: 0.0,
            jsw: 0.0,
            ns: None,
            pj: 0.0,
            cjsw: 0.0,
            php: 1.0,
            mjsw: 0.33,
            fcs: 0.5,
            tlevc: 0,
            cta: 0.0,
            ctp: 0.0,
            tpb: 0.0,
            tphp: 0.0,
            jtun: 0.0,
            jtunsw: 0.0,
            ntun: 30.0,
            xtitun: 3.0,
            keg: 1.0,
        })),
        "NJF" => Some(ModelCard::Jfet(JfetModel::defaults(Polarity::N))),
        "PJF" => Some(ModelCard::Jfet(JfetModel::defaults(Polarity::P))),
        // An op-amp that is ideal for any circuit a board puts around it:
        // 120 dB, 10 MHz, 1 Ω, rail to rail.
        "IDEAL_OPAMP" => Some(ModelCard::OpAmp(OpAmpModel {
            aol: 1e6,
            gbw: 10e6,
            rout: 1.0,
            ..OpAmpModel::default()
        })),
        // LM358-like (TI LM358 datasheet, typical): AOL 100 V/mV, GBW
        // 0.7 MHz, output swings to within 1.5 V of V+ and to ground. ROUT is
        // not a datasheet number; 50 Ω is a plausible bipolar class-AB stage.
        // VOS is zero: a typical offset has no sign, so any value here would
        // be a wrong one for most parts.
        "LM358" => Some(ModelCard::OpAmp(OpAmpModel {
            aol: 1e5,
            gbw: 0.7e6,
            rout: 50.0,
            vos: 0.0,
            ibias: 0.0,
            ios: 0.0,
            rin: 0.0,
            rin_cm: 0.0,
            cin_cm: 0.0,
            cin_diff: 0.0,
            iq: 0.0,
            cmrr: None,
            ilim: None,
            sr: None,
            drop_hi: 1.5,
            drop_lo: 0.0,
        })),
        // LM393-like (TI LM393 datasheet, typical): AOL 200 V/mV, open
        // collector sinking 4 mA at VOL = 150 mV (so ROUT = 37.5 Ω), and a
        // 1.3 µs response to 5 mV of overdrive — which is what GBW = 12 MHz
        // gives the 1 V decision node: 0.5 V / (2π · 12 MHz · 5 mV) = 1.3 µs.
        "LM393" => Some(ModelCard::Comparator(ComparatorModel {
            aol: 2e5,
            gbw: 12e6,
            rout: 37.5,
            ..ComparatorModel::default()
        })),
        // ── Regulators. *-like* cards: the datasheet's typical headline
        // numbers, lowered as described on `RegulatorModel`. ROUT is the
        // load-regulation spec over its stated current range; LINE the
        // line-regulation spec over its stated input range. ──
        //
        // AMS1117-3.3 / -5.0 (Advanced Monolithic Systems AMS1117 datasheet,
        // electrical characteristics, typical): dropout 1.1 V at 800 mA,
        // current limit 1.1 A (900 mA min), quiescent current 5 mA, line
        // regulation 1 mV over 4.75–12 V (6.5–12 V for -5.0), load
        // regulation 1 mV over 0–800 mA.
        "AMS1117-3.3" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 3.3,
            vdo: 1.1,
            ilim: 1.1,
            iq: 5e-3,
            rout: 1e-3 / 0.8,
            line: 1e-3 / (12.0 - 4.75),
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        "AMS1117-5.0" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 5.0,
            vdo: 1.1,
            ilim: 1.1,
            iq: 5e-3,
            rout: 1e-3 / 0.8,
            line: 1e-3 / (12.0 - 6.5),
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        // LM7805 (TI LM340/LM7805 datasheet, typical at 25 °C): dropout 2 V
        // at 1 A, short-circuit current 2.1 A, quiescent current 5 mA, line
        // regulation 3 mV over 7.5–20 V, load regulation 15 mV over
        // 5 mA–1.5 A.
        "LM7805" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 5.0,
            vdo: 2.0,
            ilim: 2.1,
            iq: 5e-3,
            rout: 15e-3 / 1.495,
            line: 3e-3 / (20.0 - 7.5),
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        // MCP1700-3.3 (Microchip MCP1700 datasheet, typical): dropout 178 mV
        // at 250 mA, short-circuit current 408 mA, quiescent current 1.6 µA,
        // line regulation 0.05 %/V, load regulation −0.4 % over 0.1–250 mA.
        "MCP1700-3.3" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 3.3,
            vdo: 0.178,
            ilim: 0.408,
            iq: 1.6e-6,
            rout: 0.004 * 3.3 / 0.2499,
            line: 0.0005 * 3.3,
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        // A generic 3.3 V LDO: 300 mV dropout, 500 mA limit, 50 µA quiescent.
        // Not a part; the card to reach for when a schematic says "LDO".
        "LDO" | "LDO-3.3" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 3.3,
            vdo: 0.3,
            ilim: 0.5,
            iq: 50e-6,
            rout: 0.01,
            ..RegulatorModel::defaults(RegulatorKind::Linear)
        })),
        // LM317 (TI LM317 datasheet, typical): VREF 1.25 V, IADJ 50 µA,
        // current limit 2.2 A, line regulation 0.01 %/V, load regulation
        // 0.1 % over 10 mA–1.5 A. Dropout is read off the dropout curve at
        // 1 A: about 1.7 V.
        "LM317" => Some(ModelCard::Regulator(RegulatorModel {
            vref: 1.25,
            vdo: 1.7,
            ilim: 2.2,
            iq: 50e-6,
            rout: 0.001 * 5.0 / 1.49,
            line: 0.0001 * 1.25,
            ..RegulatorModel::defaults(RegulatorKind::LinearAdjustable)
        })),
        // MP1584 (MPS MP1584EN datasheet, typical): feedback 0.8 V, peak
        // current limit 4 A (a 3 A part), quiescent current 100 µA, ~90 %
        // efficient at 12 V → 5 V, 1 A. VDO is not a datasheet number: the
        // 150 mΩ high-side switch at 3 A, about 0.45 V.
        "MP1584" => Some(ModelCard::Regulator(RegulatorModel {
            vref: 0.8,
            vdo: 0.45,
            ilim: 4.0,
            iq: 100e-6,
            rout: 0.01,
            eff: 0.9,
            ..RegulatorModel::defaults(RegulatorKind::Buck)
        })),
        // LM2596 (TI LM2596 datasheet, typical): feedback 1.23 V (-ADJ),
        // switch current limit 4.5 A, quiescent current 5 mA, switch
        // saturation 1.16 V at 3 A (the VDO), efficiency 80 % for the 5 V
        // part at 12 V, 3 A (73 % for -ADJ at 3 V).
        "LM2596-5.0" => Some(ModelCard::Regulator(RegulatorModel {
            vout: 5.0,
            vdo: 1.16,
            ilim: 4.5,
            iq: 5e-3,
            rout: 0.01,
            eff: 0.80,
            ..RegulatorModel::defaults(RegulatorKind::Buck)
        })),
        "LM2596-ADJ" => Some(ModelCard::Regulator(RegulatorModel {
            vref: 1.23,
            vdo: 1.16,
            ilim: 4.5,
            iq: 5e-3,
            rout: 0.01,
            eff: 0.73,
            ..RegulatorModel::defaults(RegulatorKind::Buck)
        })),
        "NPN" => Some(ModelCard::Bjt(BjtModel::defaults(Polarity::N))),
        "PNP" => Some(ModelCard::Bjt(BjtModel::defaults(Polarity::P))),
        "NMOS" => Some(ModelCard::Mos(MosModel {
            vto: 1.0,
            lambda: 0.02,
            ..MosModel::defaults(Polarity::N)
        })),
        "PMOS" => Some(ModelCard::Mos(MosModel {
            vto: -1.0,
            lambda: 0.02,
            ..MosModel::defaults(Polarity::P)
        })),
        _ => None,
    }
}

/// `D<name> n+ n- <model> [<area>] [m=<count>] [temp=<°C>]` — a Shockley junction diode.
#[derive(Debug, Clone, PartialEq)]
pub struct Diode {
    /// Element name as written, e.g. `D1`.
    pub name: String,
    /// SPICE instance multiplier `M`: the element behaves as this many
    /// identical devices in parallel. Every current, conductance and charge it
    /// stamps, including its terminal resistances and GMIN, is scaled by it,
    /// as ngspice's `*load.c` does. `1` for a single device.
    pub multiplier: f64,
    /// SPICE `off`: the operating point starts with this junction at 0 V and
    /// holds it there until the rest of the circuit has converged, then
    /// releases it (ngspice's `MODEINITFIX`). It selects one state of a
    /// circuit with more than one operating point and changes nothing in a
    /// circuit with one. Ignored after the operating point.
    pub off: bool,
    /// SPICE instance `ic=<vd>`: the junction voltage the charge history
    /// starts from under `.tran ... uic` (ngspice's `DIOinitCond`). Ignored
    /// without `uic`, as in SPICE.
    pub ic: Option<f64>,
    /// Anode, the terminal the netlist names first.
    pub anode: NodeRef,
    /// Cathode.
    pub cathode: NodeRef,
    /// The junction's anode side: the same node as [`Self::anode`] when the
    /// temperature-adjusted `RS` is zero, and `<name>#internal` when it is not.
    pub junction_anode: NodeRef,
    /// Model name as written, for diagnostics.
    pub model_name: String,
    /// Resolved model parameters, already scaled by the instance `AREA`.
    pub model: DiodeModel,
    /// Model parameters with `CJO`, `VJ`, `M`, and `TT` adjusted for this
    /// instance's temperature.
    pub operating_model: DiodeModel,
    /// Model series resistance after applying the instance `TEMP`, in ohms.
    pub series_resistance_ohms: f64,
    /// Instance junction temperature in Celsius. `None` preserves the
    /// engine's historical fixed 300 K behavior.
    pub temperature_celsius: Option<f64>,
    /// The breakdown voltage the reverse exponential is actually written
    /// about — ngspice's `tBrkdwnV`, i.e. `BV` moved so that the current at
    /// `−BV` is `IBV` — or `None` when the model has no `BV`. See
    /// [`super::device::breakdown_voltage`].
    pub breakdown: Option<f64>,
}

/// `E<name> n+ n- nc+ nc- <gain>` — voltage-controlled voltage source:
/// `v(n+) − v(n−) = gain · (v(nc+) − v(nc−))`. Adds a branch current.
#[derive(Debug, Clone, PartialEq)]
pub struct Vcvs {
    /// Element name as written.
    pub name: String,
    /// Positive output terminal.
    pub p: NodeRef,
    /// Negative output terminal.
    pub n: NodeRef,
    /// Positive controlling node.
    pub cp: NodeRef,
    /// Negative controlling node.
    pub cn: NodeRef,
    /// Voltage gain, V/V.
    pub gain: f64,
}

/// `G<name> n+ n- nc+ nc- <gm>` — voltage-controlled current source. A current
/// `gm · (v(nc+) − v(nc−))` flows from `n+` through the source to `n−`, as in
/// SPICE.
#[derive(Debug, Clone, PartialEq)]
pub struct Vccs {
    /// Element name as written.
    pub name: String,
    /// Terminal the current leaves the circuit at.
    pub p: NodeRef,
    /// Terminal the current returns to the circuit at.
    pub n: NodeRef,
    /// Positive controlling node.
    pub cp: NodeRef,
    /// Negative controlling node.
    pub cn: NodeRef,
    /// Transconductance, siemens.
    pub gm: f64,
}

/// A symmetric current limit in an op-amp's Norton output stage.
///
/// The output current is `clamp((v(drive) − v(output))/ROUT, −IOLIM, IOLIM)`.
/// The drive node is only a control: the limited output current does not load
/// the op-amp's internal gain node.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputCurrentLimit {
    /// Element name (`<X name>#go`).
    pub name: String,
    /// Internal op-amp drive node, used as the Thevenin voltage.
    pub drive: NodeRef,
    /// External output pin.
    pub output: NodeRef,
    /// Small-signal output resistance, ohms.
    pub rout: f64,
    /// Symmetric output-current limit, amps.
    pub ilim: f64,
}

/// A slew-limited op-amp compensation-stage transconductance.
///
/// At an operating point it contributes the full `gm · error` current. During
/// transient steps it compensates the gain-node `Rg` path and clamps only the
/// capacitor current to `±ilim`, preserving settled gain while limiting
/// `Cg · dV/dt`. `error` includes differential input, common-mode error
/// relative to the supply midpoint, and input offset voltage.
#[derive(Debug, Clone, PartialEq)]
pub struct SlewLimitedVccs {
    /// Element name (`<X name>#g`).
    pub name: String,
    /// Terminal the current leaves the circuit at.
    pub p: NodeRef,
    /// Terminal the current returns to the circuit at.
    pub n: NodeRef,
    /// Non-inverting input pin.
    pub input_plus: NodeRef,
    /// Inverting input pin.
    pub input_minus: NodeRef,
    /// Positive supply pin, used for the common-mode reference.
    pub vcc: NodeRef,
    /// Negative supply pin, used for the common-mode reference.
    pub vee: NodeRef,
    /// Transconductance, siemens.
    pub gm: f64,
    /// Common-mode error gain (linear, not dB).
    pub cm_gain: f64,
    /// Input offset voltage, volts.
    pub vos: f64,
    /// Symmetric gain-node capacitor current limit, amps.
    pub ilim: f64,
}

/// `F<name> n+ n- <vsource> <gain>` — current-controlled current source: a
/// current `gain · i(vsource)` flows from `n+` through the source to `n−`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cccs {
    /// Element name as written.
    pub name: String,
    /// Terminal the current leaves the circuit at.
    pub p: NodeRef,
    /// Terminal the current returns to the circuit at.
    pub n: NodeRef,
    /// Name of the element whose branch current controls this one.
    pub control: String,
    /// That element's branch-current index (see [`Circuit::branch_index`]).
    pub control_branch: usize,
    /// Current gain, A/A.
    pub gain: f64,
}

/// `H<name> n+ n- <vsource> <r>` — current-controlled voltage source:
/// `v(n+) − v(n−) = r · i(vsource)`. Adds a branch current.
#[derive(Debug, Clone, PartialEq)]
pub struct Ccvs {
    /// Element name as written.
    pub name: String,
    /// Positive output terminal.
    pub p: NodeRef,
    /// Negative output terminal.
    pub n: NodeRef,
    /// Name of the element whose branch current controls this one.
    pub control: String,
    /// That element's branch-current index (see [`Circuit::branch_index`]).
    pub control_branch: usize,
    /// Transresistance, ohms.
    pub ohms: f64,
}

/// `Q<name> nc nb ne [ns] <model> [<area>] [m=<count>] [temp=<°C>]` — an Ebers–Moll bipolar transistor.
#[derive(Debug, Clone, PartialEq)]
pub struct Bjt {
    /// Element name as written, e.g. `Q1`.
    pub name: String,
    /// SPICE instance multiplier `M`: the element behaves as this many
    /// identical devices in parallel. Every current, conductance and charge it
    /// stamps, including its terminal resistances and GMIN, is scaled by it,
    /// as ngspice's `*load.c` does. `1` for a single device.
    pub multiplier: f64,
    /// SPICE `off`: the operating point starts with both junctions at 0 V
    /// and holds them there until the rest of the circuit has converged
    /// (ngspice's `MODEINITFIX`). See [`Diode::off`].
    pub off: bool,
    /// SPICE instance `ic=<vbe>,<vce>`, terminal volts as written (an NPN
    /// sign convention for both types, as ngspice's `BJTicVBE`/`BJTicVCE`).
    /// Under `.tran ... uic` the junction charges start from `vbe` and
    /// `vbc = vbe - vce`. Ignored without `uic`.
    pub ic: Option<[f64; 2]>,
    /// Collector.
    pub c: NodeRef,
    /// Base.
    pub b: NodeRef,
    /// Emitter.
    pub e: NodeRef,
    /// Substrate: the optional fourth terminal, ground when omitted (as in
    /// ngspice). Its junction is always the internal diode
    /// `<name>#substrate` in [`Circuit::diodes`]; with `ISS = CJS = 0` it
    /// carries only GMIN.
    pub s: NodeRef,
    /// Collector junction node: the intrinsic collector. Without `RCO` it
    /// equals `c` when `RC = 0`, else the internal node `<name>#collector`.
    /// With `RCO` it is always the internal node `<name>#collector`.
    pub junction_c: NodeRef,
    /// Collector contact behind `RC`: the outer end of the `RCO` epitaxial
    /// region. Equals `junction_c` without `RCO`; with `RCO` it equals `c`
    /// when `RC = 0`, else the internal node `<name>#collcx`.
    pub junction_cx: NodeRef,
    /// Base junction node. Equals `b` when `RB = 0`, else the internal node
    /// `<name>#base`.
    pub junction_b: NodeRef,
    /// Emitter junction node. Equals `e` when `RE = 0`, else the internal
    /// node `<name>#emitter`.
    pub junction_e: NodeRef,
    /// Model name as written, for diagnostics.
    pub model_name: String,
    /// Resolved model parameters, already scaled by the instance `AREA`.
    pub model: BjtModel,
    /// Model parameters adjusted for this instance's explicit `TEMP`.
    pub operating_model: BjtModel,
    /// Junction thermal voltage for this instance.
    pub thermal_voltage: f64,
    /// Explicit instance junction temperature in Celsius. `None` preserves
    /// the engine's historical fixed 300 K behavior.
    pub temperature_celsius: Option<f64>,
}

/// `M<name> nd ng ns nb <model> [w=<m>] [l=<m>] [ad=<m²>] [as=<m²>] [pd=<m>] [ps=<m>] [nrd=<squares>] [nrs=<squares>] [m=<count>] [temp=<°C>]` — a level-1 MOSFET.
#[derive(Debug, Clone, PartialEq)]
pub struct Mosfet {
    /// Element name as written, e.g. `M1`.
    pub name: String,
    /// SPICE instance multiplier `M`: the element behaves as this many
    /// identical devices in parallel. Every current, conductance and charge it
    /// stamps, including its terminal resistances and GMIN, is scaled by it,
    /// as ngspice's `*load.c` does. `1` for a single device.
    pub multiplier: f64,
    /// SPICE `off`: the operating point starts with `Vgs`, `Vds` and the
    /// bulk junctions at 0 V and holds them there until the rest of the
    /// circuit has converged (ngspice's `MODEINITFIX`). See [`Diode::off`].
    pub off: bool,
    /// SPICE instance `ic=<vds>[,<vgs>[,<vbs>]]`, terminal volts as written
    /// (an NMOS sign convention for both types, as ngspice's `MOS1icVDS`,
    /// `MOS1icVGS` and `MOS1icVBS`). Under `.tran ... uic` the bulk-junction,
    /// overlap and Meyer charges start from these voltages; a value left out
    /// is taken from the terminal nodes at the uic start (`.ic` or 0 V), as
    /// ngspice's `MOS1getic` does. Ignored without `uic`.
    pub ic: [Option<f64>; 3],
    /// Drain.
    pub d: NodeRef,
    /// Gate.
    pub g: NodeRef,
    /// Source.
    pub s: NodeRef,
    /// Intrinsic drain after the effective drain resistance, or `d` when it
    /// is zero.
    pub junction_d: NodeRef,
    /// Intrinsic source before the effective source resistance, or `s` when
    /// it is zero.
    pub junction_s: NodeRef,
    /// Bulk. Its drain and source pn junctions use the MOS model's `IS`, or
    /// `JS` times `AD`/`AS` (see [`Self::junction_saturation_currents`]), and
    /// `GAMMA`/`PHI` use its bias relative to the effective source for body effect.
    pub bulk: NodeRef,
    /// Model name as written, for diagnostics.
    pub model_name: String,
    /// Resolved model parameters, as written on the card.
    pub model: MosModel,
    /// `KP`, `VTO`, `PHI`, bulk-junction `IS`/`PB` and junction capacitances
    /// adjusted for this instance's explicit `TEMP`; what the solver stamps.
    pub operating_model: MosModel,
    /// Explicit instance temperature, in Celsius. `None` preserves the
    /// historical fixed-300 K behaviour.
    pub temperature_celsius: Option<f64>,
    /// `KP·W/(L − 2·LD)` at the instance temperature, A/V² — the
    /// transconductance the solver actually stamps.
    pub beta: f64,
    /// Instance channel width `W`, metres.
    pub w: f64,
    /// Instance drawn channel length `L`, metres. See
    /// [`Self::effective_length`] for the length the physics uses.
    pub l: f64,
    /// Drain diffusion area `AD`, m², used with model `CJ` when `CBD` is absent.
    pub ad: f64,
    /// Source diffusion area `AS`, m², used with model `CJ` when `CBS` is absent.
    pub as_: f64,
    /// Drain diffusion perimeter `PD`, metres, used with model `CJSW`.
    pub pd: f64,
    /// Source diffusion perimeter `PS`, metres, used with model `CJSW`.
    pub ps: f64,
    /// Effective drain terminal resistance, from `RD` or `RSH·NRD`, ohms.
    pub drain_resistance_ohms: f64,
    /// Effective source terminal resistance, from `RS` or `RSH·NRS`, ohms.
    pub source_resistance_ohms: f64,
}

impl Mosfet {
    /// Bulk-drain and bulk-source junction saturation currents, amps, at the
    /// instance temperature. ngspice level 1 (`mos1load.c`) uses `JS·AD` and
    /// `JS·AS` only when `JS`, `AD` and `AS` are all non-zero; otherwise both
    /// junctions use `IS`.
    pub fn junction_saturation_currents(&self) -> (f64, f64) {
        let model = &self.operating_model;
        if model.js == 0.0 || self.ad == 0.0 || self.as_ == 0.0 {
            (model.is, model.is)
        } else {
            (model.js * self.ad, model.js * self.as_)
        }
    }

    /// Thermal voltage of the bulk junctions: `kT/q` at the explicit instance
    /// `TEMP`, or the engine's fixed [`super::device::THERMAL_VOLTAGE`].
    pub fn thermal_voltage(&self) -> f64 {
        self.temperature_celsius
            .map_or(super::device::THERMAL_VOLTAGE, |celsius| {
                super::device::thermal_voltage_at_temperature(celsius + 273.15)
            })
    }

    /// Effective channel length `L − 2·LD`, metres: the length ngspice's
    /// level 1 uses for `β`, intrinsic oxide capacitance and `CGBO`.
    pub fn effective_length(&self) -> f64 {
        self.l - 2.0 * self.model.ld
    }

    /// Effective linear gate overlap capacitances in G-S, G-D, G-B order.
    pub fn overlap_capacitances(&self) -> [f64; 3] {
        [
            self.model.cgso * self.w,
            self.model.cgdo * self.w,
            self.model.cgbo * self.effective_length(),
        ]
    }

    /// Zero-bias bottom and sidewall capacitances for the selected bulk
    /// junction, in that order.
    pub fn bulk_junction_capacitances(&self, drain: bool) -> [f64; 2] {
        let model = self.operating_model;
        if drain {
            [
                if model.cbd_given {
                    model.cbd
                } else {
                    model.cj * self.ad
                },
                model.cjsw * self.pd,
            ]
        } else {
            [
                if model.cbs_given {
                    model.cbs
                } else {
                    model.cj * self.as_
                },
                model.cjsw * self.ps,
            ]
        }
    }

    /// Nonlinear bulk-junction charge and differential capacitance in
    /// normalized MOSFET coordinates. `drain` selects the bulk-drain junction;
    /// otherwise the bulk-source junction is used. Explicit `CBD`/`CBS`
    /// overrides only the bottom-area term; sidewall charge remains additive.
    pub fn bulk_junction_charge(&self, voltage: f64, drain: bool) -> (f64, f64) {
        let model = self.operating_model;
        let [bottom, sidewall] = self.bulk_junction_capacitances(drain);
        let (bottom_charge, bottom_capacitance) =
            super::device::junction_depletion_charge(voltage, bottom, model.pb, model.mj, model.fc);
        let (sidewall_charge, sidewall_capacitance) = super::device::junction_depletion_charge(
            voltage, sidewall, model.pb, model.mjsw, model.fc,
        );
        (
            bottom_charge + sidewall_charge,
            bottom_capacitance + sidewall_capacitance,
        )
    }
}

/// `J<name> nd ng ns <model> [<area>] [m=<count>] [temp=<°C>]` — a SPICE level-1 JFET.
#[derive(Debug, Clone, PartialEq)]
pub struct Jfet {
    /// Element name as written, e.g. `J1`.
    pub name: String,
    /// SPICE instance multiplier `M`: the element behaves as this many
    /// identical devices in parallel. Every current, conductance and charge it
    /// stamps, including its terminal resistances and GMIN, is scaled by it,
    /// as ngspice's `*load.c` does. `1` for a single device.
    pub multiplier: f64,
    /// SPICE `off`: the operating point starts with both gate junctions at
    /// 0 V and holds them there until the rest of the circuit has converged
    /// (ngspice's `MODEINITFIX`). See [`Diode::off`].
    pub off: bool,
    /// SPICE instance `ic=<vds>,<vgs>`, terminal volts as written (an NJF
    /// sign convention for both types, as ngspice's `JFETicVDS`/`JFETicVGS`).
    /// Under `.tran ... uic` the gate charges start from `vgs` and
    /// `vgd = vgs - vds`. Ignored without `uic`.
    pub ic: Option<[f64; 2]>,
    /// Drain.
    pub d: NodeRef,
    /// Gate.
    pub g: NodeRef,
    /// Source.
    pub s: NodeRef,
    /// Intrinsic drain after `RD`, or `d` when `RD` is zero.
    pub junction_d: NodeRef,
    /// Intrinsic source before `RS`, or `s` when `RS` is zero.
    pub junction_s: NodeRef,
    /// Model name as written, for diagnostics.
    pub model_name: String,
    /// Resolved model parameters, already scaled by the instance `AREA`.
    pub model: JfetModel,
    /// Channel `VTO`/`BETA` and gate-junction `IS`/`PB`/`CGS`/`CGD` adjusted
    /// for this instance's explicit `TEMP`.
    pub operating_model: JfetModel,
    /// Explicit instance temperature, in Celsius. `None` preserves the
    /// historical channel and gate parameters.
    pub temperature_celsius: Option<f64>,
}

impl Jfet {
    /// Thermal voltage of the gate junctions: `kT/q` at the explicit instance
    /// `TEMP`, or the engine's fixed [`super::device::THERMAL_VOLTAGE`].
    pub fn thermal_voltage(&self) -> f64 {
        self.temperature_celsius
            .map_or(super::device::THERMAL_VOLTAGE, |celsius| {
                super::device::thermal_voltage_at_temperature(celsius + 273.15)
            })
    }
}

/// Voltage threshold control for a switch: closes from the voltage between
/// `pos` and `neg` rather than from a routed boolean input.
///
/// `|V(pos) − V(neg)|` is compared to `vt` with hysteresis `vh`. With
/// `invert == false` (NO contact) the switch closes at `|V| >= vt` and stays
/// closed until `|V|` falls through `vt − vh`. With `invert == true` (NC
/// contact) the switch stays closed while `|V| < vt` and only re-closes from
/// open at `|V| <= vt − vh`, so the contact is closed while the coil is
/// de-energised.
///
/// An NO/NC pair that shares the same sense nodes transfers atomically in one
/// solver update (no break-before-make delay).
#[derive(Debug, Clone, PartialEq)]
pub struct VoltageSwitchControl {
    /// Positive sense node.
    pub pos: NodeRef,
    /// Negative sense node.
    pub neg: NodeRef,
    /// Closing threshold in volts (`|V|` rising through `vt`).
    pub vt: f64,
    /// Hysteresis in volts; the opening threshold is `vt − vh` (clamped at 0).
    pub vh: f64,
    /// When true, the contact is the complement (NC) of the NO threshold rule.
    pub invert: bool,
}

/// What a SPICE `SW` or `CSW` switch compares to its threshold.
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchSense {
    /// `S`: the signed voltage `V(pos) − V(neg)`.
    Voltage {
        /// Positive control node.
        pos: NodeRef,
        /// Negative control node.
        neg: NodeRef,
    },
    /// `W`: the signed branch current of the named element.
    Current {
        /// Controlling element as written (a `V`, `L`, `E` or `H`).
        control: String,
        /// Its branch-current index, bound after the last line.
        branch: usize,
    },
}

/// SPICE's `SW`/`CSW` control: the switch closes when the sensed quantity
/// rises above `threshold + hysteresis`, opens when it falls below
/// `threshold − hysteresis`, and keeps its state in between. The quantity is
/// signed, as in ngspice's `swload.c`, unlike [`VoltageSwitchControl`].
#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdSwitchControl {
    /// The sensed voltage or current.
    pub sense: SwitchSense,
    /// `VT` in volts or `IT` in amperes.
    pub threshold: f64,
    /// `VH` in volts or `IH` in amperes; never negative.
    pub hysteresis: f64,
    /// The instance `on` flag: the state the operating point starts from,
    /// which is the state it keeps inside the hysteresis band.
    pub initially_closed: bool,
}

/// A `.model <name> SW(...)` or `CSW(...)` card, with ngspice's defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchModel {
    /// True for `CSW` (current controlled), false for `SW`.
    pub current: bool,
    /// `VT` or `IT`; default 0.
    pub threshold: f64,
    /// `VH` or `IH`; default 0.
    pub hysteresis: f64,
    /// `RON` in ohms; default 1.
    pub ron: f64,
    /// `ROFF` in ohms; default 1e12 (ngspice's `1/GMIN`).
    pub roff: f64,
}

impl SwitchModel {
    fn defaults(current: bool) -> Self {
        Self {
            current,
            threshold: 0.0,
            hysteresis: 0.0,
            ron: 1.0,
            roff: 1e12,
        }
    }
}

/// `S<name> n1 n2 <ctrl> ron=<r> roff=<r>` — an ideal switch whose control is a
/// routed boolean input name, not a circuit node.
///
/// Or `S<name> n1 n2 nc+ nc- vt=<v> [vh=<v>] [invert=0|1] ron=<r> roff=<r>` —
/// the same switch closed by the absolute voltage between `nc+` and `nc-`.
/// When `voltage` is `Some`, `ctrl` is empty and the adapter does not expose a
/// routed input for this element.
///
/// Or SPICE's own `S<name> n+ n- nc+ nc- <model> [on|off]` and
/// `W<name> n+ n- <element> <model> [on|off]`, whose `SW`/`CSW` card gives the
/// resistances and [`Self::threshold`] the control; `ctrl` is empty there too.
#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    /// Element name as written.
    pub name: String,
    /// First terminal.
    pub a: NodeRef,
    /// Second terminal.
    pub b: NodeRef,
    /// Name of the routed boolean input that opens and closes it.
    /// Empty when [`Self::voltage`] is set.
    pub ctrl: String,
    /// Closed resistance in ohms.
    pub ron: f64,
    /// Open resistance in ohms.
    pub roff: f64,
    /// Voltage-threshold control, when the switch is not digitally routed.
    pub voltage: Option<VoltageSwitchControl>,
    /// SPICE `SW`/`CSW` control, for an `S` or `W` element that names a model.
    pub threshold: Option<ThresholdSwitchControl>,
}

impl Switch {
    /// True when a routed boolean input, not the circuit, sets the state.
    pub fn is_routed(&self) -> bool {
        self.voltage.is_none() && self.threshold.is_none()
    }
}

/// `T<name> n1+ n1- n2+ n2- Z0=<ohms> TD=<seconds>` — SPICE's lossless
/// transmission line, two ports joined by a pure delay.
///
/// Each port owns a branch current that enters its `+` node; see
/// [`Circuit::transmission_line_branch`].
#[derive(Debug, Clone, PartialEq)]
pub struct TransmissionLine {
    /// Element name as written.
    pub name: String,
    /// Port 1 `+` node.
    pub p1: NodeRef,
    /// Port 1 `-` node.
    pub n1: NodeRef,
    /// Port 2 `+` node.
    pub p2: NodeRef,
    /// Port 2 `-` node.
    pub n2: NodeRef,
    /// Characteristic impedance `Z0`, ohms.
    pub z0: f64,
    /// One-way delay `TD`, seconds; `NL / F` when the line gives a frequency.
    pub delay: f64,
}

/// A parsed netlist: elements in file order plus the node table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Circuit {
    node_names: Vec<String>,
    node_index: BTreeMap<String, usize>,
    /// Resistors, in netlist order.
    pub resistors: Vec<Resistor>,
    /// Capacitors, in netlist order.
    pub capacitors: Vec<Capacitor>,
    /// Inductors, in netlist order.
    pub inductors: Vec<Inductor>,
    /// Pairwise mutual inductances, in netlist order.
    pub mutual_inductors: Vec<MutualInductor>,
    /// Voltage sources, in netlist order.
    pub voltage_sources: Vec<VoltageSource>,
    /// Current sources, in netlist order.
    pub current_sources: Vec<CurrentSource>,
    /// Switches, in netlist order.
    pub switches: Vec<Switch>,
    /// Diodes, in netlist order.
    pub diodes: Vec<Diode>,
    /// Bipolar transistors, in netlist order.
    pub bjts: Vec<Bjt>,
    /// MOSFETs, in netlist order.
    pub mosfets: Vec<Mosfet>,
    /// JFETs, in netlist order.
    pub jfets: Vec<Jfet>,
    /// Voltage-controlled voltage sources (`E`), in netlist order.
    pub vcvs: Vec<Vcvs>,
    /// Voltage-controlled current sources (`G`), in netlist order.
    pub vccs: Vec<Vccs>,
    /// Current-controlled current sources (`F`), in netlist order.
    pub cccs: Vec<Cccs>,
    /// Current-controlled voltage sources (`H`), in netlist order.
    pub ccvs: Vec<Ccvs>,
    /// Behavioural voltage sources (`B ... V=`, `E ... VALUE=`), in netlist
    /// order. Each adds a branch current.
    pub behavioural_voltages: Vec<BehaviouralSource>,
    /// Behavioural current sources (`B ... I=`, `G ... VALUE=`), in netlist
    /// order.
    pub behavioural_currents: Vec<BehaviouralSource>,
    /// Lossless transmission lines (`T`), in netlist order. Each adds two
    /// branch currents.
    pub transmission_lines: Vec<TransmissionLine>,
    /// Power-balance sinks, one per lowered `BUCK`.
    pub power_sinks: Vec<PowerSink>,
    /// Op-amp output stages with an explicit `IOLIM`.
    pub output_current_limits: Vec<OutputCurrentLimit>,
    /// Op-amp compensation stages with an explicit `SR`.
    pub slew_limited_vccs: Vec<SlewLimitedVccs>,
    /// `.ic V(node)=value` entries, in netlist order.
    pub node_ic: Vec<(usize, f64)>,
    /// `.tran ... uic`: start the transient from the initial conditions
    /// instead of from a solved operating point (see
    /// [`crate::analog::Solver::solve_operating_point`]).
    pub uic: bool,
}

impl Circuit {
    /// Number of non-ground nodes (the `N` of the MNA system).
    pub fn node_count(&self) -> usize {
        self.node_names.len()
    }

    /// Name of node `index`, as first written in the netlist.
    pub fn node_name(&self, index: usize) -> &str {
        &self.node_names[index]
    }

    /// Resolve a node name. `Some(None)` is ground; `None` means the netlist
    /// never mentions that name — which is why probes can be checked up front
    /// instead of silently reading zero.
    pub fn node(&self, name: &str) -> Option<NodeRef> {
        let key = name.trim().to_ascii_lowercase();
        if is_ground(&key) {
            return Some(None);
        }
        self.node_index.get(&key).map(|index| Some(*index))
    }

    /// Number of branch currents (the `M`): one per voltage source, then one
    /// per inductor, then one per `E`, then one per `H`, then one per
    /// behavioural voltage source, then two per `T` line, in that order.
    ///
    /// The dependent sources come last so that adding one to a netlist never
    /// renumbers a voltage source's or inductor's branch.
    pub fn branch_count(&self) -> usize {
        self.transmission_line_branch(self.transmission_lines.len())
    }

    /// Branch-current index of port 1 of `T` line `index`; port 2 is the
    /// next one. Each is the current entering the port's `+` node.
    pub fn transmission_line_branch(&self, index: usize) -> usize {
        self.voltage_sources.len()
            + self.inductors.len()
            + self.vcvs.len()
            + self.ccvs.len()
            + self.behavioural_voltages.len()
            + 2 * index
    }

    /// Branch-current index of the named voltage source, inductor, `E`, `H`
    /// or behavioural voltage source.
    pub fn branch_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.voltage_sources
            .iter()
            .map(|e| &e.name)
            .chain(self.inductors.iter().map(|e| &e.name))
            .chain(self.vcvs.iter().map(|e| &e.name))
            .chain(self.ccvs.iter().map(|e| &e.name))
            .chain(self.behavioural_voltages.iter().map(|e| &e.name))
            .position(|candidate| candidate.to_ascii_lowercase() == key)
    }

    /// Name of the element owning branch current `index`.
    pub fn branch_name(&self, index: usize) -> &str {
        let mut index = index;
        if index < self.voltage_sources.len() {
            return &self.voltage_sources[index].name;
        }
        index -= self.voltage_sources.len();
        if index < self.inductors.len() {
            return &self.inductors[index].name;
        }
        index -= self.inductors.len();
        if index < self.vcvs.len() {
            return &self.vcvs[index].name;
        }
        index -= self.vcvs.len();
        if index < self.ccvs.len() {
            return &self.ccvs[index].name;
        }
        index -= self.ccvs.len();
        if index < self.behavioural_voltages.len() {
            return &self.behavioural_voltages[index].name;
        }
        &self.transmission_lines[(index - self.behavioural_voltages.len()) / 2].name
    }

    /// `N + M`: the dimension of the MNA system.
    pub fn unknowns(&self) -> usize {
        self.node_count() + self.branch_count()
    }

    fn intern(&mut self, name: &str) -> NodeRef {
        let key = name.trim().to_ascii_lowercase();
        if is_ground(&key) {
            return None;
        }
        if let Some(index) = self.node_index.get(&key) {
            return Some(*index);
        }
        let index = self.node_names.len();
        self.node_names.push(name.trim().to_string());
        self.node_index.insert(key, index);
        Some(index)
    }

    fn has_element(&self, name: &str, pending_mutuals: &[PendingMutualInductor]) -> bool {
        let key = name.to_ascii_lowercase();
        let matches = |candidate: &str| candidate.to_ascii_lowercase() == key;
        self.resistors.iter().any(|e| matches(&e.name))
            || self.capacitors.iter().any(|e| matches(&e.name))
            || self.inductors.iter().any(|e| matches(&e.name))
            || self.voltage_sources.iter().any(|e| matches(&e.name))
            || self.current_sources.iter().any(|e| matches(&e.name))
            || self.switches.iter().any(|e| matches(&e.name))
            || self.diodes.iter().any(|e| matches(&e.name))
            || self.bjts.iter().any(|e| matches(&e.name))
            || self.mosfets.iter().any(|e| matches(&e.name))
            || self.jfets.iter().any(|e| matches(&e.name))
            || self.vcvs.iter().any(|e| matches(&e.name))
            || self.vccs.iter().any(|e| matches(&e.name))
            || self.cccs.iter().any(|e| matches(&e.name))
            || self.ccvs.iter().any(|e| matches(&e.name))
            || self.behavioural_voltages.iter().any(|e| matches(&e.name))
            || self.behavioural_currents.iter().any(|e| matches(&e.name))
            || self.transmission_lines.iter().any(|e| matches(&e.name))
            || self.power_sinks.iter().any(|e| matches(&e.name))
            || self.output_current_limits.iter().any(|e| matches(&e.name))
            || self.slew_limited_vccs.iter().any(|e| matches(&e.name))
            || pending_mutuals.iter().any(|e| matches(&e.name))
    }

    /// True when the circuit holds at least one device whose stamp depends on
    /// the solution, i.e. when a step needs Newton iteration.
    ///
    /// This is the switch that keeps the linear engine exactly what it was:
    /// when it is false, [`super::mna::Solver::advance`] runs the same code,
    /// in the same order, on the same values as before diodes existed.
    pub fn is_nonlinear(&self) -> bool {
        !self.diodes.is_empty()
            || !self.bjts.is_empty()
            || !self.mosfets.is_empty()
            || !self.jfets.is_empty()
            || !self.power_sinks.is_empty()
            || !self.output_current_limits.is_empty()
            || !self.slew_limited_vccs.is_empty()
            || !self.behavioural_voltages.is_empty()
            || !self.behavioural_currents.is_empty()
    }

    /// Whether any semiconductor carries SPICE's `off` flag.
    pub fn has_off_devices(&self) -> bool {
        self.diodes.iter().any(|device| device.off)
            || self.bjts.iter().any(|device| device.off)
            || self.mosfets.iter().any(|device| device.off)
            || self.jfets.iter().any(|device| device.off)
    }

    /// True when any independent source carries a transient function, so the
    /// solver has to re-evaluate sources against the clock each step.
    pub fn has_waveforms(&self) -> bool {
        self.voltage_sources.iter().any(|s| !s.wave.is_constant())
            || self.current_sources.iter().any(|s| !s.wave.is_constant())
            || self
                .behavioural_voltages
                .iter()
                .chain(&self.behavioural_currents)
                .any(|s| s.expression.uses_time())
    }
}

fn is_ground(lowercased: &str) -> bool {
    lowercased == "0" || lowercased == "gnd" || lowercased == "ground"
}

/// Parse a SPICE value with an optional engineering suffix (`10k`, `100n`,
/// `2.2meg`, `1e-6`). Trailing unit letters are ignored, as in SPICE, so
/// `100nF` and `10kohm` mean what they look like.
pub fn parse_spice_value(raw: &str) -> Option<f64> {
    let text = raw.trim();
    let bytes = text.as_bytes();
    let mut index = 0;

    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        index += 1;
    }
    let digits_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
    }
    if index == digits_start || (index == digits_start + 1 && bytes[digits_start] == b'.') {
        return None;
    }
    // An exponent only counts when digits actually follow it: `1e-6` is a
    // number, `1exp` is the number 1 with a suffix SPICE ignores.
    let mut exponent_at = None;
    if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        let mut probe = index + 1;
        if probe < bytes.len() && (bytes[probe] == b'+' || bytes[probe] == b'-') {
            probe += 1;
        }
        if probe < bytes.len() && bytes[probe].is_ascii_digit() {
            while probe < bytes.len() && bytes[probe].is_ascii_digit() {
                probe += 1;
            }
            exponent_at = Some(index);
            index = probe;
        }
    }

    let suffix = text[index..].to_ascii_lowercase();
    if suffix.starts_with("mil") {
        return text[..index].parse::<f64>().ok().map(|mils| mils * 25.4e-6);
    }
    let power: i32 = if suffix.starts_with("meg") {
        6
    } else {
        match suffix.as_bytes().first() {
            None => 0,
            Some(b't') => 12,
            Some(b'g') => 9,
            Some(b'k') => 3,
            Some(b'm') => -3,
            Some(b'u') => -6,
            Some(b'n') => -9,
            Some(b'p') => -12,
            Some(b'f') => -15,
            // Anything else is a unit spelling (`5ohm`, `3volt`), not a scale.
            Some(_) => 0,
        }
    };

    // Scale by moving the decimal exponent rather than multiplying, so `100n`
    // is the same double as `100e-9` — the value a user would get writing it
    // out — instead of `100.0 * 1e-9`, which is one ulp away from it.
    let (mantissa, exponent) = match exponent_at {
        Some(at) => (&text[..at], text[at + 1..index].parse::<i32>().ok()?),
        None => (&text[..index], 0),
    };
    let mantissa = mantissa.strip_suffix('.').unwrap_or(mantissa);
    format!("{mantissa}e{}", exponent + power).parse().ok()
}

/// Strip a `*` comment line and any `;` / `$` trailing comment.
/// `T<name> n1+ n1- n2+ n2- Z0=<ohms> TD=<s>`, or `F=<Hz> [NL=<wavelengths>]`
/// in place of `TD`, as in ngspice: `TD = NL / F`, and `NL` is 0.25 unless
/// given. `ZO` is ngspice's second spelling of `Z0`.
fn parse_transmission_line(
    ctx: &LineCtx<'_>,
    name: &str,
    rest: &[&str],
    circuit: &mut Circuit,
) -> Result<TransmissionLine, AnalogError> {
    const SHAPE: &str = "T<name> n1+ n1- n2+ n2- Z0=<ohms> TD=<seconds>";
    if rest.len() < 5 || rest[..4].iter().any(|node| node.contains('=')) {
        return Err(ctx.parse_err(format!("expected `{SHAPE}`")));
    }
    let (mut z0, mut delay, mut frequency, mut length) = (None, None, None, None);
    for token in &rest[4..] {
        let Some((key, value)) = token.split_once('=') else {
            return Err(ctx.parse_err(format!("expected `{SHAPE}`, found `{token}`")));
        };
        let key = key.to_ascii_lowercase();
        let slot = match key.as_str() {
            "z0" | "zo" => &mut z0,
            "td" => &mut delay,
            "f" => &mut frequency,
            "nl" => &mut length,
            "ic" => {
                return Err(ctx.parse_err(
                    "transmission-line `ic=` is not modelled: the line starts from the \
                     operating point",
                ))
            }
            other => {
                return Err(ctx.parse_err(format!(
                    "unknown parameter `{other}`; expected `{SHAPE}` or `F=<Hz> [NL=<n>]`"
                )))
            }
        };
        if slot.is_some() {
            return Err(ctx.parse_err(format!("`{key}` is given twice")));
        }
        let parsed = ctx.value(value, &key)?;
        if !parsed.is_finite() || parsed <= 0.0 {
            return Err(ctx.parse_err(format!("`{key}` must be positive and finite")));
        }
        *slot = Some(parsed);
    }
    let Some(z0) = z0 else {
        return Err(ctx.parse_err("a transmission line needs `Z0=<ohms>`"));
    };
    let delay = match (delay, frequency) {
        (Some(_), Some(_)) => {
            return Err(ctx.parse_err("give the delay once: either `TD=` or `F=`, not both"))
        }
        (Some(_), None) if length.is_some() => {
            return Err(ctx.parse_err("`NL=` needs `F=`; with `TD=` it has no meaning"))
        }
        (Some(delay), None) => delay,
        (None, Some(frequency)) => length.unwrap_or(0.25) / frequency,
        (None, None) => {
            return Err(ctx.parse_err(
                "a transmission line needs its delay: `TD=<seconds>` or `F=<Hz> [NL=<n>]`",
            ))
        }
    };
    Ok(TransmissionLine {
        name: name.to_string(),
        p1: circuit.intern(rest[0]),
        n1: circuit.intern(rest[1]),
        p2: circuit.intern(rest[2]),
        n2: circuit.intern(rest[3]),
        z0,
        delay,
    })
}

fn strip_comment(line: &str) -> &str {
    let cut = line.find([';', '$']).unwrap_or(line.len());
    line[..cut].trim()
}

struct LineCtx<'a> {
    number: usize,
    text: &'a str,
}

impl LineCtx<'_> {
    fn parse_err(&self, message: impl Into<String>) -> AnalogError {
        AnalogError::Parse {
            line: self.number,
            text: self.text.to_string(),
            message: message.into(),
        }
    }

    fn unsupported(&self) -> AnalogError {
        AnalogError::Unsupported {
            line: self.number,
            text: self.text.to_string(),
        }
    }

    fn value(&self, raw: &str, what: &str) -> Result<f64, AnalogError> {
        parse_spice_value(raw)
            .ok_or_else(|| self.parse_err(format!("{what} `{raw}` is not a SPICE value")))
    }
}

/// One device line as parsed, before its `.model` card is known.
///
/// Model cards may be written after the elements that use them, so the parser
/// collects the terminals in one pass and binds the parameters in a second.
struct PendingDevice {
    line: usize,
    text: String,
    letter: char,
    name: String,
    nodes: Vec<NodeRef>,
    model_name: String,
    width: Option<f64>,
    length: Option<f64>,
    nrd: Option<f64>,
    nrs: Option<f64>,
    ad: Option<f64>,
    as_: Option<f64>,
    pd: Option<f64>,
    ps: Option<f64>,
    temperature_celsius: Option<f64>,
    multiplier: f64,
    area: f64,
    /// Diode junction perimeter `pj=`; `None` uses the model `PJ`.
    perimeter: Option<f64>,
    /// SPICE `off`: start the operating point with this device cut off.
    off: bool,
    /// SPICE instance `ic=`, comma-separated volts; empty when not given.
    ic: Vec<f64>,
}

/// A SPICE `S`/`W` line held until its `SW`/`CSW` card and, for `W`, the
/// controlling element's branch are known.
struct PendingSwitch {
    line: usize,
    text: String,
    /// Position of the placeholder in [`Circuit::switches`].
    index: usize,
    model_name: String,
}

/// True when the tokens after an `S` name are SPICE's `n+ n- nc+ nc- <model>
/// [on|off]`: this engine's two other `S` forms always carry `key=value`.
fn names_switch_model(rest: &[&str]) -> bool {
    (5..=6).contains(&rest.len()) && !rest.iter().any(|token| token.contains('='))
}

/// The optional trailing `on`/`off` of a SPICE switch line whose fixed part
/// is `fixed` tokens long: `Some(true)` for `on`, `Some(false)` for `off` or
/// nothing, `None` when the line has any other shape.
fn switch_flag(rest: &[&str], fixed: usize) -> Option<bool> {
    match rest.len().checked_sub(fixed)? {
        0 => Some(false),
        1 if rest[fixed].eq_ignore_ascii_case("on") => Some(true),
        1 if rest[fixed].eq_ignore_ascii_case("off") => Some(false),
        _ => None,
    }
}

/// Give every SPICE `S`/`W` switch its card and bind a `W`'s controlling
/// branch, once every model and every branch-carrying element exists.
fn bind_switches(
    circuit: &mut Circuit,
    models: &BTreeMap<String, ModelCard>,
    pending: Vec<PendingSwitch>,
) -> Result<(), AnalogError> {
    for entry in pending {
        let ctx = LineCtx {
            number: entry.line,
            text: &entry.text,
        };
        let name = circuit.switches[entry.index].name.clone();
        let current = matches!(
            circuit.switches[entry.index].threshold,
            Some(ThresholdSwitchControl {
                sense: SwitchSense::Current { .. },
                ..
            })
        );
        let (letter, wanted) = if current { ('W', "CSW") } else { ('S', "SW") };
        let model = match models.get(&entry.model_name.to_ascii_lowercase()) {
            Some(ModelCard::Switch(model)) if model.current == current => *model,
            Some(card) => {
                return Err(ctx.parse_err(format!(
                "`{name}` is a {letter} element and needs a {wanted} model, but `{}` is a {} model",
                entry.model_name,
                card.kind()
            )))
            }
            None => {
                return Err(ctx.parse_err(format!(
                    "no `.model {} {wanted}(...)` line for `{name}`",
                    entry.model_name
                )))
            }
        };
        let branch = match &circuit.switches[entry.index].threshold {
            Some(ThresholdSwitchControl {
                sense: SwitchSense::Current { control, .. },
                ..
            }) => Some(circuit.branch_index(control).ok_or_else(|| {
                ctx.parse_err(format!(
                    "`{control}` carries no branch current; a W element must name a \
                     V, L, E or H element"
                ))
            })?),
            _ => None,
        };
        let switch = &mut circuit.switches[entry.index];
        switch.ron = model.ron;
        switch.roff = model.roff;
        let control = switch
            .threshold
            .as_mut()
            .expect("placeholder has a control");
        control.threshold = model.threshold;
        control.hysteresis = model.hysteresis;
        if let (SwitchSense::Current { branch: slot, .. }, Some(branch)) =
            (&mut control.sense, branch)
        {
            *slot = branch;
        }
    }
    Ok(())
}

/// A K line held until all its L elements have been parsed.
struct PendingMutualInductor {
    line: usize,
    text: String,
    name: String,
    first: String,
    second: String,
    coupling: f64,
}

/// One logical netlist line: comments stripped, `+` continuations joined.
struct LogicalLine {
    /// 1-based source line the logical line starts on.
    number: usize,
    text: String,
}

/// Join SPICE `+` continuation lines onto the line they continue.
///
/// Comment and blank lines between a line and its continuations are skipped,
/// as ngspice does, so a vendor model card with commented-out parameters
/// still reads as one card. A logical line keeps the number of its first
/// physical line, so errors point at where the card starts.
fn logical_lines(text: &str) -> Result<Vec<LogicalLine>, AnalogError> {
    let mut lines: Vec<LogicalLine> = Vec::new();
    for (offset, raw_line) in text.lines().enumerate() {
        let number = offset + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('*') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('+') {
            let Some(previous) = lines.last_mut() else {
                return Err(AnalogError::Parse {
                    line: number,
                    text: trimmed.to_string(),
                    message: "continuation line `+` has no line to continue".into(),
                });
            };
            let rest = strip_comment(rest);
            if !rest.is_empty() {
                previous.text.push(' ');
                previous.text.push_str(rest);
            }
            continue;
        }
        let line = strip_comment(trimmed);
        if !line.is_empty() {
            lines.push(LogicalLine {
                number,
                text: line.to_string(),
            });
        }
    }
    Ok(lines)
}

/// How deep `.include` and `.lib` calls may nest before the deck is refused.
const MAX_INCLUDE_DEPTH: usize = 16;

/// The file argument of a `.include` or `.lib` line, quotes removed.
fn unquote(raw: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = raw.strip_prefix(quote).and_then(|r| r.strip_suffix(quote)) {
            return inner;
        }
    }
    raw
}

/// Splices `.include <file>` and `.lib <file> <section>` into the deck.
struct IncludeExpander<'r> {
    read: &'r mut dyn FnMut(&Path) -> Result<String, String>,
    /// Files being expanded, outermost first, for the cycle check.
    stack: Vec<PathBuf>,
}

impl IncludeExpander<'_> {
    /// Expand every call in `lines`, which were read from a file in `dir`.
    ///
    /// `top` is the line of the outermost call when `lines` came from an
    /// included file: every line spliced in carries it, so an error inside a
    /// library points at the deck line that pulled the library in, and the
    /// message names the file and its own line.
    fn expand(
        &mut self,
        lines: Vec<LogicalLine>,
        dir: &Path,
        top: Option<usize>,
    ) -> Result<Vec<LogicalLine>, AnalogError> {
        let mut out = Vec::with_capacity(lines.len());
        let mut in_control = false;
        for line in lines {
            let number = top.unwrap_or(line.number);
            let directive = directive_of(&line.text);
            if in_control {
                in_control = directive.as_deref() != Some("endc");
                out.push(LogicalLine { number, ..line });
                continue;
            }
            let tokens: Vec<&str> = line.text.split_whitespace().collect();
            let ctx = LineCtx {
                number,
                text: &line.text,
            };
            match directive.as_deref() {
                Some("control") => in_control = true,
                Some("include") => {
                    let [_, file] = tokens[..] else {
                        return Err(ctx.parse_err("expected `.include <file>`"));
                    };
                    let (path, text) = self.load(&ctx, dir, file)?;
                    let spliced = self.file_lines(&ctx, &path, &text, None)?;
                    out.extend(spliced);
                    self.stack.pop();
                    continue;
                }
                // `.lib <file> <section>` calls a section. A one-token `.lib`
                // opens a section definition, which only a library file read
                // through a call may hold; elsewhere it stays unsupported.
                Some("lib") if tokens.len() == 3 => {
                    let (path, text) = self.load(&ctx, dir, tokens[1])?;
                    let spliced = self.file_lines(&ctx, &path, &text, Some(tokens[2]))?;
                    out.extend(spliced);
                    self.stack.pop();
                    continue;
                }
                Some("end") if top.is_some() => {
                    // ngspice drops a `.end` inside an included file and keeps
                    // reading the lines after it.
                    continue;
                }
                _ => {}
            }
            out.push(LogicalLine { number, ..line });
        }
        Ok(out)
    }

    /// Resolve `file` against `dir`, refuse a cycle, read it and push it on
    /// the stack. The caller pops it once its lines are spliced.
    fn load(
        &mut self,
        ctx: &LineCtx<'_>,
        dir: &Path,
        file: &str,
    ) -> Result<(PathBuf, String), AnalogError> {
        let file = unquote(file);
        let path = dir.join(file);
        if self.stack.contains(&path) {
            return Err(ctx.parse_err(format!(
                "`{}` includes itself through {}",
                path.display(),
                self.stack
                    .iter()
                    .map(|p| format!("`{}`", p.display()))
                    .collect::<Vec<_>>()
                    .join(" -> ")
            )));
        }
        if self.stack.len() >= MAX_INCLUDE_DEPTH {
            return Err(ctx.parse_err(format!(
                "`.include`/`.lib` calls nest deeper than {MAX_INCLUDE_DEPTH} files"
            )));
        }
        let text = (self.read)(&path)
            .map_err(|err| ctx.parse_err(format!("cannot read `{}`: {err}", path.display())))?;
        self.stack.push(path.clone());
        Ok((path, text))
    }

    /// The expanded lines of a file read for the call on `ctx`: all of it for
    /// `.include`, one section's body for `.lib`.
    fn file_lines(
        &mut self,
        ctx: &LineCtx<'_>,
        path: &Path,
        text: &str,
        section: Option<&str>,
    ) -> Result<Vec<LogicalLine>, AnalogError> {
        let in_file = |err: AnalogError| match err {
            AnalogError::Parse {
                line,
                text,
                message,
            } => ctx.parse_err(format!("{}:{line}: `{text}`: {message}", path.display())),
            other => other,
        };
        let lines = logical_lines(text).map_err(in_file)?;
        let lines = match section {
            None => lines,
            Some(section) => self.section(ctx, path, lines, section)?,
        };
        let dir = path.parent().unwrap_or(Path::new(""));
        self.expand(lines, dir, Some(ctx.number))
    }

    /// The body of `.lib <section>` … `.endl` in a library file.
    fn section(
        &self,
        ctx: &LineCtx<'_>,
        path: &Path,
        lines: Vec<LogicalLine>,
        wanted: &str,
    ) -> Result<Vec<LogicalLine>, AnalogError> {
        let mut body = None;
        let mut open: Option<(String, Vec<LogicalLine>)> = None;
        for line in lines {
            let tokens: Vec<&str> = line.text.split_whitespace().collect();
            match directive_of(&line.text).as_deref() {
                Some("lib") if tokens.len() == 2 => {
                    if let Some((name, _)) = &open {
                        return Err(ctx.parse_err(format!(
                            "{}:{}: section `{}` opens inside section `{name}`",
                            path.display(),
                            line.number,
                            tokens[1]
                        )));
                    }
                    open = Some((tokens[1].to_string(), Vec::new()));
                }
                Some("endl") => {
                    let Some((name, lines)) = open.take() else {
                        return Err(ctx.parse_err(format!(
                            "{}:{}: `.endl` closes no section",
                            path.display(),
                            line.number
                        )));
                    };
                    if name.eq_ignore_ascii_case(wanted) && body.is_none() {
                        body = Some(lines);
                    }
                }
                _ => {
                    if let Some((_, lines)) = &mut open {
                        lines.push(line);
                    }
                }
            }
        }
        if let Some((name, _)) = open {
            return Err(ctx.parse_err(format!(
                "{}: section `{name}` has no `.endl`",
                path.display()
            )));
        }
        body.ok_or_else(|| {
            ctx.parse_err(format!(
                "library `{}` has no section `{wanted}`",
                path.display()
            ))
        })
    }
}

/// A `.subckt` definition: its ports and its body, `.model` cards excluded.
struct SubcircuitDef {
    /// Name as written.
    name: String,
    /// Port names as written, in call order.
    ports: Vec<String>,
    /// Element lines between `.subckt` and `.ends`.
    body: Vec<LogicalLine>,
    /// Lower-cased names of the `.model` cards declared inside the body.
    local_models: Vec<String>,
    /// Lower-cased names of the elements declared in the body.
    local_elements: Vec<String>,
    /// `params:` defaults on the `.subckt` line, as written.
    params: Vec<(String, String)>,
    /// `.param` lines inside the body, as written, with their line.
    local_params: Vec<(String, String, usize)>,
}

/// Positional tokens of a `.subckt` or `X` line after `skip` tokens: up to
/// `params:` or the first `name=value` (including `name = value`).
fn positional_count(tokens: &[&str], skip: usize) -> usize {
    let mut count = 0;
    for (index, token) in tokens.iter().enumerate().skip(skip) {
        if token.contains('=')
            || token.eq_ignore_ascii_case("params:")
            || tokens
                .get(index + 1)
                .is_some_and(|next| next.starts_with('='))
        {
            break;
        }
        count += 1;
    }
    count
}

/// The `name=value` text after the positional tokens, `params:` dropped.
fn parameter_text(tokens: &[&str], from: usize) -> String {
    let rest = tokens.get(from..).unwrap_or(&[]);
    let rest = match rest.first() {
        Some(first) if first.eq_ignore_ascii_case("params:") => &rest[1..],
        _ => rest,
    };
    rest.join(" ")
}

/// Lower-cased directive name of a line, if it is a directive.
fn directive_of(text: &str) -> Option<String> {
    text.split_whitespace()
        .next()
        .and_then(|head| head.strip_prefix('.'))
        .map(str::to_ascii_lowercase)
}

/// Positional tokens of an `X` line: the element name excluded, everything
/// before `params:` or the first `key=value` token.
fn positional_call_tokens<'a>(tokens: &[&'a str]) -> Vec<&'a str> {
    tokens[1..1 + positional_count(tokens, 1)].to_vec()
}

/// Flatten every `.subckt` call into the primitives the engine solves.
///
/// The naming follows ngspice's flattener, so a probe or a routed input can
/// name the same thing in both engines: inside call `X1`, element `R1`
/// becomes `R.X1.R1` and internal node `mid` becomes `X1.mid`; a nested call
/// `X2` inside `X1` gives `R.X1.X2.R1` and `X1.X2.mid`. Ports bind to the
/// caller's nodes, and ground (`0`, `gnd`) stays global. A `.model` card
/// written inside a body is local to that subcircuit: it is renamed to
/// `<subckt>.<model>` and references in the body follow it. `F`/`H` control
/// names and `K` inductor names that name an element of the same body follow
/// the element's new name.
///
/// Parameters follow ngspice's numparam (see [`super::numparam`]). Top-level
/// `.param` lines form the global scope, resolved in dependency order. A call
/// evaluates its `name=value` overrides in the caller's scope; the
/// `.subckt` line's `params:` defaults and the body's `.param` lines are then
/// evaluated on top of the caller's scope, which is ngspice's dynamic
/// scoping: a nested subcircuit sees the parameters of the call that placed
/// it. Every `{expr}` is replaced by its number before the line is parsed,
/// so the solver never sees a parameter.
///
/// What is NOT flattened is refused by name rather than approximated: a
/// `.model` inside a body that uses that subcircuit's parameters (ngspice
/// makes one card per instance), nested `.subckt` definitions, and any
/// directive in a body other than `.model` and `.param`.
fn expand_subcircuits(lines: Vec<LogicalLine>) -> Result<Vec<LogicalLine>, AnalogError> {
    let err = |line: &LogicalLine, message: String| AnalogError::Parse {
        line: line.number,
        text: line.text.clone(),
        message,
    };
    let unsupported = |line: &LogicalLine| AnalogError::Unsupported {
        line: line.number,
        text: line.text.clone(),
    };

    // Pass 1: lift the definitions and the global `.param` lines out; keep
    // everything else in order.
    let mut defs: BTreeMap<String, SubcircuitDef> = BTreeMap::new();
    let mut top: Vec<LogicalLine> = Vec::new();
    let mut global_params: Vec<(String, String, usize)> = Vec::new();
    // Index in `top` of each hoisted local `.model`, with its subcircuit.
    let mut hoisted: Vec<(usize, String)> = Vec::new();
    let mut lines = lines.into_iter();
    let mut in_control = false;
    while let Some(line) = lines.next() {
        let directive = directive_of(&line.text);
        if in_control {
            in_control = directive.as_deref() != Some("endc");
            top.push(line);
            continue;
        }
        match directive.as_deref() {
            Some("control") => {
                in_control = true;
                top.push(line);
            }
            Some("ends") => {
                return Err(err(&line, "`.ends` without a `.subckt`".into()));
            }
            Some("param") => {
                let text = line
                    .text
                    .split_once(char::is_whitespace)
                    .map_or("", |p| p.1);
                let pairs = super::numparam::assignments(text).map_err(|m| err(&line, m))?;
                if pairs.is_empty() {
                    return Err(err(&line, "expected `.param <name>=<value> ...`".into()));
                }
                global_params.extend(pairs.into_iter().map(|(n, v)| (n, v, line.number)));
            }
            Some("subckt") => {
                let tokens: Vec<&str> = line.text.split_whitespace().collect();
                if tokens.len() < 2 {
                    return Err(err(&line, "expected `.subckt <name> <port> ...`".into()));
                }
                let port_count = positional_count(&tokens, 2);
                let params = super::numparam::assignments(&parameter_text(&tokens, 2 + port_count))
                    .map_err(|m| err(&line, m))?;
                for (index, (param, _)) in params.iter().enumerate() {
                    if params[..index].iter().any(|(other, _)| other == param) {
                        return Err(err(&line, format!("parameter `{param}` is listed twice")));
                    }
                }
                let name = tokens[1].to_string();
                let key = name.to_ascii_lowercase();
                if defs.contains_key(&key) {
                    return Err(err(&line, format!("`.subckt {name}` is declared twice")));
                }
                let ports: Vec<String> = tokens[2..2 + port_count]
                    .iter()
                    .map(|t| t.to_string())
                    .collect();
                for (index, port) in ports.iter().enumerate() {
                    if ports[..index].iter().any(|p| p.eq_ignore_ascii_case(port)) {
                        return Err(err(&line, format!("port `{port}` is listed twice")));
                    }
                    if is_ground(&port.to_ascii_lowercase()) {
                        return Err(err(&line, format!("port `{port}` is ground")));
                    }
                }
                let mut body = Vec::new();
                let mut local_models = Vec::new();
                let mut local_elements = Vec::new();
                let mut local_params = Vec::new();
                let mut closed = false;
                for inner in lines.by_ref() {
                    match directive_of(&inner.text).as_deref() {
                        Some("ends") => {
                            closed = true;
                            break;
                        }
                        Some("model") => {
                            let mut parts = inner.text.split_whitespace().skip(1);
                            let Some(model) = parts.next() else {
                                return Err(err(&inner, "expected `.model <name> <type>`".into()));
                            };
                            local_models.push(model.to_ascii_lowercase());
                            // Hoisted once per definition, under the local
                            // name, whether or not anything calls it.
                            let rest: Vec<&str> = parts.collect();
                            hoisted.push((top.len(), key.clone()));
                            top.push(LogicalLine {
                                number: inner.number,
                                text: format!(".model {name}.{model} {}", rest.join(" ")),
                            });
                        }
                        Some("param") => {
                            let text = inner
                                .text
                                .split_once(char::is_whitespace)
                                .map_or("", |p| p.1);
                            let pairs =
                                super::numparam::assignments(text).map_err(|m| err(&inner, m))?;
                            if pairs.is_empty() {
                                return Err(err(
                                    &inner,
                                    "expected `.param <name>=<value> ...`".into(),
                                ));
                            }
                            local_params
                                .extend(pairs.into_iter().map(|(n, v)| (n, v, inner.number)));
                        }
                        Some("end") => {
                            return Err(err(&inner, format!("`.end` inside `.subckt {name}`")));
                        }
                        Some(_) => return Err(unsupported(&inner)),
                        None => {
                            let head = inner.text.split_whitespace().next().unwrap_or("");
                            local_elements.push(head.to_ascii_lowercase());
                            body.push(inner);
                        }
                    }
                }
                if !closed {
                    return Err(err(&line, format!("`.subckt {name}` has no `.ends`")));
                }
                defs.insert(
                    key,
                    SubcircuitDef {
                        name,
                        ports,
                        body,
                        local_models,
                        local_elements,
                        params,
                        local_params,
                    },
                );
            }
            _ => top.push(line),
        }
    }

    let global = resolve_params(&super::numparam::Scope::new(), Vec::new(), &global_params)?;
    // A local card is hoisted once, so it can only use global parameters.
    for (index, subckt) in &hoisted {
        let line = &top[*index];
        let def = &defs[subckt];
        if let Err(super::numparam::EvalError::Unknown(name)) =
            super::numparam::substitute(&line.text, &global)
        {
            if def.params.iter().any(|(param, _)| *param == name)
                || def.local_params.iter().any(|(param, _, _)| *param == name)
            {
                return Err(err(
                    line,
                    format!(
                        "this `.model` in `.subckt {}` uses its parameter `{name}`; \
                         per-instance model cards are outside the flattener",
                        def.name
                    ),
                ));
            }
        }
    }

    // Every top-level card name, for the `Q` substrate-or-model question.
    let global_models: Vec<String> = top
        .iter()
        .filter(|line| directive_of(&line.text).as_deref() == Some("model"))
        .filter_map(|line| line.text.split_whitespace().nth(1))
        .map(str::to_ascii_lowercase)
        .collect();

    // Pass 2: replace every `{expr}` with its value and every top-level call
    // with its flattened body.
    let mut out = Vec::with_capacity(top.len());
    let mut in_control = false;
    for line in top {
        let directive = directive_of(&line.text);
        if in_control || directive.as_deref() == Some("control") {
            in_control = directive.as_deref() != Some("endc");
            out.push(line);
            continue;
        }
        let line = LogicalLine {
            text: super::numparam::substitute(&line.text, &global)
                .map_err(|e| err(&line, e.message()))?,
            number: line.number,
        };
        let tokens: Vec<&str> = line.text.split_whitespace().collect();
        let is_call = tokens[0].len() > 1
            && tokens[0].starts_with(['X', 'x'])
            && positional_call_tokens(&tokens)
                .last()
                .is_some_and(|name| defs.contains_key(&name.to_ascii_lowercase()));
        if !is_call {
            out.push(line);
            continue;
        }
        let actuals: Vec<String> = positional_call_tokens(&tokens)
            .iter()
            .map(|t| t.to_string())
            .collect();
        let mut stack = Vec::new();
        expand_call(
            &defs,
            &global_models,
            &global,
            &line,
            tokens[0],
            &actuals,
            &mut stack,
            &mut out,
        )?;
    }
    Ok(out)
}

/// Expand every `.func` call, then drop the `.func` lines.
///
/// This is ngspice's front end (`inpcom.c`): a call is replaced by the body
/// text with its arguments bound, before numparam evaluates anything, so a
/// body may use any `.param` in scope at the line that makes the call. A
/// `.func` at the top level is global wherever it is written; one inside a
/// `.subckt` body is local to that body and shadows a global one of the same
/// name. Calls are expanded inside `{...}`, and anywhere on a `.param`,
/// `.subckt` or `X` line, where a value may be written bare.
fn expand_functions(lines: Vec<LogicalLine>) -> Result<Vec<LogicalLine>, AnalogError> {
    use super::numparam::{expand_braced_calls, expand_calls, parse_function, Functions};
    let err = |line: &LogicalLine, message: String| AnalogError::Parse {
        line: line.number,
        text: line.text.clone(),
        message,
    };
    if !lines
        .iter()
        .any(|line| directive_of(&line.text).as_deref() == Some("func"))
    {
        return Ok(lines);
    }

    // Pass 1: the global table, and one local table per `.subckt` body,
    // keyed by the index of its `.subckt` line.
    let mut global = Functions::new();
    let mut local: BTreeMap<usize, Functions> = BTreeMap::new();
    let mut owner: Vec<Option<usize>> = Vec::with_capacity(lines.len());
    let mut current = None;
    let mut in_control = false;
    for (index, line) in lines.iter().enumerate() {
        let directive = directive_of(&line.text);
        if in_control {
            in_control = directive.as_deref() != Some("endc");
            owner.push(None);
            continue;
        }
        match directive.as_deref() {
            Some("control") => in_control = true,
            Some("subckt") => current = Some(index),
            Some("ends") => current = None,
            Some("func") => {
                let text = line
                    .text
                    .split_once(char::is_whitespace)
                    .map_or("", |p| p.1);
                let (name, function) = parse_function(text).map_err(|m| err(line, m))?;
                let table = match current {
                    Some(at) => local.entry(at).or_default(),
                    None => &mut global,
                };
                if table.insert(name.clone(), function).is_some() {
                    return Err(err(line, format!("`.func {name}` is defined twice")));
                }
            }
            _ => {}
        }
        owner.push(current);
    }

    // Pass 2: expand, in the scope of the line.
    let mut out = Vec::with_capacity(lines.len());
    let mut in_control = false;
    for (line, owner) in lines.into_iter().zip(owner) {
        let directive = directive_of(&line.text);
        if in_control || directive.as_deref() == Some("control") {
            in_control = directive.as_deref() != Some("endc");
            out.push(line);
            continue;
        }
        if directive.as_deref() == Some("func") {
            continue;
        }
        let scoped;
        let functions = match owner.and_then(|at| local.get(&at)) {
            Some(table) => {
                let mut merged = global.clone();
                merged.extend(table.iter().map(|(k, v)| (k.clone(), v.clone())));
                scoped = merged;
                &scoped
            }
            None => &global,
        };
        let bare = matches!(directive.as_deref(), Some("param" | "subckt"))
            || (directive.is_none() && line.text.starts_with(['X', 'x']));
        let text = if bare {
            expand_calls(&line.text, functions)
        } else {
            expand_braced_calls(&line.text, functions)
        }
        .map_err(|e| err(&line, e.message()))?;
        out.push(LogicalLine {
            number: line.number,
            text,
        });
    }
    Ok(out)
}

/// Resolve `fixed` and the written `params` on top of `base`, pointing an
/// error at the line of the parameter it names.
fn resolve_params(
    base: &super::numparam::Scope,
    fixed: Vec<(String, f64)>,
    params: &[(String, String, usize)],
) -> Result<super::numparam::Scope, AnalogError> {
    use super::numparam::Pending;
    let mut pending: Vec<(String, Pending)> = fixed
        .into_iter()
        .map(|(name, value)| (name, Pending::Fixed(value)))
        .collect();
    pending.extend(
        params
            .iter()
            .map(|(name, value, _)| (name.clone(), Pending::Expression(value.clone()))),
    );
    super::numparam::resolve(base, pending).map_err(|message| {
        let line = params
            .iter()
            .find(|(name, _, _)| message.contains(&format!("`{name}`")))
            .or(params.first())
            .map_or(0, |(_, _, line)| *line);
        AnalogError::Parse {
            line,
            text: String::new(),
            message,
        }
    })
}

/// The element letter of a name, as written.
fn first_char(name: &str) -> &str {
    name.char_indices()
        .nth(1)
        .map_or(name, |(at, _)| &name[..at])
}

/// Emit the flattened body of one call. `path` is the instance path (`X1`,
/// `X1.X2`); `actuals` are the call's node tokens followed by the subcircuit
/// name.
#[allow(clippy::too_many_arguments)]
fn expand_call(
    defs: &BTreeMap<String, SubcircuitDef>,
    global_models: &[String],
    caller_scope: &super::numparam::Scope,
    call: &LogicalLine,
    path: &str,
    actuals: &[String],
    stack: &mut Vec<String>,
    out: &mut Vec<LogicalLine>,
) -> Result<(), AnalogError> {
    let err = |message: String| AnalogError::Parse {
        line: call.number,
        text: call.text.clone(),
        message,
    };
    let (subckt, nodes) = actuals.split_last().expect("a call names its subcircuit");
    let key = subckt.to_ascii_lowercase();
    let def = &defs[&key];
    if stack.contains(&key) {
        return Err(err(format!("subcircuit `{}` calls itself", def.name)));
    }

    // The call's overrides, in the caller's scope; then the defaults it does
    // not override and the body's `.param` lines, on top of the caller's
    // scope.
    let call_tokens: Vec<&str> = call.text.split_whitespace().collect();
    let overrides = super::numparam::assignments(&parameter_text(
        &call_tokens,
        1 + positional_count(&call_tokens, 1),
    ))
    .map_err(err)?;
    let mut fixed = Vec::new();
    for (name, value) in &overrides {
        if !def.params.iter().any(|(param, _)| param == name) {
            return Err(err(format!(
                "`.subckt {}` has no parameter `{name}`",
                def.name
            )));
        }
        let value = super::numparam::evaluate_value(value, caller_scope)
            .map_err(|e| err(format!("`{name}`: {}", e.message())))?;
        fixed.push((name.clone(), value));
    }
    let defaults: Vec<(String, String, usize)> = def
        .params
        .iter()
        .filter(|(name, _)| !fixed.iter().any(|(given, _)| given == name))
        .map(|(name, value)| (name.clone(), value.clone(), call.number))
        .collect();
    let params = resolve_params(caller_scope, fixed, &defaults)?;
    let scope = resolve_params(&params, Vec::new(), &def.local_params)?;
    if nodes.len() != def.ports.len() {
        return Err(err(format!(
            "`{}` connects {} nodes but `.subckt {}` has {} ports",
            call.text.split_whitespace().next().unwrap_or(""),
            nodes.len(),
            def.name,
            def.ports.len()
        )));
    }
    stack.push(key);

    let node = |token: &str| -> String {
        let lower = token.to_ascii_lowercase();
        if is_ground(&lower) {
            return token.to_string();
        }
        match def.ports.iter().position(|p| p.eq_ignore_ascii_case(token)) {
            Some(index) => nodes[index].clone(),
            None => format!("{path}.{token}"),
        }
    };
    let element = |token: &str| -> String {
        if def.local_elements.contains(&token.to_ascii_lowercase()) {
            format!("{}.{path}.{token}", first_char(token))
        } else {
            token.to_string()
        }
    };
    let model = |token: &str| -> String {
        if def.local_models.contains(&token.to_ascii_lowercase()) {
            format!("{}.{token}", def.name)
        } else {
            token.to_string()
        }
    };
    let names_model = |token: &str| {
        let lower = token.to_ascii_lowercase();
        def.local_models.contains(&lower)
            || global_models.contains(&lower)
            || builtin_model(token).is_some()
    };

    for line in &def.body {
        let text =
            super::numparam::substitute(&line.text, &scope).map_err(|e| AnalogError::Parse {
                line: line.number,
                text: line.text.clone(),
                message: e.message(),
            })?;
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let head = tokens[0];
        let letter = head
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or(' ');

        if matches!(letter, 'E' | 'G') && tokens.len() > 3 {
            if let Some((input, transfer)) = laplace_form(&tokens[3..]) {
                // The LTspice form's control nodes become the PSpice form's
                // `V(nc+, nc-)`, so one probe rename covers both.
                out.push(LogicalLine {
                    number: line.number,
                    text: format!(
                        "{}.{path}.{head} {} {} LAPLACE {{{}}} = {{{transfer}}}",
                        first_char(head),
                        node(tokens[1]),
                        node(tokens[2]),
                        rename_probes(&input, &node, &element)
                    ),
                });
                continue;
            }
        }

        let behavioural = letter == 'B'
            || (matches!(letter, 'E' | 'G')
                && tokens.get(3).is_some_and(|token| {
                    let key = token.split('=').next().unwrap_or("");
                    key.eq_ignore_ascii_case("value") || is_table_keyword(token)
                }));
        if behavioural && tokens.len() > 3 {
            out.push(LogicalLine {
                number: line.number,
                text: format!(
                    "{}.{path}.{head} {} {} {}",
                    first_char(head),
                    node(tokens[1]),
                    node(tokens[2]),
                    rename_probes(&tokens[3..].join(" "), &node, &element)
                ),
            });
            continue;
        }

        if matches!(letter, 'E' | 'G' | 'F' | 'H')
            && tokens.get(3).is_some_and(|token| is_poly_keyword(token))
        {
            let poly =
                PolySource::parse(letter, &tokens[3..]).map_err(|message| AnalogError::Parse {
                    line: line.number,
                    text: line.text.clone(),
                    message,
                })?;
            out.push(LogicalLine {
                number: line.number,
                text: format!(
                    "{}.{path}.{head} {} {} {}",
                    first_char(head),
                    node(tokens[1]),
                    node(tokens[2]),
                    poly.rename(&node, &element)
                ),
            });
            continue;
        }

        if letter == 'X' {
            let positional = positional_call_tokens(&tokens);
            if let Some(callee) = positional
                .last()
                .filter(|name| defs.contains_key(&name.to_ascii_lowercase()))
            {
                let mut inner: Vec<String> = positional[..positional.len() - 1]
                    .iter()
                    .map(|t| node(t))
                    .collect();
                inner.push(callee.to_string());
                let inner_call = LogicalLine {
                    number: line.number,
                    text: text.clone(),
                };
                expand_call(
                    defs,
                    global_models,
                    &scope,
                    &inner_call,
                    &format!("{path}.{head}"),
                    &inner,
                    stack,
                    out,
                )?;
                continue;
            }
        }

        // Which tokens after the name are nodes, element references and the
        // model, per letter; everything else is copied as written.
        let mut renamed: Vec<String> = Vec::with_capacity(tokens.len());
        renamed.push(format!("{}.{path}.{head}", first_char(head)));
        let rest = &tokens[1..];
        let voltage_switch = rest.iter().any(|t| {
            t.split_once('=')
                .is_some_and(|(key, _)| key.eq_ignore_ascii_case("vt"))
        });
        let (node_count, element_refs, model_at): (usize, &[usize], Option<usize>) = match letter {
            'R' | 'C' | 'L' | 'V' | 'I' => (2, &[], None),
            'S' if voltage_switch => (4, &[], None),
            'S' if names_switch_model(rest) => (4, &[], Some(4)),
            'S' => (2, &[], None),
            'W' => (2, &[2], Some(3)),
            'E' | 'G' | 'T' => (4, &[], None),
            'F' | 'H' => (2, &[2], None),
            'K' => (0, &[0, 1], None),
            'D' => (2, &[], Some(2)),
            'J' => (3, &[], Some(3)),
            'M' => (4, &[], Some(4)),
            'Q' => {
                let count = if tokens.len() > 5 && !names_model(tokens[4]) {
                    4
                } else {
                    3
                };
                (count, &[], Some(count))
            }
            'X' => {
                let positional = positional_call_tokens(&tokens).len();
                let count = positional.saturating_sub(1);
                (count, &[], positional.checked_sub(1))
            }
            _ => (0, &[], None),
        };
        for (index, token) in rest.iter().enumerate() {
            renamed.push(if index < node_count {
                node(token)
            } else if element_refs.contains(&index) {
                element(token)
            } else if model_at == Some(index) {
                model(token)
            } else {
                token.to_string()
            });
        }
        out.push(LogicalLine {
            number: line.number,
            text: renamed.join(" "),
        });
    }
    stack.pop();
    Ok(())
}

/// Rename the nodes inside every `V(...)` and the element inside every
/// `I(...)` of a behavioural expression placed by a subcircuit call.
fn rename_probes(
    text: &str,
    node: &dyn Fn(&str) -> String,
    element: &dyn Fn(&str) -> String,
) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    // Text before `index` not yet copied starts here; copying whole slices
    // keeps any non-ASCII character intact.
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index].to_ascii_lowercase();
        let standalone = index == 0
            || !(bytes[index - 1].is_ascii_alphanumeric()
                || bytes[index - 1] == b'_'
                || bytes[index - 1] == b'.');
        let after_name = bytes
            .get(index + 1)
            .is_none_or(|next| !(next.is_ascii_alphanumeric() || *next == b'_'));
        if standalone && after_name && (byte == b'v' || byte == b'i') {
            let mut open = index + 1;
            while open < bytes.len() && (bytes[open] == b' ' || bytes[open] == b'\t') {
                open += 1;
            }
            if bytes.get(open) == Some(&b'(') {
                if let Some(close) = text[open..].find(')').map(|at| open + at) {
                    let rename: &dyn Fn(&str) -> String = if byte == b'v' { node } else { element };
                    let names: Vec<String> = text[open + 1..close]
                        .split(',')
                        .map(|name| rename(name.trim()))
                        .collect();
                    out.push_str(&text[copied..=index]);
                    out.push('(');
                    out.push_str(&names.join(", "));
                    out.push(')');
                    index = close + 1;
                    copied = index;
                    continue;
                }
            }
        }
        index += 1;
    }
    out.push_str(&text[copied..]);
    out
}

/// The deck's circuit temperatures, in Celsius, read before any element so
/// that a card written after the devices still applies to them.
#[derive(Clone, Copy, Default)]
struct CircuitTemperatures {
    /// ngspice's `CKTtemp`: `.temp` or `.options temp=`. Defaults to 27 °C
    /// when only `tnom` is given, which is ngspice's own default.
    temperature: Option<f64>,
    /// ngspice's `CKTnomTemp`: `.options tnom=`, the `TNOM` of every model
    /// card and `tc1`/`tc2` resistor that does not set its own.
    nominal: Option<f64>,
}

impl CircuitTemperatures {
    /// The `TNOM` a model card or `tc1`/`tc2` resistor starts from.
    fn nominal_celsius(&self) -> f64 {
        self.nominal.unwrap_or(27.0)
    }
}

/// `key=value` pairs of an `.options` card, with SPICE's optional spaces
/// around `=` (`tnom = 50`) folded away. Flags without a value are skipped.
fn option_pairs(tokens: &[&str]) -> Vec<(String, String)> {
    let folded = tokens.join(" ").replace(" =", "=").replace("= ", "=");
    folded
        .split_whitespace()
        .filter_map(|item| item.split_once('='))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .map(|(key, value)| (key.to_ascii_lowercase(), value.to_string()))
        .collect()
}

/// The deck's `.temp`, `.options temp=` and `.options tnom=`. ngspice runs the
/// whole analysis once per `.temp` value; this engine runs once, so more than
/// one value, or a temperature given twice, is refused rather than silently
/// keeping one of them.
fn circuit_temperatures(lines: &[LogicalLine]) -> Result<CircuitTemperatures, AnalogError> {
    let mut found = CircuitTemperatures::default();
    let mut in_control = false;
    for logical in lines {
        let ctx = LineCtx {
            number: logical.number,
            text: &logical.text,
        };
        let mut tokens = logical.text.split_whitespace();
        let Some(head) = tokens.next() else { continue };
        if in_control {
            in_control = !head.eq_ignore_ascii_case(".endc");
            continue;
        }
        if head.eq_ignore_ascii_case(".control") {
            in_control = true;
            continue;
        }
        let values: Vec<&str> = tokens.collect();
        let above_zero = |celsius: f64, what: &str| {
            if celsius.is_finite() && celsius > -273.15 {
                Ok(celsius)
            } else {
                Err(ctx.parse_err(format!(
                    "{what} must be finite and above absolute zero (-273.15 °C)"
                )))
            }
        };
        if head.eq_ignore_ascii_case(".temp") {
            if values.is_empty() {
                return Err(ctx.parse_err("expected `.temp <°C>`"));
            }
            if values.len() > 1 || found.temperature.is_some() {
                return Err(ctx.parse_err(
                    "only one `.temp` value is supported; ngspice would rerun the analysis per value",
                ));
            }
            let celsius = ctx.value(values[0], "temperature")?;
            found.temperature = Some(above_zero(celsius, "`.temp`")?);
        } else if head.eq_ignore_ascii_case(".options") || head.eq_ignore_ascii_case(".option") {
            for (key, raw) in option_pairs(&values) {
                let slot = match key.as_str() {
                    "temp" => &mut found.temperature,
                    "tnom" => &mut found.nominal,
                    _ => continue,
                };
                if slot.is_some() {
                    return Err(ctx.parse_err(format!(
                        "the circuit `{key}` is given twice; only one value is supported"
                    )));
                }
                let celsius = ctx.value(&raw, &key)?;
                *slot = Some(above_zero(celsius, &format!("`.options {key}`"))?);
            }
        }
    }
    if found.nominal.is_some() && found.temperature.is_none() {
        // ngspice's `CKTtemp` default. Without it the instances would keep
        // the fixed 300 K and nominal parameters, and the `tnom` would say
        // nothing.
        found.temperature = Some(27.0);
    }
    Ok(found)
}

/// Parse a netlist into a [`Circuit`].
///
/// `.include` and `.lib` are refused here, because there is no file to read
/// them from; [`parse_netlist_with_files`] reads them.
pub fn parse_netlist(text: &str) -> Result<Circuit, AnalogError> {
    parse_lines(logical_lines(text)?)
}

/// Parse a netlist whose `.include <file>` and `.lib <file> <section>` lines
/// are read through `read`.
///
/// A relative path resolves against the directory of the file that names it,
/// and against `base_dir` for the deck itself, as ngspice does. `.include`
/// splices the whole file; `.lib` splices the body of its `.lib <section>`
/// … `.endl` block (the name is not case-sensitive), and a missing section is
/// an error. Calls nest, a `.end` inside an included file is dropped as
/// ngspice drops it, and a file that includes itself is refused. Every line
/// spliced in carries the line number of the deck's own call; an error inside
/// a library names the file and its line in the message.
///
/// `read` gets the resolved path. The co-simulation adapter passes
/// `std::fs::read_to_string`; a caller with no file system can serve the
/// files from memory.
pub fn parse_netlist_with_files(
    text: &str,
    base_dir: &Path,
    read: &mut dyn FnMut(&Path) -> Result<String, String>,
) -> Result<Circuit, AnalogError> {
    let mut expander = IncludeExpander {
        read,
        stack: Vec::new(),
    };
    let lines = expander.expand(logical_lines(text)?, base_dir, None)?;
    parse_lines(lines)
}

fn parse_lines(lines: Vec<LogicalLine>) -> Result<Circuit, AnalogError> {
    let mut circuit = Circuit::default();
    let mut ended = false;
    let mut models: BTreeMap<String, ModelCard> = BTreeMap::new();
    let mut pending: Vec<PendingDevice> = Vec::new();
    let mut pending_mutuals: Vec<PendingMutualInductor> = Vec::new();
    let mut pending_switches: Vec<PendingSwitch> = Vec::new();
    let mut in_control = false;
    // Line of every `F`/`H`, in the order they were pushed, for the error when
    // the element they name carries no branch current.
    let mut controls: Vec<(char, usize, String)> = Vec::new();
    // Line of every behavioural source, in the order pushed per kind, for the
    // error when its expression names a node or element that does not exist.
    let mut behavioural_lines: Vec<(BehaviouralKind, usize, String)> = Vec::new();
    // Every `.model` name in the deck, wherever it is written. A `Q` line's
    // fourth token is the substrate node unless it names a model, which is
    // how ngspice's `inp2q.c` tells `Q c b e s mod` from `Q c b e mod`.
    let lines = expand_functions(lines)?;
    let lines = expand_subcircuits(lines)?;
    let temperatures = circuit_temperatures(&lines)?;
    let circuit_temperature = temperatures.temperature;
    let declared_models: Vec<String> = lines
        .iter()
        .filter_map(|line| {
            let mut tokens = line.text.split_whitespace();
            let head = tokens.next()?;
            head.eq_ignore_ascii_case(".model")
                .then(|| tokens.next())
                .flatten()
                .map(str::to_ascii_lowercase)
        })
        .collect();
    let names_model = |token: &str| {
        declared_models.contains(&token.to_ascii_lowercase()) || builtin_model(token).is_some()
    };

    for logical in &lines {
        let number = logical.number;
        let line = logical.text.as_str();
        let ctx = LineCtx { number, text: line };
        // A `.control` block is ngspice's scripting language, not a circuit.
        // Skipping it wholesale is what lets one deck drive both engines: the
        // in-core adapter takes its run length from the manifest, ngspice
        // takes it from the block.
        if in_control {
            if line.eq_ignore_ascii_case(".endc") {
                in_control = false;
            }
            continue;
        }
        if ended {
            return Err(ctx.parse_err("netlist continues after `.end`"));
        }

        let tokens: Vec<&str> = line.split_whitespace().collect();
        let head = tokens[0];

        if let Some(directive) = head.strip_prefix('.') {
            match directive.to_ascii_lowercase().as_str() {
                "end" => ended = true,
                "ic" => parse_ic(&mut circuit, &ctx, &tokens[1..])?,
                // The run length and step still come from the manifest; only
                // the `uic` flag changes how the run starts.
                "tran" => {
                    circuit.uic |= tokens[1..].iter().any(|t| t.eq_ignore_ascii_case("uic"));
                }
                "model" => parse_model(
                    &mut models,
                    &ctx,
                    &tokens[1..],
                    temperatures.nominal_celsius(),
                )?,
                "control" => in_control = true,
                // Analysis and housekeeping cards. This engine's run length,
                // step and outputs come from the manifest, so these say
                // nothing it can act on — but rejecting them would mean a deck
                // cannot be shared with ngspice, which is the whole point of
                // spelling the subset in SPICE.
                "options" | "option" | "op" | "print" | "plot" | "save" | "probe" | "width"
                | "nodeset" | "title" | "endc" => {}
                // Read up front by `circuit_temperatures`, like the
                // `temp`/`tnom` of `.options`.
                "temp" => {}
                _ => return Err(ctx.unsupported()),
            }
            continue;
        }

        let letter = head
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or(' ');
        if !matches!(
            letter,
            'R' | 'C'
                | 'L'
                | 'K'
                | 'V'
                | 'I'
                | 'S'
                | 'W'
                | 'D'
                | 'Q'
                | 'M'
                | 'J'
                | 'E'
                | 'G'
                | 'F'
                | 'H'
                | 'X'
                | 'B'
                | 'T'
        ) {
            return Err(ctx.unsupported());
        }
        if head.len() < 2 {
            return Err(ctx.parse_err(format!("element `{head}` has no name after `{letter}`")));
        }
        if circuit.has_element(head, &pending_mutuals) {
            return Err(ctx.parse_err(format!("element `{head}` is declared twice")));
        }

        match letter {
            'R' => {
                let rest = &tokens[1..];
                if rest.len() < 3 {
                    return Err(ctx.parse_err("expected `R<name> n1 n2 <value>`"));
                }
                let ohms = ctx.value(rest[2], "resistance")?;
                if ohms == 0.0 {
                    return Err(ctx.parse_err("resistance must be non-zero"));
                }
                let mut beta = None;
                let mut nominal_temperature_celsius = None;
                let mut temperature_celsius = None;
                let mut tc1 = None;
                let mut tc2 = None;
                let mut has_temperature_parameter = false;
                for token in &rest[3..] {
                    let (key, value) = token.split_once('=').ok_or_else(|| {
                        ctx.parse_err(
                            "resistor options must be `beta=`, `tc1=`, `tc2=`, `tnom=` or `temp=`",
                        )
                    })?;
                    let parsed = ctx.value(value, key)?;
                    match key.to_ascii_lowercase().as_str() {
                        "beta" => beta = Some(parsed),
                        "tc1" => tc1 = Some(parsed),
                        "tc2" => tc2 = Some(parsed),
                        "tnom" => {
                            nominal_temperature_celsius = Some(parsed);
                            has_temperature_parameter = true;
                        }
                        "temp" => {
                            temperature_celsius = Some(parsed);
                            has_temperature_parameter = true;
                        }
                        other => {
                            return Err(ctx.parse_err(format!(
                                "unknown resistor parameter `{other}`; expected `beta`, `tc1`, `tc2`, `tnom` or `temp`"
                            )))
                        }
                    }
                }
                let has_temperature_coefficients = tc1.is_some() || tc2.is_some();
                if beta.is_some() && has_temperature_coefficients {
                    return Err(ctx.parse_err("an NTC `beta` model cannot also use `tc1` or `tc2`"));
                }
                if beta.is_none() && !has_temperature_coefficients && has_temperature_parameter {
                    return Err(ctx.parse_err(
                        "`tnom` and `temp` require an NTC `beta` or resistor `tc1`/`tc2`",
                    ));
                }
                let ntc = if let Some(beta) = beta {
                    if beta <= 0.0 || ohms <= 0.0 {
                        return Err(ctx.parse_err("an NTC resistor needs `beta > 0` and `r25 > 0`"));
                    }
                    let nominal_temperature_celsius = nominal_temperature_celsius.unwrap_or(25.0);
                    let temperature_celsius =
                        temperature_celsius.or(circuit_temperature).unwrap_or(25.0);
                    if nominal_temperature_celsius <= -273.15 || temperature_celsius <= -273.15 {
                        return Err(ctx.parse_err("NTC temperatures must be above absolute zero"));
                    }
                    let model = NtcModel {
                        beta,
                        nominal_temperature_celsius,
                        temperature_celsius,
                    };
                    if !model.resistance_ohms(ohms).is_finite() {
                        return Err(ctx.parse_err("NTC parameters produce a non-finite resistance"));
                    }
                    Some(model)
                } else {
                    None
                };
                let temperature_coefficients = if has_temperature_coefficients {
                    let tc1 = tc1.unwrap_or(0.0);
                    let tc2 = tc2.unwrap_or(0.0);
                    let nominal_temperature_celsius = nominal_temperature_celsius
                        .unwrap_or_else(|| temperatures.nominal_celsius());
                    let temperature_celsius =
                        temperature_celsius.or(circuit_temperature).unwrap_or(27.0);
                    if !tc1.is_finite() || !tc2.is_finite() {
                        return Err(ctx.parse_err("resistor `tc1` and `tc2` must be finite"));
                    }
                    if nominal_temperature_celsius <= -273.15 || temperature_celsius <= -273.15 {
                        return Err(
                            ctx.parse_err("resistor temperatures must be above absolute zero")
                        );
                    }
                    let model = ResistorTemperatureModel {
                        tc1,
                        tc2,
                        nominal_temperature_celsius,
                        temperature_celsius,
                    };
                    let resistance = model.resistance_ohms(ohms);
                    if !resistance.is_finite() || resistance <= 0.0 {
                        return Err(ctx.parse_err(
                            "resistor temperature coefficients must produce a finite positive resistance",
                        ));
                    }
                    Some(model)
                } else {
                    None
                };
                let a = circuit.intern(rest[0]);
                let b = circuit.intern(rest[1]);
                circuit.resistors.push(Resistor {
                    name: head.to_string(),
                    a,
                    b,
                    ohms,
                    ntc,
                    temperature_coefficients,
                });
            }
            'C' => {
                let (n1, n2, value, ic) =
                    two_nodes_value_ic(&ctx, &tokens[1..], "C<name> n1 n2 <value> [ic=<v>]")?;
                let farads = ctx.value(value, "capacitance")?;
                if farads <= 0.0 {
                    return Err(ctx.parse_err("capacitance must be positive"));
                }
                let ic = ic.map(|raw| ctx.value(raw, "ic")).transpose()?;
                let a = circuit.intern(n1);
                let b = circuit.intern(n2);
                circuit.capacitors.push(Capacitor {
                    name: head.to_string(),
                    a,
                    b,
                    farads,
                    ic,
                });
            }
            'L' => {
                let (n1, n2, value, ic) =
                    two_nodes_value_ic(&ctx, &tokens[1..], "L<name> n1 n2 <value> [ic=<i>]")?;
                let henries = ctx.value(value, "inductance")?;
                if henries <= 0.0 {
                    return Err(ctx.parse_err("inductance must be positive"));
                }
                let ic = ic.map(|raw| ctx.value(raw, "ic")).transpose()?;
                let a = circuit.intern(n1);
                let b = circuit.intern(n2);
                circuit.inductors.push(Inductor {
                    name: head.to_string(),
                    a,
                    b,
                    henries,
                    ic,
                });
            }
            'K' => {
                let rest = &tokens[1..];
                if rest.len() != 3 {
                    return Err(ctx.parse_err("expected `K<name> L<name1> L<name2> <k>`"));
                }
                if ![rest[0], rest[1]].iter().all(|name| {
                    name.chars()
                        .next()
                        .is_some_and(|letter| letter.eq_ignore_ascii_case(&'L'))
                }) {
                    return Err(ctx.parse_err("a K element must name two inductors"));
                }
                let coupling = ctx.value(rest[2], "coupling coefficient")?;
                if !coupling.is_finite() || coupling.abs() > 1.0 {
                    return Err(ctx.parse_err("coupling coefficient must be finite and in [-1, 1]"));
                }
                pending_mutuals.push(PendingMutualInductor {
                    line: number,
                    text: line.to_string(),
                    name: head.to_string(),
                    first: rest[0].to_string(),
                    second: rest[1].to_string(),
                    coupling,
                });
            }
            'V' | 'I' => {
                let shape = if letter == 'V' {
                    "V<name> n+ n- <source>"
                } else {
                    "I<name> n+ n- <source>"
                };
                let rest = &tokens[1..];
                if rest.len() < 3 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let (np, nn) = (rest[0], rest[1]);
                let wave = parse_source(&ctx, &rest[2..], shape)?;
                let dc = wave.at(0.0);
                let p = circuit.intern(np);
                let n = circuit.intern(nn);
                if letter == 'V' {
                    circuit.voltage_sources.push(VoltageSource {
                        name: head.to_string(),
                        p,
                        n,
                        dc,
                        wave,
                    });
                } else {
                    circuit.current_sources.push(CurrentSource {
                        name: head.to_string(),
                        p,
                        n,
                        dc,
                        wave,
                    });
                }
            }
            'D' | 'Q' | 'M' | 'J' => {
                let (terminals, shape) = match letter {
                    'D' => (2, "D<name> n+ n- <model> [<area>] [area=<factor>] [pj=<perimeter>] [m=<count>] [temp=<°C>] [off] [ic=<vd>]"),
                    'Q' => (
                        if tokens.len() > 5 && !names_model(tokens[4]) { 4 } else { 3 },
                        "Q<name> nc nb ne [ns] <model> [<area>] [area=<factor>] [m=<count>] [temp=<°C>] [off] [ic=<vbe>,<vce>]",
                    ),
                    'M' => (
                        4,
                        "M<name> nd ng ns nb <model> [w=<m>] [l=<m>] [ad=<m²>] [as=<m²>] [pd=<m>] [ps=<m>] [nrd=<squares>] [nrs=<squares>] [m=<count>] [temp=<°C>] [off]",
                    ),
                    _ => (3, "J<name> nd ng ns <model> [<area>] [area=<factor>] [m=<count>] [temp=<°C>] [off] [ic=<vds>,<vgs>]"),
                };
                let rest = &tokens[1..];
                if rest.len() < terminals + 1 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let mut width = None;
                let mut length = None;
                let mut nrd = None;
                let mut nrs = None;
                let mut ad = None;
                let mut as_ = None;
                let mut pd = None;
                let mut ps = None;
                let mut temperature_celsius = None;
                let mut multiplier = None;
                let mut area = None;
                let mut perimeter = None;
                let mut ic = Vec::new();
                // SPICE's bare `off` flag may stand anywhere among the
                // options; it is the only option without a value.
                let mut off = false;
                let mut options = Vec::with_capacity(rest.len() - terminals - 1);
                for token in &rest[terminals + 1..] {
                    if token.eq_ignore_ascii_case("off") {
                        off = true;
                    } else {
                        options.push(*token);
                    }
                }
                let mut options = options.as_slice();
                // SPICE writes a D, Q or J area as a bare number straight
                // after the model name; `area=` is the keyword spelling.
                if letter != 'M' {
                    if let Some((first, tail)) = options.split_first() {
                        if !first.contains('=') {
                            area = Some(ctx.value(first, "area")?);
                            options = tail;
                        }
                    }
                }
                for token in options {
                    let (key, value) = token
                        .split_once('=')
                        .ok_or_else(|| ctx.parse_err(format!("expected `{shape}`")))?;
                    match key.to_ascii_lowercase().as_str() {
                        "w" if letter == 'M' => width = Some(ctx.value(value, "w")?),
                        "l" if letter == 'M' => length = Some(ctx.value(value, "l")?),
                        "nrd" if letter == 'M' => nrd = Some(ctx.value(value, "nrd")?),
                        "nrs" if letter == 'M' => nrs = Some(ctx.value(value, "nrs")?),
                        "ad" if letter == 'M' => ad = Some(ctx.value(value, "ad")?),
                        "as" if letter == 'M' => as_ = Some(ctx.value(value, "as")?),
                        "pd" if letter == 'M' => pd = Some(ctx.value(value, "pd")?),
                        "ps" if letter == 'M' => ps = Some(ctx.value(value, "ps")?),
                        "temp" => temperature_celsius = Some(ctx.value(value, "temp")?),
                        "m" => multiplier = Some(ctx.value(value, "m")?),
                        "area" if letter != 'M' && area.is_none() => {
                            area = Some(ctx.value(value, "area")?)
                        }
                        "pj" if letter == 'D' => perimeter = Some(ctx.value(value, "pj")?),
                        "ic" => {
                            ic = value
                                .split(',')
                                .map(|raw| ctx.value(raw, "ic"))
                                .collect::<Result<Vec<_>, _>>()?;
                            // ngspice's MOS1 takes one to three values,
                            // filled from vds; the others take two or one.
                            let accepted = match letter {
                                'D' => 1..=1,
                                'M' => 1..=3,
                                _ => 2..=2,
                            };
                            if !accepted.contains(&ic.len()) {
                                return Err(ctx.parse_err(match letter {
                                    'D' => "diode ic= takes one junction voltage: `ic=<vd>`",
                                    'Q' => "BJT ic= takes two voltages: `ic=<vbe>,<vce>`",
                                    'M' => {
                                        "MOSFET ic= takes one to three voltages: \
                                         `ic=<vds>[,<vgs>[,<vbs>]]`"
                                    }
                                    _ => "JFET ic= takes two voltages: `ic=<vds>,<vgs>`",
                                }));
                            }
                        }
                        other => {
                            return Err(ctx.parse_err(format!(
                                "unknown parameter `{other}`; expected `{shape}`"
                            )))
                        }
                    }
                }
                if matches!(width, Some(w) if w <= 0.0) || matches!(length, Some(l) if l <= 0.0) {
                    return Err(ctx.parse_err("w and l must be positive"));
                }
                if matches!(nrd, Some(squares) if !squares.is_finite() || squares < 0.0)
                    || matches!(nrs, Some(squares) if !squares.is_finite() || squares < 0.0)
                {
                    return Err(ctx.parse_err("nrd and nrs must be >= 0 and finite"));
                }
                if [ad, as_, pd, ps]
                    .into_iter()
                    .flatten()
                    .any(|geometry| !geometry.is_finite() || geometry < 0.0)
                {
                    return Err(ctx.parse_err("ad, as, pd and ps must be >= 0 and finite"));
                }
                if matches!(temperature_celsius, Some(temp) if !temp.is_finite() || temp <= -273.15)
                {
                    return Err(ctx.parse_err(
                        "device TEMP must be finite and above absolute zero (-273.15 °C)",
                    ));
                }
                if matches!(multiplier, Some(m) if !m.is_finite() || m <= 0.0) {
                    return Err(ctx.parse_err("device multiplier m must be positive and finite"));
                }
                if matches!(area, Some(a) if !a.is_finite() || a <= 0.0) {
                    return Err(ctx.parse_err("device area must be positive and finite"));
                }
                if matches!(perimeter, Some(p) if !p.is_finite() || p < 0.0) {
                    return Err(ctx.parse_err("diode perimeter pj must be >= 0 and finite"));
                }
                // The terminals are interned now so that node numbering still
                // follows netlist order; the model is bound after the last
                // line, because `.model` may come after its elements.
                let nodes = rest[..terminals]
                    .iter()
                    .map(|name| circuit.intern(name))
                    .collect();
                pending.push(PendingDevice {
                    line: ctx.number,
                    text: line.to_string(),
                    letter,
                    name: head.to_string(),
                    nodes,
                    model_name: rest[terminals].to_string(),
                    width,
                    length,
                    nrd,
                    nrs,
                    ad,
                    as_,
                    pd,
                    ps,
                    // An instance `temp=` wins over the deck's `.temp`, as
                    // in ngspice.
                    temperature_celsius: temperature_celsius.or(circuit_temperature),
                    multiplier: multiplier.unwrap_or(1.0),
                    area: area.unwrap_or(1.0),
                    perimeter,
                    off,
                    ic,
                });
            }
            'B' => {
                let rest = &tokens[1..];
                if rest.len() < 3 {
                    return Err(ctx.parse_err("expected `B<name> n+ n- V=<expr>` or `I=<expr>`"));
                }
                let text = rest[2..].join(" ");
                let (kind, body) = match text.split_once('=') {
                    Some((key, body)) if key.trim().eq_ignore_ascii_case("v") => {
                        (BehaviouralKind::Voltage, body)
                    }
                    Some((key, body)) if key.trim().eq_ignore_ascii_case("i") => {
                        (BehaviouralKind::Current, body)
                    }
                    _ => {
                        return Err(
                            ctx.parse_err("expected `V=<expr>` or `I=<expr>` after the two nodes")
                        )
                    }
                };
                push_behavioural(
                    &mut circuit,
                    &mut behavioural_lines,
                    &ctx,
                    head,
                    [rest[0], rest[1]],
                    kind,
                    body,
                )?;
            }
            'E' | 'G' | 'F' | 'H' if tokens.len() > 3 && is_poly_keyword(tokens[3]) => {
                let poly = PolySource::parse(letter, &tokens[3..])
                    .map_err(|message| ctx.parse_err(message))?;
                let atoms: Vec<String> = poly
                    .controls
                    .iter()
                    .map(|control| match control {
                        PolyControl::Voltage(p, n) => format!("V({p}, {n})"),
                        PolyControl::Current(element) => format!("I({element})"),
                    })
                    .collect();
                let mut coefficients = Vec::with_capacity(poly.coefficients.len());
                for raw in &poly.coefficients {
                    coefficients.push(ctx.value(raw, "POLY coefficient")?);
                }
                let kind = if matches!(letter, 'E' | 'H') {
                    BehaviouralKind::Voltage
                } else {
                    BehaviouralKind::Current
                };
                push_behavioural(
                    &mut circuit,
                    &mut behavioural_lines,
                    &ctx,
                    head,
                    [tokens[1], tokens[2]],
                    kind,
                    &poly_expression(&atoms, &coefficients),
                )?;
            }
            'E' | 'G' if laplace_form(&tokens[3..]).is_some() => {
                let (input, transfer) =
                    laplace_form(&tokens[3..]).expect("guard matched a LAPLACE form");
                let (input, transfer) = parse_laplace(&ctx, &input, &transfer)?;
                let kind = if letter == 'E' {
                    BehaviouralKind::Voltage
                } else {
                    BehaviouralKind::Current
                };
                push_laplace(
                    &mut circuit,
                    &mut behavioural_lines,
                    &ctx,
                    head,
                    [tokens[1], tokens[2]],
                    kind,
                    &input,
                    &transfer,
                )?;
            }
            'E' | 'G' if tokens.len() > 3 && is_table_keyword(tokens[3]) => {
                let (input, points) = parse_table(&ctx, &tokens[3..].join(" "))?;
                let expression =
                    Expression::table(&input, points).map_err(|message| ctx.parse_err(message))?;
                let kind = if letter == 'E' {
                    BehaviouralKind::Voltage
                } else {
                    BehaviouralKind::Current
                };
                push_expression(
                    &mut circuit,
                    &mut behavioural_lines,
                    &ctx,
                    head,
                    [tokens[1], tokens[2]],
                    kind,
                    expression,
                );
            }
            'E' | 'G'
                if tokens.len() > 3 && {
                    let key = tokens[3].split('=').next().unwrap_or("");
                    key.eq_ignore_ascii_case("value")
                } =>
            {
                let text = tokens[3..].join(" ");
                let body = text.split_once('=').map(|(_, body)| body).ok_or_else(|| {
                    ctx.parse_err(format!("expected `{letter}<name> n+ n- VALUE={{<expr>}}`"))
                })?;
                let kind = if letter == 'E' {
                    BehaviouralKind::Voltage
                } else {
                    BehaviouralKind::Current
                };
                push_behavioural(
                    &mut circuit,
                    &mut behavioural_lines,
                    &ctx,
                    head,
                    [tokens[1], tokens[2]],
                    kind,
                    body,
                )?;
            }
            'E' | 'G' => {
                let shape = if letter == 'E' {
                    "E<name> n+ n- nc+ nc- <gain>"
                } else {
                    "G<name> n+ n- nc+ nc- <gm>"
                };
                let rest = &tokens[1..];
                if rest.iter().any(|token| {
                    token
                        .get(..7)
                        .is_some_and(|head| head.eq_ignore_ascii_case("laplace"))
                }) {
                    return Err(ctx.parse_err(format!(
                        "expected `{letter}<name> n+ n- LAPLACE {{<expr>}} = {{<H(s)>}}` or \
                         `{letter}<name> n+ n- nc+ nc- LAPLACE=<H(s)>`"
                    )));
                }
                refuse_behavioural_source(&ctx, rest)?;
                if rest.len() != 5 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let value = ctx.value(rest[4], if letter == 'E' { "gain" } else { "gm" })?;
                let p = circuit.intern(rest[0]);
                let n = circuit.intern(rest[1]);
                let cp = circuit.intern(rest[2]);
                let cn = circuit.intern(rest[3]);
                let name = head.to_string();
                if letter == 'E' {
                    circuit.vcvs.push(Vcvs {
                        name,
                        p,
                        n,
                        cp,
                        cn,
                        gain: value,
                    });
                } else {
                    circuit.vccs.push(Vccs {
                        name,
                        p,
                        n,
                        cp,
                        cn,
                        gm: value,
                    });
                }
            }
            'F' | 'H' => {
                let shape = if letter == 'F' {
                    "F<name> n+ n- <vsource> <gain>"
                } else {
                    "H<name> n+ n- <vsource> <r>"
                };
                let rest = &tokens[1..];
                refuse_behavioural_source(&ctx, rest)?;
                if rest.len() != 4 {
                    return Err(ctx.parse_err(format!("expected `{shape}`")));
                }
                let value = ctx.value(rest[3], if letter == 'F' { "gain" } else { "r" })?;
                let p = circuit.intern(rest[0]);
                let n = circuit.intern(rest[1]);
                let name = head.to_string();
                let control = rest[2].to_string();
                // The controlling element may be declared later, and its
                // branch number is only final once every `V`, `L`, `E` and `H`
                // (the lowered op-amps' included) exists; bound after the
                // last line.
                controls.push((letter, ctx.number, line.to_string()));
                if letter == 'F' {
                    circuit.cccs.push(Cccs {
                        name,
                        p,
                        n,
                        control,
                        control_branch: usize::MAX,
                        gain: value,
                    });
                } else {
                    circuit.ccvs.push(Ccvs {
                        name,
                        p,
                        n,
                        control,
                        control_branch: usize::MAX,
                        ohms: value,
                    });
                }
            }
            'X' => {
                let rest = &tokens[1..];
                if rest.len() < 2 {
                    return Err(ctx.unsupported());
                }
                // Whether this is an op-amp or a real subcircuit call depends on
                // a `.model` line that may not have been read yet, so it is
                // decided with the other devices, after the last line.
                let (model_name, terminals) = rest.split_last().expect("checked above");
                let nodes = terminals.iter().map(|name| circuit.intern(name)).collect();
                pending.push(PendingDevice {
                    line: ctx.number,
                    text: line.to_string(),
                    letter,
                    name: head.to_string(),
                    nodes,
                    model_name: model_name.to_string(),
                    width: None,
                    length: None,
                    nrd: None,
                    nrs: None,
                    ad: None,
                    as_: None,
                    pd: None,
                    ps: None,
                    temperature_celsius: None,
                    multiplier: 1.0,
                    area: 1.0,
                    perimeter: None,
                    off: false,
                    ic: Vec::new(),
                });
            }
            'T' => {
                let line = parse_transmission_line(&ctx, head, &tokens[1..], &mut circuit)?;
                circuit.transmission_lines.push(line);
            }
            'W' => {
                let rest = &tokens[1..];
                let Some(initially_closed) = switch_flag(rest, 4) else {
                    return Err(
                        ctx.parse_err("expected `W<name> n+ n- <element> <model> [on|off]`")
                    );
                };
                let a = circuit.intern(rest[0]);
                let b = circuit.intern(rest[1]);
                pending_switches.push(PendingSwitch {
                    line: ctx.number,
                    text: line.to_string(),
                    index: circuit.switches.len(),
                    model_name: rest[3].to_string(),
                });
                circuit.switches.push(Switch {
                    name: head.to_string(),
                    a,
                    b,
                    ctrl: String::new(),
                    ron: 1.0,
                    roff: 1.0,
                    voltage: None,
                    threshold: Some(ThresholdSwitchControl {
                        sense: SwitchSense::Current {
                            control: rest[2].to_string(),
                            branch: usize::MAX,
                        },
                        threshold: 0.0,
                        hysteresis: 0.0,
                        initially_closed,
                    }),
                });
            }
            'S' if names_switch_model(&tokens[1..]) => {
                let rest = &tokens[1..];
                let Some(initially_closed) = switch_flag(rest, 5) else {
                    return Err(ctx.parse_err("expected `S<name> n+ n- nc+ nc- <model> [on|off]`"));
                };
                let a = circuit.intern(rest[0]);
                let b = circuit.intern(rest[1]);
                let pos = circuit.intern(rest[2]);
                let neg = circuit.intern(rest[3]);
                pending_switches.push(PendingSwitch {
                    line: ctx.number,
                    text: line.to_string(),
                    index: circuit.switches.len(),
                    model_name: rest[4].to_string(),
                });
                circuit.switches.push(Switch {
                    name: head.to_string(),
                    a,
                    b,
                    ctrl: String::new(),
                    ron: 1.0,
                    roff: 1.0,
                    voltage: None,
                    threshold: Some(ThresholdSwitchControl {
                        sense: SwitchSense::Voltage { pos, neg },
                        threshold: 0.0,
                        hysteresis: 0.0,
                        initially_closed,
                    }),
                });
            }
            'S' => {
                let digital = "S<name> n1 n2 <ctrl> ron=<r> roff=<r>";
                let voltage = "S<name> n1 n2 nc+ nc- vt=<v> [vh=<v>] [invert=0|1] ron=<r> roff=<r>";
                let rest = &tokens[1..];
                let voltage_form = rest.iter().any(|token| {
                    token
                        .split_once('=')
                        .map(|(key, _)| key.eq_ignore_ascii_case("vt"))
                        .unwrap_or(false)
                });
                if voltage_form {
                    if rest.len() < 6 {
                        return Err(ctx.parse_err(format!("expected `{voltage}`")));
                    }
                    let mut ron = None;
                    let mut roff = None;
                    let mut vt = None;
                    let mut vh = 0.0;
                    let mut invert = false;
                    for token in &rest[4..] {
                        let (key, value) = token
                            .split_once('=')
                            .ok_or_else(|| ctx.parse_err(format!("expected `{voltage}`")))?;
                        match key.to_ascii_lowercase().as_str() {
                            "ron" => ron = Some(ctx.value(value, "ron")?),
                            "roff" => roff = Some(ctx.value(value, "roff")?),
                            "vt" => vt = Some(ctx.value(value, "vt")?),
                            "vh" => vh = ctx.value(value, "vh")?,
                            "invert" => {
                                let parsed = ctx.value(value, "invert")?;
                                if parsed != 0.0 && parsed != 1.0 {
                                    return Err(ctx.parse_err("invert must be 0 or 1"));
                                }
                                invert = parsed != 0.0;
                            }
                            other => {
                                return Err(
                                    ctx.parse_err(format!("unknown switch parameter `{other}`"))
                                )
                            }
                        }
                    }
                    let (Some(ron), Some(roff), Some(vt)) = (ron, roff, vt) else {
                        return Err(ctx.parse_err(format!("expected `{voltage}`")));
                    };
                    if ron <= 0.0 || roff <= 0.0 {
                        return Err(ctx.parse_err("ron and roff must be positive"));
                    }
                    if vt <= 0.0 {
                        return Err(ctx.parse_err("vt must be positive"));
                    }
                    if vh < 0.0 || vh > vt {
                        return Err(ctx.parse_err("vh must be in [0, vt]"));
                    }
                    let a = circuit.intern(rest[0]);
                    let b = circuit.intern(rest[1]);
                    let pos = circuit.intern(rest[2]);
                    let neg = circuit.intern(rest[3]);
                    circuit.switches.push(Switch {
                        name: head.to_string(),
                        a,
                        b,
                        ctrl: String::new(),
                        ron,
                        roff,
                        voltage: Some(VoltageSwitchControl {
                            pos,
                            neg,
                            vt,
                            vh,
                            invert,
                        }),
                        threshold: None,
                    });
                } else {
                    if rest.len() != 5 {
                        return Err(ctx.parse_err(format!("expected `{digital}`")));
                    }
                    let mut ron = None;
                    let mut roff = None;
                    for token in &rest[3..] {
                        let (key, value) = token
                            .split_once('=')
                            .ok_or_else(|| ctx.parse_err(format!("expected `{digital}`")))?;
                        match key.to_ascii_lowercase().as_str() {
                            "ron" => ron = Some(ctx.value(value, "ron")?),
                            "roff" => roff = Some(ctx.value(value, "roff")?),
                            other => {
                                return Err(
                                    ctx.parse_err(format!("unknown switch parameter `{other}`"))
                                )
                            }
                        }
                    }
                    let (Some(ron), Some(roff)) = (ron, roff) else {
                        return Err(ctx.parse_err(format!("expected `{digital}`")));
                    };
                    if ron <= 0.0 || roff <= 0.0 {
                        return Err(ctx.parse_err("ron and roff must be positive"));
                    }
                    let a = circuit.intern(rest[0]);
                    let b = circuit.intern(rest[1]);
                    circuit.switches.push(Switch {
                        name: head.to_string(),
                        a,
                        b,
                        ctrl: rest[2].to_string(),
                        ron,
                        roff,
                        voltage: None,
                        threshold: None,
                    });
                }
            }
            _ => unreachable!("element letter filtered above"),
        }
    }

    bind_mutual_inductors(&mut circuit, pending_mutuals)?;
    bind_models(&mut circuit, &models, pending)?;
    bind_switches(&mut circuit, &models, pending_switches)?;
    bind_controls(&mut circuit, &controls)?;
    bind_behavioural(&mut circuit, &behavioural_lines)?;
    Ok(circuit)
}

/// Parse one behavioural expression and add its source to the circuit.
fn push_behavioural(
    circuit: &mut Circuit,
    lines: &mut Vec<(BehaviouralKind, usize, String)>,
    ctx: &LineCtx<'_>,
    name: &str,
    terminals: [&str; 2],
    kind: BehaviouralKind,
    body: &str,
) -> Result<(), AnalogError> {
    // A brace group inside the expression (`I={k}*V(a)` after numparam) is a
    // parenthesised term, as ngspice reads it.
    let body = body.trim().replace('{', "(").replace('}', ")");
    let expression = Expression::parse(&body).map_err(|message| ctx.parse_err(message))?;
    push_expression(circuit, lines, ctx, name, terminals, kind, expression);
    Ok(())
}

/// Add a behavioural source with an already parsed expression.
fn push_expression(
    circuit: &mut Circuit,
    lines: &mut Vec<(BehaviouralKind, usize, String)>,
    ctx: &LineCtx<'_>,
    name: &str,
    terminals: [&str; 2],
    kind: BehaviouralKind,
    expression: Expression,
) {
    let source = BehaviouralSource {
        name: name.to_string(),
        p: circuit.intern(terminals[0]),
        n: circuit.intern(terminals[1]),
        kind,
        expression,
        controls: Vec::new(),
    };
    match kind {
        BehaviouralKind::Voltage => circuit.behavioural_voltages.push(source),
        BehaviouralKind::Current => circuit.behavioural_currents.push(source),
    }
    lines.push((kind, ctx.number, ctx.text.to_string()));
}

/// The input and transfer text of an `E`/`G` `LAPLACE` source, from the
/// tokens after its two output nodes: PSpice's `LAPLACE {<expr>} = {<H(s)>}`
/// or LTspice's `nc+ nc- LAPLACE=<H(s)>`, whose input is `V(nc+, nc-)`.
/// `None` when the tokens are some other form.
fn laplace_form(tokens: &[&str]) -> Option<(String, String)> {
    let starts_laplace = |token: &str| {
        token
            .get(..7)
            .is_some_and(|head| head.eq_ignore_ascii_case("laplace"))
    };
    let first = tokens.first()?;
    if starts_laplace(first) && !first[7..].trim_start().starts_with('=') {
        // `LAPLACE {in} = {H}`: two brace groups, the `=` optional.
        let text = tokens.join(" ");
        let after = text[7..].trim_start();
        let close = after.strip_prefix('{')?.find('}')? + 1;
        let input = after[1..close].to_string();
        let rest = after[close + 1..].trim_start();
        let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
        return Some((input, rest.to_string()));
    }
    let third = tokens.get(2)?;
    if starts_laplace(third) {
        let text = tokens[2..].join(" ");
        let rest = text[7..].trim_start().strip_prefix('=')?;
        return Some((
            format!("V({}, {})", tokens[0], tokens[1]),
            rest.trim().to_string(),
        ));
    }
    None
}

/// Strip the braces a `LAPLACE` input and transfer may carry and read the
/// transfer as a rational function of `s`.
fn parse_laplace(
    ctx: &LineCtx<'_>,
    input: &str,
    transfer: &str,
) -> Result<(String, Transfer), AnalogError> {
    let unbrace = |text: &str| text.trim().replace('{', "(").replace('}', ")");
    let transfer = Transfer::parse(&unbrace(transfer)).map_err(|message| ctx.parse_err(message))?;
    Ok((unbrace(input), transfer))
}

/// Lower `E`/`G ... LAPLACE` to a state-space network of elements the solver
/// already has (see the module notes): one node and one capacitor per pole,
/// linear `G` sources for the companion matrix, and two behavioural sources
/// for the input and the output.
#[allow(clippy::too_many_arguments)]
fn push_laplace(
    circuit: &mut Circuit,
    lines: &mut Vec<(BehaviouralKind, usize, String)>,
    ctx: &LineCtx<'_>,
    name: &str,
    terminals: [&str; 2],
    kind: BehaviouralKind,
    input: &str,
    transfer: &Transfer,
) -> Result<(), AnalogError> {
    let numerator = &transfer.numerator;
    let denominator = &transfer.denominator;
    let order = denominator.len() - 1;
    if numerator.len() > denominator.len() {
        return Err(ctx.parse_err(
            "the LAPLACE transfer is improper (numerator order above denominator order): \
             it would differentiate its input",
        ));
    }
    if denominator[0] == 0.0 {
        return Err(ctx.parse_err(
            "the LAPLACE transfer has a pole at s = 0 (an integrator): it has no DC \
             operating point",
        ));
    }
    // Frequency scaling `s = w0·p`, with `w0` the geometric mean of the pole
    // magnitudes, keeps the companion coefficients near 1 for a 1 ms filter
    // and a 1 GHz one alike; each state capacitor is then 1/w0 farads.
    let w0 = if order == 0 {
        1.0
    } else {
        (denominator[0].abs() / denominator[order].abs()).powf(1.0 / order as f64)
    };
    let lead = denominator[order] * w0.powi(order as i32);
    let a: Vec<f64> = (0..order)
        .map(|k| denominator[k] * w0.powi(k as i32) / lead)
        .collect();
    let b: Vec<f64> = (0..=order)
        .map(|k| numerator.get(k).unwrap_or(&0.0) * w0.powi(k as i32) / lead)
        .collect();
    // Controllable canonical form: x_k' = x_(k+1), x_n' = u − Σ a_k·x_(k+1),
    // y = Σ (b_k − b_n·a_k)·x_(k+1) + b_n·u.
    let states: Vec<String> = (1..=order).map(|k| format!("{name}#s{k}")).collect();
    let ground = circuit.intern("0");
    for (index, state) in states.iter().enumerate() {
        let node = circuit.intern(state);
        circuit.capacitors.push(Capacitor {
            name: format!("{name}#c{}", index + 1),
            a: node,
            b: ground,
            farads: 1.0 / w0,
            ic: None,
        });
        if index + 1 < order {
            let next = circuit.intern(&states[index + 1]);
            circuit.vccs.push(Vccs {
                name: format!("{name}#g{}", index + 1),
                p: ground,
                n: node,
                cp: next,
                cn: ground,
                gm: 1.0,
            });
        }
    }
    if let Some(last) = states.last() {
        let last_node = circuit.intern(last);
        for (k, coefficient) in a.iter().enumerate() {
            if *coefficient != 0.0 {
                let control = circuit.intern(&states[k]);
                circuit.vccs.push(Vccs {
                    name: format!("{name}#a{k}"),
                    p: ground,
                    n: last_node,
                    cp: control,
                    cn: ground,
                    gm: -coefficient,
                });
            }
        }
        push_behavioural(
            circuit,
            lines,
            ctx,
            &format!("{name}#u"),
            ["0", last],
            BehaviouralKind::Current,
            input,
        )?;
    }
    let direct = b[order];
    let mut terms: Vec<String> = states
        .iter()
        .zip(&a)
        .zip(&b)
        .filter_map(|((state, a_k), b_k)| {
            let c = b_k - direct * a_k;
            (c != 0.0).then(|| format!("({c:e})*V({state})"))
        })
        .collect();
    if direct != 0.0 {
        terms.push(format!("({direct:e})*({input})"));
    }
    let output = if terms.is_empty() {
        "0".to_string()
    } else {
        terms.join(" + ")
    };
    push_behavioural(circuit, lines, ctx, name, terminals, kind, &output)
}

/// `TABLE` or `TABLE{...}`: the token that starts an `E`/`G` table.
fn is_table_keyword(token: &str) -> bool {
    token
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case("table"))
        && token[5..].chars().next().is_none_or(|c| c == '{')
}

/// Read `TABLE {<expr>} = (x1, y1) (x2, y2) ...`: the input expression and
/// the points. The `=` is optional; a point's values may carry a scale
/// suffix.
fn parse_table(ctx: &LineCtx<'_>, text: &str) -> Result<(String, Vec<(f64, f64)>), AnalogError> {
    let usage = "expected `TABLE {<expr>} = (x1, y1) (x2, y2) ...`";
    let after = text[5..].trim_start();
    let close = after
        .strip_prefix('{')
        .and_then(|inner| inner.find('}'))
        .ok_or_else(|| ctx.parse_err(usage))?;
    let input = after[1..=close].to_string();
    let rest = after[close + 2..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest);
    let mut values = Vec::new();
    for raw in rest
        .split(|c: char| c == '(' || c == ')' || c == ',' || c.is_whitespace())
        .filter(|raw| !raw.is_empty())
    {
        values.push(ctx.value(raw, "TABLE point")?);
    }
    if values.len() % 2 != 0 {
        return Err(ctx.parse_err("`TABLE` points come in (x, y) pairs"));
    }
    let points = values.chunks(2).map(|pair| (pair[0], pair[1])).collect();
    Ok((input, points))
}

/// Bind every `V(...)` and `I(...)` a behavioural expression reads.
///
/// Done after [`bind_controls`], once every node exists and the branch
/// numbering is final.
fn bind_behavioural(
    circuit: &mut Circuit,
    lines: &[(BehaviouralKind, usize, String)],
) -> Result<(), AnalogError> {
    for kind in [BehaviouralKind::Voltage, BehaviouralKind::Current] {
        let count = match kind {
            BehaviouralKind::Voltage => circuit.behavioural_voltages.len(),
            BehaviouralKind::Current => circuit.behavioural_currents.len(),
        };
        let mut lines = lines.iter().filter(|(k, _, _)| *k == kind);
        for index in 0..count {
            let (_, number, text) = lines.next().expect("one line per behavioural source");
            let ctx = LineCtx {
                number: *number,
                text,
            };
            let source = match kind {
                BehaviouralKind::Voltage => &circuit.behavioural_voltages[index],
                BehaviouralKind::Current => &circuit.behavioural_currents[index],
            };
            let node = |name: &str| {
                circuit
                    .node(name)
                    .ok_or_else(|| ctx.parse_err(format!("node `{name}` is not in this netlist")))
            };
            let mut bound = Vec::with_capacity(source.expression.controls().len());
            for control in source.expression.controls() {
                bound.push(match control {
                    ControlName::Voltage(p, n) => Control::Voltage(
                        node(p)?,
                        match n {
                            Some(n) => node(n)?,
                            None => None,
                        },
                    ),
                    ControlName::Current(element) => {
                        Control::Branch(circuit.branch_index(element).ok_or_else(|| {
                            ctx.parse_err(format!(
                                "`{element}` carries no branch current; I(...) must name a \
                                 V, L, E, H or voltage-form B element"
                            ))
                        })?)
                    }
                });
            }
            match kind {
                BehaviouralKind::Voltage => circuit.behavioural_voltages[index].controls = bound,
                BehaviouralKind::Current => circuit.behavioural_currents[index].controls = bound,
            }
        }
    }
    Ok(())
}

/// True for the `POLY` keyword of a SPICE2 polynomial source, written
/// `POLY(2)`, `POLY (2)` or `POLY`.
fn is_poly_keyword(token: &str) -> bool {
    let upper = token.to_ascii_uppercase();
    upper == "POLY" || upper.starts_with("POLY(")
}

/// One controlling quantity of a `POLY` source.
#[derive(Debug, Clone, PartialEq)]
enum PolyControl {
    /// `nc+ nc-` of an `E` or `G` source.
    Voltage(String, String),
    /// The element whose branch current an `F` or `H` source reads.
    Current(String),
}

/// A SPICE2 polynomial dependent source, after the two output nodes:
/// `POLY(nd) <controls> <p0> <p1> ...`.
///
/// `E`/`G` name `nd` node pairs, `F`/`H` name `nd` elements. A pair may be
/// written `a b`, `(a,b)` or `(a, b)`, as in ngspice.
#[derive(Debug, Clone, PartialEq)]
struct PolySource {
    controls: Vec<PolyControl>,
    /// Coefficients as written; the caller reads their values.
    coefficients: Vec<String>,
}

impl PolySource {
    /// Parse `tokens`, which start at the `POLY` keyword.
    fn parse(letter: char, tokens: &[&str]) -> Result<Self, String> {
        let text = tokens.join(" ");
        let text = text[4..].replace(['(', ')', ','], " ");
        let mut words = text.split_whitespace();
        let dimensions = words
            .next()
            .and_then(|word| word.parse::<usize>().ok())
            .filter(|count| *count >= 1)
            .ok_or("expected `POLY(<n>)` with a whole number of controls n >= 1")?;
        let words: Vec<&str> = words.collect();
        if let Some(option) = words.iter().find(|word| word.contains('=')) {
            return Err(format!(
                "`{option}`: POLY initial-condition guesses are outside the supported subset"
            ));
        }
        let per_control = if matches!(letter, 'E' | 'G') { 2 } else { 1 };
        let control_words = dimensions * per_control;
        if words.len() <= control_words {
            return Err(format!(
                "POLY({dimensions}) needs {control_words} control {} and at least one coefficient",
                if per_control == 2 {
                    "nodes"
                } else {
                    "elements"
                }
            ));
        }
        let controls = words[..control_words]
            .chunks(per_control)
            .map(|chunk| match chunk {
                [p, n] => PolyControl::Voltage(p.to_string(), n.to_string()),
                [element] => PolyControl::Current(element.to_string()),
                _ => unreachable!("chunks of one or two"),
            })
            .collect();
        let coefficients = words[control_words..]
            .iter()
            .map(|word| word.to_string())
            .collect();
        Ok(Self {
            controls,
            coefficients,
        })
    }

    /// The same source with each control renamed, in canonical
    /// `POLY(n) a b ... p0 p1 ...` text, for a subcircuit body.
    fn rename(&self, node: &dyn Fn(&str) -> String, element: &dyn Fn(&str) -> String) -> String {
        let mut words = vec![format!("POLY({})", self.controls.len())];
        for control in &self.controls {
            match control {
                PolyControl::Voltage(p, n) => {
                    words.push(node(p));
                    words.push(node(n));
                }
                PolyControl::Current(name) => words.push(element(name)),
            }
        }
        words.extend(self.coefficients.iter().cloned());
        words.join(" ")
    }
}

/// The behavioural expression of a SPICE2 polynomial over `atoms`.
///
/// Coefficients follow SPICE2's order: `p0`, the `n` linear terms, then each
/// higher degree with its control indices non-decreasing in lexicographic
/// order (`x1², x1·x2, x2²`, then `x1³, x1²·x2, ...`), for as many terms as
/// coefficients are given. A single coefficient is the constant `p0`, which
/// is what ngspice-42 does; SPICE2 read it as `p1`.
fn poly_expression(atoms: &[String], coefficients: &[f64]) -> String {
    let mut terms = Vec::with_capacity(coefficients.len());
    let mut indices: Vec<usize> = Vec::new();
    for &coefficient in coefficients {
        let mut term = format!("({coefficient:e})");
        for &index in &indices {
            term.push_str(&format!("*{}", atoms[index]));
        }
        terms.push(term);
        indices = next_monomial(indices, atoms.len());
    }
    terms.join(" + ")
}

/// The monomial after `indices` (non-decreasing control indices) in SPICE2
/// order; after the last monomial of a degree comes the first of the next.
fn next_monomial(mut indices: Vec<usize>, dimensions: usize) -> Vec<usize> {
    // Raise the rightmost index that can still grow and reset the ones after
    // it to that value; when none can, start the next degree at all zeros.
    if let Some(at) = (0..indices.len())
        .rev()
        .find(|&at| indices[at] + 1 < dimensions)
    {
        let value = indices[at] + 1;
        for index in &mut indices[at..] {
            *index = value;
        }
        indices
    } else {
        vec![0; indices.len() + 1]
    }
}

/// Refuse the nonlinear dependent-source forms by name, so they read as
/// "outside the subset" rather than as a malformed linear source.
fn refuse_behavioural_source(ctx: &LineCtx<'_>, rest: &[&str]) -> Result<(), AnalogError> {
    let behavioural = rest.iter().any(|token| {
        let upper = token.to_ascii_uppercase();
        upper.starts_with("POLY")
            || upper.starts_with("VALUE")
            || upper.starts_with("TABLE")
            || upper.starts_with("FREQ")
            || upper.starts_with("CHEBYSHEV")
            || upper.contains('{')
    });
    if behavioural {
        Err(ctx.unsupported())
    } else {
        Ok(())
    }
}

/// Resolve each `F`/`H` to the branch current of the element it names.
///
/// Done last because the branch numbering is final only once every `V`, `L`,
/// `E` and `H` exists, including the ones an op-amp lowers into.
fn bind_controls(
    circuit: &mut Circuit,
    controls: &[(char, usize, String)],
) -> Result<(), AnalogError> {
    let resolve = |circuit: &Circuit, letter: char, index: usize, name: &str| {
        circuit.branch_index(name).ok_or_else(|| {
            let message = format!(
                "`{name}` carries no branch current; an F or H element must name a \
                 V, L, E or H element"
            );
            // A lowered regulator's own F comes after every written one and
            // names an E it just made, so only a written line can fail here;
            // the fallback keeps a would-be bug an error rather than a panic.
            match controls.iter().filter(|(l, _, _)| *l == letter).nth(index) {
                Some((_, line, text)) => AnalogError::Parse {
                    line: *line,
                    text: text.clone(),
                    message,
                },
                None => AnalogError::Config(message),
            }
        })
    };
    for index in 0..circuit.cccs.len() {
        let branch = resolve(circuit, 'F', index, &circuit.cccs[index].control)?;
        circuit.cccs[index].control_branch = branch;
    }
    for index in 0..circuit.ccvs.len() {
        let branch = resolve(circuit, 'H', index, &circuit.ccvs[index].control)?;
        circuit.ccvs[index].control_branch = branch;
    }
    for index in 0..circuit.power_sinks.len() {
        let name = circuit.power_sinks[index].control.clone();
        let branch = circuit.branch_index(&name).ok_or_else(|| {
            AnalogError::Config(format!("power sink control `{name}` carries no branch"))
        })?;
        circuit.power_sinks[index].control_branch = branch;
    }
    Ok(())
}

/// Resolve K elements after every inductor is known, allowing forward references.
fn bind_mutual_inductors(
    circuit: &mut Circuit,
    pending: Vec<PendingMutualInductor>,
) -> Result<(), AnalogError> {
    for coupling in pending {
        let ctx = LineCtx {
            number: coupling.line,
            text: &coupling.text,
        };
        let find_inductor = |name: &str| {
            circuit
                .inductors
                .iter()
                .position(|inductor| inductor.name.eq_ignore_ascii_case(name))
        };
        let first = find_inductor(&coupling.first).ok_or_else(|| {
            ctx.parse_err(format!(
                "`{}` is not an inductor in this netlist",
                coupling.first
            ))
        })?;
        let second = find_inductor(&coupling.second).ok_or_else(|| {
            ctx.parse_err(format!(
                "`{}` is not an inductor in this netlist",
                coupling.second
            ))
        })?;
        if first == second {
            return Err(ctx.parse_err("a K element must couple two different inductors"));
        }
        if circuit.mutual_inductors.iter().any(|existing| {
            (existing.first == first && existing.second == second)
                || (existing.first == second && existing.second == first)
        }) {
            return Err(ctx.parse_err("the same inductor pair cannot be coupled twice"));
        }
        let first_l = circuit.inductors[first].henries;
        let second_l = circuit.inductors[second].henries;
        let mutual_henries = coupling.coupling * (first_l.sqrt() * second_l.sqrt());
        if !mutual_henries.is_finite() {
            return Err(ctx.parse_err("inductor values produce non-finite mutual inductance"));
        }
        circuit.mutual_inductors.push(MutualInductor {
            name: coupling.name,
            first,
            second,
            mutual_henries,
        });
    }
    Ok(())
}

/// Attach each device's model card, once every `.model` line has been read.
fn bind_models(
    circuit: &mut Circuit,
    models: &BTreeMap<String, ModelCard>,
    pending: Vec<PendingDevice>,
) -> Result<(), AnalogError> {
    for device in pending {
        let ctx = LineCtx {
            number: device.line,
            text: &device.text,
        };
        let key = device.model_name.to_ascii_lowercase();
        let resolved = models
            .get(&key)
            .copied()
            .or_else(|| builtin_model(&device.model_name));
        if device.letter == 'X' {
            // Only an op-amp, comparator or regulator card makes an `X` line
            // ours; any other `X` is a call into a subcircuit library.
            match resolved {
                Some(ModelCard::OpAmp(model)) => {
                    lower_opamp(circuit, &ctx, &device, model)?;
                }
                Some(ModelCard::Comparator(model)) => {
                    lower_comparator(circuit, &ctx, &device, model)?;
                }
                Some(ModelCard::Regulator(model)) => {
                    lower_regulator(circuit, &ctx, &device, model)?;
                }
                _ => return Err(ctx.unsupported()),
            }
            continue;
        }
        let card = resolved.ok_or_else(|| {
            ctx.parse_err(format!(
                "no `.model {}` line, and `{}` is not one of the built-in models \
                     `D`, `NPN`, `PNP`, `NMOS`, `PMOS`, `NJF`, `PJF`, `1N4733A`, `SMAJ5.0A`",
                device.model_name, device.model_name
            ))
        })?;
        let wrong_kind = |wanted: &str| {
            ctx.parse_err(format!(
                "`{}` is a {} element but `{}` is a {} model",
                device.name,
                wanted,
                device.model_name,
                card.kind()
            ))
        };
        match device.letter {
            'D' => {
                let ModelCard::Diode(model) = card else {
                    return Err(wrong_kind("D"));
                };
                let model = model.with_area(device.area);
                if model.is <= 0.0
                    || model.n <= 0.0
                    || !model.jsw.is_finite()
                    || model.jsw < 0.0
                    || matches!(model.ns, Some(ns) if !ns.is_finite() || ns <= 0.0)
                    || !model.pj.is_finite()
                    || model.pj < 0.0
                    || !model.eg.is_finite()
                    || model.eg < 0.0
                    || !model.xti.is_finite()
                    || !model.tnom_celsius.is_finite()
                    || model.tnom_celsius <= -273.15
                    || !model.tcv.is_finite()
                    || !model.trs1.is_finite()
                    || !model.trs2.is_finite()
                    || !model.ttt1.is_finite()
                    || !model.ttt2.is_finite()
                    || !model.tm1.is_finite()
                    || !model.tm2.is_finite()
                    || !model.ik.is_finite()
                    || model.ik < 0.0
                    || !model.ikr.is_finite()
                    || model.ikr < 0.0
                    || !model.isr.is_finite()
                    || model.isr < 0.0
                    || !model.nr.is_finite()
                    || model.nr <= 0.0
                    || !model.rs.is_finite()
                    || model.rs < 0.0
                    || !model.tt.is_finite()
                    || model.tt < 0.0
                    || !model.vp.is_finite()
                    || model.vp < 0.0
                    || !model.cjo.is_finite()
                    || model.cjo < 0.0
                    || !model.vj.is_finite()
                    || model.vj <= 0.0
                    || !model.m.is_finite()
                    || model.m < 0.0
                    || !model.fc.is_finite()
                    || model.fc < 0.0
                    || model.fc >= 1.0
                    || !model.cjsw.is_finite()
                    || model.cjsw < 0.0
                    || !model.php.is_finite()
                    || model.php <= 0.0
                    || !model.mjsw.is_finite()
                    || model.mjsw < 0.0
                    || !model.fcs.is_finite()
                    || model.fcs < 0.0
                    || model.fcs >= 1.0
                {
                    return Err(ctx.parse_err(
                        "a diode model needs IS > 0, N > 0, JSW >= 0, NS > 0, PJ >= 0, EG >= 0, finite XTI/TCV/TRS1/TRS2/TTT1/TTT2/TM1/TM2, TNOM above absolute zero, IK/IKR >= 0, ISR >= 0, NR > 0, RS >= 0, TT >= 0, VP >= 0, CJO >= 0, VJ > 0, M >= 0, 0 <= FC < 1, CJSW >= 0, PHP > 0, MJSW >= 0 and 0 <= FCS < 1"
                            .to_string(),
                    ));
                }
                if !model.jtun.is_finite()
                    || model.jtun < 0.0
                    || !model.jtunsw.is_finite()
                    || model.jtunsw < 0.0
                    || !model.ntun.is_finite()
                    || model.ntun <= 0.0
                    || !model.xtitun.is_finite()
                    || !model.keg.is_finite()
                {
                    return Err(ctx.parse_err(
                        "a diode model needs JTUN >= 0, JTUNSW >= 0, NTUN > 0 and finite XTITUN/KEG"
                            .to_string(),
                    ));
                }
                let model = model.with_perimeter(device.perimeter);
                let (is_at_temperature, thermal_voltage) =
                    if let Some(temp_celsius) = device.temperature_celsius {
                        let temperature_kelvin = temp_celsius + 273.15;
                        let nominal_kelvin = model.tnom_celsius + 273.15;
                        (
                            super::device::diode_saturation_current_at_temperature(
                                model.is,
                                model.n,
                                model.eg,
                                model.xti,
                                nominal_kelvin,
                                temperature_kelvin,
                            ),
                            super::device::thermal_voltage_at_temperature(temperature_kelvin),
                        )
                    } else {
                        (model.is, super::device::THERMAL_VOLTAGE)
                    };
                let breakdown = match model.bv {
                    None => None,
                    Some(nominal_bv) => {
                        let nbv = model.nbv.unwrap_or(model.n);
                        if nominal_bv <= 0.0 || model.ibv <= 0.0 || nbv <= 0.0 {
                            return Err(ctx.parse_err(
                                "a diode model with BV needs BV > 0, IBV > 0 and NBV > 0"
                                    .to_string(),
                            ));
                        }
                        let bv = device
                            .temperature_celsius
                            .map(|temperature| {
                                nominal_bv * (1.0 + model.tcv * (temperature - model.tnom_celsius))
                            })
                            .unwrap_or(nominal_bv);
                        if !bv.is_finite() || bv <= 0.0 {
                            return Err(ctx.parse_err(
                                "temperature-adjusted diode BV must be finite and > 0".to_string(),
                            ));
                        }
                        Some(super::device::breakdown_voltage_at_thermal_voltage(
                            bv,
                            model.ibv,
                            is_at_temperature,
                            nbv,
                            thermal_voltage,
                        ))
                    }
                };
                let series_resistance_ohms =
                    model.series_resistance_ohms(device.temperature_celsius);
                if !series_resistance_ohms.is_finite() || series_resistance_ohms < 0.0 {
                    return Err(ctx.parse_err(
                        "temperature-adjusted diode RS must be finite and >= 0".to_string(),
                    ));
                }
                let mut operating_model = model;
                operating_model.tt = model.transit_time_seconds(device.temperature_celsius);
                operating_model.m =
                    model.grading_coefficient_at_temperature(device.temperature_celsius);
                if let (Some(temperature_celsius), 1) = (device.temperature_celsius, model.tlevc) {
                    // ngspice-42 `diotemp.c` `TLEVC=1`: linear in ΔT from
                    // `TNOM`, with `M` still on its `TM1`/`TM2` law.
                    let delta = temperature_celsius - model.tnom_celsius;
                    operating_model.cjo = model.cjo * (1.0 + model.cta * delta);
                    operating_model.vj = model.vj - model.tpb * delta;
                    operating_model.cjsw = model.cjsw * (1.0 + model.ctp * delta);
                    operating_model.php = model.php - model.tphp * delta;
                } else if let Some(temperature_celsius) = device.temperature_celsius {
                    let nominal_kelvin = model.tnom_celsius + 273.15;
                    let temperature_kelvin = temperature_celsius + 273.15;
                    (operating_model.cjo, operating_model.vj) =
                        super::device::diode_depletion_parameters_at_temperature(
                            model.cjo,
                            model.vj,
                            operating_model.m,
                            nominal_kelvin,
                            temperature_kelvin,
                        );
                    // ngspice-42 applies the same junction law to the
                    // sidewall with `PHP` and `MJSW`; `TM1`/`TM2` do not
                    // touch `MJSW`.
                    (operating_model.cjsw, operating_model.php) =
                        super::device::diode_depletion_parameters_at_temperature(
                            model.cjsw,
                            model.php,
                            model.mjsw,
                            nominal_kelvin,
                            temperature_kelvin,
                        );
                }
                if !operating_model.tt.is_finite() || operating_model.tt < 0.0 {
                    return Err(ctx.parse_err(
                        "temperature-adjusted diode TT must be finite and >= 0".to_string(),
                    ));
                }
                if !operating_model.m.is_finite() || operating_model.m < 0.0 {
                    return Err(ctx.parse_err(
                        "temperature-adjusted diode M must be finite and >= 0".to_string(),
                    ));
                }
                if !operating_model.cjo.is_finite()
                    || operating_model.cjo < 0.0
                    || !operating_model.vj.is_finite()
                    || operating_model.vj <= 0.0
                    || !operating_model.cjsw.is_finite()
                    || operating_model.cjsw < 0.0
                    || !operating_model.php.is_finite()
                    || operating_model.php <= 0.0
                {
                    return Err(ctx.parse_err(
                        "temperature-adjusted diode CJO/CJSW must be finite and >= 0 and VJ/PHP must be finite and > 0"
                            .to_string(),
                    ));
                }
                // A series resistance needs somewhere to drop its voltage, so
                // it gets the internal node SPICE also creates. It is a real
                // node: it costs an unknown and `v(d1#internal)` probes it.
                let junction_anode = if series_resistance_ohms > 0.0 {
                    circuit.intern(&format!("{}#internal", device.name))
                } else {
                    device.nodes[0]
                };
                circuit.diodes.push(Diode {
                    name: device.name,
                    multiplier: device.multiplier,
                    off: device.off,
                    ic: device.ic.first().copied(),
                    anode: device.nodes[0],
                    cathode: device.nodes[1],
                    junction_anode,
                    model_name: device.model_name,
                    model,
                    operating_model,
                    series_resistance_ohms,
                    temperature_celsius: device.temperature_celsius,
                    breakdown,
                });
            }
            'Q' => {
                let ModelCard::Bjt(model) = card else {
                    return Err(wrong_kind("Q"));
                };
                let unscaled_model = model;
                let model = model.with_area(device.area);
                if model.is <= 0.0 || model.bf <= 0.0 || model.br <= 0.0 {
                    return Err(
                        ctx.parse_err("a BJT model needs IS > 0, BF > 0 and BR > 0".to_string())
                    );
                }
                if [model.ibe, model.ibc]
                    .into_iter()
                    .flatten()
                    .any(|current| !current.is_finite() || current <= 0.0)
                {
                    return Err(ctx.parse_err("BJT IBE and IBC must be finite and > 0".to_string()));
                }
                if !model.is.is_finite()
                    || !model.bf.is_finite()
                    || !model.br.is_finite()
                    || !model.ise.is_finite()
                    || !model.isc.is_finite()
                    || !model.tnom_celsius.is_finite()
                    || model.tnom_celsius <= -273.15
                    || !model.eg.is_finite()
                    || model.eg < 0.0
                    || !model.xti.is_finite()
                    || !model.xtb.is_finite()
                    || !model.trb1.is_finite()
                    || !model.trb2.is_finite()
                    || !model.trbm1.is_finite()
                    || !model.trbm2.is_finite()
                    || !model.tirb1.is_finite()
                    || !model.tirb2.is_finite()
                    || !model.trc1.is_finite()
                    || !model.trc2.is_finite()
                    || !model.tre1.is_finite()
                    || !model.tre2.is_finite()
                    || !model.ttf1.is_finite()
                    || !model.ttf2.is_finite()
                    || !model.ttr1.is_finite()
                    || !model.ttr2.is_finite()
                    || !model.tmje1.is_finite()
                    || !model.tmje2.is_finite()
                    || !model.tmjc1.is_finite()
                    || !model.tmjc2.is_finite()
                    || !model.cte.is_finite()
                    || !model.ctc.is_finite()
                    || !model.tvje.is_finite()
                    || !model.tvjc.is_finite()
                    || !model.tbf1.is_finite()
                    || !model.tbf2.is_finite()
                    || !model.tbr1.is_finite()
                    || !model.tbr2.is_finite()
                    || !model.tikf1.is_finite()
                    || !model.tikf2.is_finite()
                    || !model.tikr1.is_finite()
                    || !model.tikr2.is_finite()
                    || !model.tnf1.is_finite()
                    || !model.tnf2.is_finite()
                    || !model.tnr1.is_finite()
                    || !model.tnr2.is_finite()
                    || !model.tne1.is_finite()
                    || !model.tne2.is_finite()
                    || !model.tnc1.is_finite()
                    || !model.tnc2.is_finite()
                    || !model.tvaf1.is_finite()
                    || !model.tvaf2.is_finite()
                    || !model.tvar1.is_finite()
                    || !model.tvar2.is_finite()
                    || !model.titf1.is_finite()
                    || !model.titf2.is_finite()
                    || !model.tis1.is_finite()
                    || !model.tis2.is_finite()
                    || !model.tise1.is_finite()
                    || !model.tise2.is_finite()
                    || !model.tisc1.is_finite()
                    || !model.tisc2.is_finite()
                    || !model.tiss1.is_finite()
                    || !model.tiss2.is_finite()
                    || model.tlevc > 1
                {
                    return Err(ctx.parse_err(
                        "a BJT model needs finite temperature parameters, TLEVC=0 or 1, and TNOM above absolute zero"
                            .to_string(),
                    ));
                }
                if model.nf <= 0.0 || model.nr <= 0.0 {
                    return Err(ctx.parse_err("a BJT model needs NF > 0 and NR > 0".to_string()));
                }
                if !model.ptf.is_finite() || model.ptf < 0.0 {
                    return Err(ctx.parse_err("a BJT model needs a finite PTF >= 0".to_string()));
                }
                if model.vaf.is_nan() || model.vaf < 0.0 || model.var.is_nan() || model.var < 0.0 {
                    return Err(ctx.parse_err(
                        "a BJT model needs VAF >= 0 and VAR >= 0 (zero disables Early effect)"
                            .to_string(),
                    ));
                }
                if model.ikf.is_nan() || model.ikf <= 0.0 || model.ikr.is_nan() || model.ikr <= 0.0
                {
                    return Err(ctx.parse_err(
                        "a BJT model needs IKF > 0 and IKR > 0 (infinity disables high-injection rolloff)"
                            .to_string(),
                    ));
                }
                if model.nkf.is_some_and(|nkf| !nkf.is_finite() || nkf < 0.0) {
                    return Err(ctx.parse_err(
                        "a BJT model needs NKF >= 0 when the high-current beta exponent is supplied"
                            .to_string(),
                    ));
                }
                if model.ise < 0.0
                    || model.isc < 0.0
                    || model.ne <= 0.0
                    || model.nc <= 0.0
                    || !model.ne.is_finite()
                    || !model.nc.is_finite()
                {
                    return Err(ctx.parse_err(
                        "a BJT model needs ISE >= 0, ISC >= 0, NE > 0 and NC > 0".to_string(),
                    ));
                }
                if !model.cje.is_finite()
                    || model.cje < 0.0
                    || !model.vje.is_finite()
                    || model.vje <= 0.0
                    || !model.mje.is_finite()
                    || model.mje < 0.0
                    || !model.cjc.is_finite()
                    || model.cjc < 0.0
                    || !model.xcjc.is_finite()
                    || !(0.0..=1.0).contains(&model.xcjc)
                    || !model.vjc.is_finite()
                    || model.vjc <= 0.0
                    || !model.mjc.is_finite()
                    || model.mjc < 0.0
                    || !model.fc.is_finite()
                    || model.fc < 0.0
                    || model.fc >= 1.0
                    || !model.tf.is_finite()
                    || model.tf < 0.0
                    || !model.xtf.is_finite()
                    || model.xtf < 0.0
                    || model.vtf.is_nan()
                    || model.vtf < 0.0
                    || !model.itf.is_finite()
                    || model.itf < 0.0
                    || !model.tr.is_finite()
                    || model.tr < 0.0
                {
                    return Err(ctx.parse_err(
                        "a BJT model needs CJE/CJC/TF/TR/XTF/ITF >= 0, 0 <= XCJC <= 1, VJE/VJC > 0, MJE/MJC >= 0, VTF >= 0 and 0 <= FC < 1"
                            .to_string(),
                    ));
                }
                if !model.rb.is_finite()
                    || model.rb < 0.0
                    || model
                        .rbm
                        .is_some_and(|rbm| !rbm.is_finite() || rbm < 0.0 || rbm > model.rb)
                    || model.irb.is_nan()
                    || model.irb <= 0.0
                    || !model.rc.is_finite()
                    || model.rc < 0.0
                    || !model.re.is_finite()
                    || model.re < 0.0
                {
                    return Err(ctx.parse_err(
                        "a BJT model needs RB/RC/RE >= 0, 0 <= RBM <= RB and IRB > 0".to_string(),
                    ));
                }
                if model.rco.is_some_and(|rco| !rco.is_finite())
                    || !model.vo.is_finite()
                    || model.vo <= 0.0
                    || !model.gamma.is_finite()
                    || model.gamma < 0.0
                    || !model.qco.is_finite()
                    || model.qco < 0.0
                    || !model.vg.is_finite()
                    || model.cn.is_some_and(|cn| !cn.is_finite())
                    || model.d.is_some_and(|d| !d.is_finite())
                {
                    return Err(ctx.parse_err(
                        "a BJT quasi-saturation model needs finite RCO/VG/CN/D, VO > 0, GAMMA >= 0 and QCO >= 0"
                            .to_string(),
                    ));
                }
                // ngspice `bjtsetup.c` raises a supplied `RCO` below 0.01 Ω.
                let model = BjtModel {
                    rco: model.rco.map(|rco| rco.max(0.01)),
                    ..model
                };
                // `TLEV=3` raises the card's own `IS`/`ISE`/`ISC`/`ISS` to a
                // power before `AREA` multiplies them (ngspice `bjttemp.c`).
                // Every other temperature law is a pure factor, so it
                // commutes with the area scaling and keeps the old order.
                let (operating_model, thermal_voltage) = if model.tlev == 3
                    && device.temperature_celsius.is_some()
                {
                    let (unscaled, thermal_voltage) = super::device::bjt_parameters_at_temperature(
                        BjtModel {
                            rco: model.rco,
                            ..unscaled_model
                        },
                        device.temperature_celsius,
                    );
                    (unscaled.with_area(device.area), thermal_voltage)
                } else {
                    super::device::bjt_parameters_at_temperature(model, device.temperature_celsius)
                };
                if !operating_model.is.is_finite()
                    || operating_model.is <= 0.0
                    || !operating_model.bf.is_finite()
                    || operating_model.bf <= 0.0
                    || !operating_model.br.is_finite()
                    || operating_model.br <= 0.0
                    || !operating_model.ise.is_finite()
                    || operating_model.ise < 0.0
                    || !operating_model.isc.is_finite()
                    || operating_model.isc < 0.0
                    || !operating_model.tf.is_finite()
                    || operating_model.tf < 0.0
                    || !operating_model.tr.is_finite()
                    || operating_model.tr < 0.0
                    || !operating_model.rb.is_finite()
                    || operating_model.rb < 0.0
                    || (model.rb > 0.0 && operating_model.rb <= 0.0)
                    || operating_model.rbm.is_some_and(|rbm| {
                        !rbm.is_finite() || rbm < 0.0 || rbm > operating_model.rb
                    })
                    || operating_model.irb.is_nan()
                    || operating_model.irb <= 0.0
                    || (model.irb.is_finite() && !operating_model.irb.is_finite())
                    || !operating_model.rc.is_finite()
                    || operating_model.rc < 0.0
                    || (model.rc > 0.0 && operating_model.rc <= 0.0)
                    || !operating_model.re.is_finite()
                    || operating_model.re < 0.0
                    || (model.re > 0.0 && operating_model.re <= 0.0)
                    || !operating_model.cje.is_finite()
                    || operating_model.cje < 0.0
                    || !operating_model.vje.is_finite()
                    || operating_model.vje <= 0.0
                    || !operating_model.mje.is_finite()
                    || operating_model.mje < 0.0
                    || !operating_model.cjc.is_finite()
                    || operating_model.cjc < 0.0
                    || !operating_model.vjc.is_finite()
                    || operating_model.vjc <= 0.0
                    || !operating_model.mjc.is_finite()
                    || operating_model.mjc < 0.0
                    || !thermal_voltage.is_finite()
                    || thermal_voltage <= 0.0
                    || operating_model
                        .rco
                        .is_some_and(|rco| !rco.is_finite() || rco <= 0.0)
                    || !operating_model.vo.is_finite()
                    || operating_model.vo <= 0.0
                    || !operating_model.gamma.is_finite()
                    || operating_model.gamma < 0.0
                    || !operating_model.nf.is_finite()
                    || operating_model.nf <= 0.0
                    || !operating_model.nr.is_finite()
                    || operating_model.nr <= 0.0
                    || !operating_model.ne.is_finite()
                    || operating_model.ne <= 0.0
                    || !operating_model.nc.is_finite()
                    || operating_model.nc <= 0.0
                    || operating_model.ikf.is_nan()
                    || operating_model.ikf <= 0.0
                    || operating_model.ikr.is_nan()
                    || operating_model.ikr <= 0.0
                    || operating_model.vaf.is_nan()
                    || operating_model.vaf < 0.0
                    || operating_model.var.is_nan()
                    || operating_model.var < 0.0
                    || !operating_model.itf.is_finite()
                    || operating_model.itf < 0.0
                {
                    return Err(ctx.parse_err(
                        "temperature-adjusted BJT currents, beta, resistances and junction charges must remain finite and valid"
                            .to_string(),
                    ));
                }
                let junction_node = |circuit: &mut Circuit,
                                     terminal: NodeRef,
                                     resistance: f64,
                                     suffix: &str|
                 -> Result<NodeRef, AnalogError> {
                    if resistance == 0.0 {
                        return Ok(terminal);
                    }
                    let internal_name = format!("{}#{suffix}", device.name);
                    if circuit.node(&internal_name).is_some() {
                        return Err(ctx.parse_err(format!(
                            "internal BJT node `{internal_name}` is already used by the netlist"
                        )));
                    }
                    Ok(circuit.intern(&internal_name))
                };
                // ngspice `bjtsetup.c`: `RC` runs from the collector to the
                // contact node; with `RCO` the epitaxial region then runs
                // from that contact to a separate intrinsic collector.
                let (junction_cx, junction_c) = if operating_model.rco.is_some() {
                    let contact =
                        junction_node(circuit, device.nodes[0], operating_model.rc, "collcx")?;
                    let intrinsic = junction_node(circuit, contact, 1.0, "collector")?;
                    (contact, intrinsic)
                } else {
                    let contact =
                        junction_node(circuit, device.nodes[0], operating_model.rc, "collector")?;
                    (contact, contact)
                };
                let junction_b =
                    junction_node(circuit, device.nodes[1], operating_model.rb, "base")?;
                let junction_e =
                    junction_node(circuit, device.nodes[2], operating_model.re, "emitter")?;
                let substrate = device.nodes.get(3).copied().flatten();
                // ngspice's substrate junction (`bjtload.c`) is a pn junction
                // from the substrate to the internal collector (vertical) or
                // base (lateral), oriented by `type·SUBS`: `ISS`/`NS` current
                // plus `CJS`/`VJS`/`MJS` depletion charge with no `FC`. That
                // is exactly a diode with `FC = 0`, so it is lowered onto the
                // diode path rather than given a second copy of the
                // junction solver. A three-terminal BJT gets it too, with the
                // substrate at ground: ngspice ties the substrate node to
                // ground there, so with `ISS = CJS = 0` the junction is
                // GMIN alone, a 1 pS leak from the collector (vertical) or
                // base (lateral) that a high-impedance node can see.
                if !(model.cjs.is_finite()
                    && model.cjs >= 0.0
                    && model.vjs.is_finite()
                    && model.vjs > 0.0
                    && model.mjs.is_finite()
                    && model.mjs >= 0.0
                    && model.iss.is_finite()
                    && model.iss >= 0.0
                    && model.ns.is_finite()
                    && model.ns > 0.0
                    && operating_model.cjs.is_finite()
                    && operating_model.cjs >= 0.0
                    && operating_model.vjs.is_finite()
                    && operating_model.vjs > 0.0
                    && operating_model.ns.is_finite()
                    && operating_model.ns > 0.0
                    && operating_model.iss.is_finite())
                {
                    return Err(ctx.parse_err(
                        "a BJT substrate junction needs CJS >= 0, VJS > 0, MJS >= 0, ISS >= 0 and NS > 0, all finite at the instance temperature"
                            .to_string(),
                    ));
                }
                let internal_name = format!("{}#substrate", device.name);
                let contact = if model.substrate_is_vertical() {
                    junction_c
                } else {
                    junction_b
                };
                let polarity = if model.substrate_is_vertical() {
                    model.polarity.sign()
                } else {
                    -model.polarity.sign()
                };
                let (anode, cathode) = if polarity > 0.0 {
                    (substrate, contact)
                } else {
                    (contact, substrate)
                };
                // The card is already at the instance temperature, so its
                // own `TNOM` is set there with `EG = XTI = 0`: the diode
                // laws then leave `IS` untouched and only pick up `Vt(T)`.
                let junction = DiodeModel {
                    is: operating_model.iss,
                    n: operating_model.ns,
                    cjo: operating_model.cjs,
                    vj: operating_model.vjs,
                    m: operating_model.mjs.min(0.999),
                    fc: 0.0,
                    eg: 0.0,
                    xti: 0.0,
                    tnom_celsius: device.temperature_celsius.unwrap_or(27.0),
                    ..DiodeModel::default()
                };
                circuit.diodes.push(Diode {
                    name: internal_name,
                    multiplier: device.multiplier,
                    off: device.off,
                    ic: None,
                    anode,
                    cathode,
                    junction_anode: anode,
                    model_name: device.model_name.clone(),
                    model: junction,
                    operating_model: junction,
                    series_resistance_ohms: 0.0,
                    temperature_celsius: device.temperature_celsius,
                    breakdown: None,
                });
                circuit.bjts.push(Bjt {
                    name: device.name,
                    multiplier: device.multiplier,
                    off: device.off,
                    ic: (device.ic.len() == 2).then(|| [device.ic[0], device.ic[1]]),
                    c: device.nodes[0],
                    b: device.nodes[1],
                    e: device.nodes[2],
                    s: substrate,
                    junction_c,
                    junction_cx,
                    junction_b,
                    junction_e,
                    model_name: device.model_name,
                    model,
                    operating_model,
                    thermal_voltage,
                    temperature_celsius: device.temperature_celsius,
                });
            }
            'M' => {
                let ModelCard::Mos(mut model) = card else {
                    return Err(wrong_kind("M"));
                };
                // ngspice level 1 (mos1temp.c): with `TOX` and no `KP`, the
                // transconductance comes from the process, `UO·1e-4·εox/TOX`.
                if !model.kp_given {
                    if let Some(tox) = model.tox {
                        if !model.uo.is_finite() || model.uo <= 0.0 {
                            return Err(ctx.parse_err(
                                "a MOSFET model deriving KP from TOX needs UO > 0 and finite",
                            ));
                        }
                        if tox.is_finite() && tox > 0.0 {
                            model.kp =
                                model.uo * 1e-4 * super::device::OXIDE_PERMITTIVITY_F_PER_M / tox;
                        }
                    }
                }
                // ngspice level 1 (mos1temp.c): with `TOX` and `NSUB`, every
                // one of `PHI`, `GAMMA` and `VTO` the card omits comes from
                // the substrate doping, evaluated at `TNOM`.
                if let (Some(nsub), Some(tox)) = (model.nsub, model.tox) {
                    if tox.is_finite() && tox > 0.0 {
                        super::device::mos_process_threshold(&mut model, tox, nsub)
                            .map_err(|message| ctx.parse_err(message))?;
                    }
                }
                if model.kp <= 0.0 || model.is <= 0.0 || model.gamma < 0.0 {
                    return Err(ctx.parse_err(
                        "a MOSFET model needs KP > 0, IS > 0 and GAMMA >= 0".to_string(),
                    ));
                }
                if model.gamma > 0.0 && model.phi <= 0.0 {
                    return Err(
                        ctx.parse_err("a MOSFET model with GAMMA > 0 needs PHI > 0".to_string())
                    );
                }
                let w = device.width.unwrap_or(model.w);
                let l = device.length.unwrap_or(model.l);
                if w <= 0.0 || l <= 0.0 {
                    return Err(ctx.parse_err("a MOSFET model needs W > 0 and L > 0".to_string()));
                }
                let effective_length = l - 2.0 * model.ld;
                if !model.ld.is_finite() || effective_length.is_nan() || effective_length <= 0.0 {
                    return Err(ctx.parse_err(
                        "a MOSFET's effective channel length L - 2*LD must be positive",
                    ));
                }
                if ![model.cgso, model.cgdo, model.cgbo]
                    .into_iter()
                    .all(|capacitance| capacitance.is_finite() && capacitance >= 0.0)
                {
                    return Err(ctx.parse_err(
                        "a MOSFET model needs CGSO/CGDO/CGBO >= 0 and finite".to_string(),
                    ));
                }
                if !model.cbd.is_finite()
                    || model.cbd < 0.0
                    || !model.cbs.is_finite()
                    || model.cbs < 0.0
                    || !model.cj.is_finite()
                    || model.cj < 0.0
                    || !model.cjsw.is_finite()
                    || model.cjsw < 0.0
                    || !model.pb.is_finite()
                    || model.pb <= 0.0
                    || !model.mj.is_finite()
                    || model.mj < 0.0
                    || !model.mjsw.is_finite()
                    || model.mjsw < 0.0
                    || !model.fc.is_finite()
                    || model.fc < 0.0
                    || model.fc >= 1.0
                {
                    return Err(ctx.parse_err(
                        "a MOSFET model needs CBD/CBS/CJ/CJSW >= 0, PB > 0, MJ/MJSW >= 0 and 0 <= FC < 1",
                    ));
                }
                let ad = device.ad.unwrap_or(0.0);
                let as_ = device.as_.unwrap_or(0.0);
                let pd = device.pd.unwrap_or(0.0);
                let ps = device.ps.unwrap_or(0.0);
                if !model.js.is_finite()
                    || model.js < 0.0
                    || !(model.js * ad).is_finite()
                    || !(model.js * as_).is_finite()
                {
                    return Err(
                        ctx.parse_err("a MOSFET model needs JS >= 0 with finite JS*AD and JS*AS")
                    );
                }
                let junction_capacitances = [
                    if model.cbd_given {
                        model.cbd
                    } else {
                        model.cj * ad
                    },
                    model.cjsw * pd,
                    if model.cbs_given {
                        model.cbs
                    } else {
                        model.cj * as_
                    },
                    model.cjsw * ps,
                    (if model.cbd_given {
                        model.cbd
                    } else {
                        model.cj * ad
                    }) + model.cjsw * pd,
                    (if model.cbs_given {
                        model.cbs
                    } else {
                        model.cj * as_
                    }) + model.cjsw * ps,
                ];
                if junction_capacitances
                    .into_iter()
                    .any(|capacitance| !capacitance.is_finite())
                {
                    return Err(ctx.parse_err(
                        "a MOSFET's effective junction capacitances from CJ/CJSW and AD/AS/PD/PS must be finite",
                    ));
                }
                if !model.rd.is_finite()
                    || model.rd < 0.0
                    || !model.rs.is_finite()
                    || model.rs < 0.0
                    || !model.rsh.is_finite()
                    || model.rsh < 0.0
                {
                    return Err(ctx.parse_err("a MOSFET model needs RD/RS/RSH >= 0 and finite"));
                }
                if !(model.cgso * w).is_finite()
                    || !(model.cgdo * w).is_finite()
                    || !(model.cgbo * effective_length).is_finite()
                {
                    return Err(ctx.parse_err(
                        "a MOSFET model's effective overlap capacitances must be finite"
                            .to_string(),
                    ));
                }
                if let Some(tox) = model.tox {
                    if !tox.is_finite() || tox <= 0.0 {
                        return Err(ctx.parse_err("a MOSFET model needs TOX > 0 and finite"));
                    }
                    let cox =
                        super::device::OXIDE_PERMITTIVITY_F_PER_M * w * effective_length / tox;
                    if !cox.is_finite() {
                        return Err(ctx.parse_err(
                            "a MOSFET model's effective oxide capacitance must be finite",
                        ));
                    }
                }
                if model.channel_charge_model == MosChannelChargeModel::WardDutton
                    && model.tox.is_none()
                {
                    return Err(ctx.parse_err(
                        "MOSFET `CHARGEMOD=WARD_DUTTON` needs `TOX` to define oxide charge",
                    ));
                }
                if !model.tnom_celsius.is_finite() || model.tnom_celsius <= -273.15 {
                    return Err(ctx.parse_err(
                        "a MOSFET model needs a finite TNOM above absolute zero (-273.15 °C)",
                    ));
                }
                if device.temperature_celsius.is_some() && model.phi <= 0.0 {
                    return Err(ctx.parse_err(
                        "a MOSFET instance with TEMP needs PHI > 0 for the surface-potential law",
                    ));
                }
                let operating_model =
                    super::device::mos_parameters_at_temperature(model, device.temperature_celsius);
                if !operating_model.vto.is_finite()
                    || !operating_model.kp.is_finite()
                    || operating_model.kp <= 0.0
                    || !(operating_model.kp * w / effective_length).is_finite()
                    || !operating_model.is.is_finite()
                    || operating_model.is <= 0.0
                    || !operating_model.phi.is_finite()
                    || (device.temperature_celsius.is_some() && operating_model.phi <= 0.0)
                    || !operating_model.pb.is_finite()
                    || operating_model.pb <= 0.0
                    || [
                        operating_model.cbd,
                        operating_model.cbs,
                        operating_model.cj,
                        operating_model.cjsw,
                    ]
                    .into_iter()
                    .any(|capacitance| !capacitance.is_finite() || capacitance < 0.0)
                {
                    return Err(ctx.parse_err(
                        "temperature-adjusted MOSFET parameters must be finite with KP, IS, PB > 0, \
                         PHI > 0 and junction capacitances >= 0",
                    ));
                }
                let junction_node = |circuit: &mut Circuit,
                                     terminal: NodeRef,
                                     resistance: f64,
                                     suffix: &str|
                 -> Result<NodeRef, AnalogError> {
                    if resistance == 0.0 {
                        return Ok(terminal);
                    }
                    let internal_name = format!("{}#{suffix}", device.name);
                    if circuit.node(&internal_name).is_some() {
                        return Err(ctx.parse_err(format!(
                            "internal MOSFET node `{internal_name}` is already used by the netlist"
                        )));
                    }
                    Ok(circuit.intern(&internal_name))
                };
                // SPICE accepts the model's direct resistance or its sheet
                // resistance times the instance square count. An explicitly
                // supplied direct value takes precedence for that terminal,
                // including zero, which disables its series resistor.
                let nrd = device.nrd.unwrap_or(1.0);
                let nrs = device.nrs.unwrap_or(1.0);
                let drain_resistance_ohms = if model.rd_given {
                    model.rd
                } else {
                    model.rsh * nrd
                };
                let source_resistance_ohms = if model.rs_given {
                    model.rs
                } else {
                    model.rsh * nrs
                };
                if !drain_resistance_ohms.is_finite()
                    || !source_resistance_ohms.is_finite()
                    || (drain_resistance_ohms > 0.0 && !(1.0 / drain_resistance_ohms).is_finite())
                    || (source_resistance_ohms > 0.0 && !(1.0 / source_resistance_ohms).is_finite())
                {
                    return Err(ctx.parse_err(
                        "a MOSFET's effective RD/RS from RSH and NRD/NRS must be finite",
                    ));
                }
                let junction_d =
                    junction_node(circuit, device.nodes[0], drain_resistance_ohms, "drain")?;
                let junction_s =
                    junction_node(circuit, device.nodes[2], source_resistance_ohms, "source")?;
                circuit.mosfets.push(Mosfet {
                    name: device.name,
                    multiplier: device.multiplier,
                    off: device.off,
                    ic: core::array::from_fn(|index| device.ic.get(index).copied()),
                    d: device.nodes[0],
                    g: device.nodes[1],
                    s: device.nodes[2],
                    junction_d,
                    junction_s,
                    bulk: device.nodes[3],
                    model_name: device.model_name,
                    model,
                    operating_model,
                    temperature_celsius: device.temperature_celsius,
                    beta: operating_model.kp * w / effective_length,
                    w,
                    l,
                    ad,
                    as_,
                    pd,
                    ps,
                    drain_resistance_ohms,
                    source_resistance_ohms,
                });
            }
            'J' => {
                let ModelCard::Jfet(model) = card else {
                    return Err(wrong_kind("J"));
                };
                let model = model.with_area(device.area);
                if !model.vto.is_finite()
                    || !model.beta.is_finite()
                    || model.beta <= 0.0
                    || !model.is.is_finite()
                    || model.is <= 0.0
                    || !model.n.is_finite()
                    || model.n <= 0.0
                    || !model.tnom_celsius.is_finite()
                    || model.tnom_celsius <= -273.15
                    || !model.tcv.is_finite()
                    || !model.bex.is_finite()
                    || !model.eg.is_finite()
                    || !model.xti.is_finite()
                    || model.vtotc.is_some_and(|value| !value.is_finite())
                    || model.betatce.is_some_and(|value| !value.is_finite())
                {
                    return Err(ctx.parse_err(
                        "a JFET model needs finite temperature parameters, TNOM above absolute zero, BETA > 0, IS > 0 and N > 0",
                    ));
                }
                if !model.rd.is_finite()
                    || model.rd < 0.0
                    || !model.rs.is_finite()
                    || model.rs < 0.0
                {
                    return Err(ctx.parse_err("a JFET model needs RD/RS >= 0 and finite"));
                }
                if !model.b.is_finite() || model.b <= 0.0 {
                    return Err(ctx.parse_err("a JFET model needs B > 0 and finite"));
                }
                if !model.cgs.is_finite()
                    || model.cgs < 0.0
                    || !model.cgd.is_finite()
                    || model.cgd < 0.0
                    || !model.pb.is_finite()
                    || model.pb <= 0.0
                    || !model.fc.is_finite()
                    || model.fc < 0.0
                    || model.fc >= 1.0
                {
                    return Err(
                        ctx.parse_err("a JFET model needs CGS/CGD >= 0, PB > 0 and 0 <= FC < 1")
                    );
                }
                let operating_model = super::device::jfet_parameters_at_temperature(
                    model,
                    device.temperature_celsius,
                );
                if !operating_model.vto.is_finite()
                    || !operating_model.beta.is_finite()
                    || operating_model.beta <= 0.0
                    || !operating_model.is.is_finite()
                    || operating_model.is <= 0.0
                    || !operating_model.pb.is_finite()
                    || operating_model.pb <= 0.0
                    || !operating_model.cgs.is_finite()
                    || operating_model.cgs < 0.0
                    || !operating_model.cgd.is_finite()
                    || operating_model.cgd < 0.0
                {
                    return Err(ctx.parse_err(
                        "temperature-adjusted JFET parameters must be finite with BETA, IS, PB > 0 \
                         and CGS/CGD >= 0",
                    ));
                }
                let junction_node = |circuit: &mut Circuit,
                                     terminal: NodeRef,
                                     resistance: f64,
                                     suffix: &str|
                 -> Result<NodeRef, AnalogError> {
                    if resistance == 0.0 {
                        return Ok(terminal);
                    }
                    let internal_name = format!("{}#{suffix}", device.name);
                    if circuit.node(&internal_name).is_some() {
                        return Err(ctx.parse_err(format!(
                            "internal JFET node `{internal_name}` is already used by the netlist"
                        )));
                    }
                    Ok(circuit.intern(&internal_name))
                };
                let junction_d = junction_node(circuit, device.nodes[0], model.rd, "drain")?;
                let junction_s = junction_node(circuit, device.nodes[2], model.rs, "source")?;
                circuit.jfets.push(Jfet {
                    name: device.name,
                    multiplier: device.multiplier,
                    off: device.off,
                    ic: (device.ic.len() == 2).then(|| [device.ic[0], device.ic[1]]),
                    d: device.nodes[0],
                    g: device.nodes[1],
                    s: device.nodes[2],
                    junction_d,
                    junction_s,
                    model_name: device.model_name,
                    model,
                    operating_model,
                    temperature_celsius: device.temperature_celsius,
                });
            }
            _ => unreachable!("device letter filtered above"),
        }
    }
    Ok(())
}

/// Parse the value part of a `V`/`I` line: `[dc] <value>`, `SIN(...)`,
/// `SFFM(...)`, `EXP(...)`, `PULSE(...)`, or `PWL(...)`.
///
/// SPICE allows the argument list to be written with or without commas and
/// with the parenthesis detached from the keyword (`SIN (0 5 1k)`), so the
/// tokens are re-joined and split on the punctuation rather than trusted to
/// arrive one per argument.
fn parse_source(ctx: &LineCtx<'_>, tokens: &[&str], shape: &str) -> Result<Waveform, AnalogError> {
    let joined = tokens.join(" ");
    let upper = joined.trim().to_ascii_uppercase();

    if upper.starts_with("PWL") {
        let raw = joined.trim();
        let rest = raw[3..].trim_start();
        if rest.starts_with('(') {
            let close = rest
                .find(')')
                .ok_or_else(|| ctx.parse_err("`PWL(` is missing its closing `)`"))?;
            let body = &rest[1..close];
            let options = &rest[close + 1..];
            let values = body
                .split([',', ' ', '\t'])
                .filter(|token| !token.is_empty())
                .map(|token| ctx.value(token, "PWL time or value"))
                .collect::<Result<Vec<_>, _>>()?;
            if values.len() < 4 || values.len() % 2 != 0 {
                return Err(ctx.parse_err(
                    "expected `PWL(t1 v1 t2 v2 [tn vn ...])` with at least two points",
                ));
            }
            let points: Vec<(f64, f64)> = values
                .chunks_exact(2)
                .map(|pair| (pair[0], pair[1]))
                .collect();
            if points.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
                return Err(ctx.parse_err("PWL time points must be strictly increasing"));
            }

            let mut repeat_time = None;
            let mut delay = 0.0;
            let mut has_delay = false;
            for option in options
                .split([',', ' ', '\t'])
                .filter(|token| !token.is_empty())
            {
                let Some((key, raw_value)) = option.split_once('=') else {
                    return Err(ctx.parse_err("PWL options use `r=repeat_time` or `td=delay`"));
                };
                let value = ctx.value(raw_value, "PWL option")?;
                match key.to_ascii_lowercase().as_str() {
                    "r" if repeat_time.is_none() => repeat_time = Some(value),
                    "r" => return Err(ctx.parse_err("PWL repeat option `r=` is repeated")),
                    "td" if !has_delay => {
                        delay = value;
                        has_delay = true;
                    }
                    "td" => return Err(ctx.parse_err("PWL delay option `td=` is repeated")),
                    other => {
                        return Err(ctx.parse_err(format!(
                            "unsupported PWL option `{other}`; use `r=` or `td=`"
                        )))
                    }
                }
            }
            if delay < 0.0 {
                return Err(ctx.parse_err("PWL delay must not be negative"));
            }
            if let Some(repeat_time) = repeat_time {
                if !points.iter().any(|(time, _)| *time == repeat_time)
                    || repeat_time >= points.last().expect("at least two points").0
                {
                    return Err(
                        ctx.parse_err("PWL `r=` must name a time point before the final point")
                    );
                }
            }
            return Ok(Waveform::Pwl {
                points,
                repeat_time,
                delay,
            });
        }
    }

    for keyword in ["SIN", "SFFM", "EXP", "PULSE"] {
        let Some(rest) = upper.strip_prefix(keyword) else {
            continue;
        };
        let rest = rest.trim_start();
        if !rest.starts_with('(') {
            continue;
        }
        let body = joined.trim()[joined.trim().len() - rest.len()..].trim();
        let body = body
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
            .ok_or_else(|| ctx.parse_err(format!("`{keyword}(` is missing its closing `)`")))?;
        let args: Vec<f64> = body
            .split([',', ' ', '\t'])
            .filter(|token| !token.is_empty())
            .map(|token| ctx.value(token, "source parameter"))
            .collect::<Result<_, _>>()?;
        let arg = |index: usize, fallback: f64| args.get(index).copied().unwrap_or(fallback);

        if keyword == "SIN" {
            if args.len() < 3 || args.len() > 5 {
                return Err(ctx.parse_err("expected `SIN(vo va freq [td [theta]])`"));
            }
            return Ok(Waveform::Sin {
                offset: args[0],
                amplitude: args[1],
                frequency: args[2],
                delay: arg(3, 0.0),
                theta: arg(4, 0.0),
            });
        }
        if keyword == "SFFM" {
            if args.len() != 5 {
                return Err(ctx.parse_err("expected `SFFM(vo va fc mdi fs)`"));
            }
            return Ok(Waveform::Sffm {
                offset: args[0],
                amplitude: args[1],
                carrier_frequency: args[2],
                modulation_index: args[3],
                signal_frequency: args[4],
            });
        }
        if keyword == "EXP" {
            if args.len() != 6 {
                return Err(ctx.parse_err("expected `EXP(v1 v2 td1 tau1 td2 tau2)`"));
            }
            let [td1, tau1, td2, tau2] = [args[2], args[3], args[4], args[5]];
            if td1 < 0.0 || td2 < 0.0 {
                return Err(ctx.parse_err("EXP delay times TD1 and TD2 must not be negative"));
            }
            if tau1 <= 0.0 || tau2 <= 0.0 {
                return Err(ctx.parse_err("EXP time constants TAU1 and TAU2 must be positive"));
            }
            if td2 < td1 {
                return Err(ctx.parse_err("EXP fall delay TD2 must be at or after rise delay TD1"));
            }
            return Ok(Waveform::Exp {
                v1: args[0],
                v2: args[1],
                td1,
                tau1,
                td2,
                tau2,
            });
        }
        if args.len() < 2 || args.len() > 7 {
            return Err(ctx.parse_err("expected `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])`"));
        }
        let period = arg(6, 0.0);
        let rise = arg(3, 0.0);
        let fall = arg(4, 0.0);
        let width = arg(5, period);
        if rise < 0.0 || fall < 0.0 || width < 0.0 || period < 0.0 {
            return Err(ctx.parse_err("PULSE times must not be negative"));
        }
        if period > 0.0 && rise + width + fall > period {
            return Err(ctx.parse_err(
                "PULSE tr + pw + tf is longer than per, so the pulse never returns to v1"
                    .to_string(),
            ));
        }
        return Ok(Waveform::Pulse {
            v1: args[0],
            v2: args[1],
            delay: arg(2, 0.0),
            rise,
            fall,
            width,
            period,
        });
    }

    let raw = match tokens.len() {
        1 => tokens[0],
        2 if tokens[0].eq_ignore_ascii_case("dc") => tokens[1],
        _ => {
            return Err(ctx.parse_err(format!(
                "expected `{shape}`, where `<source>` is `[dc] <value>`, \
                 `SIN(vo va freq [td [theta]])`, `SFFM(vo va fc mdi fs)`, \
                 `EXP(v1 v2 td1 tau1 td2 tau2)`, \
                 `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])`, or \
                 `PWL(t1 v1 ... tn vn) [r=repeat_time] [td=delay]`"
            )))
        }
    };
    Ok(Waveform::Dc(ctx.value(raw, "source value")?))
}

/// Parse `.model <name> <type>(<param>=<value> ...)`.
///
/// Unknown parameters are accepted and dropped on purpose: a vendor model card
/// carries a dozen charge and temperature parameters this engine has no term
/// for, and refusing the card would mean the user has to hand-edit every
/// datasheet model to run it. The docs say what survives; [`super::device`]
/// says what it costs.
fn parse_model(
    models: &mut BTreeMap<String, ModelCard>,
    ctx: &LineCtx<'_>,
    tokens: &[&str],
    default_tnom_celsius: f64,
) -> Result<(), AnalogError> {
    if tokens.len() < 2 {
        return Err(ctx.parse_err("expected `.model <name> <type>(<param>=<value> ...)`"));
    }
    let name = tokens[0];
    let key = name.to_ascii_lowercase();
    if models.contains_key(&key) {
        return Err(ctx.parse_err(format!("`.model {name}` is declared twice")));
    }

    // `NPN(BF=100)`, `NPN (BF=100)` and `NPN BF=100` all mean the same thing.
    let rest = tokens[1..].join(" ");
    let (kind, body) = match rest.find('(') {
        Some(at) => (
            rest[..at].trim().to_string(),
            rest[at + 1..]
                .trim_end()
                .strip_suffix(')')
                .ok_or_else(|| ctx.parse_err("`.model` is missing its closing `)`"))?
                .to_string(),
        ),
        None => {
            let mut parts = rest.splitn(2, char::is_whitespace);
            (
                parts.next().unwrap_or("").trim().to_string(),
                parts.next().unwrap_or("").to_string(),
            )
        }
    };

    let mut card = match kind.to_ascii_uppercase().as_str() {
        "D" => ModelCard::Diode(DiodeModel::default()),
        "NPN" => ModelCard::Bjt(BjtModel::defaults(Polarity::N)),
        "PNP" => ModelCard::Bjt(BjtModel::defaults(Polarity::P)),
        "NMOS" => ModelCard::Mos(MosModel::defaults(Polarity::N)),
        "PMOS" => ModelCard::Mos(MosModel::defaults(Polarity::P)),
        "NJF" => ModelCard::Jfet(JfetModel::defaults(Polarity::N)),
        "PJF" => ModelCard::Jfet(JfetModel::defaults(Polarity::P)),
        "OPAMP" => ModelCard::OpAmp(OpAmpModel::default()),
        "COMP" => ModelCard::Comparator(ComparatorModel::default()),
        "LDO" => ModelCard::Regulator(RegulatorModel::defaults(RegulatorKind::Linear)),
        "LDOADJ" => ModelCard::Regulator(RegulatorModel::defaults(RegulatorKind::LinearAdjustable)),
        "BUCK" => ModelCard::Regulator(RegulatorModel::defaults(RegulatorKind::Buck)),
        "SW" => ModelCard::Switch(SwitchModel::defaults(false)),
        "CSW" => ModelCard::Switch(SwitchModel::defaults(true)),
        other => {
            return Err(ctx.parse_err(format!(
                "`.model` type `{other}` is not one of `D`, `NPN`, `PNP`, `NMOS`, `PMOS`, `NJF`, `PJF`, \
                 `SW`, `CSW`, `OPAMP`, `COMP`, `LDO`, `LDOADJ`, `BUCK`"
            )))
        }
    };
    // `.options tnom=` is the card's `TNOM` until the card sets its own.
    match &mut card {
        ModelCard::Diode(model) => model.tnom_celsius = default_tnom_celsius,
        ModelCard::Bjt(model) => model.tnom_celsius = default_tnom_celsius,
        ModelCard::Mos(model) => model.tnom_celsius = default_tnom_celsius,
        ModelCard::Jfet(model) => model.tnom_celsius = default_tnom_celsius,
        _ => {}
    }

    for item in body.split([',', ' ', '\t']).filter(|part| !part.is_empty()) {
        let (param, raw) = item
            .split_once('=')
            .ok_or_else(|| ctx.parse_err(format!("`{item}` is not a `<param>=<value>` pair")))?;
        // The one parameter that is a word, not a number.
        if let ModelCard::Comparator(model) = &mut card {
            if param.eq_ignore_ascii_case("OUTPUT") {
                model.output = match raw.to_ascii_uppercase().as_str() {
                    "OC" | "OD" => ComparatorOutput::OpenCollector,
                    "PP" => ComparatorOutput::PushPull,
                    other => {
                        return Err(ctx.parse_err(format!(
                            "comparator `OUTPUT={other}` is not `OC` (open collector), \
                             `OD` (open drain) or `PP` (push-pull)"
                        )))
                    }
                };
                continue;
            }
        }
        if let ModelCard::Mos(model) = &mut card {
            if param.eq_ignore_ascii_case("CHARGEMOD") {
                model.channel_charge_model = match raw.to_ascii_uppercase().as_str() {
                    "MEYER" => MosChannelChargeModel::Meyer,
                    "WARD_DUTTON" | "WARD-DUTTON" => MosChannelChargeModel::WardDutton,
                    other => {
                        return Err(ctx.parse_err(format!(
                            "MOSFET `CHARGEMOD={other}` is not `MEYER` or `WARD_DUTTON`"
                        )))
                    }
                };
                continue;
            }
        }
        let value = ctx.value(raw, param)?;
        let param = param.to_ascii_uppercase();
        match (&mut card, param.as_str()) {
            (ModelCard::Diode(model), "IS") => model.is = value,
            (ModelCard::Diode(model), "N") => model.n = value,
            (ModelCard::Diode(model), "EG") => model.eg = value,
            (ModelCard::Diode(model), "XTI") => model.xti = value,
            (ModelCard::Diode(model), "TNOM") => model.tnom_celsius = value,
            (ModelCard::Diode(model), "TCV" | "TBV1") => model.tcv = value,
            (ModelCard::Diode(model), "TRS" | "TRS1") => model.trs1 = value,
            (ModelCard::Diode(model), "TRS2") => model.trs2 = value,
            (ModelCard::Diode(model), "IK" | "IKF") => model.ik = value,
            (ModelCard::Diode(model), "IKR") => model.ikr = value,
            (ModelCard::Diode(model), "ISR") => model.isr = value,
            (ModelCard::Diode(model), "NR") => model.nr = value,
            (ModelCard::Diode(model), "RS") => model.rs = value,
            (ModelCard::Diode(model), "TT") => model.tt = value,
            (ModelCard::Diode(model), "TTT1") => model.ttt1 = value,
            (ModelCard::Diode(model), "TTT2") => model.ttt2 = value,
            (ModelCard::Diode(model), "TM1") => model.tm1 = value,
            (ModelCard::Diode(model), "TM2") => model.tm2 = value,
            (ModelCard::Diode(model), "VP") => model.vp = value,
            (ModelCard::Diode(model), "BV") => model.bv = Some(value),
            (ModelCard::Diode(model), "IBV") => model.ibv = value,
            (ModelCard::Diode(model), "NBV") => model.nbv = Some(value),
            (ModelCard::Diode(model), "CJO") => model.cjo = value,
            (ModelCard::Diode(model), "VJ") => model.vj = value,
            (ModelCard::Diode(model), "M") => model.m = value,
            (ModelCard::Diode(model), "FC") => model.fc = value,
            (ModelCard::Diode(model), "JSW" | "ISW") => model.jsw = value,
            (ModelCard::Diode(model), "NS") => model.ns = Some(value),
            (ModelCard::Diode(model), "PJ") => model.pj = value,
            (ModelCard::Diode(model), "CJSW" | "CJP") => model.cjsw = value,
            (ModelCard::Diode(model), "PHP") => model.php = value,
            (ModelCard::Diode(model), "MJSW") => model.mjsw = value,
            (ModelCard::Diode(model), "FCS") => model.fcs = value,
            (ModelCard::Diode(model), "TLEVC") => {
                if value != 0.0 && value != 1.0 {
                    return Err(ctx.parse_err("diode TLEVC must be 0 or 1".to_string()));
                }
                model.tlevc = value as u8;
            }
            (ModelCard::Diode(model), "CTA" | "CTC") => model.cta = value,
            (ModelCard::Diode(model), "CTP") => model.ctp = value,
            (ModelCard::Diode(model), "TPB" | "TVJ") => model.tpb = value,
            (ModelCard::Diode(model), "TPHP") => model.tphp = value,
            (ModelCard::Diode(model), "JTUN") => model.jtun = value,
            (ModelCard::Diode(model), "JTUNSW") => model.jtunsw = value,
            (ModelCard::Diode(model), "NTUN") => model.ntun = value,
            (ModelCard::Diode(model), "XTITUN") => model.xtitun = value,
            (ModelCard::Diode(model), "KEG") => model.keg = value,
            (ModelCard::OpAmp(model), "AOL") => model.aol = value,
            (ModelCard::OpAmp(model), "GBW") => model.gbw = value,
            (ModelCard::OpAmp(model), "ROUT") => model.rout = value,
            (ModelCard::OpAmp(model), "VOS") => model.vos = value,
            (ModelCard::OpAmp(model), "IBIAS") => model.ibias = value,
            (ModelCard::OpAmp(model), "IOS") => model.ios = value,
            (ModelCard::OpAmp(model), "CINCM") => model.cin_cm = value,
            (ModelCard::OpAmp(model), "CIND") => model.cin_diff = value,
            (ModelCard::OpAmp(model), "RIN") => model.rin = value,
            (ModelCard::OpAmp(model), "RINCM") => model.rin_cm = value,
            (ModelCard::OpAmp(model), "IQ") => model.iq = value,
            (ModelCard::OpAmp(model), "CMRR") => model.cmrr = Some(value),
            (ModelCard::OpAmp(model), "IOLIM") => model.ilim = Some(value),
            (ModelCard::OpAmp(model), "SR") => model.sr = Some(value),
            (ModelCard::OpAmp(model), "DROP_HI") => model.drop_hi = value,
            (ModelCard::OpAmp(model), "DROP_LO") => model.drop_lo = value,
            (ModelCard::Comparator(model), "AOL") => model.aol = value,
            (ModelCard::Comparator(model), "GBW") => model.gbw = value,
            (ModelCard::Comparator(model), "ROUT") => model.rout = value,
            (ModelCard::Comparator(model), "VOS") => model.vos = value,
            (ModelCard::Comparator(model), "IBIAS") => model.ibias = value,
            (ModelCard::Comparator(model), "IOS") => model.ios = value,
            (ModelCard::Comparator(model), "CINCM") => model.cin_cm = value,
            (ModelCard::Comparator(model), "CIND") => model.cin_diff = value,
            (ModelCard::Comparator(model), "RIN") => model.rin = value,
            (ModelCard::Comparator(model), "RINCM") => model.rin_cm = value,
            (ModelCard::Comparator(model), "IQ") => model.iq = value,
            (ModelCard::Comparator(model), "CMRR") => model.cmrr = Some(value),
            (ModelCard::Comparator(model), "VHYS") => model.vhys = value,
            // This engine's own card types have no vendor parameters to
            // tolerate, so a misspelt one is an error rather than a silently
            // ideal op-amp.
            (ModelCard::OpAmp(_), other) => {
                return Err(ctx.parse_err(format!(
                    "`{other}` is not an OPAMP parameter; expected AOL, GBW, ROUT, VOS, \
                     IBIAS, IOS, CINCM, CIND, RIN, RINCM, IQ, CMRR, IOLIM, SR, DROP_HI, DROP_LO"
                )))
            }
            (ModelCard::Regulator(model), param) => {
                let kind = model.kind;
                let allowed: &[&str] = match kind {
                    RegulatorKind::Linear => &["VOUT", "VDO", "ILIM", "IQ", "ROUT", "LINE"],
                    RegulatorKind::LinearAdjustable => {
                        &["VREF", "VDO", "ILIM", "IADJ", "ROUT", "LINE"]
                    }
                    RegulatorKind::Buck => &[
                        "VOUT", "VREF", "VDO", "DMAX", "ILIM", "IQ", "ROUT", "EFF", "AOL",
                    ],
                };
                if !allowed.contains(&param) {
                    return Err(ctx.parse_err(format!(
                        "`{param}` is not a {} parameter; expected {}",
                        ModelCard::Regulator(*model).kind(),
                        allowed.join(", ")
                    )));
                }
                match param {
                    "VOUT" => model.vout = value,
                    "VREF" => model.vref = value,
                    "VDO" => model.vdo = value,
                    "DMAX" => model.dmax = value,
                    "ILIM" => model.ilim = value,
                    "IQ" | "IADJ" => model.iq = value,
                    "ROUT" => model.rout = value,
                    "LINE" => model.line = value,
                    "EFF" => model.eff = value,
                    "AOL" => model.aol = value,
                    _ => unreachable!("checked against `allowed`"),
                }
            }
            (ModelCard::Comparator(_), other) => {
                return Err(ctx.parse_err(format!(
                    "`{other}` is not a COMP parameter; expected AOL, GBW, ROUT, VOS, IBIAS, IOS, CINCM, CIND, RIN, RINCM, IQ, CMRR, VHYS, \
                     OUTPUT"
                )))
            }
            (ModelCard::Switch(model), "RON") => model.ron = value,
            (ModelCard::Switch(model), "ROFF") => model.roff = value,
            (ModelCard::Switch(model), "VT") if !model.current => model.threshold = value,
            (ModelCard::Switch(model), "VH") if !model.current => model.hysteresis = value,
            (ModelCard::Switch(model), "IT") if model.current => model.threshold = value,
            (ModelCard::Switch(model), "IH") if model.current => model.hysteresis = value,
            // A switch card has four parameters; a fifth is another
            // simulator's extension (LTspice `LSER`, `VSER`, `ILIMIT`) that
            // would change the answer if it were dropped.
            (ModelCard::Switch(model), other) => {
                let (kind, expected) = if model.current {
                    ("CSW", "IT, IH, RON, ROFF")
                } else {
                    ("SW", "VT, VH, RON, ROFF")
                };
                return Err(ctx.parse_err(format!(
                    "`{other}` is not a {kind} parameter; expected {expected}"
                )));
            }
            (ModelCard::Bjt(model), "IS") => model.is = value,
            (ModelCard::Bjt(model), "IBE") => model.ibe = Some(value),
            (ModelCard::Bjt(model), "IBC") => model.ibc = Some(value),
            (ModelCard::Bjt(model), "BF") => model.bf = value,
            (ModelCard::Bjt(model), "BR") => model.br = value,
            (ModelCard::Bjt(model), "NF") => model.nf = value,
            (ModelCard::Bjt(model), "NR") => model.nr = value,
            (ModelCard::Bjt(model), "VAF" | "VA") => model.vaf = value,
            (ModelCard::Bjt(model), "VAR" | "VB") => model.var = value,
            (ModelCard::Bjt(model), "IKF" | "IK") => model.ikf = value,
            (ModelCard::Bjt(model), "IKR") => model.ikr = value,
            (ModelCard::Bjt(model), "NKF" | "NK") => model.nkf = Some(value),
            (ModelCard::Bjt(model), "ISE" | "C2") => model.ise = value,
            (ModelCard::Bjt(model), "NE") => model.ne = value,
            (ModelCard::Bjt(model), "ISC" | "C4") => model.isc = value,
            (ModelCard::Bjt(model), "NC") => model.nc = value,
            (ModelCard::Bjt(model), "TNOM" | "TREF") => model.tnom_celsius = value,
            (ModelCard::Bjt(model), "EG") => model.eg = value,
            (ModelCard::Bjt(model), "XTI") => model.xti = value,
            (ModelCard::Bjt(model), "XTB") => model.xtb = value,
            (ModelCard::Bjt(model), "CJE") => model.cje = value,
            (ModelCard::Bjt(model), "VJE" | "PE") => model.vje = value,
            (ModelCard::Bjt(model), "MJE" | "ME") => model.mje = value,
            (ModelCard::Bjt(model), "TMJE1") => model.tmje1 = value,
            (ModelCard::Bjt(model), "TMJE2") => model.tmje2 = value,
            (ModelCard::Bjt(model), "CJC") => model.cjc = value,
            (ModelCard::Bjt(model), "XCJC") => model.xcjc = value,
            (ModelCard::Bjt(model), "VJC" | "PC") => model.vjc = value,
            (ModelCard::Bjt(model), "MJC" | "MC") => model.mjc = value,
            (ModelCard::Bjt(model), "TMJC1") => model.tmjc1 = value,
            (ModelCard::Bjt(model), "TMJC2") => model.tmjc2 = value,
            (ModelCard::Bjt(model), "TLEVC") => {
                if value != 0.0 && value != 1.0 {
                    return Err(ctx.parse_err("BJT TLEVC must be 0 or 1".to_string()));
                }
                model.tlevc = value as u8;
            }
            (ModelCard::Bjt(model), "CTE") => model.cte = value,
            (ModelCard::Bjt(model), "CTC") => model.ctc = value,
            (ModelCard::Bjt(model), "TVJE") => model.tvje = value,
            (ModelCard::Bjt(model), "TVJC") => model.tvjc = value,
            (ModelCard::Bjt(model), "FC") => model.fc = value,
            (ModelCard::Bjt(model), "TF") => model.tf = value,
            (ModelCard::Bjt(model), "TTF1") => model.ttf1 = value,
            (ModelCard::Bjt(model), "TTF2") => model.ttf2 = value,
            (ModelCard::Bjt(model), "XTF") => model.xtf = value,
            (ModelCard::Bjt(model), "VTF") => model.vtf = value,
            (ModelCard::Bjt(model), "ITF") => model.itf = value,
            (ModelCard::Bjt(model), "PTF") => model.ptf = value,
            (ModelCard::Bjt(model), "TR") => model.tr = value,
            (ModelCard::Bjt(model), "TTR1") => model.ttr1 = value,
            (ModelCard::Bjt(model), "TTR2") => model.ttr2 = value,
            (ModelCard::Bjt(model), "RB") => model.rb = value,
            (ModelCard::Bjt(model), "TRB1" | "TRB") => model.trb1 = value,
            (ModelCard::Bjt(model), "TRB2") => model.trb2 = value,
            (ModelCard::Bjt(model), "RBM") => model.rbm = Some(value),
            (ModelCard::Bjt(model), "TRBM1" | "TRM1") => model.trbm1 = value,
            (ModelCard::Bjt(model), "TRBM2" | "TRM2") => model.trbm2 = value,
            (ModelCard::Bjt(model), "IRB") => model.irb = value,
            (ModelCard::Bjt(model), "TIRB1") => model.tirb1 = value,
            (ModelCard::Bjt(model), "TIRB2") => model.tirb2 = value,
            (ModelCard::Bjt(model), "RC") => model.rc = value,
            (ModelCard::Bjt(model), "TRC1" | "TRC") => model.trc1 = value,
            (ModelCard::Bjt(model), "TRC2") => model.trc2 = value,
            (ModelCard::Bjt(model), "RE") => model.re = value,
            (ModelCard::Bjt(model), "TRE1" | "TRE") => model.tre1 = value,
            (ModelCard::Bjt(model), "TRE2") => model.tre2 = value,
            (ModelCard::Bjt(model), "CJS" | "CCS" | "CSUB") => model.cjs = value,
            (ModelCard::Bjt(model), "VJS" | "PS") => model.vjs = value,
            (ModelCard::Bjt(model), "MJS" | "MS") => model.mjs = value,
            (ModelCard::Bjt(model), "TMJS1") => model.tmjs1 = value,
            (ModelCard::Bjt(model), "TMJS2") => model.tmjs2 = value,
            (ModelCard::Bjt(model), "CTS") => model.cts = value,
            (ModelCard::Bjt(model), "TVJS") => model.tvjs = value,
            (ModelCard::Bjt(model), "ISS") => model.iss = value,
            (ModelCard::Bjt(model), "NS") => model.ns = value,
            (ModelCard::Bjt(model), "TNS1") => model.tns1 = value,
            (ModelCard::Bjt(model), "TNS2") => model.tns2 = value,
            (ModelCard::Bjt(model), "TLEV") => {
                if ![0.0, 1.0, 3.0].contains(&value) {
                    return Err(ctx.parse_err("BJT TLEV must be 0, 1 or 3".to_string()));
                }
                model.tlev = value as u8;
            }
            (ModelCard::Bjt(model), "TBF1") => {
                model.tbf1 = value;
                model.tbf_given = true;
            }
            (ModelCard::Bjt(model), "TBF2") => {
                model.tbf2 = value;
                model.tbf_given = true;
            }
            (ModelCard::Bjt(model), "TBR1") => {
                model.tbr1 = value;
                model.tbr_given = true;
            }
            (ModelCard::Bjt(model), "TBR2") => {
                model.tbr2 = value;
                model.tbr_given = true;
            }
            (ModelCard::Bjt(model), "TIKF1") => model.tikf1 = value,
            (ModelCard::Bjt(model), "TIKF2") => model.tikf2 = value,
            (ModelCard::Bjt(model), "TIKR1") => model.tikr1 = value,
            (ModelCard::Bjt(model), "TIKR2") => model.tikr2 = value,
            (ModelCard::Bjt(model), "TNF1") => model.tnf1 = value,
            (ModelCard::Bjt(model), "TNF2") => model.tnf2 = value,
            (ModelCard::Bjt(model), "TNR1") => model.tnr1 = value,
            (ModelCard::Bjt(model), "TNR2") => model.tnr2 = value,
            (ModelCard::Bjt(model), "TNE1") => model.tne1 = value,
            (ModelCard::Bjt(model), "TNE2") => model.tne2 = value,
            (ModelCard::Bjt(model), "TNC1") => model.tnc1 = value,
            (ModelCard::Bjt(model), "TNC2") => model.tnc2 = value,
            (ModelCard::Bjt(model), "TVAF1") => model.tvaf1 = value,
            (ModelCard::Bjt(model), "TVAF2") => model.tvaf2 = value,
            (ModelCard::Bjt(model), "TVAR1") => model.tvar1 = value,
            (ModelCard::Bjt(model), "TVAR2") => model.tvar2 = value,
            (ModelCard::Bjt(model), "TITF1") => model.titf1 = value,
            (ModelCard::Bjt(model), "TITF2") => model.titf2 = value,
            (ModelCard::Bjt(model), "TIS1") => model.tis1 = value,
            (ModelCard::Bjt(model), "TIS2") => model.tis2 = value,
            (ModelCard::Bjt(model), "TISE1") => model.tise1 = value,
            (ModelCard::Bjt(model), "TISE2") => model.tise2 = value,
            (ModelCard::Bjt(model), "TISC1") => model.tisc1 = value,
            (ModelCard::Bjt(model), "TISC2") => model.tisc2 = value,
            (ModelCard::Bjt(model), "TISS1") => model.tiss1 = value,
            (ModelCard::Bjt(model), "TISS2") => model.tiss2 = value,
            (ModelCard::Bjt(model), "SUBS") => {
                if value != 1.0 && value != -1.0 {
                    return Err(ctx.parse_err("BJT SUBS must be 1 (vertical) or -1 (lateral)"));
                }
                model.subs = Some(value as i8);
            }
            (ModelCard::Bjt(model), "RCO") => model.rco = Some(value),
            (ModelCard::Bjt(model), "VO") => model.vo = value,
            (ModelCard::Bjt(model), "GAMMA") => model.gamma = value,
            (ModelCard::Bjt(model), "QCO") => model.qco = value,
            (ModelCard::Bjt(model), "QUASIMOD") => {
                if value != 0.0 && value != 1.0 {
                    return Err(ctx.parse_err("BJT QUASIMOD must be 0 or 1"));
                }
                model.quasimod = value as u8;
            }
            (ModelCard::Bjt(model), "VG") => model.vg = value,
            (ModelCard::Bjt(model), "CN") => model.cn = Some(value),
            (ModelCard::Bjt(model), "D") => model.d = Some(value),
            (ModelCard::Mos(model), "VTO" | "VT0") => {
                model.vto = value;
                model.vto_given = true;
            }
            (ModelCard::Mos(model), "KP") => {
                model.kp = value;
                model.kp_given = true;
            }
            (ModelCard::Mos(model), "UO" | "U0") => model.uo = value,
            (ModelCard::Mos(model), "LD") => model.ld = value,
            (ModelCard::Mos(model), "LAMBDA") => model.lambda = value,
            (ModelCard::Mos(model), "RD") => {
                model.rd = value;
                model.rd_given = true;
            }
            (ModelCard::Mos(model), "RS") => {
                model.rs = value;
                model.rs_given = true;
            }
            (ModelCard::Mos(model), "RSH") => model.rsh = value,
            (ModelCard::Mos(model), "GAMMA") => {
                model.gamma = value;
                model.gamma_given = true;
            }
            (ModelCard::Mos(model), "PHI") => {
                model.phi = value;
                model.phi_given = true;
            }
            (ModelCard::Mos(model), "NSUB") => model.nsub = Some(value),
            (ModelCard::Mos(model), "NSS") => model.nss = value,
            (ModelCard::Mos(model), "TPG") => {
                if ![-1.0, 0.0, 1.0].contains(&value) {
                    return Err(ctx.parse_err("MOSFET TPG must be -1, 0 or 1".to_string()));
                }
                model.tpg = value;
            }
            (ModelCard::Mos(model), "IS") => model.is = value,
            (ModelCard::Mos(model), "JS") => model.js = value,
            (ModelCard::Mos(model), "CBD") => {
                model.cbd = value;
                model.cbd_given = true;
            }
            (ModelCard::Mos(model), "CBS") => {
                model.cbs = value;
                model.cbs_given = true;
            }
            (ModelCard::Mos(model), "CJ") => model.cj = value,
            (ModelCard::Mos(model), "CJSW") => model.cjsw = value,
            (ModelCard::Mos(model), "PB") => model.pb = value,
            (ModelCard::Mos(model), "MJ") => model.mj = value,
            (ModelCard::Mos(model), "MJSW") => model.mjsw = value,
            (ModelCard::Mos(model), "FC") => model.fc = value,
            (ModelCard::Mos(model), "W") => model.w = value,
            (ModelCard::Mos(model), "L") => model.l = value,
            (ModelCard::Mos(model), "CGSO") => model.cgso = value,
            (ModelCard::Mos(model), "CGDO") => model.cgdo = value,
            (ModelCard::Mos(model), "CGBO") => model.cgbo = value,
            (ModelCard::Mos(model), "TOX") => model.tox = Some(value),
            (ModelCard::Mos(model), "TNOM") => model.tnom_celsius = value,
            // A level this engine cannot honour is the one parameter worth
            // refusing: silently solving a BSIM card with Shichman-Hodges
            // would be wrong by orders of magnitude, not by a capacitance.
            (ModelCard::Mos(_), "LEVEL") if value != 1.0 => {
                return Err(ctx.parse_err(format!(
                    "MOSFET `LEVEL={value}` is not modelled in-core; only level 1 \
                     (Shichman-Hodges) is. Use `adapter: external_process` with \
                     `tools/cosim/labwired_ngspice.py`"
                )))
            }
            (ModelCard::Jfet(model), "VTO" | "VT0") => model.vto = value,
            (ModelCard::Jfet(model), "BETA") => model.beta = value,
            (ModelCard::Jfet(model), "LAMBDA") => model.lambda = value,
            (ModelCard::Jfet(model), "B") => model.b = value,
            (ModelCard::Jfet(model), "RD") => model.rd = value,
            (ModelCard::Jfet(model), "RS") => model.rs = value,
            (ModelCard::Jfet(model), "IS") => model.is = value,
            (ModelCard::Jfet(model), "N") => model.n = value,
            (ModelCard::Jfet(model), "CGS") => model.cgs = value,
            (ModelCard::Jfet(model), "CGD") => model.cgd = value,
            (ModelCard::Jfet(model), "PB") => model.pb = value,
            (ModelCard::Jfet(model), "FC") => model.fc = value,
            (ModelCard::Jfet(model), "TNOM") => model.tnom_celsius = value,
            (ModelCard::Jfet(model), "VTOTC") => model.vtotc = Some(value),
            (ModelCard::Jfet(model), "TCV") => model.tcv = value,
            (ModelCard::Jfet(model), "BETATCE") => model.betatce = Some(value),
            (ModelCard::Jfet(model), "BEX") => model.bex = value,
            (ModelCard::Jfet(model), "EG") => model.eg = value,
            (ModelCard::Jfet(model), "XTI") => model.xti = value,
            (ModelCard::Jfet(_), "LEVEL") if value != 1.0 => {
                return Err(ctx.parse_err(format!(
                    "JFET `LEVEL={value}` is not modelled in-core; only level 1 is. Use \
                     `adapter: external_process` with `tools/cosim/labwired_ngspice.py`"
                )))
            }
            _ => {}
        }
    }

    if let ModelCard::Switch(model) = &card {
        if model.ron <= 0.0 || model.roff <= 0.0 {
            return Err(ctx.parse_err("RON and ROFF must be positive"));
        }
        if model.hysteresis < 0.0 {
            // ngspice gives a negative value a second, time-dependent
            // meaning; it is not the band described here.
            return Err(
                ctx.parse_err("negative switch hysteresis is not modelled; use `VH`/`IH` >= 0")
            );
        }
    }
    models.insert(key, card);
    Ok(())
}

// ---------------------------------------------------------------------------
// Op-amp and comparator lowering
// ---------------------------------------------------------------------------

/// Resistance of the op-amp's and comparator's internal gain nodes, ohms.
///
/// Any value gives the same transfer function (the transconductance and the
/// capacitor scale with it); 1 MΩ keeps a saturated stage's clamp current in
/// the tenths of an amp rather than the kiloamps a 1 Ω node would push through
/// its clamp diode, which is what keeps the clamp within millivolts of the rail.
pub const MACRO_GAIN_RESISTANCE: f64 = 1e6;

/// The internal clamp diode of the op-amp and comparator macros.
///
/// `N = 0.02` puts its thermal voltage at 0.52 mV: a knee 50× sharper than a
/// silicon junction, so a clamped output sits within ~15 mV of its rail at
/// half an amp of clamp current. It is an ordinary diode to the solver —
/// limited by `pnjlim`, stamped like any other — which is the point: the
/// saturation nonlinearity is one Newton already knows how to converge.
pub const MACRO_CLAMP_DIODE: DiodeModel = DiodeModel {
    is: 1e-12,
    n: 0.02,
    ik: 0.0,
    ikr: 0.0,
    isr: 0.0,
    nr: 2.0,
    rs: 0.0,
    tt: 0.0,
    ttt1: 0.0,
    ttt2: 0.0,
    tm1: 0.0,
    tm2: 0.0,
    vp: 0.0,
    bv: None,
    ibv: 1e-3,
    nbv: None,
    cjo: 0.0,
    vj: 1.0,
    m: 0.5,
    fc: 0.5,
    eg: 1.11,
    xti: 3.0,
    tnom_celsius: 27.0,
    tcv: 0.0,
    trs1: 0.0,
    trs2: 0.0,
    jsw: 0.0,
    ns: None,
    pj: 0.0,
    cjsw: 0.0,
    php: 1.0,
    mjsw: 0.33,
    fcs: 0.5,
    tlevc: 0,
    cta: 0.0,
    ctp: 0.0,
    tpb: 0.0,
    tphp: 0.0,
    jtun: 0.0,
    jtunsw: 0.0,
    ntun: 30.0,
    xtitun: 3.0,
    keg: 1.0,
};

/// Loop gain of the push-pull comparator's output node, V/V from the 1 V
/// decision node. Large enough that the output is always hard against a rail
/// except for the few nanoseconds the decision node spends crossing.
pub const COMPARATOR_PP_GAIN: f64 = 1e4;

/// Builds the primitives one `X` line lowers into, named `<X name>#<part>`.
struct Lowering<'c> {
    circuit: &'c mut Circuit,
    prefix: String,
}

impl Lowering<'_> {
    fn node(&mut self, part: &str) -> NodeRef {
        let name = format!("{}#{part}", self.prefix);
        self.circuit.intern(&name)
    }

    fn name(&self, part: &str) -> String {
        format!("{}#{part}", self.prefix)
    }

    fn resistor(&mut self, part: &str, a: NodeRef, b: NodeRef, ohms: f64) {
        let name = self.name(part);
        self.circuit.resistors.push(Resistor {
            name,
            a,
            b,
            ohms,
            ntc: None,
            temperature_coefficients: None,
        });
    }

    fn capacitor(&mut self, part: &str, a: NodeRef, b: NodeRef, farads: f64) {
        let name = self.name(part);
        self.circuit.capacitors.push(Capacitor {
            name,
            a,
            b,
            farads,
            ic: None,
        });
    }

    fn output_current_limit(
        &mut self,
        part: &str,
        drive: NodeRef,
        output: NodeRef,
        rout: f64,
        ilim: f64,
    ) {
        let name = self.name(part);
        self.circuit.output_current_limits.push(OutputCurrentLimit {
            name,
            drive,
            output,
            rout,
            ilim,
        });
    }

    fn vccs(&mut self, part: &str, p: NodeRef, n: NodeRef, cp: NodeRef, cn: NodeRef, gm: f64) {
        let name = self.name(part);
        self.circuit.vccs.push(Vccs {
            name,
            p,
            n,
            cp,
            cn,
            gm,
        });
    }

    fn current(&mut self, part: &str, p: NodeRef, n: NodeRef, amps: f64) {
        let name = self.name(part);
        self.circuit.current_sources.push(CurrentSource {
            name,
            p,
            n,
            dc: amps,
            wave: Waveform::Dc(amps),
        });
    }

    fn voltage(&mut self, part: &str, p: NodeRef, n: NodeRef, volts: f64) {
        let name = self.name(part);
        self.circuit.voltage_sources.push(VoltageSource {
            name,
            p,
            n,
            dc: volts,
            wave: Waveform::Dc(volts),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn vcvs(&mut self, part: &str, p: NodeRef, n: NodeRef, cp: NodeRef, cn: NodeRef, gain: f64) {
        let name = self.name(part);
        self.circuit.vcvs.push(Vcvs {
            name,
            p,
            n,
            cp,
            cn,
            gain,
        });
    }

    fn clamp(&mut self, part: &str, anode: NodeRef, cathode: NodeRef) {
        let name = self.name(part);
        self.circuit.diodes.push(Diode {
            name,
            multiplier: 1.0,
            off: false,
            ic: None,
            anode,
            cathode,
            junction_anode: anode,
            model_name: "clamp".to_string(),
            model: MACRO_CLAMP_DIODE,
            operating_model: MACRO_CLAMP_DIODE,
            series_resistance_ohms: 0.0,
            temperature_celsius: None,
            breakdown: None,
        });
    }

    /// A rail offset by `drop` volts toward the inside of the supply, or the
    /// rail itself when `drop` is zero (no extra unknowns).
    fn offset_rail(&mut self, part: &str, rail: NodeRef, drop: f64, below: bool) -> NodeRef {
        if drop == 0.0 {
            return rail;
        }
        let node = self.node(part);
        let source = format!("v{part}");
        if below {
            // v(rail) − v(node) = drop
            self.voltage(&source, rail, node, drop);
        } else {
            // v(node) − v(rail) = drop
            self.voltage(&source, node, rail, drop);
        }
        node
    }
}

fn macro_pins(
    ctx: &LineCtx<'_>,
    device: &PendingDevice,
    kind: &str,
) -> Result<[NodeRef; 5], AnalogError> {
    <[NodeRef; 5]>::try_from(device.nodes.as_slice()).map_err(|_| {
        ctx.parse_err(format!(
            "`{}` is an {kind} with {} pins; expected `X<name> in+ in- v+ v- out {}`",
            device.name,
            device.nodes.len(),
            device.model_name
        ))
    })
}

fn check_macro(ctx: &LineCtx<'_>, aol: f64, gbw: f64, rout: f64) -> Result<(), AnalogError> {
    if !(aol > 1.0 && gbw > 0.0 && rout > 0.0) {
        return Err(ctx.parse_err("an OPAMP/COMP model needs AOL > 1, GBW > 0 and ROUT > 0"));
    }
    Ok(())
}

/// Split a macro's average input bias current and signed input offset current
/// into the currents drawn by its non-inverting and inverting pins.
fn input_bias_currents(ctx: &LineCtx<'_>, ibias: f64, ios: f64) -> Result<(f64, f64), AnalogError> {
    if !ibias.is_finite() {
        return Err(ctx.parse_err("IBIAS must be finite"));
    }
    if !ios.is_finite() {
        return Err(ctx.parse_err("IOS must be finite"));
    }
    let half_offset = 0.5 * ios;
    let input_plus = ibias + half_offset;
    let input_minus = ibias - half_offset;
    if !input_plus.is_finite() || !input_minus.is_finite() {
        return Err(ctx.parse_err("IBIAS and IOS produce a non-finite input current"));
    }
    Ok((input_plus, input_minus))
}

/// Check the fixed common-mode and differential capacitances on a macro input.
fn check_input_capacitances(
    ctx: &LineCtx<'_>,
    cin_cm: f64,
    cin_diff: f64,
) -> Result<(), AnalogError> {
    if !cin_cm.is_finite() || cin_cm < 0.0 {
        return Err(ctx.parse_err("CINCM must be finite and non-negative"));
    }
    if !cin_diff.is_finite() || cin_diff < 0.0 {
        return Err(ctx.parse_err("CIND must be finite and non-negative"));
    }
    Ok(())
}

/// Check the fixed differential resistance between macro inputs.
fn check_input_resistance(ctx: &LineCtx<'_>, rin: f64) -> Result<(), AnalogError> {
    if !rin.is_finite() || rin < 0.0 {
        return Err(ctx.parse_err("RIN must be finite and non-negative"));
    }
    Ok(())
}

/// Check the fixed common-mode input resistance on a macro.
fn check_common_mode_input_resistance(ctx: &LineCtx<'_>, rin_cm: f64) -> Result<(), AnalogError> {
    if !rin_cm.is_finite() || rin_cm < 0.0 {
        return Err(ctx.parse_err("RINCM must be finite and non-negative"));
    }
    Ok(())
}

/// Convert an optional macro CMRR in dB into common-mode-to-differential gain.
fn check_common_mode_rejection(
    ctx: &LineCtx<'_>,
    cmrr_db: Option<f64>,
) -> Result<f64, AnalogError> {
    let Some(cmrr_db) = cmrr_db else {
        return Ok(0.0);
    };
    if !cmrr_db.is_finite() || cmrr_db < 0.0 {
        return Err(ctx.parse_err("CMRR must be finite and non-negative"));
    }
    Ok(libm::pow(10.0, -cmrr_db / 20.0))
}

/// Quiescent supply current flows only from `v+` to `v−` in these macro cards.
fn check_quiescent_current(ctx: &LineCtx<'_>, iq: f64) -> Result<(), AnalogError> {
    if !iq.is_finite() || iq < 0.0 {
        return Err(ctx.parse_err("IQ must be finite and non-negative"));
    }
    Ok(())
}

/// Lower the macro's fixed input capacitances as ordinary MNA capacitors.
fn lower_input_capacitances(
    lower: &mut Lowering<'_>,
    inp: NodeRef,
    inn: NodeRef,
    vee: NodeRef,
    cin_cm: f64,
    cin_diff: f64,
) {
    if cin_cm > 0.0 {
        lower.capacitor("cinp", inp, vee, cin_cm);
        lower.capacitor("cinm", inn, vee, cin_cm);
    }
    if cin_diff > 0.0 {
        lower.capacitor("cind", inp, inn, cin_diff);
    }
}

/// Lower the macro's differential input resistance as an ordinary MNA resistor.
fn lower_input_resistance(lower: &mut Lowering<'_>, inp: NodeRef, inn: NodeRef, rin: f64) {
    if rin > 0.0 {
        lower.resistor("rin", inp, inn, rin);
    }
}

/// Lower two equal common-mode input resistors, one from each input to `v−`.
fn lower_common_mode_input_resistance(
    lower: &mut Lowering<'_>,
    inp: NodeRef,
    inn: NodeRef,
    vee: NodeRef,
    rin_cm: f64,
) {
    if rin_cm > 0.0 {
        lower.resistor("rincmp", inp, vee, rin_cm);
        lower.resistor("rincmm", inn, vee, rin_cm);
    }
}

/// Lower an op-amp into the MNA circuit.
fn lower_opamp(
    circuit: &mut Circuit,
    ctx: &LineCtx<'_>,
    device: &PendingDevice,
    model: OpAmpModel,
) -> Result<(), AnalogError> {
    let [inp, inn, vcc, vee, out] = macro_pins(ctx, device, "op-amp")?;
    check_macro(ctx, model.aol, model.gbw, model.rout)?;
    if model.drop_hi < 0.0 || model.drop_lo < 0.0 {
        return Err(ctx.parse_err("DROP_HI and DROP_LO must not be negative"));
    }
    check_input_capacitances(ctx, model.cin_cm, model.cin_diff)?;
    check_input_resistance(ctx, model.rin)?;
    check_common_mode_input_resistance(ctx, model.rin_cm)?;
    check_quiescent_current(ctx, model.iq)?;
    if let Some(ilim) = model.ilim {
        if !ilim.is_finite() || ilim <= 0.0 {
            return Err(ctx.parse_err("IOLIM must be finite and positive"));
        }
    }
    let cm_gain = check_common_mode_rejection(ctx, model.cmrr)?;
    let (ibias_plus, ibias_minus) = input_bias_currents(ctx, model.ibias, model.ios)?;
    let rg = MACRO_GAIN_RESISTANCE;
    let gm = model.aol / rg;
    let cg = model.aol / (2.0 * core::f64::consts::PI * model.gbw * rg);
    let slew_current_limit = if let Some(sr) = model.sr {
        if !sr.is_finite() || sr <= 0.0 {
            return Err(ctx.parse_err("SR must be finite and positive"));
        }
        let ilim = cg * sr;
        if !ilim.is_finite() || ilim <= 0.0 {
            return Err(ctx.parse_err("SR and macro parameters produce an invalid slew current"));
        }
        Some(ilim)
    } else {
        None
    };

    let mut lower = Lowering {
        circuit,
        prefix: device.name.clone(),
    };
    lower_input_capacitances(&mut lower, inp, inn, vee, model.cin_cm, model.cin_diff);
    lower_input_resistance(&mut lower, inp, inn, model.rin);
    lower_common_mode_input_resistance(&mut lower, inp, inn, vee, model.rin_cm);
    if model.iq > 0.0 {
        lower.current("iq", vcc, vee, model.iq);
    }
    let p = lower.node("p");
    if let Some(ilim) = slew_current_limit {
        let source = SlewLimitedVccs {
            name: lower.name("g"),
            p: None,
            n: p,
            input_plus: inp,
            input_minus: inn,
            vcc,
            vee,
            gm,
            cm_gain,
            vos: model.vos,
            ilim,
        };
        lower.circuit.slew_limited_vccs.push(source);
    } else {
        // gm·(v(in+) − v(in−)) flows from ground through the source into p.
        lower.vccs("g", None, p, inp, inn, gm);
        if cm_gain > 0.0 {
            // The input-referred error is the common-mode voltage relative to
            // the midpoint of the supply rails, attenuated by 10^(−CMRR/20).
            let gcm = 0.5 * gm * cm_gain;
            lower.vccs("gcmp", None, p, inp, vee, gcm);
            lower.vccs("gcmm", None, p, inn, vcc, gcm);
        }
        if model.vos != 0.0 {
            lower.current("ios", None, p, gm * model.vos);
        }
    }
    lower.resistor("rg", p, None, rg);
    lower.capacitor("cg", p, None, cg);
    if ibias_plus != 0.0 {
        lower.current("ibp", inp, vee, ibias_plus);
    }
    if ibias_minus != 0.0 {
        lower.current("ibn", inn, vee, ibias_minus);
    }
    let hi = lower.offset_rail("hi", vcc, model.drop_hi, true);
    let lo = lower.offset_rail("lo", vee, model.drop_lo, false);
    lower.clamp("dhi", p, hi);
    lower.clamp("dlo", lo, p);
    if let Some(ilim) = model.ilim {
        lower.output_current_limit("go", p, out, model.rout, ilim);
    } else {
        // Thevenin v(p) behind ROUT, as a Norton pair: no extra unknown.
        lower.vccs("go", None, out, p, None, 1.0 / model.rout);
        lower.resistor("ro", out, None, model.rout);
    }
    Ok(())
}

/// Lower a comparator `X` line; see [`ComparatorModel`] for the circuit.
fn lower_comparator(
    circuit: &mut Circuit,
    ctx: &LineCtx<'_>,
    device: &PendingDevice,
    model: ComparatorModel,
) -> Result<(), AnalogError> {
    let [inp, inn, vcc, vee, out] = macro_pins(ctx, device, "comparator")?;
    check_macro(ctx, model.aol, model.gbw, model.rout)?;
    if model.vhys < 0.0 {
        return Err(ctx.parse_err("VHYS must not be negative"));
    }
    check_input_capacitances(ctx, model.cin_cm, model.cin_diff)?;
    check_input_resistance(ctx, model.rin)?;
    check_common_mode_input_resistance(ctx, model.rin_cm)?;
    check_quiescent_current(ctx, model.iq)?;
    let cm_gain = check_common_mode_rejection(ctx, model.cmrr)?;
    let (ibias_plus, ibias_minus) = input_bias_currents(ctx, model.ibias, model.ios)?;
    let rq = MACRO_GAIN_RESISTANCE;
    let gm = model.aol / rq;
    let cq = model.aol / (2.0 * core::f64::consts::PI * model.gbw * rq);

    let mut lower = Lowering {
        circuit,
        prefix: device.name.clone(),
    };
    lower_input_capacitances(&mut lower, inp, inn, vee, model.cin_cm, model.cin_diff);
    lower_input_resistance(&mut lower, inp, inn, model.rin);
    lower_common_mode_input_resistance(&mut lower, inp, inn, vee, model.rin_cm);
    if model.iq > 0.0 {
        lower.current("iq", vcc, vee, model.iq);
    }
    // The decision node q, referenced to v−: high means "in− is above in+",
    // i.e. the output should be LOW.
    let q = lower.node("q");
    lower.vccs("g", vee, q, inn, inp, gm);
    if cm_gain > 0.0 {
        // The comparator's input polarity is opposite the op-amp's: common-
        // mode error is subtracted from (v(in−) − v(in+)). Reference it to
        // the midpoint of the two supply pins and apply the CMRR attenuation.
        let gcm = 0.5 * gm * cm_gain;
        lower.vccs("gcmp", q, vee, inp, vee, gcm);
        lower.vccs("gcmm", q, vee, inn, vcc, gcm);
    }
    lower.resistor("rq", q, vee, rq);
    lower.capacitor("cq", q, vee, cq);
    if model.vos != 0.0 {
        // −gm·VOS into q, i.e. +VOS on v(in+) − v(in−).
        lower.current("ios", q, vee, gm * model.vos);
    }
    if ibias_plus != 0.0 {
        lower.current("ibp", inp, vee, ibias_plus);
    }
    if ibias_minus != 0.0 {
        lower.current("ibn", inn, vee, ibias_minus);
    }
    if model.vhys != 0.0 {
        // gm·VHYS·(v(q) − v(v−) − ½) into q: positive feedback that holds
        // whichever state q is in until the input crosses by VHYS/2.
        let gh = gm * model.vhys;
        lower.vccs("gh", vee, q, q, vee, gh);
        lower.current("ih", q, vee, 0.5 * gh);
    }
    let q_hi = lower.offset_rail("qhi", vee, 1.0, false);
    lower.clamp("dqh", q, q_hi);
    lower.clamp("dql", vee, q);

    match model.output {
        ComparatorOutput::OpenCollector => {
            // An N-channel switch, gate q against v−: VTO = 0.5 V is the middle
            // of q's span, and KP = 2/ROUT makes the on-resistance at
            // v(q) − v(v−) = 1 V exactly ROUT.
            let name = lower.name("mo");
            let kp = 2.0 / model.rout;
            let mos = MosModel {
                polarity: Polarity::N,
                vto: 0.5,
                vto_given: true,
                kp,
                kp_given: true,
                uo: 600.0,
                ld: 0.0,
                lambda: 0.0,
                rd: 0.0,
                rd_given: false,
                rs: 0.0,
                rs_given: false,
                rsh: 0.0,
                gamma: 0.0,
                gamma_given: true,
                phi: 0.6,
                phi_given: true,
                nsub: None,
                nss: 0.0,
                tpg: 1.0,
                is: 1e-14,
                js: 0.0,
                cbd: 0.0,
                cbd_given: false,
                cbs: 0.0,
                cbs_given: false,
                cj: 0.0,
                cjsw: 0.0,
                pb: 0.8,
                mj: 0.5,
                mjsw: 0.5,
                fc: 0.5,
                w: 1.0,
                l: 1.0,
                cgso: 0.0,
                cgdo: 0.0,
                cgbo: 0.0,
                tox: None,
                channel_charge_model: MosChannelChargeModel::Meyer,
                tnom_celsius: 27.0,
            };
            lower.circuit.mosfets.push(Mosfet {
                name,
                multiplier: 1.0,
                off: false,
                ic: [None; 3],
                d: out,
                g: q,
                s: vee,
                junction_d: out,
                junction_s: vee,
                bulk: vee,
                model_name: "comparator-output".to_string(),
                model: mos,
                operating_model: mos,
                temperature_celsius: None,
                beta: kp,
                w: 1.0,
                l: 1.0,
                ad: 0.0,
                as_: 0.0,
                pd: 0.0,
                ps: 0.0,
                drain_resistance_ohms: 0.0,
                source_resistance_ohms: 0.0,
            });
        }
        ComparatorOutput::PushPull => {
            let p = lower.node("p");
            let rp = MACRO_GAIN_RESISTANCE;
            let gp = COMPARATOR_PP_GAIN / rp;
            // −gp·(v(q) − v(v−) − ½) into p, referenced to ground.
            lower.vccs("gp", p, None, q, vee, gp);
            lower.current("ip", None, p, 0.5 * gp);
            lower.resistor("rp", p, None, rp);
            lower.clamp("dph", p, vcc);
            lower.clamp("dpl", vee, p);
            lower.vccs("go", None, out, p, None, 1.0 / model.rout);
            lower.resistor("ro", out, None, model.rout);
        }
    }
    Ok(())
}

/// The pin order of a regulator `X` line, for the error when it is wrong.
fn regulator_shape(model: &RegulatorModel) -> &'static str {
    match model.kind {
        RegulatorKind::Linear => "in out gnd",
        RegulatorKind::LinearAdjustable => "in out adj",
        RegulatorKind::Buck if model.is_adjustable_buck() => "in out gnd fb",
        RegulatorKind::Buck => "in out gnd",
    }
}

/// Lower a regulator `X` line; see [`RegulatorModel`] for the circuit.
fn lower_regulator(
    circuit: &mut Circuit,
    ctx: &LineCtx<'_>,
    device: &PendingDevice,
    model: RegulatorModel,
) -> Result<(), AnalogError> {
    if model.kind == RegulatorKind::Buck && (model.vout > 0.0) == model.is_adjustable_buck() {
        return Err(ctx.parse_err(
            "a BUCK model sets exactly one of VOUT (fixed, pins `in out gnd`) and VREF \
             (adjustable, pins `in out gnd fb`)",
        ));
    }
    let shape = regulator_shape(&model);
    let wanted = shape.split(' ').count();
    if device.nodes.len() != wanted {
        return Err(ctx.parse_err(format!(
            "`{}` is a regulator with {} pins; expected `X<name> {shape} {}`",
            device.name,
            device.nodes.len(),
            device.model_name
        )));
    }
    let (input, out, reference) = (device.nodes[0], device.nodes[1], device.nodes[2]);
    let feedback = device.nodes.get(3).copied();
    let valid = model.ilim > 0.0
        && model.rout > 0.0
        && model.vdo >= 0.0
        && model.iq >= 0.0
        && model.dmax > 0.0
        && model.dmax <= 1.0
        && model.eff > 0.0
        && model.eff <= 1.0
        && model.aol > 1.0
        && model.line.is_finite();
    if !valid {
        return Err(ctx.parse_err(
            "a regulator model needs ILIM > 0, ROUT > 0, VDO >= 0, IQ >= 0, \
             0 < DMAX <= 1, 0 < EFF <= 1 and AOL > 1",
        ));
    }
    let adjustable_buck = model.is_adjustable_buck();
    let setpoint = if adjustable_buck {
        model.vref
    } else {
        model.setpoint()
    };
    if setpoint <= 0.0 {
        return Err(ctx.parse_err("a regulator's VOUT or VREF must be positive"));
    }

    let ra = MACRO_GAIN_RESISTANCE;
    let mut lower = Lowering {
        circuit,
        prefix: device.name.clone(),
    };

    // The set point a, as a ground-referenced voltage: what v(out) − v(ref)
    // should be.
    let a = lower.node("a");
    lower.resistor("ra", a, None, ra);
    match feedback {
        Some(fb) => {
            // Error amplifier: AOL/Ra · (VREF − (v(fb) − v(ref))) into a.
            let gm = model.aol / ra;
            lower.current("iref", None, a, gm * model.vref);
            lower.vccs("gerr", a, None, fb, reference, gm);
        }
        None => {
            // VSET plus LINE·(v(in) − v(ref) − VSET − VDO): the line term is
            // zero at the input where regulation begins.
            let base = setpoint - model.line * (setpoint + model.vdo);
            lower.current("iset", None, a, base / ra);
            if model.line != 0.0 {
                lower.vccs("gline", None, a, input, reference, model.line / ra);
            }
        }
    }

    // Dropout: a ≤ hd = max(DMAX·(v(in) − v(ref)) − VDO, 0). The max is a
    // clamp diode holding hf at ground through 1 kΩ when the headroom is
    // negative; an E buffers it into hd, so the set point's clamp current
    // never flows through that 1 kΩ.
    let h0 = lower.node("h0");
    lower.vcvs("eh", h0, None, input, reference, model.dmax);
    let h1 = lower.node("h1");
    lower.voltage("vh", h0, h1, model.vdo);
    let hf = lower.node("hf");
    lower.resistor("rh", h1, hf, 1e3);
    lower.clamp("dhz", None, hf);
    let hd = lower.node("hd");
    lower.vcvs("ehd", hd, None, hf, None, 1.0);
    lower.clamp("dh", a, hd);

    // Output: v(oi) = v(ref) + v(a), behind ROUT.
    let oi = lower.node("oi");
    lower.vcvs("eo", oi, reference, a, None, 1.0);
    lower.resistor("ro", oi, out, model.rout);
    let eo = lower.name("eo");

    // Current limit: a ≤ a + K·(ILIM − I(out)), i.e. the clamp conducts once
    // I(out) passes ILIM (by the clamp's few millivolts over K). The output
    // current comes from the output E's branch (H: v(m) = K·i(eo) = −K·I).
    let m = lower.node("m");
    let name = lower.name("hm");
    lower.circuit.ccvs.push(Ccvs {
        name,
        p: m,
        n: None,
        control: eo.clone(),
        control_branch: usize::MAX,
        ohms: REGULATOR_LIMIT_GAIN,
    });
    let l0 = lower.node("l0");
    lower.vcvs("el", l0, m, a, None, 1.0);
    let l1 = lower.node("l1");
    lower.voltage("vl", l1, l0, REGULATOR_LIMIT_GAIN * model.ilim);
    let name = lower.name("dl");
    lower.circuit.diodes.push(Diode {
        name,
        multiplier: 1.0,
        off: false,
        ic: None,
        anode: a,
        cathode: l1,
        junction_anode: a,
        model_name: "limit-clamp".to_string(),
        model: REGULATOR_LIMIT_DIODE,
        operating_model: REGULATOR_LIMIT_DIODE,
        series_resistance_ohms: 0.0,
        temperature_celsius: None,
        breakdown: None,
    });

    // Input current. The output E draws I(out) from ref; a linear part takes
    // exactly that from in (F cancels it at ref), a buck takes the power.
    if model.kind == RegulatorKind::Buck {
        let name = lower.name("pin");
        lower.circuit.power_sinks.push(PowerSink {
            name,
            p: input,
            n: reference,
            op: out,
            on: reference,
            control: eo,
            control_branch: usize::MAX,
            eff: model.eff,
        });
    } else {
        let name = lower.name("fi");
        lower.circuit.cccs.push(Cccs {
            name,
            p: input,
            n: reference,
            control: eo,
            control_branch: usize::MAX,
            gain: -1.0,
        });
    }

    // Quiescent (or adjust-pin) current, from in into ref, in proportion to
    // how far the part is into regulation.
    if model.iq > 0.0 {
        match feedback {
            Some(fb) => lower.vccs("gq", input, reference, fb, reference, model.iq / model.vref),
            None => lower.vccs("gq", input, reference, a, None, model.iq / setpoint),
        }
    }
    Ok(())
}

#[allow(clippy::type_complexity)]
fn two_nodes_value_ic<'a>(
    ctx: &LineCtx<'_>,
    tokens: &[&'a str],
    shape: &str,
) -> Result<(&'a str, &'a str, &'a str, Option<&'a str>), AnalogError> {
    match tokens.len() {
        3 => Ok((tokens[0], tokens[1], tokens[2], None)),
        4 => {
            let (key, value) = tokens[3]
                .split_once('=')
                .ok_or_else(|| ctx.parse_err(format!("expected `{shape}`")))?;
            if !key.eq_ignore_ascii_case("ic") {
                return Err(ctx.parse_err(format!("unknown parameter `{key}`; expected `{shape}`")));
            }
            Ok((tokens[0], tokens[1], tokens[2], Some(value)))
        }
        _ => Err(ctx.parse_err(format!("expected `{shape}`"))),
    }
}

fn parse_ic(circuit: &mut Circuit, ctx: &LineCtx<'_>, tokens: &[&str]) -> Result<(), AnalogError> {
    // `.ic V(a)=5 V(b)=1.25`, tolerating spaces around `=`.
    let joined = tokens.join(" ").replace(" =", "=").replace("= ", "=");
    if joined.trim().is_empty() {
        return Err(ctx.parse_err("expected `.ic V(node)=<value>`"));
    }
    for item in joined.split_whitespace() {
        let (lhs, rhs) = item
            .split_once('=')
            .ok_or_else(|| ctx.parse_err("expected `.ic V(node)=<value>`"))?;
        let lhs = lhs.trim();
        let node_name = lhs
            .strip_prefix('V')
            .or_else(|| lhs.strip_prefix('v'))
            .and_then(|rest| rest.strip_prefix('('))
            .and_then(|rest| rest.strip_suffix(')'))
            .ok_or_else(|| {
                ctx.parse_err(format!(
                    "`{lhs}` is not a node reference; expected `V(node)`"
                ))
            })?;
        let value = ctx.value(rhs, "initial condition")?;
        let node = circuit.intern(node_name);
        match node {
            Some(index) => circuit.node_ic.push((index, value)),
            None => return Err(ctx.parse_err("`.ic` cannot set the ground node")),
        }
    }
    Ok(())
}
