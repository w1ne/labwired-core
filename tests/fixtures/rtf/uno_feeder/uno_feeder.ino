// Realtime-factor workload: Uno + 16x2 I2C LCD (0x27) + 4x4 keypad + PWM motor.
// Shape of a pet feeder: scan the keypad, redraw a countdown on the LCD every
// 200 ms, and pulse the motor on D11. Runs forever; the gate stops it on a
// fixed cycle budget. Prints one marker so a dead boot cannot pass as fast.
#include <Wire.h>
#include <LiquidCrystal_I2C.h>
#include <Keypad.h>

LiquidCrystal_I2C lcd(0x27, 16, 2);
const byte ROWS = 4, COLS = 4;
char keys[ROWS][COLS] = {
  {'1', '2', '3', 'A'}, {'4', '5', '6', 'B'}, {'7', '8', '9', 'C'}, {'*', '0', '#', 'D'}};
byte rowPins[ROWS] = {9, 8, 7, 6};
byte colPins[COLS] = {5, 4, 3, 2};
Keypad keypad = Keypad(makeKeymap(keys), rowPins, colPins, ROWS, COLS);

const int MOTOR = 11;
unsigned long lastDraw = 0;
unsigned int frame = 0;

void setup() {
  Serial.begin(9600);
  lcd.init();
  lcd.backlight();
  pinMode(MOTOR, OUTPUT);
  Serial.println("RTF_READY uno_feeder");
}

void loop() {
  char k = keypad.getKey();
  if (k) Serial.println(k);
  unsigned long now = millis();
  if (now - lastDraw >= 200) {
    lastDraw = now;
    frame++;
    lcd.setCursor(0, 0);
    lcd.print("Next feed ");
    lcd.print(99 - (frame % 100));
    lcd.print("s ");
    lcd.setCursor(0, 1);
    lcd.print((frame & 4) ? "Motor ON " : "Motor OFF");
    analogWrite(MOTOR, (frame & 4) ? 180 : 0);
  }
}
