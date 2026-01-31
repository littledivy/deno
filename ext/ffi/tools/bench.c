#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

// FFI targets

__attribute__((noinline)) void ffi_nop(void) { asm volatile(""); }
__attribute__((noinline)) uint32_t ffi_add_u32(uint32_t a, uint32_t b) { return a + b; }
__attribute__((noinline)) double ffi_return_f64(void) { return 3.14159265358979; }
__attribute__((noinline)) uint32_t ffi_hash(const void *buf, uint32_t len) {
  const uint8_t *p = buf;
  uint32_t h = 2166136261u;
  for (uint32_t i = 0; i < len; i++) { h ^= p[i]; h *= 16777619u; }
  return h;
}

// Mock data & helper stubs

static int32_t mock_i32[8] = {42, 7};
static uint8_t mock_buf[64] = "hello turbocall benchmark!";
static volatile uint32_t sink_u32;
static volatile double sink_f64;

int32_t impl_get_i32(void *a, int32_t i, uint8_t *e) { (void)a; (void)e; return mock_i32[i]; }
void *impl_get_buffer(void *a, int32_t i, uint8_t *e) { (void)a; (void)i; (void)e; return mock_buf; }
void impl_ret_u32(void *rv, uint32_t v) { (void)rv; sink_u32 = v; }
void impl_ret_f64(void *rv, double v) { (void)rv; sink_f64 = v; }

// Asm .data slots & trampoline externs

extern void *ffi_target_nop, *ffi_target_add_u32, *ffi_target_hash, *ffi_target_return_f64;
extern void *slow_get_i32, *slow_get_buffer, *slow_ret_u32, *slow_ret_f64;

extern uint8_t slow_trampoline_nop_void(void *, void *);
extern uint8_t slow_trampoline_add_u32(void *, void *);
extern uint8_t slow_trampoline_hash(void *, void *);
extern uint8_t slow_trampoline_return_f64(void *, void *);

static void setup(void) {
  ffi_target_nop = (void *)ffi_nop;
  ffi_target_add_u32 = (void *)ffi_add_u32;
  ffi_target_hash = (void *)ffi_hash;
  ffi_target_return_f64 = (void *)ffi_return_f64;
  slow_get_i32 = (void *)impl_get_i32;
  slow_get_buffer = (void *)impl_get_buffer;
  slow_ret_u32 = (void *)impl_ret_u32;
  slow_ret_f64 = (void *)impl_ret_f64;
}

// Bench

static uint64_t now_ns(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec * 1000000000ULL + ts.tv_nsec;
}

#define BENCH(label, expr, n) do { \
  for (int _w = 0; _w < 1000; _w++) { expr; } \
  uint64_t _t = now_ns(); \
  for (int64_t _i = 0; _i < (n); _i++) { expr; } \
  double _ns = (double)(now_ns() - _t) / (double)(n); \
  printf("    %-24s %8.2f ns/call\n", label, _ns); \
} while(0)

static uint64_t rv_slot;
static volatile uint8_t tok;

int main(int argc, char **argv) {
  int64_t n = 10000000;
  if (argc > 1) n = atoll(argv[1]);

  setup();

  printf("turbocall bench (%lld iters)\n\n", (long long)n);

  printf("  nop  () -> void\n");
  BENCH("direct", ffi_nop(), n);
  BENCH("trampoline", tok = slow_trampoline_nop_void(NULL, &rv_slot), n);

  printf("\n  add_u32  (u32, u32) -> u32\n");
  BENCH("direct", sink_u32 = ffi_add_u32(42, 7), n);
  BENCH("trampoline", tok = slow_trampoline_add_u32(NULL, &rv_slot), n);

  printf("\n  hash  (buf, u32) -> u32\n");
  BENCH("direct", sink_u32 = ffi_hash(mock_buf, 26), n);
  BENCH("trampoline", tok = slow_trampoline_hash(NULL, &rv_slot), n);

  printf("\n  return_f64  () -> f64\n");
  BENCH("direct", sink_f64 = ffi_return_f64(), n);
  BENCH("trampoline", tok = slow_trampoline_return_f64(NULL, &rv_slot), n);

  printf("\n");
}
