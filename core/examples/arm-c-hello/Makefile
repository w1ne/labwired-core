CC = arm-none-eabi-gcc
LD = arm-none-eabi-ld
OBJCOPY = arm-none-eabi-objcopy

CFLAGS = -mcpu=cortex-m0 -mthumb -g -O0 -ffreestanding -nostdlib
LDFLAGS = -T link.ld

SRCS = src/main.c
OBJS = $(SRCS:.c=.o)
TARGET = target/firmware

all: $(TARGET)

$(TARGET): $(OBJS)
	mkdir -p target
	$(CC) $(CFLAGS) $(LDFLAGS) $^ -o $@

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -rf target src/*.o

.PHONY: all clean
