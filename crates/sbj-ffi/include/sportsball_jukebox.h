/*
 * Sportsball Jukebox engine C API.
 *
 * Keep in sync with crates/sbj-ffi/src/lib.rs.
 *
 * Conventions:
 *  - Functions returning SbjStatus report failures there; the reason is
 *    available from sbj_last_error_message() on the same thread.
 *  - All functions tolerate NULL handles (SBJ_STATUS_NULL_POINTER).
 *  - A session may be used from any thread. Sounds are immutable and may
 *    be shared across threads.
 *  - Strings are NUL-terminated UTF-8.
 */
#ifndef SPORTSBALL_JUKEBOX_H
#define SPORTSBALL_JUKEBOX_H

#include <stdbool.h>
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
    SBJ_STATUS_NO_OUTPUT = 5,
    SBJ_STATUS_DEVICE_ERROR = 6,
    SBJ_STATUS_MEDIA_ERROR = 7,
    SBJ_STATUS_TRUNCATED = 8,
} SbjStatus;

typedef enum SbjOutputHealth {
    SBJ_OUTPUT_RUNNING = 0,
    SBJ_OUTPUT_DEVICE_LOST = 1, /* unplugged: reopen on another device */
    SBJ_OUTPUT_FAILED = 2,
} SbjOutputHealth;

#define SBJ_ID_LEN 512
#define SBJ_NAME_LEN 256

typedef struct SbjDeviceInfo {
    char id[SBJ_ID_LEN];     /* persistent; pass to sbj_session_open_output */
    char name[SBJ_NAME_LEN]; /* for display */
    uint16_t output_channels;
    uint32_t sample_rate;
    bool is_default;
} SbjDeviceInfo;

typedef struct SbjOutputInfo {
    bool is_open;
    char device_id[SBJ_ID_LEN];
    uint32_t sample_rate;
    uint16_t channels;
    SbjOutputHealth health;
    uint64_t xruns; /* device-reported glitches since opening */
} SbjOutputInfo;

typedef struct SbjCueState {
    bool assigned;
    bool playing;
    bool looping;
    float gain;
    double position_seconds;
    double duration_seconds;
} SbjCueState;

typedef struct SbjSession SbjSession;
typedef struct SbjSound SbjSound;

/* ---- General ---------------------------------------------------------- */

const char *sbj_version(void);

/* Copies the last error message on this thread into buf (NUL-terminated
 * when len > 0). Returns the full message length, excluding the NUL. */
size_t sbj_last_error_message(char *buf, size_t len);

/* True if this header's struct layouts match the library's. Call at
 * startup: sbj_abi_check(sizeof(SbjDeviceInfo), sizeof(SbjOutputInfo),
 * sizeof(SbjCueState)). */
bool sbj_abi_check(size_t device_info_size, size_t output_info_size, size_t cue_state_size);

/* ---- Sounds ----------------------------------------------------------- */

/* Decodes a file fully into memory. SLOW: call off the main thread.
 * target_sample_rate: the open output's rate (0 if none) so resampling
 * happens here too. Returns NULL on failure. Free with sbj_sound_free. */
SbjSound *sbj_sound_load(const char *path, uint32_t target_sample_rate,
                         SbjStatus *status_out);
void sbj_sound_free(SbjSound *sound);
double sbj_sound_duration_seconds(const SbjSound *sound);
uint16_t sbj_sound_channels(const SbjSound *sound);

/* ---- Session ---------------------------------------------------------- */

/* cue_count: 1..1024. Returns NULL on invalid arguments. */
SbjSession *sbj_session_create(uint32_t cue_count);
void sbj_session_destroy(SbjSession *session);

/* Housekeeping; call on the UI refresh timer. */
SbjStatus sbj_session_poll(const SbjSession *session);

SbjStatus sbj_session_backend_name(const SbjSession *session, char *buf, size_t len);

/* Re-scans devices; then read each with sbj_session_device_info. */
uint32_t sbj_session_device_count(const SbjSession *session);
SbjStatus sbj_session_device_info(const SbjSession *session, uint32_t index,
                                  SbjDeviceInfo *out);

/* device_id NULL = system default. Replaces any open output. */
SbjStatus sbj_session_open_output(const SbjSession *session, const char *device_id);
SbjStatus sbj_session_close_output(const SbjSession *session);
SbjStatus sbj_session_output_info(const SbjSession *session, SbjOutputInfo *out);

/* ---- Cues ------------------------------------------------------------- */

/* The session keeps its own reference; the caller still frees `sound`. */
SbjStatus sbj_session_set_cue_sound(const SbjSession *session, uint32_t cue,
                                    const SbjSound *sound);
SbjStatus sbj_session_clear_cue(const SbjSession *session, uint32_t cue);

/* Plays from the start (restarts if playing). Empty cue: no-op. */
SbjStatus sbj_session_fire_cue(const SbjSession *session, uint32_t cue);
SbjStatus sbj_session_stop_cue(const SbjSession *session, uint32_t cue, double fade_seconds);
SbjStatus sbj_session_stop_all(const SbjSession *session, double fade_seconds);

/* Applies on the next fire if the cue is playing. */
SbjStatus sbj_session_set_cue_loop(const SbjSession *session, uint32_t cue, bool looping);
SbjStatus sbj_session_set_cue_gain(const SbjSession *session, uint32_t cue, float gain);
SbjStatus sbj_session_set_master_gain(const SbjSession *session, float gain);

SbjStatus sbj_session_cue_state(const SbjSession *session, uint32_t cue, SbjCueState *out);

#ifdef __cplusplus
}
#endif

#endif /* SPORTSBALL_JUKEBOX_H */
