/* The WCH SDK's debug.c targets newlib and provides _write(). Picolibc's
 * printf writes through a `stdout` FILE instead; route it to that _write. */
#include <stdio.h>
int _write(int fd, char *buf, int size);
static int wch_putc(char c, FILE *f) { (void)f; _write(1, &c, 1); return (unsigned char)c; }
static FILE wch_stdout = FDEV_SETUP_STREAM(wch_putc, NULL, NULL, _FDEV_SETUP_WRITE);
FILE *const stdout = &wch_stdout;
