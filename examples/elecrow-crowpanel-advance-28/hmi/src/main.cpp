// Elecrow CrowPanel Advance 2.8 HMI — self-contained ON/OFF lamp example.
// Uses the same ST7789 pinout as Elecrow's own V1.2 LovyanGFX example.
// This is an original firmware adaptation for a browser-controlled demo,
// not Elecrow's full LVGL application or a touch-controller model.
#include <Arduino.h>
#include <SPI.h>

namespace {
constexpr int SCK_PIN=42, MOSI_PIN=39, CS_PIN=40, DC_PIN=41, BL_PIN=38;
constexpr int ON_TEST_PIN=9, OFF_TEST_PIN=10, LAMP_OUT_PIN=18;
constexpr uint16_t WHITE=0xFFFF, BLACK=0x0000, CHARCOAL=0x18E3,
  GRAY=0x632C, RED=0xD860, GREEN=0x1C85, YELLOW=0xFFE0;
SPIClass lcd(FSPI);
bool lamp=false;
void cmd(uint8_t c,const uint8_t* args=nullptr,size_t count=0){
  digitalWrite(CS_PIN,LOW);digitalWrite(DC_PIN,LOW);lcd.transfer(c);
  if(count){digitalWrite(DC_PIN,HIGH);lcd.writeBytes(args,count);}
  digitalWrite(CS_PIN,HIGH);
}
void fill(uint16_t x,uint16_t y,uint16_t w,uint16_t h,uint16_t color){
  if(!w||!h)return;
  const uint8_t cols[]={uint8_t(x>>8),uint8_t(x),uint8_t((x+w-1)>>8),uint8_t(x+w-1)};
  const uint8_t rows[]={uint8_t(y>>8),uint8_t(y),uint8_t((y+h-1)>>8),uint8_t(y+h-1)};
  cmd(0x2A,cols,4);cmd(0x2B,rows,4);cmd(0x2C);
  uint8_t px[128];for(size_t i=0;i<sizeof(px);i+=2){px[i]=uint8_t(color>>8);px[i+1]=uint8_t(color);}
  digitalWrite(DC_PIN,HIGH);digitalWrite(CS_PIN,LOW);
  for(uint16_t row=0;row<h;row++)for(uint16_t col=0;col<w;col+=64){
    uint16_t n=(w-col<64)?w-col:64;lcd.writeBytes(px,n*2);
  }
  digitalWrite(CS_PIN,HIGH);
}
// Minimal glyph data used for the HMI labels; no graphics dependency.
const uint8_t* glyph(char c){
  static const uint8_t O[]={0x3e,0x41,0x41,0x41,0x3e};
  static const uint8_t N[]={0x7f,0x02,0x0c,0x10,0x7f};
  static const uint8_t F[]={0x7f,0x09,0x09,0x09,0x01};
  static const uint8_t L[]={0x7f,0x40,0x40,0x40,0x40};
  static const uint8_t A[]={0x7e,0x11,0x11,0x11,0x7e};
  static const uint8_t M[]={0x7f,0x02,0x0c,0x02,0x7f};
  static const uint8_t P[]={0x7f,0x09,0x09,0x09,0x06};
  switch(c){case 'O':return O;case 'N':return N;case 'F':return F;
    case 'L':return L;case 'A':return A;case 'M':return M;case 'P':return P;
    default:return nullptr;}
}
void label(const char* txt,int x,int y,int scale,uint16_t color){
  while(*txt){const uint8_t* g=glyph(*txt++);
    if(g)for(int col=0;col<5;col++)for(int row=0;row<7;row++)if(g[col]&(1<<row))
      fill(x+col*scale,y+row*scale,scale,scale,color);
    x+=scale*7;
  }
}
void bulb(bool on){
  fill(34,65,113,131,WHITE); // erase previous lamp, partial region
  // Horizontal colored segments approximate a lamp glass envelope.
  const uint16_t c=on?YELLOW:GRAY;
  for(int y=-38;y<=38;y++){
    int n=0;while(n*n+y*y<38*38)n++;
    fill(uint16_t(90-n),uint16_t(116+y),uint16_t(2*n),1,c);
  }
  fill(80,150,20,8,c);fill(77,158,26,7,CHARCOAL);fill(80,167,20,5,CHARCOAL);
}
void drawFrame(){
  fill(0,0,320,35,CHARCOAL);
  fill(5,43,153,173,WHITE);
  fill(177,55,132,65,RED);
  fill(177,132,132,65,GRAY);
  label("LAMP",9,11,2,WHITE);
  label("ON",222,71,5,WHITE);
  label("OFF",198,145,5,WHITE);
  bulb(false);
}
void setLamp(bool on){
  if(on==lamp)return;
  lamp=on;digitalWrite(LAMP_OUT_PIN,on?HIGH:LOW);
  bulb(on);
  Serial.println(on?"LAMP_ON":"LAMP_OFF");
}
}
void setup(){
  Serial.begin(115200);
  pinMode(LAMP_OUT_PIN,OUTPUT);digitalWrite(LAMP_OUT_PIN,LOW);
  pinMode(ON_TEST_PIN,INPUT_PULLUP);pinMode(OFF_TEST_PIN,INPUT_PULLUP);
  pinMode(CS_PIN,OUTPUT);pinMode(DC_PIN,OUTPUT);pinMode(BL_PIN,OUTPUT);
  digitalWrite(CS_PIN,HIGH);digitalWrite(DC_PIN,HIGH);digitalWrite(BL_PIN,HIGH);
  lcd.begin(SCK_PIN,-1,MOSI_PIN,CS_PIN);
  lcd.beginTransaction(SPISettings(20000000,MSBFIRST,SPI_MODE0));
  cmd(0x01);delay(10);cmd(0x11);delay(10);
  const uint8_t colmod=0x55,madctl=0x20;
  cmd(0x3A,&colmod,1);cmd(0x36,&madctl,1);cmd(0x21);cmd(0x29);
  drawFrame();
  Serial.println("CROWPANEL_HMI_READY");
}
void loop(){
  static bool lastOn=false,lastOff=false;
  bool on=digitalRead(ON_TEST_PIN)==LOW,off=digitalRead(OFF_TEST_PIN)==LOW;
  if(on&&!lastOn)setLamp(true);
  if(off&&!lastOff)setLamp(false);
  lastOn=on;lastOff=off;
  delay(25);
}
