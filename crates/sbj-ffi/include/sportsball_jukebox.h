/*
 * Sportsball Jukebox engine C API.
 *
 * Keep in sync with crates/sbj-ffi/src/lib.rs. Positions are in sample
 * frames at the engine's sample rate. Audio buffers are interleaved float32.
 *
 * Threading: a handle may be used from one thread at a time. In Phase 0 the
 * caller pulls audio with sbj_engine_render; from Phase 1 the engine owns
 * its Core Audio stream and render is only used for offline work.
 */
#ifndef SPORTSBALL_JUKEBOX_H
#define SPORTSBALL_JUKEBOX_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum SbjStatus {
    SBJ_STATUS_OK = 0,
    SBJ_STATUS_NULL_POINTER = 1,
    SBJ_STATUS_INVALID_ARGUMENT = 2,
    SBJ_STATUS_QUEUE_FULL = 3,
    SBJ_STATUS_INTERNAL_ERROR = 4,
} SbjStatus;

typedef struct SbjEngine SbjEngine;

/* Library version, static string. */
const char *sbj_version(void);

/* Returns NULL on invalid arguments. Free with sbj_engine_destroy. */
SbjEngine *sbj_engine_create(uint32_t sample_rate, uint16_t channels);
void sbj_engine_destroy(SbjEngine *engine);

/* Temporary: loads a timeline with one sine-wave clip. */
SbjStatus sbj_engine_load_test_tone(SbjEngine *engine, float frequency_hz, float seconds);

SbjStatus sbj_engine_play(SbjEngine *engine);
SbjStatus sbj_engine_pause(SbjEngine *engine);
SbjStatus sbj_engine_stop(SbjEngine *engine);
SbjStatus sbj_engine_seek(SbjEngine *engine, uint64_t position);
uint64_t sbj_engine_position(const SbjEngine *engine);

/* `out` must hold frames * channels floats. */
SbjStatus sbj_engine_render(SbjEngine *engine, float *out, size_t frames);

#ifdef __cplusplus
}
#endif

#endif /* SPORTSBALL_JUKEBOX_H */
