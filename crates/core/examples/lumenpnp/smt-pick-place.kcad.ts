// LabWired SMT pick-and-place verification model.
// Mechanical architecture follows the open-source Opulo LumenPnP v4 design.
// Original LumenPnP hardware (c) Opulo, CERN-OHL-W-2.0.
// This derivative omits upstream branding and uses simplified purchased parts.
// Dimensions in mm. The contract is in geometry-contract.json.
// Z is clearance above the board top at world Z = 160.

const xMm = param('xMm', 150, { min: 0, max: 400 });
const yMm = param('yMm', 180, { min: 0, max: 400 });
const zMm = param('zMm', 25, { min: 0, max: 50 });
const aDeg = param('aDeg', 0, { min: -180, max: 180 });
const bDeg = param('bDeg', 0, { min: -180, max: 180 });
// All linear axes use a 20-tooth GT2 pitch circumference of 40 mm.
const zPitchRadius = 40 / (2 * Math.PI);

const machine = assembly('labwired-smt-pick-place');
const ink = '#252c34';
const black = '#161c22';
const gold = '#e1b54d';
const steel = '#a9b4bf';
const blue = '#2470aa';
let connectorIndex = 0;

function frameAt(part, name, point) {
  part.connector(name, { type: 'frame', origin: { kind: 'vec3', value: point } });
}

function axisAt(part, name, point, axis) {
  part.connector(name, { type: 'axis', origin: { kind: 'vec3', value: point }, axis });
}

function fixed(name, shape, parent, parentName, point, material = 'aluminum-6061') {
  const density = { rubber: 1100, copper: 8960, glass: 2500, 'molded-package': 1800 }[material];
  const part = machine.part(name, shape, density ? { density } : { material });
  const connection = `mount-${connectorIndex++}`;
  frameAt(parent, connection, point);
  frameAt(part, 'mount', point);
  machine.mate(`${name}-fixed`, `${parentName}.${connection}`, `${name}.mount`, 'fastened');
  return part;
}

function extrusion(length, axis, origin) {
  // Four recessed V slots around a 20 mm aluminium extrusion.
  let stock = box(20, 20, length).translate(-10, -10, 0)
    .subtract(box(6.0, 3.1, length + 0.2).translate(-3, 6.9, -0.1))
    .subtract(box(6.0, 3.1, length + 0.2).translate(-3, -10, -0.1))
    .subtract(box(3.1, 6.0, length + 0.2).translate(6.9, -3, -0.1))
    .subtract(box(3.1, 6.0, length + 0.2).translate(-10, -3, -0.1));
  if (axis === 'x') stock = stock.rotate([0, 1, 0], 90);
  if (axis === 'y') stock = stock.rotate([1, 0, 0], -90);
  return stock.translate(...origin).finish('anodized-black', { color: ink });
}

function rail(length, axis, origin) {
  let body = box(12, length, 8).translate(-6, 0, 0);
  const holes = [];
  for (let s = 15; s < length - 8; s += 25) holes.push({ u: 0, v: s - length / 2 });
  body = body.holes('top', { positions: holes, diameter: 3.4, depth: 'through', counterbore: { diameter: 6.0, depth: 3.0 } });
  if (axis === 'x') body = body.rotate([0, 0, 1], -90);
  return body.translate(...origin).finish('steel', { color: steel });
}

function plate(w, h, t, point, color = black) {
  return extrudeRoundedRect(w, h, 2.5, t).translate(...point).finish('anodized-black', { color });
}

function motorCase(size, depth) {
  return box(size, size, depth).translate(-size / 2, -size / 2, 0)
    .hole('top', { u: 0, v: 0, diameter: 7, depth: 'through' })
    .finish('anodized-black', { color: '#343a43' });
}

function nozzle() {
  // The tip sealing face is the plane z=160, OD 0.6, bore 0.5.
  return path().moveTo(0.25, 160).lineTo(0.30, 160)
    .lineTo(0.8, 164).lineTo(1.6, 166).lineTo(1.6, 172)
    .lineTo(4.2, 172).lineTo(4.2, 178).lineTo(3.0, 180)
    .lineTo(3.0, 247).lineTo(0.25, 247).close().revolve()
    .finish('brass', { color: '#d3ad65' });
}

// The 600 mm extrusion frame and 550 mm rail class follow the reference.
const lowerFront = machine.part('frame-front', extrusion(600, 'x', [-70, -60, 30]), { material: 'aluminum-6061' });
const lowerLeft = fixed('frame-left', extrusion(600, 'y', [-80, -60, 30]), lowerFront, 'frame-front', [-70, -55, 35]);
const lowerRight = fixed('frame-right', extrusion(600, 'y', [540, -60, 30]), lowerFront, 'frame-front', [530, -55, 35]);
const lowerBack = fixed('frame-back', extrusion(600, 'x', [-70, 540, 30]), lowerLeft, 'frame-left', [-70, 535, 35]);

const legPoints = [[-80, -60], [540, -60], [-80, 540], [540, 540]];
const legs = [];
for (let i = 0; i < legPoints.length; i++) {
  const [x, y] = legPoints[i];
  const leg = box(32, 32, 235).translate(x - 16, y - 16, 5)
    .subtract(box(18, 34, 72).translate(x - 9, y - 17, 50))
    .subtract(box(18, 34, 72).translate(x - 9, y - 17, 145))
    .subtract(box(34, 20.4, 20.4).translate(x - 17, y - 10.2, 19.8))
    .subtract(box(20.4, 34, 20.4).translate(x - 10.2, y - 17, 19.8))
    .subtract(box(20.4, 34, 20.4).translate(x - 10.2, y - 17, 136.6))
    .subtract(box(20.4, 34, 20.4).translate(x - 10.2, y - 17, 219.8))
    .finish('abs', { color: black });
  const parent = i % 2 === 0 ? lowerLeft : lowerRight;
  const parentName = i % 2 === 0 ? 'frame-left' : 'frame-right';
  const p = fixed(`leg-${i}`, leg, parent, parentName, [x + 6, y + (i < 2 ? 5 : -5), 40.2], 'abs');
  legs.push(p);
  fixed(`rubber-foot-${i}`, box(34, 34, 5).translate(x - 17, y - 17, 0).finish('rubber'), p, `leg-${i}`, [x, y, 5], 'rubber');
}

const topLeft = fixed('y-extrusion-left', extrusion(600, 'y', [-80, -60, 230]), legs[0], 'leg-0', [-69.9, -55, 235]);
const topRight = fixed('y-extrusion-right', extrusion(600, 'y', [540, -60, 230]), legs[1], 'leg-1', [550.1, -55, 235]);
const stageSideLeft = fixed('stage-support-left', extrusion(600, 'y', [-80, -60, 146.8]), legs[0], 'leg-0', [-69.9, -55, 151.8]);
fixed('stage-support-right', extrusion(600, 'y', [540, -60, 146.8]), legs[1], 'leg-1', [550.1, -55, 151.8]);
const stageFront = fixed('stage-support-front', extrusion(600, 'x', [-70, 20, 146.8]), stageSideLeft, 'stage-support-left', [-70, 20, 146.8]);
const stageBack = fixed('stage-support-back', extrusion(600, 'x', [-70, 410, 146.8]), stageSideLeft, 'stage-support-left', [-70, 410, 146.8]);

const stageHoles = [];
for (let x = -30; x < 475; x += 40) {
  for (let y = 0; y < 450; y += 40) {
    // Keep board and tray support material intact.
    if ((x > 185 && x < 335 && y > 185 && y < 295) || (x > 30 && x < 90 && y > 40 && y < 80)) continue;
    stageHoles.push({ u: x - 215, v: y - 215 });
  }
}
let stageShape = box(540, 480, 1.6).translate(-55, -25, 156.8)
  .hole('top', {u: -60, v: -70, diameter: 35, depth: 'through'})
  .holes({byNormal:'Z', atZ:158.4}, { positions: stageHoles, diameter: 3.2, depth: 'through' })
  .finish('anodized-black', { color: '#38444c' });
const stage = fixed('staging-plate', stageShape, stageFront, 'stage-support-front', [200, 20, 156.8], 'mild-steel');

const yLeft = fixed('rail-y-left', rail(550, 'y', [-80, -35, 240]), topLeft, 'y-extrusion-left', [-80, 200, 240], 'mild-steel');
fixed('rail-y-right', rail(550, 'y', [540, -35, 240]), topRight, 'y-extrusion-right', [540, 200, 240], 'mild-steel');
// Root of the moving gantry. The primary Y joint sits on its physical carriage.
function yCarriageShape(x) {
  return box(28, 46, 13).translate(x - 14, 42, 248)
    .subtract(box(12.4, 46.2, 7.5).translate(x - 6.2, 41.9, 247.8))
    .finish('steel', { color: '#65747f' });
}
const yCarriageLeft = machine.part('y-carriage-0', yCarriageShape(-80), { material:'mild-steel' });
axisAt(yLeft, 'y-guide', [-74, 65, 248], [0, 1, 0]);
axisAt(yCarriageLeft, 'y-guide', [-74, 65, 248], [0, 1, 0]);
machine.mate('Y', 'rail-y-left.y-guide', 'y-carriage-0.y-guide', 'prismatic', { pose: yMm, limitsMm: [0, 400] });
const gantryEndLeft = fixed('gantry-end-0', plate(38, 54, 5, [-80, 65, 261], gold), yCarriageLeft, 'y-carriage-0', [-80,65,261]);
const gantry = fixed('gantry-y', extrusion(600, 'x', [-70,65,276]), gantryEndLeft, 'gantry-end-0', [-65,60,266]);
const gantryEndRight = fixed('gantry-end-1', plate(38, 54, 5, [540,65,261], gold), gantry, 'gantry-y', [525,60,266]);
fixed('y-carriage-1', yCarriageShape(540), gantryEndRight, 'gantry-end-1', [540,65,261], 'mild-steel');
const xRail = fixed('rail-x', rail(550, 'x', [-45, 65, 286]), gantry, 'gantry-y', [200, 70, 286], 'mild-steel');
const xBelt = fixed('belt-x', box(580, 1.5, 6).translate(-60, 75, 270).finish('rubber'), gantry, 'gantry-y', [200, 75, 271], 'rubber');

// Motor bodies are dimensioned envelopes. They carry no vendor surface detail.
const yMotorMountLeft = fixed('motor-y-mount-left', plate(50, 50, 5, [-80, 540, 240], gold), legs[2], 'leg-2', [-92, 552, 240]);
const yMotorMountRight = fixed('motor-y-mount-right', plate(50, 50, 5, [540, 540, 240], gold), legs[3], 'leg-3', [552, 552, 240]);
const yMotor = fixed('motor-y-left', motorCase(42, 38).translate(-80, 540, 245), yMotorMountLeft, 'motor-y-mount-left', [-80, 540, 245], 'mild-steel');
fixed('motor-y-right', motorCase(42, 38).translate(540, 540, 245), yMotorMountRight, 'motor-y-mount-right', [540, 540, 245], 'mild-steel');
const xMotorMount = fixed('motor-x-mount', plate(50, 50, 5, [-80, 65, 286], gold), gantry, 'gantry-y', [-65, 65, 286]);
fixed('motor-x', motorCase(42, 38).translate(-80, 65, 291), xMotorMount, 'motor-x-mount', [-80, 65, 291], 'mild-steel');

// X head has a vertical back plate, top rail shoe, two small Z guide rails.
const headShape = box(90, 7, 172).translate(-21, 35, 185)
  .holes('front',{positions:[{u:zPitchRadius-24,v:-81},{u:24-zPitchRadius,v:-81},{u:zPitchRadius-24,v:77},{u:24-zPitchRadius,v:77}],diameter:6.4,depth:'through'})
  .union(box(90, 38, 6).translate(-21, 35, 306))
  .union(box(26, 30, 4).translate(11, 40, 249))
  .finish('anodized-black', { color: '#384553' });
const xShoe = box(46, 28, 12).translate(1, 51, 294)
  .subtract(box(46.2, 12.4, 7.4).translate(0.9, 58.8, 293.9))
  .finish('steel', { color: '#65747f' });
const xCarriage = machine.part('x-carriage', xShoe, {material:'mild-steel'});
axisAt(xRail, 'x-guide', [24, 59, 294], [1, 0, 0]);
axisAt(xCarriage, 'x-guide', [24, 59, 294], [1, 0, 0]);
machine.mate('X', 'rail-x.x-guide', 'x-carriage.x-guide', 'prismatic', { pose: xMm, limitsMm: [0,400] });
const head = fixed('head-x', headShape, xCarriage, 'x-carriage', [24,65,306]);
const zMotor = fixed('motor-z', motorCase(42, 38).rotate([1, 0, 0], 90).translate(zPitchRadius, 80, 348), head, 'head-x', [zPitchRadius, 42, 337], 'mild-steel');

const slides = [];
const tips = [];
for (let i = 0; i < 2; i++) {
  const x = i * 48;
  const letter = i === 0 ? 'a' : 'b';
  const zRail = box(9, 4, 100).translate(x - 4.5, 31, 197)
    .finish('steel', { color: steel });
  const zr = fixed(`rail-z-${letter}`, zRail, head, 'head-x', [x, 35, 247], 'mild-steel');
  const slideShape = box(24, 8, 40).translate(x - 12, 23, 204)
    .union(box(32, 39.1, 5).translate(x - 16, -16, 239).hole('top', { u: 0, v: -3.55, diameter: 7.2, depth: 'through' }))
    .subtract(box(2,4,5.5).translate(x-1,19.5,238.8))
    .finish('anodized-black', { color: blue });
  const slide = machine.part(`slide-z-${letter}`, slideShape, { material: 'aluminum-6061' });
  axisAt(zr, 'z-guide', [x, 31, 224], [0, 0, 1]);
  axisAt(slide, 'z-guide', [x, 31, 224], [0, 0, 1]);
  machine.mate(`Z${letter.toUpperCase()}`, `rail-z-${letter}.z-guide`, `slide-z-${letter}.z-guide`, 'prismatic', { pose: i === 0 ? zMm : 25, limitsMm: [0, 50] });
  slides.push(slide);
  const motor = fixed(`motor-${letter}`, motorCase(28, 33).translate(x, 0, 206), slide, `slide-z-${letter}`, [x, 0, 239], 'mild-steel');
  const bearing = cylinder(8, 7).hole('top', { u: 0, v: 0, diameter: 6.2, depth: 'through' })
    .translate(x, 0, 198).finish('steel');
  fixed(`nozzle-bearing-${letter}`, bearing, motor, `motor-${letter}`, [x, 0, 206], 'mild-steel');
  const tip = machine.part(`nozzle-${letter}`, nozzle().translate(x, 0, 0), { material: 'mild-steel' });
  axisAt(motor, 'rotary-shaft', [x, 0, 222], [0, 0, 1]);
  axisAt(tip, 'rotary-shaft', [x, 0, 222], [0, 0, 1]);
  axisAt(tip, 'sealing-face', [x, 0, 160], [0, 0, -1]);
  machine.mate(letter.toUpperCase(), `motor-${letter}.rotary-shaft`, `nozzle-${letter}.rotary-shaft`, 'revolute', { pose: i === 0 ? aDeg : bDeg, limitsDeg: [-180, 180] });
  tips.push(tip);
  const coupler = cylinder(14, 5.5).hole('top', { u: 0, v: 0, diameter: 6.2, depth: 'through' })
    .translate(x, 0, 244).finish('steel');
  fixed(`pneumatic-fitting-${letter}`, coupler, slide, `slide-z-${letter}`, [x, 0, 244], 'mild-steel');
}
// Ideal no-slip belt coupling. Both slider clamps contact the two vertical runs.
const zBeltShape = extrudeRoundedRect(49.6,158+2*(zPitchRadius+0.8),zPitchRadius+0.8,3)
  .subtract(extrudeRoundedRect(46.4,158+2*(zPitchRadius-0.8),zPitchRadius-0.8,3.4).translate(0,0,-0.2))
  .rotate([1,0,0],90).translate(24,23,269).finish('rubber');
function zPulleyBody(x,z) {
  return cylinder(3,zPitchRadius-0.8).rotate([1,0,0],90).translate(x,26,z)
    .union(cylinder(17,3).rotate([1,0,0],90).translate(x,42,z)).finish('steel');
}
const zIdler = fixed('z-idler-lower-left',zPulleyBody(zPitchRadius,190),head,'head-x',[zPitchRadius,42,190],'mild-steel');
fixed('z-idler-lower-right',zPulleyBody(48-zPitchRadius,190),head,'head-x',[48-zPitchRadius,42,190],'mild-steel');
fixed('z-idler-upper-right',zPulleyBody(48-zPitchRadius,348),head,'head-x',[48-zPitchRadius,42,348],'mild-steel');
const zBelt = fixed('z-belt',zBeltShape,zIdler,'z-idler-lower-left',[0.8,23,190],'rubber');
const zPulley = machine.part('z-drive-pulley',zPulleyBody(zPitchRadius,348),{material:'mild-steel'});
axisAt(zMotor,'drive-shaft',[zPitchRadius,42,348],[0,-1,0]);
axisAt(zPulley,'drive-shaft',[zPitchRadius,42,348],[0,-1,0]);
machine.mate('Z-drive','motor-z.drive-shaft','z-drive-pulley.drive-shaft','revolute', {pose:zMm.multiply(360/40),limitsDeg:[0,50*360/40]});
machine.coupleMates('ZA',{source:'Z-drive',ratio:40/360});
machine.coupleMates('ZB', { source: 'ZA', ratio: -1, offset: 50 });
machine.transmission('z-primary-belt',{kind:'belt',sourceMate:'Z-drive',drivenMates:['ZA'],actuator:'motor-z',input:'z-drive-pulley',output:'slide-z-a',path:['motor-z','z-drive-pulley','z-belt','slide-z-a'],ratio:40/360,notes:'20-tooth GT2, 40 mm per revolution. Ideal no-slip coupling with rigid belt.'});
machine.transmission('z-counterweight-belt',{kind:'belt',sourceMate:'ZA',drivenMates:['ZB'],input:'slide-z-a',output:'slide-z-b',path:['slide-z-a','z-belt','slide-z-b'],ratio:-1});

const boardShape = box(120, 80, 1.6).translate(200, 200, 158.4)
  .holes('top', { positions: [{u:-55,v:-35},{u:55,v:-35},{u:-55,v:35},{u:55,v:35}], diameter: 3.2, depth: 'through' })
  .finish('abs', { color: '#17786b' });
const board = fixed('work-pcb', boardShape, stage, 'staging-plate', [260, 240, 158.4], 'abs');
for (const [id, cx, cy, w, d, rotation] of [['u1',230,225,5,5,0],['u2',265,225,4,5,0],['r1',290,250,3.2,1.6,45]]) {
  const land = box(w + 0.8, d + 0.8, 0.035).translate(cx - (w + 0.8)/2, cy - (d + 0.8)/2, 160)
    .subtract(box(w - 0.6, d - 0.6, 0.08).translate(cx - (w - 0.6)/2, cy - (d - 0.6)/2, 159.98))
    .translate(-cx,-cy,0).rotate([0,0,1],rotation).translate(cx,cy,0).finish('brass');
  fixed(`pads-${id}`, land, board, 'work-pcb', [cx, cy, 160], 'copper');
}

let trayShape = box(42, 12, 1.6).translate(41, 54, 158.4);
for (let i = 0; i < 4; i++) {
  // Pocket rims surround a flat package support plane at world Z=160.
  const x = 50 + 8 * i;
  const rim = box(7, 8, 1.2).translate(x - 3.5, 56, 159.9)
    .subtract(box(5.8, 5.8, 1.5).translate(x - 2.9, 57.1, 159.85));
  trayShape = trayShape.union(rim);
}
const tray = fixed('component-tray', trayShape.finish('abs', {color:'#263443'}), stage, 'staging-plate', [62, 60, 158.4], 'abs');
const payloads = [{ name:'part-qfn',x:50,w:5,d:5 },{ name:'part-chip',x:66,w:3.2,d:1.6 },{ name:'part-soic',x:74,w:5,d:4 }];
for (const p of payloads) {
  // These initial tray constraints describe the captured starting scene.
  // The co-simulation releases and attaches the bodies on measured vacuum events.
  // Molded-package effective density 1800 kg/m3 is an explicit plant assumption.
  fixed(p.name, box(p.w, p.d, 1.4).translate(p.x-p.w/2,60-p.d/2,160).finish('abs',{color:'#20242a'}), tray, 'component-tray', [p.x,60,160], 'molded-package');
}

// Downward camera at the gantry reference point; upward camera beside board.
const cameraBody = cylinder(21, 12).translate(24, 63, 228).finish('anodized-black');
const camera = fixed('down-camera', cameraBody, head, 'head-x', [24, 63, 249], 'abs');
fixed('down-camera-lens', cylinder(2, 8).translate(24, 63, 226).finish('glass',{color:'#2b718d'}), camera, 'down-camera', [24,63,228], 'glass');
const upCamera = fixed('up-camera', cylinder(18, 17).translate(155, 145, 140.4).finish('anodized-black'), stage, 'staging-plate', [155,145,158.4], 'abs');
fixed('up-camera-lens', cylinder(1.5, 9).translate(155,145,158.4).finish('glass',{color:'#286a85'}), upCamera, 'up-camera', [155,145,158.4], 'glass');

// Rotating nozzle drive supports. Prismatic guides are represented by their real rail/carriage mates.
machine.mechanicalJoint('z-belt-drive', {mate:'Z-drive',actuator:'motor-z',shaft:'z-drive-pulley',supports:['motor-z'],output:'z-drive-pulley'});
machine.mechanicalJoint('a-drive', { mate: 'A', actuator: 'motor-a', shaft: 'nozzle-a', supports: ['nozzle-bearing-a'], output: 'nozzle-a' });
machine.mechanicalJoint('b-drive', { mate: 'B', actuator: 'motor-b', shaft: 'nozzle-b', supports: ['nozzle-bearing-b'], output: 'nozzle-b' });

// The first assembly part, frame-front, is the grounded machine reference.
// Firmware co-simulation checks vacuum lift and changing package contact.
setRenderEnvironment({ preset: 'studio', intensity: 1.1 });
setCameraTarget(220, 220, 160);
return machine.solvedModel({});
