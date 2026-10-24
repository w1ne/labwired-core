/* Linux ARM EABI only. UAPI structures verified against pinned dev APK.
 * No libc/newlib, private sensor state, radio firmware or host RAM handshake. */
#include "phases.h"
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef signed long long s64;
struct kernel_termios { u32 iflag, oflag, cflag, lflag; u8 line, cc[19]; };
struct kernel_timespec { s64 sec, nsec; };
struct kernel_pollfd { int fd; short events, revents; };
struct i2c_msg { u16 addr, flags, len; u8 *buf; };
struct i2c_rdwr { struct i2c_msg *msgs; u32 nmsgs; };
_Static_assert(sizeof(int)==4 && sizeof(void*)==4, "ARM32");
_Static_assert(sizeof(struct kernel_termios)==36, "termios NCCS19");
_Static_assert(__builtin_offsetof(struct kernel_termios,cc)==17, "termios cc");
_Static_assert(sizeof(struct kernel_pollfd)==8, "pollfd");
_Static_assert(sizeof(struct kernel_timespec)==16, "signed time64");
_Static_assert(sizeof(struct i2c_msg)==12 && __builtin_offsetof(struct i2c_msg,buf)==8, "i2c_msg");
_Static_assert(sizeof(struct i2c_rdwr)==8, "i2c_rdwr");

static long call3(long n,long a,long b,long c) {
    register long r0 __asm__("r0")=a;
    register long r1 __asm__("r1")=b;
    register long r2 __asm__("r2")=c;
    register long r7 __asm__("r7")=n;
    __asm__ volatile("svc 0" : "+r"(r0) : "r"(r1),"r"(r2),"r"(r7) : "memory", "cc");
    return r0;
}
static unsigned length(const char *s) { unsigned n=0; while(s[n] && n<1024) ++n; return n; }
static void text(const char *s) {
    unsigned n=length(s),at=0;
    for(unsigned tries=0;at<n && tries<1024;++tries) {
        long r=call3(4,1,(long)(s+at),n-at);
        if(r>0 && (unsigned)r<=n-at) at+=(unsigned)r;
        else if(r!=-4 && r!=-11) call3(1,90,0,0);
    }
    if(at!=n) call3(1,91,0,0);
}
static void number(long value) {
    char b[16]; unsigned n=0; u32 v=(u32)value;
    if(value<0) { text("-"); v=0-v; }
    do { b[n++]=(char)('0'+v%10); v/=10; } while(v);
    while(n) { char c[2]={b[--n],0}; text(c); }
}
static void failed(const char *reason,long r) {
    text("LW_BASKET_FAILURE helper="); text(reason); text(" result="); number(r); text("\n");
    call3(1,1,0,0); for(;;) {}
}
static u32 now(void) {
    struct kernel_timespec t={0,0}; long r=call3(403,1,(long)&t,0);
    if(r || t.sec<0 || t.sec>3600 || t.nsec<0 || t.nsec>=1000000000) failed("clock_time64",r);
    return (u32)t.sec*1000+(u32)t.nsec/1000000;
}
static int remaining(u32 deadline) { int r=(int)(deadline-now()); return r>0?r:0; }
static void wait_ms(unsigned ms) {
    u32 deadline=now()+ms;
    for(unsigned tries=0;remaining(deadline)>0 && tries<1024;++tries) {
        long r=call3(168,0,0,remaining(deadline));
        if(r && r!=-4) failed("phase_poll",r);
    }
    if(remaining(deadline)) failed("phase_wait_bound",-1);
}
static int wait_fd(int fd,short events,u32 deadline) {
    for(unsigned tries=0;tries<1024;++tries) {
        int ms=remaining(deadline); if(!ms) return 0;
        struct kernel_pollfd p={fd,events,0}; long r=call3(168,(long)&p,1,ms);
        if(r==-4) continue;
        if(r<0 || (p.revents & (8|16|32))) failed("uart_poll",r);
        if(r>0 && (p.revents & events)) return 1;
        if(!r) return 0;
    }
    failed("poll_retry_bound",-1); return 0;
}
static void io_result(const char *kind,long r,unsigned count) {
    text("LW_BASKET_IO kind="); text(kind); text(" ret="); number(r);
    text(" count="); number(count); text(" errno="); number(r<0?-r:0); text("\n");
}
static long transaction(int fd,struct i2c_msg *m,unsigned count) {
    struct i2c_rdwr request={m,count}; long r=call3(54,fd,0x0707,(long)&request);
    io_result("i2c_rdwr",r,count); return r;
}
static void configure(int fd) {
    for(unsigned i=0;i<CONFIG_COUNT;++i) {
        u8 b[2]={configuration[i][0],configuration[i][1]};
        struct i2c_msg m={I2C_ADDRESS,0,2,b};
        long r=transaction(fd,&m,1); if(r!=1) failed("i2c_config_message_count",r);
    }
}
static long read_register(int fd,u8 reg,u8 *out,u16 count) {
    struct i2c_msg m[2]={{I2C_ADDRESS,0,1,&reg},{I2C_ADDRESS,1,count,out}};
    return transaction(fd,m,2);
}
static void sensor(int fd,unsigned phase) {
    u8 devid=0,axes[6]={0}; long r=read_register(fd,0,&devid,1);
    text("LW_BASKET_DEVID value="); number(devid); text("\n");
    if(r!=2 || devid!=DEVID) failed("i2c_devid",r);
    r=read_register(fd,AXIS_REGISTER,axes,6); if(r!=2) failed("i2c_axis_message_count",r);
    text("LW_BASKET_AXES raw=");
    for(unsigned i=0;i<3;++i) {
        int v=(short)((u16)axes[i*2] | ((u16)axes[i*2+1]<<8));
        number(v); text(i==2?"\n":",");
        if(v!=phases[phase].raw[i]) failed("i2c_axis_value",v);
    }
}
static void raw_uart(int fd) {
    struct kernel_termios t; long r=call3(54,fd,0x5401,(long)&t);
    io_result("tcgets",r,0); if(r) failed("tcgets",r);
    t.iflag=0; t.oflag=0; t.lflag=0; t.cflag=0x1000|0x800|0x80|0x30|0x2; /* B115200, CLOCAL, CREAD, CS8 */
    t.line=0; for(unsigned i=0;i<19;++i) t.cc[i]=0;
    r=call3(54,fd,0x5402,(long)&t); io_result("tcsets",r,0); if(r) failed("tcsets",r);
}
static void escaped(const char *s,unsigned n) {
    for(unsigned i=0;i<n;++i) {
        char b[2]={s[i],0};
        if(s[i]=='\r') text("\\r"); else if(s[i]=='\n') text("\\n");
        else if(s[i]>=32 && s[i]<=126) text(b); else failed("uart_nonascii",s[i]);
    }
}
static void uart(int fd,const char *command,const char *expected,int off) {
    char output[128],input[128]; unsigned n=length(command),written=0,received=0;
    if(n>120) failed("uart_command_bound",n);
    for(unsigned i=0;i<n;++i) output[i]=command[i];
    output[n++]='\r'; output[n++]='\n';
    /* Discard old input through the real Linux tty, then require a new exact reply. */
    long r=call3(54,fd,0x540b,0); if(r) failed("uart_flush",r);
    u32 deadline=now()+UART_DEADLINE_MS;
    for(unsigned tries=0;written<n && tries<1024;++tries) {
        if(!wait_fd(fd,4,deadline)) failed("uart_write_deadline",written);
        r=call3(4,fd,(long)(output+written),n-written);
        if(r==-4 || r==-11) continue;
        if(r<=0 || (unsigned)r>n-written) failed("uart_write",r);
        written+=(unsigned)r;
    }
    if(written!=n) failed("uart_write_bound",written);
    for(unsigned tries=0;tries<2048;++tries) {
        if(!wait_fd(fd,1,deadline)) break;
        if(received==sizeof(input)) failed("uart_reply_bound",received);
        r=call3(3,fd,(long)(input+received),sizeof(input)-received);
        if(r==-4 || r==-11) continue;
        if(r<=0 || (unsigned)r>sizeof(input)-received) failed("uart_read",r);
        received+=(unsigned)r;
    }
    if(remaining(deadline)) failed("uart_read_retry_bound",received);
    text("LW_BASKET_UART command="); text(command); text(" written="); number(written);
    text(" received="); number(received); text(" errno=0 reply="); escaped(input,received); text("\n");
    if(off) { if(received) failed("unpowered_uart_responded",received); return; }
    unsigned wanted=length(expected); if(received!=wanted) failed("uart_reply_length",received);
    for(unsigned i=0;i<wanted;++i) if(input[i]!=expected[i]) failed("uart_reply_bytes",i);
}
__attribute__((used)) int basket_main(void) {
    int i2c=call3(5,(long)"/dev/i2c-3",2,0); if(i2c<0) failed("open_i2c",i2c);
    int serial=call3(5,(long)"/dev/ttyS4",2|0x100|0x800,0); if(serial<0) failed("open_uart",serial);
    raw_uart(serial);
    for(unsigned i=0;i<PHASE_COUNT;++i) {
        text("LW_BASKET_PHASE_READY seq="); number(i+1); text(" id="); text(phases[i].id); text("\n");
        wait_ms(READY_WAIT_MS);
        if(phases[i].powered) {
            configure(i2c); sensor(i2c,i);
            for(unsigned j=0;j<UART_EXCHANGE_COUNT;++j) uart(serial,exchanges[j].command,exchanges[j].reply,0);
        } else {
            u8 devid=0; long r=read_register(i2c,0,&devid,1);
            if(r!=-OFF_ERRNO) failed("unpowered_i2c_errno",r);
            uart(serial,exchanges[0].command,"",1);
        }
        text("LW_BASKET_PHASE_DONE seq="); number(i+1); text(" id="); text(phases[i].id); text("\n");
    }
    long a=call3(6,i2c,0,0),b=call3(6,serial,0,0); if(a || b) failed("close",a?a:b);
    text("LW_BASKET_HELPER_ACCEPTED\n"); return 0;
}
__attribute__((naked,noreturn)) void _start(void) {
    __asm__ volatile("bl basket_main\nmov r7,#1\nsvc 0\nb .");
}
