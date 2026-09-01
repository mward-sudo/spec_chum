/* Spec Chum living-room headless renderer (SwiftUI embed). See docs/LIVING_ROOM.md */

#ifndef SPEC_CHUM_ROOM_H
#define SPEC_CHUM_ROOM_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Thread affinity: all sc_room_* calls for a given handle must be serialized on
 * one dedicated queue/thread. Prefer a background serial queue so Bevy does not
 * block AppKit (Spectrum input / 50 Hz host clock). Do not call the same handle
 * concurrently from multiple threads.
 *
 * Errors: sc_room_last_error returns a heap string owned by the caller —
 * free with sc_room_string_free (same contract as host_api sc_last_error).
 */

void *sc_room_create(uint32_t width, uint32_t height);
void sc_room_destroy(void *handle);
int sc_room_set_framebuffer(void *handle, const uint8_t *rgba, uint32_t len);
int sc_room_skip_intro(void *handle);
/** steps > 0 pull back; steps < 0 toward full-screen CRT. */
int sc_room_nudge_zoom(void *handle, int steps);
uint32_t sc_room_zoom_preset(void *handle);
/**
 * Set Bevy `Time` step for the next `sc_room_tick` (display delta seconds).
 * Clamped to a sane range. Call before each display-paced tick.
 */
void sc_room_set_frame_delta_seconds(void *handle, float dt_seconds);
int sc_room_tick(void *handle);
/** Recreate the offscreen (and present) target. Returns 0 on success. */
int sc_room_resize(void *handle, uint32_t width, uint32_t height);
/**
 * macOS: bind an IOSurface for zero-copy present (GPU blit each tick).
 * `iosurface` is CFTypeRef / IOSurfaceRef retained by the caller for the
 * room lifetime (or until the next set/resize/destroy). Pass NULL to clear.
 * width/height must match the surface. No-op / -1 on non-macOS.
 */
int sc_room_set_present_iosurface(
    void *handle,
    void *iosurface,
    uint32_t width,
    uint32_t height
);
uint32_t sc_room_width(void *handle);
uint32_t sc_room_height(void *handle);
/** CPU readback pointer (tests / fallback). Prefer IOSurface present on Mac. */
const uint8_t *sc_room_frame_ptr(void *handle);
uint32_t sc_room_frame_byte_len(void *handle);
/**
 * Rolling Bevy tick timings for diagnosis (`SPEC_CHUM_ROOM_PERF=1` also logs to stderr).
 * All times are microseconds. `thread_hint`: 0 unset, 1 AppKit main, 2 room queue.
 */
typedef struct ScRoomPerfSnapshot {
    uint64_t ticks;
    uint64_t last_tick_us;
    uint64_t avg_window_us;
    uint64_t max_window_us;
    uint64_t max_tick_us;
    uint32_t width;
    uint32_t height;
    uint32_t zoom_preset;
    uint8_t has_present;
    uint8_t thread_hint;
    uint8_t _pad[2];
} ScRoomPerfSnapshot;

/** Fill `out` (may be null → -1). Returns 0 on success. */
int sc_room_perf_snapshot(void *handle, ScRoomPerfSnapshot *out);
/**
 * Tag the next ticks for telemetry: 1 = AppKit main, 2 = dedicated room queue.
 * Call from the host before `sc_room_tick` so samples show which thread blocked.
 */
void sc_room_perf_set_thread_hint(uint32_t hint);

/*
 * Agent debug HTTP on the live sc_* session (#210). Requires SPEC_CHUM_AGENT_TOKEN
 * or SPEC_CHUM_AGENT_INSECURE=1 (same as egui SPEC_CHUM_AGENT=1). Pass the sc_create
 * handle — not a sc_room_* handle.
 */
int sc_agent_embed_start(void *sc_handle);
int sc_agent_embed_stop(void *sc_handle);

char *sc_room_last_error(void);
void sc_room_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* SPEC_CHUM_ROOM_H */
