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

const float *sc_audio_ptr(void *handle);
unsigned int sc_audio_frames(void *handle);
unsigned int sc_audio_sample_rate(void *handle);

int sc_set_key(void *handle, unsigned int row, unsigned int bit, int pressed);
int sc_clear_keys(void *handle);

/* Heap strings — free with sc_string_free */
char *sc_status(void *handle);
char *sc_last_error(void);
void sc_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* SPEC_CHUM_HOST_H */
