#ifndef SPEC_CHUM_HOST_H
#define SPEC_CHUM_HOST_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Model IDs — keep in sync with host_api::ModelId */
enum {
    SC_MODEL_48K = 0,
    SC_MODEL_128K = 1,
    SC_MODEL_PLUS3 = 2
};

void *sc_create(unsigned int model, int with_border);
void sc_destroy(void *handle);

int sc_set_model(void *handle, unsigned int model);
int sc_load_rom(void *handle, const char *path);
int sc_load_rom_bytes(void *handle, const uint8_t *data, size_t len);
int sc_reset(void *handle);

void sc_set_running(void *handle, int running);
void sc_set_border(void *handle, int with_border);
void sc_run_frame(void *handle);

const uint8_t *sc_framebuffer_ptr(void *handle);
unsigned int sc_framebuffer_width(void *handle);
unsigned int sc_framebuffer_height(void *handle);

int sc_open_tape(void *handle, const char *path);
int sc_tape_play(void *handle);
int sc_tape_pause(void *handle);
int sc_tape_rewind(void *handle);
int sc_tape_playing(void *handle);
int sc_has_tape(void *handle);
int sc_tape_progress(void *handle,
                     unsigned int *block_index,
                     unsigned int *block_count,
                     unsigned int *pulse_index,
                     unsigned int *pulse_count);
/* Instant flash-load (1) vs EAR-only (0); speed multiplier 1..64 */
int sc_tape_get_load_options(void *handle, int *flash_load, unsigned int *speed);
int sc_tape_set_load_options(void *handle, int flash_load, unsigned int speed);

const float *sc_audio_ptr(void *handle);
unsigned int sc_audio_frames(void *handle);
unsigned int sc_audio_sample_rate(void *handle);

int sc_set_key(void *handle, unsigned int row, unsigned int bit, int pressed);
int sc_clear_keys(void *handle);

/* Heap strings — free with sc_string_free */
char *sc_status(void *handle);
char *sc_last_error(void);
void sc_string_free(char *s);

/* Debug / observability (see docs/DEBUGGING.md) */
/* cats: bitmask — cpu=1 bus=2 tape=4 ula=8 machine=16 ay=32 disk=64 mem=128; 0 disables */
void sc_debug_init_from_env(void);
void sc_debug_set_categories(unsigned int cats);
unsigned int sc_debug_get_categories(void);
void sc_debug_clear(void);
/* Heap UTF-8 dump; free with sc_string_free. */
char *sc_debug_dump(void);
int sc_debug_dump_to_file(const char *path);
unsigned int sc_debug_event_count(void);

int sc_peek(void *handle, unsigned int addr, uint8_t *out);
int sc_poke(void *handle, unsigned int addr, uint8_t value);
/* Heap UTF-8 JSON of Inspect; free with sc_string_free. */
char *sc_inspect_json(void *handle);
/* Fill pc,sp,af,bc,de,hl,ix,iy (8 uint16). Returns 0 on success, -1 on error. */
int sc_regs(void *handle, unsigned short *pc, unsigned short *sp, unsigned short *af, unsigned short *bc, unsigned short *de, unsigned short *hl, unsigned short *ix, unsigned short *iy);
int sc_step(void *handle); /* one step_once; 0 ok, -1 no machine */
void sc_set_paused(void *handle, int paused);
int sc_add_breakpoint(void *handle, unsigned int pc);
/* Returns break reason: 0 none, 1 pc, 2 mem, 3 port, 4 halt, 5 budget, -1 error */
int sc_run_until_break(void *handle, unsigned int max_insns);
/* Heap UTF-8 JSON of the trace ring; free with sc_string_free. */
char *sc_debug_dump_json(void);

#ifdef __cplusplus
}
#endif

#endif /* SPEC_CHUM_HOST_H */
