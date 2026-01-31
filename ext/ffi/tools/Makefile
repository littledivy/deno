ARCH := $(shell uname -m)
ifeq ($(ARCH),arm64)
  ARCH_NAME := aarch64
else ifeq ($(ARCH),aarch64)
  ARCH_NAME := aarch64
else
  ARCH_NAME := $(ARCH)
endif

ASM   ?= trampoline_$(ARCH_NAME).s
CC    ?= clang
ITERS ?= 10000000

.PHONY: bench disasm clean

bench: bench_runner
	./bench_runner $(ITERS)

bench_runner: bench.c $(ASM)
	$(CC) -O2 -o $@ bench.c -x assembler $(ASM)

disasm: bench_runner
	objdump -d $<

clean:
	rm -f bench_runner
