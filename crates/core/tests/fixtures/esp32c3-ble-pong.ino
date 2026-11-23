// BLE Pong — two ESP32-C3s, one image, connectionless BLE.
//
// Roles settle themselves. GPIO3 has an internal pull-up, so a jumper to GND
// forces that board to be the HOST; unstrapped, the LOWER BLE address hosts.
// The host owns ball physics and the score; the guest owns only its own
// paddle. Each board publishes its state in its BLE ADVERTISING payload and
// reads the other's from its scan callback — no connection, no pairing.
//
// VALIDATED ON SILICON 2026-08-07: two real C3 SuperMinis (9c:cc:01:d0:98:e0
// and 9c:cc:01:d0:5b:78), unstrapped. 5b:78 takes HOST via the MAC fallback
// (BLE address is the base MAC +2, so 0x7a < 0xe2), the ball advances, and the
// score climbs on both screens.
//
// ── WHY THE GUEST DEAD-RECKONS ─────────────────────────────────────────────
// The guest used to render only what arrived over the air, so its frame rate
// WAS the radio update rate. With the old PUBLISH_MS of 700 that meant 1.4
// world updates/s: the ball teleported, and the two screens could drift up to
// 70 px apart (2 px per 20 ms loop × 700 ms), which breaks this lab's whole
// premise of showing the same game twice.
//
// Worse, the FIRST advertisement is emitted at the end of setup() while
// `isHost` is still false — a 5-byte GUEST-shaped frame with no world state.
// At 700 ms the guest therefore received nothing usable for the whole first
// period, `haveHostFrame` stayed false, and it painted the static fallback
// forever: a dead panel, on hardware that was working perfectly.
//
// Raising the publish rate alone is the wrong fix — it is bounded by the
// radio, costs power, and a fast republish cadence stalls the LabWired twin's
// simulated RW-BLE controller (silicon is unaffected; the twin is not). So the
// host now sends its VELOCITY with the position and the guest integrates it
// locally every frame, snapping to truth when a packet lands. Motion is smooth
// at the LOOP rate, not the packet rate, and the cadence is free to stay slow.
//
// Payload 12 B: E5 02 <tag> <seq> <ballX> <ballY> <vx> <vy> <hostY> <guestY>
// <scoreH> <scoreG>. vx/vy are signed int8. Legacy advertising has room (31 B).
//
// Dead reckoning alone still jitters: the guest's prediction and the host's
// physics are not phase-aligned, so each packet lands 2-4 px from where the
// guest had predicted, and taking it outright turns that error into a visible
// jump at the packet rate. The guest therefore keeps a truth track (tgtX/tgtY)
// and walks the DRAWN position toward it a pixel per frame. See SNAP_PX.

#include <Wire.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>
#include <BLEDevice.h>
#include <BLEScan.h>
#include <BLEAdvertisedDevice.h>

#define W 128
#define H 64
#define PADDLE_H 16
#define PADDLE_W 3
#define STRAP_PIN 3

// Node identity. NOT the BLE MAC alone: 0xA1 is the strapped host, 0xB2 the
// fallback guest tag; any real MAC byte other than those two is preferred so
// two unstrapped boards still differ.
#define TAG_HOST  0xA1
#define TAG_GUEST 0xB2

// With dead reckoning this only bounds how far prediction may drift before a
// correction (≈10 frames × 2 px), not how smooth the guest looks.
#define PUBLISH_MS 100

// The ease is a correction mechanism, not a transport. Above this much error
// the ball is not mispredicted, it is somewhere else — a respawn after a point.
// Easing across a whole court would render as the ball SLIDING back to centre,
// so a discontinuity this large is taken literally instead.
#define SNAP_PX 12

static Adafruit_SSD1306 oled(W, H, &Wire, -1);
static BLEAdvertising* adv = nullptr;
static uint8_t myTag = 0, peerTag = 0, rally = 0, scoreH = 0, scoreG = 0, seq = 0;
static bool strapHost = false, isHost = false, roleLocked = false;
static bool haveOled = false, tagClashWarned = false, haveHostFrame = false;
static uint32_t lastPublishMs = 0;
static int ballX = W / 2, ballY = H / 2, vx = 2, vy = 1;
static int myY = H / 2 - PADDLE_H / 2, peerY = H / 2 - PADDLE_H / 2;
static int leftY = H / 2 - PADDLE_H / 2, rightY = H / 2 - PADDLE_H / 2;
// The truth track: the host's last packet, advanced by the same physics. What
// is DRAWN (ballX/ballY) chases it. Keeping the two separate is what lets a
// correction be spent over several frames instead of in one jump.
static int tgtX = W / 2, tgtY = H / 2;
// Paddles get the same treatment on their own drawn copies — the peer's paddle
// only changes when a packet lands, so drawing it raw steps it once per packet.
static int drawLeftY = H / 2 - PADDLE_H / 2, drawRightY = H / 2 - PADDLE_H / 2;

// ── Sharp GP2Y0A21: Vo is a distance-keyed analogue voltage ────────────────
// 10 cm ≈ 3.1 V (~3847 counts), 80 cm ≈ 0.5 V (~620 counts); beyond 80 cm it
// holds a ~0.4 V floor (far is NOT zero). Monotonic nearer→higher.
#define SHARP_PIN 1
#define RAW_NEAR 3600
#define RAW_FAR   650
#define RAW_NOISE  60

static uint16_t sharpRaw() {
  uint32_t acc = 0;
  for (int i = 0; i < 8; i++) acc += analogRead(SHARP_PIN);
  return acc >> 3;
}

// Shared by host physics and guest prediction so the two cannot drift apart
// through divergent rules. Paddle hits and scoring stay host-only — a guest
// must never invent a score.
static void advanceBall(int &x, int &y, int &dx, int &dy) {
  x += dx;
  y += dy;
  if (y <= 0 || y >= H - 2) dy = -dy;
  if (x <= PADDLE_W) x = PADDLE_W;
  if (x >= W - PADDLE_W - 2) x = W - PADDLE_W - 2;
}

// One pixel per call toward the target. Deliberately not proportional: on
// integer pixel coordinates a fractional step either truncates to zero and
// never converges, or overshoots and oscillates.
static void ease(int &v, int t) {
  if (v < t) v++;
  else if (v > t) v--;
}

class Rx : public BLEAdvertisedDeviceCallbacks {
  void onResult(BLEAdvertisedDevice d) {
    std::string m = d.getManufacturerData();
    if (m.size() < 5 || (uint8_t)m[0] != 0xE5 || (uint8_t)m[1] != 0x02) return;
    if ((uint8_t)m[2] == myTag) {
      // Our own frame — or a peer that picked the same tag, indistinguishable
      // from here. Say it once rather than going silently deaf.
      if (!tagClashWarned && peerTag == 0) {
        tagClashWarned = true;
        Serial.println("TAG CLASH - strap one node: GPIO3 to GND");
      }
      return;
    }
    peerTag = (uint8_t)m[2];

    if (!roleLocked) {
      if (!strapHost) isHost = (myTag < peerTag);
      roleLocked = true;
      // Publish immediately on the role decision. The setup() advertisement
      // went out while isHost was still false, so it was a guest-shaped frame
      // with no world state; waiting a whole PUBLISH_MS to correct that is
      // what left the peer with nothing to draw.
      if (isHost) { publishNow(); }
    }

    if (isHost) {
      if (m.size() == 5) peerY = (uint8_t)m[4];
    } else if (m.size() >= 12) {
      // Adopt the host's velocity so the frames until the next packet are
      // predicted along the right vector. The POSITION is not taken outright
      // unless it has to be — see below.
      int hx = (uint8_t)m[4];
      int hy = (uint8_t)m[5];
      // First frame, or a discontinuity too large to be prediction error:
      // take it literally. Otherwise leave the drawn ball where it is and let
      // loop() walk it onto the new truth.
      if (!haveHostFrame || abs(hx - ballX) > SNAP_PX || abs(hy - ballY) > SNAP_PX) {
        ballX = hx;
        ballY = hy;
      }
      tgtX = hx;
      tgtY = hy;
      vx = (int8_t)m[6];
      vy = (int8_t)m[7];
      leftY = (uint8_t)m[8];
      rightY = (uint8_t)m[9];
      scoreH = (uint8_t)m[10];
      scoreG = (uint8_t)m[11];
      peerY = leftY;
      haveHostFrame = true;
    }
  }
public:
  static void publishNow();
};

static void publish() {
  char b[12];
  b[0] = (char)0xE5; b[1] = (char)0x02; b[2] = (char)myTag; b[3] = (char)seq++;
  int n;
  if (isHost) {
    b[4] = (char)ballX;  b[5] = (char)ballY;
    b[6] = (char)(int8_t)vx; b[7] = (char)(int8_t)vy;
    b[8] = (char)myY;    b[9] = (char)peerY;
    b[10] = (char)scoreH; b[11] = (char)scoreG;
    n = 12;
  } else {
    b[4] = (char)myY;
    n = 5;
  }
  BLEAdvertisementData ad;
  ad.setManufacturerData(std::string(b, n));
  adv->stop();
  adv->setAdvertisementData(ad);
  adv->start();
}

void Rx::publishNow() { publish(); lastPublishMs = millis(); }

static void draw() {
  if (!haveOled) return;
  int dl, dr, dbx, dby; uint8_t dsh, dsg;
  if (isHost) {
    dl = myY; dr = peerY; dbx = ballX; dby = ballY; dsh = scoreH; dsg = scoreG;
  } else if (haveHostFrame) {
    dl = leftY; dr = rightY; dbx = ballX; dby = ballY; dsh = scoreH; dsg = scoreG;
  } else {
    dl = peerY; dr = myY; dbx = ballX; dby = ballY; dsh = scoreH; dsg = scoreG;
  }
  // Twice per frame: a paddle tracks a hand, which moves faster than one pixel
  // per 20 ms loop, and a single step per frame reads as lag rather than smoothing.
  ease(drawLeftY, dl);
  ease(drawLeftY, dl);
  ease(drawRightY, dr);
  ease(drawRightY, dr);

  oled.clearDisplay();
  oled.fillRect(0, drawLeftY, PADDLE_W, PADDLE_H, SSD1306_WHITE);
  oled.fillRect(W - PADDLE_W, drawRightY, PADDLE_W, PADDLE_H, SSD1306_WHITE);
  oled.fillRect(dbx, dby, 2, 2, SSD1306_WHITE);
  oled.setTextSize(2);
  oled.setTextColor(SSD1306_WHITE);
  oled.setCursor(W / 2 - 18, 0);
  oled.print(dsh); oled.print(":"); oled.print(dsg);
  oled.display();
}

void setup() {
  Serial.begin(115200);
  pinMode(STRAP_PIN, INPUT_PULLUP);
  delay(10);
  strapHost = (digitalRead(STRAP_PIN) == LOW);
  isHost = strapHost;
  if (strapHost) roleLocked = true;

  Wire.begin(8, 9);
  haveOled = oled.begin(SSD1306_SWITCHCAPVCC, 0x3C);

  BLEDevice::init("pong");

  // Identity: strap wins, then a real MAC byte, then a fixed fallback. The
  // engine mints a distinct factory eFuse MAC per die (labwired-core #828), so
  // the MAC branch is live on both twin and silicon.
  uint8_t macTag = (*BLEDevice::getAddress().getNative())[5];
  if (strapHost) myTag = TAG_HOST;
  else if (macTag != 0 && macTag != TAG_HOST) myTag = macTag;
  else myTag = TAG_GUEST;

  adv = BLEDevice::getAdvertising();
  adv->setMinInterval(0x20);  // 20 ms, units of 0.625 ms
  adv->setMaxInterval(0x40);  // 40 ms
  publish();
  lastPublishMs = millis();

  BLEScan* s = BLEDevice::getScan();
  // wantDuplicates MUST be true: latest-value-wins on every advertising
  // report. Default false delivers the peer once and then freezes.
  s->setAdvertisedDeviceCallbacks(new Rx(), true);
  s->setActiveScan(false);
  s->setInterval(100);
  s->setWindow(80);   // 80% duty so a peer update is not missed
  s->start(0, nullptr, false);

  Serial.print("ROLE ");
  Serial.print(isHost ? "HOST" : "GUEST");
  Serial.print(" tag="); Serial.print(myTag);
  Serial.print(" strap="); Serial.print(strapHost ? 1 : 0);
  Serial.print(" oled="); Serial.print(haveOled ? 1 : 0);
  Serial.println(" sharp=analog");
}

void loop() {
  uint16_t raw = sharpRaw();
  if (raw > RAW_NOISE) {
    int c = raw < RAW_FAR ? RAW_FAR : (raw > RAW_NEAR ? RAW_NEAR : raw);
    myY = (int)((long)(RAW_NEAR - c) * (H - PADDLE_H) / (RAW_NEAR - RAW_FAR));
  }

  if (isHost) {
    advanceBall(ballX, ballY, vx, vy);
    if (ballX <= PADDLE_W) {
      if (ballY + 2 >= myY && ballY <= myY + PADDLE_H) { vx = -vx; rally++; }
      else { scoreG++; ballX = W / 2; ballY = H / 2; vx = 2; }
    }
    if (ballX >= W - PADDLE_W - 2) {
      if (ballY + 2 >= peerY && ballY <= peerY + PADDLE_H) { vx = -vx; rally++; }
      else { scoreH++; ballX = W / 2; ballY = H / 2; vx = -2; }
    }
  } else if (haveHostFrame) {
    // Dead reckoning: predict along the last known velocity every frame, so the
    // ball moves at the LOOP rate rather than the packet rate. Both the drawn
    // ball and the truth track advance; the ease then closes whatever gap the
    // last correction left, a pixel at a time.
    advanceBall(ballX, ballY, vx, vy);
    advanceBall(tgtX, tgtY, vx, vy);
    ease(ballX, tgtX);
    ease(ballY, tgtY);
  }

  uint32_t now = millis();
  if ((uint32_t)(now - lastPublishMs) >= PUBLISH_MS) { lastPublishMs = now; publish(); }
  draw();

  Serial.print(isHost ? "H" : "G");
  Serial.print(" ball="); Serial.print(ballX); Serial.print(","); Serial.print(ballY);
  Serial.print(" v="); Serial.print(vx); Serial.print(","); Serial.print(vy);
  Serial.print(" me="); Serial.print(myY);
  Serial.print(" peer="); Serial.print(peerY);
  Serial.print(" score="); Serial.print(scoreH); Serial.print(":"); Serial.print(scoreG);
  Serial.print(" rally="); Serial.println(rally);

  delay(20);
}
